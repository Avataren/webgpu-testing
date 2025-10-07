// renderer/renderer.rs
use crate::asset::{Assets, Handle, Mesh};
use crate::renderer::batch::InstanceData;
use crate::renderer::lights::{
    LightsUniform, ShadowsUniform, MAX_DIRECTIONAL_LIGHTS, MAX_POINT_LIGHTS, MAX_SPOT_LIGHTS,
};
use crate::renderer::material::MaterialFlags;
use crate::renderer::{
    postprocess::PostProcess, CameraUniform, Depth, LightsData, Material, RenderBatcher,
    RenderPass, Vertex,
};
use crate::scene::components::DepthState;
use crate::scene::Camera;
use crate::settings::RenderSettings;

use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use std::{cmp::Ordering, collections::HashMap, mem, num::NonZeroU64};
use wgpu::util::DeviceExt;
use winit::{dpi::PhysicalSize, window::Window};

const INITIAL_OBJECTS_CAPACITY: u32 = 1024;
const POINT_SHADOW_FACE_COUNT: usize = 6;
const POINT_SHADOW_LAYERS: u32 = (MAX_POINT_LIGHTS * POINT_SHADOW_FACE_COUNT) as u32;

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

struct RenderContext {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    size: PhysicalSize<u32>,
    depth: Depth,
    supports_bindless_textures: bool,
    sample_count: u32,
}

struct RenderPipeline {
    pipelines: HashMap<PipelineKey, wgpu::RenderPipeline>,
    depth_prepass: wgpu::RenderPipeline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct PipelineKey {
    depth_test: bool,
    depth_write: bool,
    alpha_blend: bool,
}

impl PipelineKey {
    fn new(depth_test: bool, depth_write: bool, alpha_blend: bool) -> Self {
        Self {
            depth_test,
            depth_write,
            alpha_blend,
        }
    }
}

#[derive(Debug, Clone)]
struct OrderedBatch {
    mesh: Handle<Mesh>,
    pass: RenderPass,
    depth_state: DepthState,
    instances: Vec<InstanceData>,
    alpha_blend: bool,
    first_instance: u32,
}

struct PreparedBatches {
    batches: Vec<OrderedBatch>,
    opaque_range: std::ops::Range<usize>,
    transparent_range: std::ops::Range<usize>,
    overlay_range: std::ops::Range<usize>,
}

impl PreparedBatches {
    fn all(&self) -> &[OrderedBatch] {
        &self.batches
    }

    fn opaque(&self) -> &[OrderedBatch] {
        &self.batches[self.opaque_range.clone()]
    }

    fn transparent(&self) -> &[OrderedBatch] {
        &self.batches[self.transparent_range.clone()]
    }

    fn overlay(&self) -> &[OrderedBatch] {
        &self.batches[self.overlay_range.clone()]
    }
}

struct DynamicObjectsBuffer {
    buffer: wgpu::Buffer,
    capacity: u32,
    bind_group: wgpu::BindGroup,
    bind_layout: wgpu::BindGroupLayout,
    scratch: Vec<crate::renderer::ObjectData>,
}

struct CameraBuffer {
    buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    bind_layout: wgpu::BindGroupLayout,
}

struct LightsBuffer {
    buffer: wgpu::Buffer,
    shadow_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    bind_layout: wgpu::BindGroupLayout,
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

            match &mut self.texture_binder {
                TextureBindingModel::Bindless(bindless) => {
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
                        );
                        let pipeline = self.pipeline.pipeline(pipeline_key);
                        rpass.set_pipeline(pipeline);
                        rpass.set_bind_group(0, &self.camera_buffer.bind_group, &[]);
                        rpass.set_bind_group(1, &self.objects_buffer.bind_group, &[]);
                        rpass.set_bind_group(2, &self.lights_buffer.bind_group, &[]);
                        rpass.set_bind_group(3, bindless.global_bind_group(), &[]);

                        let instance_count = batch.instances.len() as u32;
                        rpass.set_vertex_buffer(0, mesh.vertex_buffer().slice(..));
                        rpass.set_index_buffer(mesh.index_buffer().slice(..), mesh.index_format());
                        rpass.draw_indexed(
                            0..mesh.index_count(),
                            0,
                            batch.first_instance..(batch.first_instance + instance_count),
                        );
                    }
                }
                TextureBindingModel::Classic(classic) => {
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
                            let bind_group = classic.bind_group_for_material(
                                &self.context.device,
                                assets,
                                material,
                            );
                            rpass.set_bind_group(3, bind_group, &[]);

                            let mut run_length = 1usize;
                            while local_offset + run_length < instances.len()
                                && instances[local_offset + run_length].material == material
                            {
                                run_length += 1;
                            }

                            let start_instance = batch.first_instance + local_offset as u32;
                            let end_instance = start_instance + run_length as u32;
                            rpass.draw_indexed(
                                0..mesh.index_count(),
                                0,
                                start_instance..end_instance,
                            );

                            local_offset += run_length;
                        }
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
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            match &mut self.texture_binder {
                TextureBindingModel::Bindless(bindless) => {
                    for batch in prepared_batches.overlay() {
                        let Some(mesh) = assets.meshes.get(batch.mesh) else {
                            log::warn!("Skipping batch with invalid mesh handle");
                            continue;
                        };

                        let pipeline_key = PipelineKey::new(
                            batch.depth_state.depth_test,
                            batch.depth_state.depth_write,
                            batch.alpha_blend,
                        );
                        let pipeline = self.pipeline.pipeline(pipeline_key);
                        rpass.set_pipeline(pipeline);
                        rpass.set_bind_group(0, &self.camera_buffer.bind_group, &[]);
                        rpass.set_bind_group(1, &self.objects_buffer.bind_group, &[]);
                        rpass.set_bind_group(2, &self.lights_buffer.bind_group, &[]);
                        rpass.set_bind_group(3, bindless.global_bind_group(), &[]);

                        let instance_count = batch.instances.len() as u32;
                        rpass.set_vertex_buffer(0, mesh.vertex_buffer().slice(..));
                        rpass.set_index_buffer(mesh.index_buffer().slice(..), mesh.index_format());
                        rpass.draw_indexed(
                            0..mesh.index_count(),
                            0,
                            batch.first_instance..(batch.first_instance + instance_count),
                        );
                    }
                }
                TextureBindingModel::Classic(classic) => {
                    for batch in prepared_batches.overlay() {
                        let Some(mesh) = assets.meshes.get(batch.mesh) else {
                            log::warn!("Skipping batch with invalid mesh handle");
                            continue;
                        };

                        let pipeline_key = PipelineKey::new(
                            batch.depth_state.depth_test,
                            batch.depth_state.depth_write,
                            batch.alpha_blend,
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
                            let bind_group = classic.bind_group_for_material(
                                &self.context.device,
                                assets,
                                material,
                            );
                            rpass.set_bind_group(3, bind_group, &[]);

                            let mut run_length = 1usize;
                            while local_offset + run_length < instances.len()
                                && instances[local_offset + run_length].material == material
                            {
                                run_length += 1;
                            }

                            let start_instance = batch.first_instance + local_offset as u32;
                            let end_instance = start_instance + run_length as u32;
                            rpass.draw_indexed(
                                0..mesh.index_count(),
                                0,
                                start_instance..end_instance,
                            );

                            local_offset += run_length;
                        }
                    }
                }
            }
        }

        self.context.queue.submit(Some(encoder.finish()));
        frame.present();
        Ok(())
    }
}

impl RenderContext {
    async fn new(window: &Window, size: PhysicalSize<u32>, settings: &RenderSettings) -> Self {
        #[cfg(target_arch = "wasm32")]
        {
            log::info!("Checking WebGPU/WebGL availability...");
        }

        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            #[cfg(target_arch = "wasm32")]
            backends: wgpu::Backends::BROWSER_WEBGPU | wgpu::Backends::GL,
            #[cfg(not(target_arch = "wasm32"))]
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        log::info!("Instance created, creating surface...");

        // SAFETY: The window is owned by the App struct and lives for the entire
        // duration of the program. The surface is also owned by the App (via Renderer).
        // The window is guaranteed to outlive the surface because the Renderer is
        // dropped before the window when the App is destroyed.
        let surface = unsafe {
            instance
                .create_surface_unsafe(
                    wgpu::SurfaceTargetUnsafe::from_window(window)
                        .expect("Failed to create surface target"),
                )
                .expect("Failed to create surface")
        };

        log::info!("Surface created successfully!");

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .expect("Failed to find adapter");

        log::info!("Using adapter: {:?}", adapter.get_info());
        log::info!("Using backend: {:?}", adapter.get_info().backend);
        let adapter_features = adapter.features();
        log::info!("Adapter features: {:?}", adapter_features);

        // FORCE TRADITIONAL PATH FOR TESTING
        // Set to true to force traditional, false to allow bindless (when available)
        let force_traditional = false;

        let mut required_features = wgpu::Features::empty();
        let supports_bindless_textures = if force_traditional {
            log::warn!("Bindless textures DISABLED (forced for testing)");
            false
        } else if adapter_features
            .contains(wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING)
        {
            required_features |=
                wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING
                    | wgpu::Features::TEXTURE_BINDING_ARRAY;
            log::info!("Bindless textures enabled");
            true
        } else {
            log::warn!("Bindless textures not supported");
            false
        };

        if adapter_features.contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES) {
            required_features |= wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES;
        }

        if adapter_features.contains(wgpu::Features::FLOAT32_FILTERABLE) {
            required_features |= wgpu::Features::FLOAT32_FILTERABLE;
        }
        // Only set special limits if using bindless
        let mut limits = if supports_bindless_textures {
            wgpu::Limits {
                max_binding_array_elements_per_shader_stage: 256,
                ..wgpu::Limits::default()
            }
        } else {
            wgpu::Limits::default()
        };

        // The renderer now fits all lighting and shadow resources into a single
        // bind group, so the default limit of 4 is sufficient. We still request
        // at least 4 bind groups explicitly to ensure compatibility on adapters
        // that expose a lower default.
        limits.max_bind_groups = limits.max_bind_groups.max(4);

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("Device"),
                required_features,
                required_limits: limits,
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
            })
            .await
            .expect("Failed to create device");

        let surface_caps = surface.get_capabilities(&adapter);

        let format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb()) // Prefer LINEAR format
            .unwrap_or_else(|| {
                surface_caps
                    .formats
                    .iter()
                    .copied()
                    .find(|f| f.is_srgb())
                    .unwrap_or(surface_caps.formats[0])
            });

        // let format = surface_caps
        //     .formats
        //     .iter()
        //     .copied()
        //     .find(|f| f.is_srgb())
        //     .unwrap_or(surface_caps.formats[0]);

        let present_mode = settings.present_mode(&surface_caps.present_modes);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let depth = Depth::new(&device, size, settings.sample_count);

        Self {
            surface,
            device,
            queue,
            config,
            size,
            depth,
            supports_bindless_textures,
            sample_count: settings.sample_count,
        }
    }

    fn resize(&mut self, new_size: PhysicalSize<u32>) {
        if new_size.width == 0 || new_size.height == 0 {
            return;
        }
        self.size = new_size;
        self.config.width = new_size.width;
        self.config.height = new_size.height;
        self.surface.configure(&self.device, &self.config);
        self.depth = Depth::new(&self.device, new_size, self.sample_count);
    }
}

impl CameraBuffer {
    fn new(device: &wgpu::Device) -> Self {
        let camera = CameraUniform::new();
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("CameraBuffer"),
            contents: bytemuck::bytes_of(&camera),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("CameraBindLayout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT, // <-- Add FRAGMENT here
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: Some(
                        NonZeroU64::new(mem::size_of::<CameraUniform>() as u64).unwrap(),
                    ),
                },
                count: None,
            }],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("CameraBindGroup"),
            layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });

        Self {
            buffer,
            bind_group,
            bind_layout,
        }
    }
}

impl LightsBuffer {
    fn new(device: &wgpu::Device, shadows: &ShadowResources) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("LightsBindLayout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: Some(
                            NonZeroU64::new(mem::size_of::<LightsUniform>() as u64).unwrap(),
                        ),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: Some(
                            NonZeroU64::new(mem::size_of::<ShadowsUniform>() as u64).unwrap(),
                        ),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
            ],
        });

        let initial = LightsUniform::zeroed();
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("LightsBuffer"),
            contents: bytemuck::bytes_of(&initial),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

        let shadow_initial = ShadowsUniform::zeroed();
        let shadow_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ShadowUniformBuffer"),
            contents: bytemuck::bytes_of(&shadow_initial),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("LightsBindGroup"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: shadow_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(shadows.directional_array_view()),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(shadows.sampler()),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(shadows.spot_array_view()),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(shadows.sampler()),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(shadows.point_array_view()),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::Sampler(shadows.sampler()),
                },
            ],
        });

        Self {
            buffer,
            shadow_buffer,
            bind_group,
            bind_layout: layout,
        }
    }

    fn update(&mut self, queue: &wgpu::Queue, lights: &LightsData) {
        let data = LightsUniform::from_data(lights);
        queue.write_buffer(&self.buffer, 0, bytemuck::bytes_of(&data));
        let shadow_data = ShadowsUniform::from_data(lights);

        queue.write_buffer(&self.shadow_buffer, 0, bytemuck::bytes_of(&shadow_data));
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ShadowViewUniform {
    view_proj: [[f32; 4]; 4],
}

struct ShadowArray {
    _texture: wgpu::Texture,
    array_view: wgpu::TextureView,
    layer_views: Vec<wgpu::TextureView>,
}

impl ShadowArray {
    fn new(device: &wgpu::Device, label: &str, layers: u32, size: u32) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: layers.max(1),
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });

        let array_view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some(&format!("{label}ArrayView")),
            format: Some(wgpu::TextureFormat::Depth32Float),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            aspect: wgpu::TextureAspect::All,
            base_mip_level: 0,
            mip_level_count: None,
            base_array_layer: 0,
            array_layer_count: Some(layers.max(1)),
            ..Default::default()
        });

        let mut layer_views = Vec::with_capacity(layers.max(1) as usize);
        for layer in 0..layers.max(1) {
            layer_views.push(texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some(&format!("{label}Layer{layer}")),
                format: Some(wgpu::TextureFormat::Depth32Float),
                dimension: Some(wgpu::TextureViewDimension::D2),
                aspect: wgpu::TextureAspect::All,
                base_mip_level: 0,
                mip_level_count: None,
                base_array_layer: layer,
                array_layer_count: Some(1),
                ..Default::default()
            }));
        }

        Self {
            _texture: texture,
            array_view,
            layer_views,
        }
    }

    fn layer_view(&self, index: usize) -> &wgpu::TextureView {
        let clamped = index.min(self.layer_views.len().saturating_sub(1));
        if clamped != index {
            log::warn!(
                "Shadow layer index {} clamped to {} (max: {})",
                index,
                clamped,
                self.layer_views.len() - 1
            );
        }
        &self.layer_views[clamped]
    }

    fn array_view(&self) -> &wgpu::TextureView {
        &self.array_view
    }
}

struct ShadowResources {
    directional: ShadowArray,
    spot: ShadowArray,
    point: ShadowArray,
    sampler: wgpu::Sampler,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    _uniform_layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    staging_buffer: wgpu::Buffer,
}

impl ShadowResources {
    fn new(device: &wgpu::Device, objects: &DynamicObjectsBuffer, shadow_map_size: u32) -> Self {
        let directional = ShadowArray::new(
            device,
            "DirectionalShadowMap",
            MAX_DIRECTIONAL_LIGHTS as u32,
            shadow_map_size,
        );
        let spot = ShadowArray::new(
            device,
            "SpotShadowMap",
            MAX_SPOT_LIGHTS as u32,
            shadow_map_size,
        );
        let point = ShadowArray::new(
            device,
            "PointShadowMap",
            POINT_SHADOW_LAYERS,
            shadow_map_size,
        );

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ShadowSampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            lod_min_clamp: 0.0,
            lod_max_clamp: 1.0,
            ..Default::default()
        });

        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ShadowUniformLayout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: Some(
                        NonZeroU64::new(mem::size_of::<ShadowViewUniform>() as u64).unwrap(),
                    ),
                },
                count: None,
            }],
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ShadowUniformBuffer"),
            size: mem::size_of::<ShadowViewUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Create staging buffer that can hold matrices for all shadow types
        let max_shadows = (MAX_DIRECTIONAL_LIGHTS
            + MAX_SPOT_LIGHTS
            + MAX_POINT_LIGHTS * POINT_SHADOW_FACE_COUNT) as u64;
        let staging_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ShadowStagingBuffer"),
            size: mem::size_of::<ShadowViewUniform>() as u64 * max_shadows,
            usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        log::info!("Created staging buffer:");
        log::info!("  Max shadows: {}", max_shadows);
        log::info!(
            "  Uniform size: {} bytes",
            mem::size_of::<ShadowViewUniform>()
        );
        log::info!(
            "  Total buffer size: {} bytes",
            mem::size_of::<ShadowViewUniform>() as u64 * max_shadows
        );
        log::info!(
            "  Breakdown: {} dir + {} spot + {} point faces",
            MAX_DIRECTIONAL_LIGHTS,
            MAX_SPOT_LIGHTS,
            MAX_POINT_LIGHTS * POINT_SHADOW_FACE_COUNT
        );

        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ShadowUniformBindGroup"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ShadowShader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shader/shadow.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ShadowPipelineLayout"),
            bind_group_layouts: &[&uniform_layout, &objects.bind_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ShadowPipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Vertex::layout()],
                compilation_options: Default::default(),
            },
            fragment: None,
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: Some(wgpu::Face::Back),
                front_face: wgpu::FrontFace::Ccw,
                polygon_mode: wgpu::PolygonMode::Fill,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState {
                    constant: 0,
                    slope_scale: 0.0,
                    clamp: 0.0,
                },
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        Self {
            directional,
            spot,
            point,
            sampler,
            uniform_buffer,
            uniform_bind_group,
            _uniform_layout: uniform_layout,
            pipeline,
            staging_buffer,
        }
    }

    fn directional_array_view(&self) -> &wgpu::TextureView {
        self.directional.array_view()
    }

    fn spot_array_view(&self) -> &wgpu::TextureView {
        self.spot.array_view()
    }

    fn point_array_view(&self) -> &wgpu::TextureView {
        self.point.array_view()
    }

    fn sampler(&self) -> &wgpu::Sampler {
        &self.sampler
    }

    fn render(
        &mut self,
        context: &RenderContext,
        encoder: &mut wgpu::CommandEncoder,
        assets: &Assets,
        batches: &[OrderedBatch],
        lights: &LightsData,
        objects: &DynamicObjectsBuffer,
    ) {
        if batches.is_empty() {
            return;
        }

        let queue = &context.queue;
        let uniform_size = mem::size_of::<ShadowViewUniform>() as u64;
        let mut staging_offset = 0u64;

        // ========================================================================
        // STAGE 1: Write all shadow matrices to staging buffer
        // ========================================================================

        // Directional lights
        for (_index, shadow) in lights
            .directional_shadows()
            .iter()
            .enumerate()
            .take(MAX_DIRECTIONAL_LIGHTS)
        {
            if shadow.params[0] == 0.0 {
                continue;
            }
            let matrix = glam::Mat4::from_cols_array_2d(&shadow.view_proj);
            let uniform = ShadowViewUniform {
                view_proj: matrix.to_cols_array_2d(),
            };
            queue.write_buffer(
                &self.staging_buffer,
                staging_offset,
                bytemuck::bytes_of(&uniform),
            );
            staging_offset += uniform_size;
        }

        // Spot lights
        let spot_start_offset = staging_offset;
        for (_index, shadow) in lights
            .spot_shadows()
            .iter()
            .enumerate()
            .take(MAX_SPOT_LIGHTS)
        {
            if shadow.params[0] == 0.0 {
                continue;
            }
            let matrix = glam::Mat4::from_cols_array_2d(&shadow.view_proj);
            let uniform = ShadowViewUniform {
                view_proj: matrix.to_cols_array_2d(),
            };
            queue.write_buffer(
                &self.staging_buffer,
                staging_offset,
                bytemuck::bytes_of(&uniform),
            );
            staging_offset += uniform_size;
        }

        // Point lights
        let point_start_offset = staging_offset;

        for (_index, shadow) in lights
            .point_shadows()
            .iter()
            .enumerate()
            .take(MAX_POINT_LIGHTS)
        {
            if shadow.params[0] == 0.0 {
                //log::info!("Point light {} - shadows disabled, skipping", index);
                continue;
            }

            for face in 0..POINT_SHADOW_FACE_COUNT {
                let matrix = glam::Mat4::from_cols_array_2d(&shadow.view_proj[face]);
                let uniform = ShadowViewUniform {
                    view_proj: matrix.to_cols_array_2d(),
                };
                queue.write_buffer(
                    &self.staging_buffer,
                    staging_offset,
                    bytemuck::bytes_of(&uniform),
                );
                staging_offset += uniform_size;
            }
        }
        // ========================================================================
        // STAGE 2: Encode copy + render commands in order
        // ========================================================================

        // Directional lights
        staging_offset = 0;
        for (index, shadow) in lights
            .directional_shadows()
            .iter()
            .enumerate()
            .take(MAX_DIRECTIONAL_LIGHTS)
        {
            if shadow.params[0] == 0.0 {
                continue;
            }

            // Copy from staging to uniform buffer
            encoder.copy_buffer_to_buffer(
                &self.staging_buffer,
                staging_offset,
                &self.uniform_buffer,
                0,
                uniform_size,
            );

            // Render shadow pass
            self.render_pass(
                encoder,
                self.directional.layer_view(index),
                assets,
                batches,
                objects,
            );

            staging_offset += uniform_size;
        }

        // Spot lights
        let mut spot_staging_offset = spot_start_offset;
        for (index, shadow) in lights
            .spot_shadows()
            .iter()
            .enumerate()
            .take(MAX_SPOT_LIGHTS)
        {
            if shadow.params[0] == 0.0 {
                continue;
            }

            encoder.copy_buffer_to_buffer(
                &self.staging_buffer,
                spot_staging_offset,
                &self.uniform_buffer,
                0,
                uniform_size,
            );

            self.render_pass(
                encoder,
                self.spot.layer_view(index),
                assets,
                batches,
                objects,
            );

            spot_staging_offset += uniform_size;
        }

        // Point lights
        let mut point_staging_offset = point_start_offset;

        for (index, shadow) in lights
            .point_shadows()
            .iter()
            .enumerate()
            .take(MAX_POINT_LIGHTS)
        {
            if shadow.params[0] == 0.0 {
                continue;
            }

            for face in 0..POINT_SHADOW_FACE_COUNT {
                let layer_index = index * POINT_SHADOW_FACE_COUNT + face;
                // log::info!(
                //     "  Face {} - copying from staging offset {}, layer {}",
                //     face,
                //     point_staging_offset,
                //     layer_index
                // );

                encoder.copy_buffer_to_buffer(
                    &self.staging_buffer,
                    point_staging_offset,
                    &self.uniform_buffer,
                    0,
                    uniform_size,
                );

                self.render_pass(
                    encoder,
                    self.point.layer_view(layer_index),
                    assets,
                    batches,
                    objects,
                );

                point_staging_offset += uniform_size;
            }
        }
    }

    fn render_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        assets: &Assets,
        batches: &[OrderedBatch],
        objects: &DynamicObjectsBuffer,
    ) {
        if batches.is_empty() {
            return;
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ShadowPass"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.uniform_bind_group, &[]);
        pass.set_bind_group(1, &objects.bind_group, &[]);

        for batch in batches {
            if matches!(batch.pass, RenderPass::Transparent | RenderPass::Overlay) {
                continue;
            }
            let Some(mesh) = assets.meshes.get(batch.mesh) else {
                continue;
            };

            let instance_count = batch.instances.len() as u32;
            pass.set_vertex_buffer(0, mesh.vertex_buffer().slice(..));
            pass.set_index_buffer(mesh.index_buffer().slice(..), mesh.index_format());
            let mut current_range_start: Option<u32> = None;

            for (local_index, instance) in batch.instances.iter().enumerate() {
                let global_index = batch.first_instance + local_index as u32;
                if instance.material.is_unlit() {
                    if let Some(start) = current_range_start.take() {
                        pass.draw_indexed(0..mesh.index_count(), 0, start..global_index);
                    }
                } else if current_range_start.is_none() {
                    current_range_start = Some(global_index);
                }
            }

            if let Some(start) = current_range_start.take() {
                pass.draw_indexed(
                    0..mesh.index_count(),
                    0,
                    start..(batch.first_instance + instance_count),
                );
            }
        }
    }
}

impl DynamicObjectsBuffer {
    fn new(device: &wgpu::Device, capacity: u32) -> Self {
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ObjectsBindLayout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                // CHANGE THIS LINE - add FRAGMENT to visibility
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let buffer_size =
            (capacity as usize * mem::size_of::<crate::renderer::ObjectData>()) as u64;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ObjectsBuffer"),
            size: buffer_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ObjectsBindGroup"),
            layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });

        Self {
            buffer,
            capacity,
            bind_group,
            bind_layout,
            scratch: Vec::with_capacity(capacity as usize),
        }
    }

    fn update(
        &mut self,
        context: &RenderContext,
        batches: &[OrderedBatch],
    ) -> Result<(), wgpu::SurfaceError> {
        self.scratch.clear();
        for batch in batches {
            self.scratch.extend(batch.instances.iter().map(|inst| {
                crate::renderer::ObjectData::from_material(inst.transform.matrix(), &inst.material)
            }));
        }

        let required = self.scratch.len() as u32;
        if required > self.capacity {
            self.grow(context, required);
        }

        if !self.scratch.is_empty() {
            context
                .queue
                .write_buffer(&self.buffer, 0, bytemuck::cast_slice(&self.scratch));
        }

        Ok(())
    }

    fn grow(&mut self, context: &RenderContext, required: u32) {
        let new_capacity = required.max(self.capacity * 2);
        log::info!(
            "Growing objects buffer: {} -> {}",
            self.capacity,
            new_capacity
        );

        let buffer_size =
            (new_capacity as usize * mem::size_of::<crate::renderer::ObjectData>()) as u64;
        self.buffer = context.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ObjectsBuffer"),
            size: buffer_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        self.bind_group = context
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("ObjectsBindGroup"),
                layout: &self.bind_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.buffer.as_entire_binding(),
                }],
            });

        self.capacity = new_capacity;
    }
}

const MAX_TEXTURES: usize = 256;

enum TextureBindingModel {
    Bindless(BindlessTextureBinder),
    Classic(TraditionalTextureBinder),
}

impl TextureBindingModel {
    fn update(&mut self, device: &wgpu::Device, assets: &Assets) {
        match self {
            TextureBindingModel::Bindless(binder) => binder.update(device, assets),
            TextureBindingModel::Classic(binder) => binder.update(device, assets),
        }
    }
}

struct BindlessTextureBinder {
    layout: wgpu::BindGroupLayout,
    linear_sampler: wgpu::Sampler,
    nearest_sampler: wgpu::Sampler,
    _fallback_texture: wgpu::Texture,
    fallback_view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
}

impl BindlessTextureBinder {
    fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout) -> Self {
        let linear_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("BindlessSamplerLinear"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let nearest_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("BindlessSamplerNearest"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let fallback_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("BindlessFallbackTexture"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let fallback_view = fallback_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let bind_group = Self::create_bind_group_with_views(
            device,
            layout,
            &linear_sampler,
            &nearest_sampler,
            vec![&fallback_view; MAX_TEXTURES],
        );

        Self {
            layout: layout.clone(),
            linear_sampler,
            nearest_sampler,
            _fallback_texture: fallback_texture,
            fallback_view,
            bind_group,
        }
    }

    fn create_bind_group_with_views(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        linear_sampler: &wgpu::Sampler,
        nearest_sampler: &wgpu::Sampler,
        views: Vec<&wgpu::TextureView>,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("BindlessTextureBindGroup"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureViewArray(&views),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(linear_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(nearest_sampler),
                },
            ],
        })
    }

    fn update(&mut self, device: &wgpu::Device, assets: &Assets) {
        let fallback = &self.fallback_view;
        let views: Vec<&wgpu::TextureView> = (0..MAX_TEXTURES)
            .map(|i| {
                assets
                    .textures
                    .get(crate::asset::Handle::new(i))
                    .map(|t| &t.view)
                    .unwrap_or(fallback)
            })
            .collect();

        self.bind_group = Self::create_bind_group_with_views(
            device,
            &self.layout,
            &self.linear_sampler,
            &self.nearest_sampler,
            views,
        );

        log::debug!(
            "Updated bindless texture array with {} textures",
            assets.textures.len()
        );
    }

    fn global_bind_group(&self) -> &wgpu::BindGroup {
        &self.bind_group
    }
}

struct TraditionalTextureBinder {
    layout: wgpu::BindGroupLayout,
    linear_sampler: wgpu::Sampler,
    nearest_sampler: wgpu::Sampler,
    _fallback_texture: wgpu::Texture,
    fallback_view: wgpu::TextureView,
    material_bind_groups: HashMap<Material, wgpu::BindGroup>,
}

impl TraditionalTextureBinder {
    fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout) -> Self {
        let linear_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("TraditionalSamplerLinear"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let nearest_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("TraditionalSamplerNearest"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let fallback_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("TraditionalFallbackTexture"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let fallback_view = fallback_texture.create_view(&wgpu::TextureViewDescriptor::default());

        Self {
            layout: layout.clone(),
            linear_sampler,
            nearest_sampler,
            _fallback_texture: fallback_texture,
            fallback_view,
            material_bind_groups: HashMap::new(),
        }
    }

    fn create_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        linear_sampler: &wgpu::Sampler,
        nearest_sampler: &wgpu::Sampler,
        views: [&wgpu::TextureView; 5],
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("TraditionalTextureBindGroup"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(views[0]),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(views[1]),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(views[2]),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(views[3]),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(views[4]),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(linear_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::Sampler(nearest_sampler),
                },
            ],
        })
    }

    fn view_or_fallback<'a>(
        assets: &'a Assets,
        fallback: &'a wgpu::TextureView,
        index: u32,
    ) -> &'a wgpu::TextureView {
        assets
            .textures
            .get(crate::asset::Handle::new(index as usize))
            .map(|t| &t.view)
            .unwrap_or(fallback)
    }

    fn update(&mut self, _device: &wgpu::Device, _assets: &Assets) {
        // When assets change we clear cached bind groups to force recreation with new views
        self.material_bind_groups.clear();
    }

    fn bind_group_for_material(
        &mut self,
        device: &wgpu::Device,
        assets: &Assets,
        material: Material,
    ) -> &wgpu::BindGroup {
        let layout = self.layout.clone();
        let linear_sampler = self.linear_sampler.clone();
        let nearest_sampler = self.nearest_sampler.clone();
        let fallback_view = self.fallback_view.clone();

        self.material_bind_groups
            .entry(material)
            .or_insert_with(|| {
                let fallback_view_ref = &fallback_view;
                let base_color_view = if material
                    .flags
                    .contains(MaterialFlags::USE_BASE_COLOR_TEXTURE)
                {
                    Self::view_or_fallback(assets, fallback_view_ref, material.base_color_texture)
                } else {
                    fallback_view_ref
                };
                let metallic_roughness_view = if material
                    .flags
                    .contains(MaterialFlags::USE_METALLIC_ROUGHNESS_TEXTURE)
                {
                    Self::view_or_fallback(
                        assets,
                        fallback_view_ref,
                        material.metallic_roughness_texture,
                    )
                } else {
                    fallback_view_ref
                };
                let normal_view = if material.flags.contains(MaterialFlags::USE_NORMAL_TEXTURE) {
                    Self::view_or_fallback(assets, fallback_view_ref, material.normal_texture)
                } else {
                    fallback_view_ref
                };
                let emissive_view = if material.flags.contains(MaterialFlags::USE_EMISSIVE_TEXTURE)
                {
                    Self::view_or_fallback(assets, fallback_view_ref, material.emissive_texture)
                } else {
                    fallback_view_ref
                };
                let occlusion_view = if material
                    .flags
                    .contains(MaterialFlags::USE_OCCLUSION_TEXTURE)
                {
                    Self::view_or_fallback(assets, fallback_view_ref, material.occlusion_texture)
                } else {
                    fallback_view_ref
                };

                Self::create_bind_group(
                    device,
                    &layout,
                    &linear_sampler,
                    &nearest_sampler,
                    [
                        base_color_view,
                        metallic_roughness_view,
                        normal_view,
                        emissive_view,
                        occlusion_view,
                    ],
                )
            })
    }
}

impl RenderPipeline {
    fn new(
        context: &RenderContext,
        camera: &CameraBuffer,
        objects: &DynamicObjectsBuffer,
        lights: &LightsBuffer,
        sample_count: u32,
    ) -> (Self, TextureBindingModel) {
        let (texture_bind_layout, texture_binder, shader_source) = if context
            .supports_bindless_textures
        {
            let layout =
                context
                    .device
                    .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                        label: Some("TextureArrayBindGroupLayout"),
                        entries: &[
                            wgpu::BindGroupLayoutEntry {
                                binding: 0,
                                visibility: wgpu::ShaderStages::FRAGMENT,
                                ty: wgpu::BindingType::Texture {
                                    sample_type: wgpu::TextureSampleType::Float {
                                        filterable: true,
                                    },
                                    view_dimension: wgpu::TextureViewDimension::D2,
                                    multisampled: false,
                                },
                                count: std::num::NonZero::new(MAX_TEXTURES as u32),
                            },
                            wgpu::BindGroupLayoutEntry {
                                binding: 1,
                                visibility: wgpu::ShaderStages::FRAGMENT,
                                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                                count: None,
                            },
                            wgpu::BindGroupLayoutEntry {
                                binding: 2,
                                visibility: wgpu::ShaderStages::FRAGMENT,
                                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                                count: None,
                            },
                        ],
                    });

            let binder =
                TextureBindingModel::Bindless(BindlessTextureBinder::new(&context.device, &layout));
            let shader_source = Self::shader_source(true);
            (layout, binder, shader_source)
        } else {
            let layout =
                context
                    .device
                    .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                        label: Some("MaterialTextureBindGroupLayout"),
                        entries: &[
                            wgpu::BindGroupLayoutEntry {
                                binding: 0,
                                visibility: wgpu::ShaderStages::FRAGMENT,
                                ty: wgpu::BindingType::Texture {
                                    sample_type: wgpu::TextureSampleType::Float {
                                        filterable: true,
                                    },
                                    view_dimension: wgpu::TextureViewDimension::D2,
                                    multisampled: false,
                                },
                                count: None,
                            },
                            wgpu::BindGroupLayoutEntry {
                                binding: 1,
                                visibility: wgpu::ShaderStages::FRAGMENT,
                                ty: wgpu::BindingType::Texture {
                                    sample_type: wgpu::TextureSampleType::Float {
                                        filterable: true,
                                    },
                                    view_dimension: wgpu::TextureViewDimension::D2,
                                    multisampled: false,
                                },
                                count: None,
                            },
                            wgpu::BindGroupLayoutEntry {
                                binding: 2,
                                visibility: wgpu::ShaderStages::FRAGMENT,
                                ty: wgpu::BindingType::Texture {
                                    sample_type: wgpu::TextureSampleType::Float {
                                        filterable: true,
                                    },
                                    view_dimension: wgpu::TextureViewDimension::D2,
                                    multisampled: false,
                                },
                                count: None,
                            },
                            wgpu::BindGroupLayoutEntry {
                                binding: 3,
                                visibility: wgpu::ShaderStages::FRAGMENT,
                                ty: wgpu::BindingType::Texture {
                                    sample_type: wgpu::TextureSampleType::Float {
                                        filterable: true,
                                    },
                                    view_dimension: wgpu::TextureViewDimension::D2,
                                    multisampled: false,
                                },
                                count: None,
                            },
                            wgpu::BindGroupLayoutEntry {
                                binding: 4,
                                visibility: wgpu::ShaderStages::FRAGMENT,
                                ty: wgpu::BindingType::Texture {
                                    sample_type: wgpu::TextureSampleType::Float {
                                        filterable: true,
                                    },
                                    view_dimension: wgpu::TextureViewDimension::D2,
                                    multisampled: false,
                                },
                                count: None,
                            },
                            wgpu::BindGroupLayoutEntry {
                                binding: 5,
                                visibility: wgpu::ShaderStages::FRAGMENT,
                                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                                count: None,
                            },
                            wgpu::BindGroupLayoutEntry {
                                binding: 6,
                                visibility: wgpu::ShaderStages::FRAGMENT,
                                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                                count: None,
                            },
                        ],
                    });

            let binder = TextureBindingModel::Classic(TraditionalTextureBinder::new(
                &context.device,
                &layout,
            ));
            let shader_source = Self::shader_source(false);
            (layout, binder, shader_source)
        };

        let shader = context
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Shader"),
                source: wgpu::ShaderSource::Wgsl(shader_source.into()),
            });

        let pipeline_layout =
            context
                .device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("PipelineLayout"),
                    bind_group_layouts: &[
                        &camera.bind_layout,
                        &objects.bind_layout,
                        &lights.bind_layout,
                        &texture_bind_layout,
                    ],
                    push_constant_ranges: &[],
                });

        let mut pipelines = HashMap::new();
        for &depth_test in &[false, true] {
            for &depth_write in &[false, true] {
                for &alpha_blend in &[false, true] {
                    let key = PipelineKey::new(depth_test, depth_write, alpha_blend);
                    let pipeline = Self::create_pipeline(
                        context,
                        &pipeline_layout,
                        &shader,
                        depth_test,
                        depth_write,
                        alpha_blend,
                        sample_count,
                    );
                    pipelines.insert(key, pipeline);
                }
            }
        }

        let depth_shader_src = include_str!("../shader/depth_prepass.wgsl");
        let depth_shader = context
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("DepthPrepassShader"),
                source: wgpu::ShaderSource::Wgsl(depth_shader_src.into()),
            });
        let depth_pipeline_layout =
            context
                .device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("DepthPrepassLayout"),
                    bind_group_layouts: &[&camera.bind_layout, &objects.bind_layout],
                    push_constant_ranges: &[],
                });
        let depth_prepass = Self::create_depth_prepass_pipeline(
            context,
            &depth_pipeline_layout,
            &depth_shader,
            sample_count,
        );

        (
            Self {
                pipelines,
                depth_prepass,
            },
            texture_binder,
        )
    }

    fn shader_source(bindless: bool) -> String {
        let bindings = if bindless {
            include_str!("../shader/bindings_bindless.wgsl")
        } else {
            include_str!("../shader/bindings_traditional.wgsl")
        };
        format!("{bindings}\n{}", include_str!("../shader/common.wgsl"))
    }

    fn create_pipeline(
        context: &RenderContext,
        pipeline_layout: &wgpu::PipelineLayout,
        shader: &wgpu::ShaderModule,
        depth_test: bool,
        depth_write: bool,
        alpha_blend: bool,
        sample_count: u32,
    ) -> wgpu::RenderPipeline {
        let depth_compare = if depth_test {
            wgpu::CompareFunction::LessEqual
        } else {
            wgpu::CompareFunction::Always
        };

        let blend_state = if alpha_blend {
            Some(wgpu::BlendState::ALPHA_BLENDING)
        } else {
            Some(wgpu::BlendState::REPLACE)
        };

        context
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Pipeline"),
                layout: Some(pipeline_layout),
                vertex: wgpu::VertexState {
                    module: shader,
                    entry_point: Some("vs_main"),
                    buffers: &[Vertex::layout()],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: context.config.format,
                        blend: blend_state,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    cull_mode: Some(wgpu::Face::Back),
                    front_face: wgpu::FrontFace::Ccw,
                    polygon_mode: wgpu::PolygonMode::Fill,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: context.depth.format,
                    depth_write_enabled: depth_write,
                    depth_compare,
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: sample_count,
                    mask: !0,
                    alpha_to_coverage_enabled: false,
                },
                multiview: None,
                cache: None,
            })
    }

    fn pipeline(&self, key: PipelineKey) -> &wgpu::RenderPipeline {
        self.pipelines.get(&key).expect("missing pipeline variant")
    }

    fn create_depth_prepass_pipeline(
        context: &RenderContext,
        pipeline_layout: &wgpu::PipelineLayout,
        shader: &wgpu::ShaderModule,
        sample_count: u32,
    ) -> wgpu::RenderPipeline {
        context
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("DepthPrepassPipeline"),
                layout: Some(pipeline_layout),
                vertex: wgpu::VertexState {
                    module: shader,
                    entry_point: Some("vs_main"),
                    buffers: &[Vertex::layout()],
                    compilation_options: Default::default(),
                },
                fragment: None,
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    cull_mode: Some(wgpu::Face::Back),
                    front_face: wgpu::FrontFace::Ccw,
                    polygon_mode: wgpu::PolygonMode::Fill,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: context.depth.format,
                    depth_write_enabled: true,
                    depth_compare: wgpu::CompareFunction::LessEqual,
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: sample_count,
                    mask: !0,
                    alpha_to_coverage_enabled: false,
                },
                multiview: None,
                cache: None,
            })
    }

    fn depth_prepass(&self) -> &wgpu::RenderPipeline {
        &self.depth_prepass
    }
}
