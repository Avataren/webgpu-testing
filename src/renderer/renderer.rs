// renderer/renderer.rs
use crate::asset::{Assets, Mesh};
use crate::renderer::internal::{
    CameraBuffer, DynamicObjectsBuffer, LightsBuffer, OrderedBatch, PipelineKey, PreparedBatches,
    RenderContext, RenderPipeline, ShadowResources, TextureBindingModel,
};
use crate::renderer::{
    lights::{MAX_DIRECTIONAL_LIGHTS, MAX_POINT_LIGHTS, MAX_SPOT_LIGHTS},
    postprocess::PostProcess,
    CameraUniform, LightsData, RenderBatcher, RenderPass, Vertex,
};
use crate::scene::Camera;
use crate::settings::RenderSettings;

use glam::Vec3;
use winit::{dpi::PhysicalSize, window::Window};

const INITIAL_OBJECTS_CAPACITY: u32 = 1024 * 10;
const POINT_SHADOW_FACE_COUNT: u32 = 6;

#[cfg(feature = "egui")]
type UiHook =
    Box<dyn FnOnce(&wgpu::Device, &wgpu::Queue, &mut wgpu::CommandEncoder, &wgpu::TextureView)>;
pub struct RenderFrame {
    pub frame: wgpu::SurfaceTexture,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RendererStats {
    pub batch_count: u32,
    pub instance_count: u32,
    pub depth_prepass_draw_calls: u32,
    pub opaque_draw_calls: u32,
    pub transparent_draw_calls: u32,
    pub overlay_draw_calls: u32,
    pub shadow_draw_calls: u32,
}

impl RendererStats {
    pub fn total_draw_calls(&self) -> u32 {
        self.depth_prepass_draw_calls
            + self.opaque_draw_calls
            + self.transparent_draw_calls
            + self.overlay_draw_calls
            + self.shadow_draw_calls
    }
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
    stats: RendererStats,
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
            stats: RendererStats::default(),
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

        let mut frame_stats = RendererStats::default();
        frame_stats.batch_count = prepared_batches.all().len() as u32;
        frame_stats.instance_count = prepared_batches
            .all()
            .iter()
            .map(|batch| batch.instances.len() as u32)
            .sum();

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
                frame_stats.depth_prepass_draw_calls += 1;
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

            frame_stats.opaque_draw_calls +=
                self.record_batches(&mut rpass, assets, prepared_batches.opaque());
            frame_stats.transparent_draw_calls +=
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

            frame_stats.overlay_draw_calls +=
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

        frame_stats.shadow_draw_calls = estimate_shadow_draw_calls(prepared_batches.all(), lights);

        self.stats = frame_stats;

        self.context.queue.submit(Some(encoder.finish()));
        Ok(RenderFrame { frame })
    }

    // Add helper method to get surface format
    pub fn surface_format(&self) -> wgpu::TextureFormat {
        self.context.config.format
    }

    pub fn surface_size(&self) -> PhysicalSize<u32> {
        self.context.size
    }

    pub fn sample_count(&self) -> u32 {
        self.context.sample_count
    }

    pub fn last_frame_stats(&self) -> RendererStats {
        self.stats
    }

    fn record_batches(
        &mut self,
        rpass: &mut wgpu::RenderPass<'_>,
        assets: &Assets,
        batches: &[OrderedBatch],
    ) -> u32 {
        if batches.is_empty() {
            return 0;
        }

        let mut draw_calls = 0u32;

        if let Some(bindless_group) = self.texture_binder.global_bind_group() {
            for batch in batches {
                let Some(mesh) = self.setup_batch_state(rpass, assets, batch) else {
                    continue;
                };
                rpass.set_bind_group(3, bindless_group, &[]);
                self.draw_full_batch(rpass, mesh, batch);
                draw_calls += 1;
            }
        } else {
            for batch in batches {
                let Some(mesh) = self.setup_batch_state(rpass, assets, batch) else {
                    continue;
                };
                draw_calls += self.draw_classic_batch(rpass, assets, mesh, batch) as u32;
            }
        }
        draw_calls
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
    ) -> usize {
        self.set_geometry_buffers(pass, mesh);
        let mut start_instance = batch.first_instance;
        let mut draw_calls = 0usize;

        for run in &batch.material_runs {
            let end_instance = start_instance + run.count;
            let Some(bind_group) = self.texture_binder.bind_group_for_material(
                &self.context.device,
                assets,
                run.material,
            ) else {
                start_instance = end_instance;
                continue;
            };

            pass.set_bind_group(3, bind_group, &[]);
            pass.draw_indexed(0..mesh.index_count(), 0, start_instance..end_instance);

            draw_calls += 1;
            start_instance = end_instance;
        }

        draw_calls
    }

    fn set_geometry_buffers(&self, pass: &mut wgpu::RenderPass<'_>, mesh: &Mesh) {
        pass.set_vertex_buffer(0, mesh.vertex_buffer().slice(..));
        pass.set_index_buffer(mesh.index_buffer().slice(..), mesh.index_format());
    }
}

fn estimate_shadow_draw_calls(batches: &[OrderedBatch], lights: &LightsData) -> u32 {
    if batches.is_empty() {
        return 0;
    }

    let per_pass_draws: u32 = batches.iter().map(count_shadow_draws_for_batch).sum();

    if per_pass_draws == 0 {
        return 0;
    }

    let directional_passes = lights
        .directional_shadows()
        .iter()
        .take(MAX_DIRECTIONAL_LIGHTS)
        .filter(|shadow| shadow.params[0] != 0.0)
        .count() as u32;

    let spot_passes = lights
        .spot_shadows()
        .iter()
        .take(MAX_SPOT_LIGHTS)
        .filter(|shadow| shadow.params[0] != 0.0)
        .count() as u32;

    let point_passes = lights
        .point_shadows()
        .iter()
        .take(MAX_POINT_LIGHTS)
        .filter(|shadow| shadow.params[0] != 0.0)
        .count() as u32
        * POINT_SHADOW_FACE_COUNT;

    let total_passes = directional_passes + spot_passes + point_passes;
    per_pass_draws * total_passes
}

fn count_shadow_draws_for_batch(batch: &OrderedBatch) -> u32 {
    if matches!(batch.pass, RenderPass::Transparent | RenderPass::Overlay) {
        return 0;
    }

    if batch.lit_instance_count == 0 {
        0
    } else {
        1
    }
}

fn mesh_for_batch<'a>(assets: &'a Assets, batch: &OrderedBatch) -> Option<&'a Mesh> {
    let mesh = assets.meshes.get(batch.mesh);
    if mesh.is_none() {
        log::warn!("Skipping batch with invalid mesh handle");
    }
    mesh
}
