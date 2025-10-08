// renderer/renderer.rs
use crate::asset::Assets;
use crate::renderer::batch::InstanceData;
use crate::renderer::internal::{
    CameraBuffer, DynamicObjectsBuffer, LightsBuffer, OrderedBatch, PipelineKey, PreparedBatches,
    RenderContext, RenderPipeline, ShadowResources, TextureBindingModel,
};
use crate::renderer::{
    postprocess::PostProcess, CameraUniform, LightsData, RenderBatcher, RenderPass, Vertex,
};
use crate::scene::Camera;
use crate::settings::RenderSettings;

use glam::Vec3;
use std::cmp::Ordering;
use winit::{dpi::PhysicalSize, window::Window};

const INITIAL_OBJECTS_CAPACITY: u32 = 1024;

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
}

impl Renderer {
    pub async fn new(window: &Window, settings: RenderSettings) -> Self {
        let size = window.inner_size();
        let context = RenderContext::new(window, size, &settings).await;
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
            settings.sample_count,
        );
        let postprocess = PostProcess::new(
            &context.device,
            &context.queue,
            &context.config,
            settings.sample_count,
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
        }
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

    fn prepare_batches(&self, batcher: &RenderBatcher) -> PreparedBatches {
        let mut opaque = Vec::new();
        let mut transparent = Vec::new();
        let mut overlay = Vec::new();
        let camera_pos = self.camera_position;

        for batch in batcher.iter() {
            let mut instances: Vec<InstanceData> = batch.instances.to_vec();

            if matches!(batch.pass, RenderPass::Transparent | RenderPass::Overlay) {
                instances.sort_by(|a, b| {
                    let da = (a.transform.translation - camera_pos).length_squared();
                    let db = (b.transform.translation - camera_pos).length_squared();
                    db.partial_cmp(&da).unwrap_or(Ordering::Equal)
                });
            }

            let alpha_blend = matches!(batch.pass, RenderPass::Transparent | RenderPass::Overlay)
                || instances
                    .iter()
                    .any(|inst| inst.material.requires_separate_pass());

            let ordered = OrderedBatch {
                mesh: batch.mesh,
                pass: batch.pass,
                depth_state: batch.depth_state,
                instances,
                alpha_blend,
                first_instance: 0,
            };

            match ordered.pass {
                RenderPass::Opaque => opaque.push(ordered),
                RenderPass::Transparent => transparent.push(ordered),
                RenderPass::Overlay => overlay.push(ordered),
            }
        }

        transparent.sort_by(|a, b| {
            let da = a
                .instances
                .iter()
                .map(|inst| (inst.transform.translation - camera_pos).length_squared())
                .fold(0.0, f32::max);
            let db = b
                .instances
                .iter()
                .map(|inst| (inst.transform.translation - camera_pos).length_squared())
                .fold(0.0, f32::max);
            db.partial_cmp(&da).unwrap_or(Ordering::Equal)
        });

        overlay.sort_by(|a, b| {
            let da = a
                .instances
                .iter()
                .map(|inst| (inst.transform.translation - camera_pos).length_squared())
                .fold(0.0, f32::max);
            let db = b
                .instances
                .iter()
                .map(|inst| (inst.transform.translation - camera_pos).length_squared())
                .fold(0.0, f32::max);
            db.partial_cmp(&da).unwrap_or(Ordering::Equal)
        });

        let mut batches = Vec::with_capacity(opaque.len() + transparent.len() + overlay.len());
        let opaque_start = 0;
        batches.extend(opaque);
        let opaque_end = batches.len();
        let transparent_start = batches.len();
        batches.extend(transparent);
        let transparent_end = batches.len();
        let overlay_start = batches.len();
        batches.extend(overlay);
        let overlay_end = batches.len();

        let mut offset = 0u32;
        for batch in &mut batches {
            batch.first_instance = offset;
            offset += batch.instances.len() as u32;
        }

        PreparedBatches {
            batches,
            opaque_range: opaque_start..opaque_end,
            transparent_range: transparent_start..transparent_end,
            overlay_range: overlay_start..overlay_end,
        }
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
    ) -> Result<(), wgpu::SurfaceError> {
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

        let prepared_batches = self.prepare_batches(batcher);

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

        let depth_view = &self.context.depth.view;
        let sampled_depth_view = &self.context.depth.sampled_view;

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("DepthPrepass"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth_view,
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

                let Some(mesh) = assets.meshes.get(batch.mesh) else {
                    log::warn!("Skipping batch with invalid mesh handle");
                    continue;
                };

                pass.set_vertex_buffer(0, mesh.vertex_buffer().slice(..));
                pass.set_index_buffer(mesh.index_buffer().slice(..), mesh.index_format());

                let instance_count = batch.instances.len() as u32;
                pass.draw_indexed(
                    0..mesh.index_count(),
                    0,
                    batch.first_instance..(batch.first_instance + instance_count),
                );
            }
        }

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
                    view: depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            if let Some(bindless_group) = self.texture_binder.global_bind_group() {
                for batch in prepared_batches
                    .opaque()
                    .iter()
                    .chain(prepared_batches.transparent().iter())
                {
                    let Some(mesh) = assets.meshes.get(batch.mesh) else {
                        log::warn!("Skipping batch with invalid mesh handle");
                        continue;
                    };

                    let pipeline_key = PipelineKey::new(
                        batch.depth_state.depth_test,
                        batch.depth_state.depth_write,
                        batch.alpha_blend,
                        self.context.sample_count,
                    );
                    let pipeline = self.pipeline.pipeline(pipeline_key);
                    rpass.set_pipeline(pipeline);
                    rpass.set_bind_group(0, &self.camera_buffer.bind_group, &[]);
                    rpass.set_bind_group(1, &self.objects_buffer.bind_group, &[]);
                    rpass.set_bind_group(2, &self.lights_buffer.bind_group, &[]);
                    rpass.set_bind_group(3, bindless_group, &[]);

                    let instance_count = batch.instances.len() as u32;
                    rpass.set_vertex_buffer(0, mesh.vertex_buffer().slice(..));
                    rpass.set_index_buffer(mesh.index_buffer().slice(..), mesh.index_format());
                    rpass.draw_indexed(
                        0..mesh.index_count(),
                        0,
                        batch.first_instance..(batch.first_instance + instance_count),
                    );
                }
            } else {
                for batch in prepared_batches
                    .opaque()
                    .iter()
                    .chain(prepared_batches.transparent().iter())
                {
                    let Some(mesh) = assets.meshes.get(batch.mesh) else {
                        log::warn!("Skipping batch with invalid mesh handle");
                        continue;
                    };

                    let pipeline_key = PipelineKey::new(
                        batch.depth_state.depth_test,
                        batch.depth_state.depth_write,
                        batch.alpha_blend,
                        self.context.sample_count,
                    );
                    let pipeline = self.pipeline.pipeline(pipeline_key);
                    rpass.set_pipeline(pipeline);
                    rpass.set_bind_group(0, &self.camera_buffer.bind_group, &[]);
                    rpass.set_bind_group(1, &self.objects_buffer.bind_group, &[]);
                    rpass.set_bind_group(2, &self.lights_buffer.bind_group, &[]);

                    let instances = &batch.instances;
                    rpass.set_vertex_buffer(0, mesh.vertex_buffer().slice(..));
                    rpass.set_index_buffer(mesh.index_buffer().slice(..), mesh.index_format());

                    let mut local_offset = 0usize;
                    while local_offset < instances.len() {
                        let material = instances[local_offset].material;
                        let Some(bind_group) = self.texture_binder.bind_group_for_material(
                            &self.context.device,
                            assets,
                            material,
                        ) else {
                            local_offset += 1;
                            continue;
                        };
                        rpass.set_bind_group(3, bind_group, &[]);

                        let mut run_length = 1usize;
                        while local_offset + run_length < instances.len()
                            && instances[local_offset + run_length].material == material
                        {
                            run_length += 1;
                        }

                        let start_instance = batch.first_instance + local_offset as u32;
                        let end_instance = start_instance + run_length as u32;
                        rpass.draw_indexed(0..mesh.index_count(), 0, start_instance..end_instance);

                        local_offset += run_length;
                    }
                }
            }
        }

        self.postprocess.execute(
            &mut encoder,
            &self.context.device,
            sampled_depth_view,
            &view,
        );

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

            if let Some(bindless_group) = self.texture_binder.global_bind_group() {
                for batch in prepared_batches.overlay() {
                    let Some(mesh) = assets.meshes.get(batch.mesh) else {
                        log::warn!("Skipping batch with invalid mesh handle");
                        continue;
                    };

                    let pipeline_key = PipelineKey::new(
                        batch.depth_state.depth_test,
                        batch.depth_state.depth_write,
                        batch.alpha_blend,
                        1,
                    );
                    let pipeline = self.pipeline.pipeline(pipeline_key);
                    rpass.set_pipeline(pipeline);
                    rpass.set_bind_group(0, &self.camera_buffer.bind_group, &[]);
                    rpass.set_bind_group(1, &self.objects_buffer.bind_group, &[]);
                    rpass.set_bind_group(2, &self.lights_buffer.bind_group, &[]);
                    rpass.set_bind_group(3, bindless_group, &[]);

                    let instance_count = batch.instances.len() as u32;
                    rpass.set_vertex_buffer(0, mesh.vertex_buffer().slice(..));
                    rpass.set_index_buffer(mesh.index_buffer().slice(..), mesh.index_format());
                    rpass.draw_indexed(
                        0..mesh.index_count(),
                        0,
                        batch.first_instance..(batch.first_instance + instance_count),
                    );
                }
            } else {
                for batch in prepared_batches.overlay() {
                    let Some(mesh) = assets.meshes.get(batch.mesh) else {
                        log::warn!("Skipping batch with invalid mesh handle");
                        continue;
                    };

                    let pipeline_key = PipelineKey::new(
                        batch.depth_state.depth_test,
                        batch.depth_state.depth_write,
                        batch.alpha_blend,
                        1,
                    );
                    let pipeline = self.pipeline.pipeline(pipeline_key);
                    rpass.set_pipeline(pipeline);
                    rpass.set_bind_group(0, &self.camera_buffer.bind_group, &[]);
                    rpass.set_bind_group(1, &self.objects_buffer.bind_group, &[]);
                    rpass.set_bind_group(2, &self.lights_buffer.bind_group, &[]);

                    let instances = &batch.instances;
                    rpass.set_vertex_buffer(0, mesh.vertex_buffer().slice(..));
                    rpass.set_index_buffer(mesh.index_buffer().slice(..), mesh.index_format());

                    let mut local_offset = 0usize;
                    while local_offset < instances.len() {
                        let material = instances[local_offset].material;
                        let Some(bind_group) = self.texture_binder.bind_group_for_material(
                            &self.context.device,
                            assets,
                            material,
                        ) else {
                            local_offset += 1;
                            continue;
                        };
                        rpass.set_bind_group(3, bind_group, &[]);

                        let mut run_length = 1usize;
                        while local_offset + run_length < instances.len()
                            && instances[local_offset + run_length].material == material
                        {
                            run_length += 1;
                        }

                        let start_instance = batch.first_instance + local_offset as u32;
                        let end_instance = start_instance + run_length as u32;
                        rpass.draw_indexed(0..mesh.index_count(), 0, start_instance..end_instance);

                        local_offset += run_length;
                    }
                }
            }
        }

        self.context.queue.submit(Some(encoder.finish()));
        frame.present();
        Ok(())
    }
}
