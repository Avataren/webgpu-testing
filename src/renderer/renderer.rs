// renderer/renderer.rs
use crate::asset::{Assets, Mesh};
use crate::renderer::batch::InstanceData;
use crate::renderer::internal::{
    CameraBuffer, DynamicObjectsBuffer, LightsBuffer, OrderedBatch, PipelineKey, PreparedBatches,
    RenderContext, RenderPipeline, ShadowResources, TextureBindingModel,
};
use crate::renderer::{postprocess::PostProcess, CameraUniform, LightsData, RenderBatcher, Vertex};
use crate::scene::Camera;
use crate::settings::RenderSettings;

use glam::Vec3;
use winit::{dpi::PhysicalSize, window::Window};

const INITIAL_OBJECTS_CAPACITY: u32 = 1024 * 10;

#[cfg(feature = "egui")]
type UiHook =
    Box<dyn FnOnce(&wgpu::Device, &wgpu::Queue, &mut wgpu::CommandEncoder, &wgpu::TextureView)>;
pub struct RenderFrame {
    pub frame: wgpu::SurfaceTexture,
}

pub struct Renderer {
    context: RenderContext,
    pipeline: RenderPipeline,
    texture_binder: TextureBindingModel,
    objects_buffer: DynamicObjectsBuffer,
    camera_buffer: CameraBuffer,
    lights_buffer: LightsBuffer,
    shadows: ShadowResources,
    postprocess: PostProcess,
    camera_position: Vec3,
    camera_target: Vec3,
    camera_up: Vec3,
    settings: RenderSettings,
    #[cfg(feature = "egui")]
    ui_hook: Option<UiHook>,
}

impl Renderer {
    pub async fn new(window: &Window, mut settings: RenderSettings) -> Self {
        let size = window.inner_size();
        let context = RenderContext::new(window, size, &settings).await;
        let sample_count = context.sample_count;
        settings.sample_count = sample_count;
        let camera_buffer = CameraBuffer::new(&context.device);
        let objects_buffer = DynamicObjectsBuffer::new(&context.device, INITIAL_OBJECTS_CAPACITY);
        let shadows =
            ShadowResources::new(&context.device, &objects_buffer, settings.shadow_map_size);
        let lights_buffer = LightsBuffer::new(&context.device, &shadows);
        let (pipeline, texture_binder) = RenderPipeline::new(
            &context,
            &camera_buffer,
            &objects_buffer,
            &lights_buffer,
            sample_count,
        );
        let postprocess = PostProcess::new(
            &context.device,
            &context.queue,
            &context.config,
            sample_count,
        );

        Self {
            context,
            pipeline,
            texture_binder,
            objects_buffer,
            camera_buffer,
            lights_buffer,
            shadows,
            postprocess,
            camera_position: Vec3::ZERO,
            camera_target: Vec3::ZERO,
            camera_up: Vec3::Y,
            settings,
            #[cfg(feature = "egui")]
            ui_hook: None,
        }
    }

    // Setter to install the per-frame hook (only compiled with egui feature)
    #[cfg(feature = "egui")]
    pub fn set_ui_hook(&mut self, hook: UiHook) {
        self.ui_hook = Some(hook);
    }

    pub fn get_device(&self) -> &wgpu::Device {
        &self.context.device
    }

    pub fn get_queue(&self) -> &wgpu::Queue {
        &self.context.queue
    }

    pub fn settings(&self) -> &RenderSettings {
        &self.settings
    }

    pub fn resize(&mut self, new_size: PhysicalSize<u32>) {
        self.context.resize(new_size);
        self.postprocess.resize(
            &self.context.device,
            &self.context.queue,
            self.context.config.width,
            self.context.config.height,
            self.context.config.format,
        );
    }

    pub fn aspect_ratio(&self) -> f32 {
        self.context.config.width as f32 / self.context.config.height.max(1) as f32
    }

    pub fn set_camera(&mut self, camera: &Camera, aspect: f32) {
        self.camera_position = camera.position(); // Store it
        self.camera_target = camera.target;
        self.camera_up = camera.up;
        let vp = camera.view_proj(aspect);
        let uni = CameraUniform::from_matrix(vp, camera.position());
        self.context
            .queue
            .write_buffer(&self.camera_buffer.buffer, 0, bytemuck::bytes_of(&uni));
        let proj = camera.proj(aspect);
        self.postprocess
            .update_camera(&self.context.queue, proj, camera.near, camera.far);
    }

    pub fn camera_position(&self) -> Vec3 {
        self.camera_position
    }

    pub fn camera_target(&self) -> Vec3 {
        self.camera_target
    }

    pub fn camera_up(&self) -> Vec3 {
        self.camera_up
    }

    pub fn set_lights(&mut self, lights: &LightsData) {
        self.lights_buffer.update(&self.context.queue, lights);
    }

    pub fn create_mesh(&self, vertices: &[Vertex], indices: &[u32]) -> crate::asset::Mesh {
        crate::asset::Mesh::from_vertices(&self.context.device, vertices, indices)
    }

    pub fn update_texture_bind_group(&mut self, assets: &Assets) {
        self.texture_binder.update(&self.context.device, assets);
    }

    pub fn render(
        &mut self,
        assets: &Assets,
        batcher: &RenderBatcher,
        lights: &LightsData,
    ) -> Result<RenderFrame, wgpu::SurfaceError> {
        let frame = self.context.surface.get_current_texture()?;
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder =
            self.context
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Encoder"),
                });

        let prepared_batches = PreparedBatches::from_batcher(batcher, self.camera_position);

        self.objects_buffer
            .update(&self.context, prepared_batches.all())?;
        self.lights_buffer.update(&self.context.queue, lights);

        self.shadows.render(
            &self.context,
            &mut encoder,
            assets,
            prepared_batches.all(),
            lights,
            &self.objects_buffer,
        );

        let (scene_view, resolve_target) = self.postprocess.scene_color_views();
        let depth_view = self.context.depth.view.clone();
        let sampled_depth_view = self.context.depth.sampled_view.clone();

        // Depth-only prepass
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("DepthPrepass"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            pass.set_pipeline(self.pipeline.depth_prepass());
            pass.set_bind_group(0, &self.camera_buffer.bind_group, &[]);
            pass.set_bind_group(1, &self.objects_buffer.bind_group, &[]);

            for batch in prepared_batches.opaque() {
                if batch.alpha_blend
                    || !batch.depth_state.depth_write
                    || !batch.depth_state.depth_test
                {
                    continue;
                }
                let Some(mesh) = mesh_for_batch(assets, batch) else {
                    continue;
                };
                self.draw_full_batch(&mut pass, mesh, batch);
            }
        }

        // Main color pass (to postprocess scene target)
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("MainPass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: scene_view,
                    depth_slice: None,
                    resolve_target,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.231,
                            g: 0.269,
                            b: 0.338,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            self.record_batches(&mut rpass, assets, prepared_batches.opaque());
            self.record_batches(&mut rpass, assets, prepared_batches.transparent());
        }

        // Resolve scene → swapchain
        self.postprocess.execute(
            &mut encoder,
            &self.context.device,
            &sampled_depth_view,
            &view,
        );

        // Overlay pass (your overlays draw after UI if you keep it here;
        // if you want UI on top of overlays, move this block above ui_hook).
        if !prepared_batches.overlay().is_empty() {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("OverlayPass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            self.record_batches(&mut rpass, assets, prepared_batches.overlay());
        }

        // --- EGUI (optional) ---
        #[cfg(feature = "egui")]
        if let Some(hook) = self.ui_hook.take() {
            // The hook will create a render pass on `view`,
            // call `forget_lifetime()`, and render egui.
            hook(
                &self.context.device,
                &self.context.queue,
                &mut encoder,
                &view,
            );
        }

        self.context.queue.submit(Some(encoder.finish()));
        Ok(RenderFrame { frame })
    }

    // Add helper method to get surface format
    pub fn surface_format(&self) -> wgpu::TextureFormat {
        self.context.config.format
    }

    fn record_batches(
        &mut self,
        rpass: &mut wgpu::RenderPass<'_>,
        assets: &Assets,
        batches: &[OrderedBatch],
    ) {
        if batches.is_empty() {
            return;
        }

        if let Some(bindless_group) = self.texture_binder.global_bind_group() {
            for batch in batches {
                let Some(mesh) = self.setup_batch_state(rpass, assets, batch) else {
                    continue;
                };
                rpass.set_bind_group(3, bindless_group, &[]);
                self.draw_full_batch(rpass, mesh, batch);
            }
        } else {
            for batch in batches {
                let Some(mesh) = self.setup_batch_state(rpass, assets, batch) else {
                    continue;
                };
                self.draw_classic_batch(rpass, assets, mesh, batch);
            }
        }
    }

    fn setup_batch_state<'a>(
        &self,
        rpass: &mut wgpu::RenderPass<'_>,
        assets: &'a Assets,
        batch: &OrderedBatch,
    ) -> Option<&'a Mesh> {
        let mesh = mesh_for_batch(assets, batch)?;
        let pipeline_key = PipelineKey::new(
            batch.depth_state.depth_test,
            batch.depth_state.depth_write,
            batch.alpha_blend,
            batch.pass.color_sample_count(self.context.sample_count),
        );
        let pipeline = self.pipeline.pipeline(pipeline_key);
        rpass.set_pipeline(pipeline);
        rpass.set_bind_group(0, &self.camera_buffer.bind_group, &[]);
        rpass.set_bind_group(1, &self.objects_buffer.bind_group, &[]);
        rpass.set_bind_group(2, &self.lights_buffer.bind_group, &[]);
        Some(mesh)
    }

    fn draw_full_batch(&self, pass: &mut wgpu::RenderPass<'_>, mesh: &Mesh, batch: &OrderedBatch) {
        self.set_geometry_buffers(pass, mesh);
        let instance_count = batch.instances.len() as u32;
        pass.draw_indexed(
            0..mesh.index_count(),
            0,
            batch.first_instance..(batch.first_instance + instance_count),
        );
    }

    fn draw_classic_batch(
        &mut self,
        pass: &mut wgpu::RenderPass<'_>,
        assets: &Assets,
        mesh: &Mesh,
        batch: &OrderedBatch,
    ) {
        self.set_geometry_buffers(pass, mesh);

        let instances = &batch.instances;
        let mut local_offset = 0usize;

        while local_offset < instances.len() {
            let material = instances[local_offset].material;
            let Some(bind_group) =
                self.texture_binder
                    .bind_group_for_material(&self.context.device, assets, material)
            else {
                local_offset += 1;
                continue;
            };

            let run_length = material_run_length(instances, local_offset);
            let start_instance = batch.first_instance + local_offset as u32;
            let end_instance = start_instance + run_length as u32;

            pass.set_bind_group(3, bind_group, &[]);
            pass.draw_indexed(0..mesh.index_count(), 0, start_instance..end_instance);

            local_offset += run_length;
        }
    }

    fn set_geometry_buffers(&self, pass: &mut wgpu::RenderPass<'_>, mesh: &Mesh) {
        pass.set_vertex_buffer(0, mesh.vertex_buffer().slice(..));
        pass.set_index_buffer(mesh.index_buffer().slice(..), mesh.index_format());
    }
}

fn material_run_length(instances: &[InstanceData], start: usize) -> usize {
    let material = instances[start].material;
    let mut length = 1usize;
    while start + length < instances.len() && instances[start + length].material == material {
        length += 1;
    }
    length
}

fn mesh_for_batch<'a>(assets: &'a Assets, batch: &OrderedBatch) -> Option<&'a Mesh> {
    let mesh = assets.meshes.get(batch.mesh);
    if mesh.is_none() {
        log::warn!("Skipping batch with invalid mesh handle");
    }
    mesh
}
