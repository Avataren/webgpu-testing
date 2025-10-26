// renderer/renderer.rs
use crate::asset::{Assets, Mesh};
use crate::environment::Environment;
use crate::renderer::batch::InstanceData;
use crate::renderer::frame_graph::{
    FrameGraph, FrameGraphColorAttachment, FrameGraphDepthAttachment, PassPlan,
};
use crate::renderer::internal::shadows::ShadowInvocation;
use crate::renderer::internal::{
    CameraBuffer, DynamicObjectsBuffer, EnvironmentResources, LightsBuffer, OrderedBatch,
    PipelineKey, PreparedBatches, RenderContext, RenderPipeline, ShadowResources,
    TextureBindingModel,
};
use crate::renderer::{
    lights::{MAX_DIRECTIONAL_LIGHTS, MAX_POINT_LIGHTS, MAX_SPOT_LIGHTS},
    postprocess::{PostProcess, PostProcessCamera, PostProcessEffects},
    CameraUniform, CustomRenderContext, CustomRenderRequest, CustomRenderStage, LightsData,
    Material, RenderBatcher, RenderPass, RenderRegion, Vertex,
};
use crate::scene::{Camera, CameraProjection, Scene};
use crate::settings::RenderSettings;

use glam::Vec3;
use std::mem::size_of;
#[cfg(target_arch = "wasm32")]
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use winit::{dpi::PhysicalSize, window::Window};

const INITIAL_OBJECTS_CAPACITY: u32 = 1024 * 100;
const POINT_SHADOW_FACE_COUNT: u32 = 6;
const PICK_COPY_BYTES_PER_ROW: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;

#[derive(Default)]
struct PickState {
    pending_request: Option<PickRequest>,
    pending_readback: Option<PendingPickReadback>,
    ready_value: Option<u64>,
}

struct PickRequest {
    x: u32,
    y: u32,
}

struct PendingPickReadback {
    buffer: wgpu::Buffer,
    status: Arc<Mutex<Option<Result<(), wgpu::BufferAsyncError>>>>,
    mapping_requested: bool,
}

impl PendingPickReadback {
    fn new(buffer: wgpu::Buffer) -> Self {
        Self {
            buffer,
            status: Arc::new(Mutex::new(None)),
            mapping_requested: false,
        }
    }

    fn ensure_mapping_requested(&mut self) {
        if self.mapping_requested {
            return;
        }

        let slice = self.buffer.slice(..PICK_COPY_BYTES_PER_ROW as u64);
        let status_clone = Arc::clone(&self.status);
        slice.map_async(wgpu::MapMode::Read, move |result| {
            if let Ok(mut guard) = status_clone.lock() {
                *guard = Some(result);
            }
        });

        self.mapping_requested = true;
    }

    fn poll(&mut self, device: &wgpu::Device) -> Option<Result<u64, wgpu::BufferAsyncError>> {
        self.ensure_mapping_requested();

        {
            let guard = self.status.lock().expect("pick readback status poisoned");
            if guard.is_none() {
                drop(guard);
                let _ = device.poll(wgpu::PollType::Poll);
            } else {
                drop(guard);
            }
        }

        let mut guard = self.status.lock().expect("pick readback status poisoned");
        let result = guard.take();
        drop(guard);

        result.map(|status| match status {
            Ok(()) => {
                let slice = self.buffer.slice(..size_of::<u64>() as u64);
                let data = slice.get_mapped_range();
                let mut bytes = [0u8; 8];
                bytes.copy_from_slice(&data[..8]);
                drop(data);
                self.buffer.unmap();
                self.mapping_requested = false;
                Ok(u64::from_le_bytes(bytes))
            }
            Err(err) => {
                self.buffer.unmap();
                self.mapping_requested = false;
                Err(err)
            }
        })
    }
}

impl Drop for PendingPickReadback {
    fn drop(&mut self) {
        if self.mapping_requested {
            self.buffer.unmap();
            self.mapping_requested = false;
        }
    }
}
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
    pub gizmo_draw_calls: u32,
    pub shadow_draw_calls: u32,
}

impl RendererStats {
    pub fn total_draw_calls(&self) -> u32 {
        self.depth_prepass_draw_calls
            + self.opaque_draw_calls
            + self.transparent_draw_calls
            + self.overlay_draw_calls
            + self.gizmo_draw_calls
            + self.shadow_draw_calls
    }
}

pub struct Renderer {
    texture_binder: TextureBindingModel,
    objects_buffer: DynamicObjectsBuffer,
    camera_buffer: CameraBuffer,
    lights_buffer: LightsBuffer,
    environment: EnvironmentResources,
    shadows: ShadowResources,
    postprocess: PostProcess,
    camera_position: Vec3,
    camera_target: Vec3,
    camera_up: Vec3,
    camera_projection: CameraProjection,
    camera_fov_y: f32,
    camera_aspect: f32,
    settings: RenderSettings,
    pick_active: bool,
    pick_state: PickState,
    #[cfg(feature = "egui")]
    ui_hook: Option<UiHook>,
    stats: RendererStats,
    pipeline: RenderPipeline,
    context: RenderContext,
    render_region: Option<RenderRegion>,
}

impl Renderer {
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn new(window: Arc<Window>, settings: RenderSettings) -> Self {
        let size = window.inner_size();
        let context = RenderContext::new(window, size, &settings).await;
        Self::from_context(context, settings)
    }

    #[cfg(target_arch = "wasm32")]
    pub async fn new(window: Rc<Window>, settings: RenderSettings) -> Self {
        let size = window.inner_size();
        let context = RenderContext::new(window, size, &settings).await;
        Self::from_context(context, settings)
    }

    fn from_context(context: RenderContext, mut settings: RenderSettings) -> Self {
        let sample_count = context.sample_count;
        let aspect = context.config.width as f32 / context.config.height.max(1) as f32;
        settings.sample_count = sample_count;
        let camera_buffer = CameraBuffer::new(&context.device);
        let environment = EnvironmentResources::new(&context.device, &context.queue);
        let objects_buffer = DynamicObjectsBuffer::new(&context.device, INITIAL_OBJECTS_CAPACITY);
        let shadows =
            ShadowResources::new(&context.device, &objects_buffer, settings.shadow_map_size);
        let lights_buffer = LightsBuffer::new(&context.device, &shadows, &environment);
        let (pipeline, texture_binder) = RenderPipeline::new(
            &context,
            &camera_buffer,
            &objects_buffer,
            &lights_buffer,
            sample_count,
        );
        let mut postprocess = PostProcess::new(
            &context.device,
            &context.queue,
            &context.config,
            sample_count,
        );
        postprocess.set_depth_view(&context.depth.sampled_view);

        Self {
            context,
            pipeline,
            texture_binder,
            objects_buffer,
            camera_buffer,
            lights_buffer,
            environment,
            shadows,
            postprocess,
            camera_position: Vec3::ZERO,
            camera_target: Vec3::ZERO,
            camera_up: Vec3::Y,
            camera_projection: CameraProjection::default(),
            camera_fov_y: 60f32.to_radians(),
            camera_aspect: aspect,
            settings,
            pick_active: false,
            pick_state: PickState::default(),
            #[cfg(feature = "egui")]
            ui_hook: None,
            stats: RendererStats::default(),
            render_region: None,
        }
    }

    // Setter to install the per-frame hook (only compiled with egui feature)
    #[cfg(feature = "egui")]
    pub fn set_ui_hook(&mut self, hook: UiHook) {
        self.ui_hook = Some(hook);
    }

    pub fn set_render_region(&mut self, region: Option<RenderRegion>) {
        let clamped =
            region.and_then(|r| r.clamp(self.context.config.width, self.context.config.height));
        self.render_region = clamped;
    }

    pub fn supports_bindless_textures(&self) -> bool {
        self.context.supports_bindless_textures
    }

    pub fn set_pick_active(&mut self, active: bool) {
        self.pick_active = active;
    }

    pub fn is_pick_active(&self) -> bool {
        self.pick_active
    }

    pub fn request_pick(&mut self, coords: [u32; 2]) -> bool {
        if self.pick_state.pending_request.is_some() || self.pick_state.pending_readback.is_some() {
            return false;
        }

        self.pick_state.ready_value = None;
        self.pick_state.pending_request = Some(PickRequest {
            x: coords[0],
            y: coords[1],
        });
        true
    }

    pub fn poll_pick_result(&mut self) -> Option<u64> {
        if let Some(value) = self.pick_state.ready_value.take() {
            return Some(value);
        }

        if let Some(readback) = self.pick_state.pending_readback.as_mut() {
            if let Some(result) = readback.poll(&self.context.device) {
                self.pick_state.pending_readback = None;
                return Some(match result {
                    Ok(value) => value,
                    Err(error) => {
                        log::error!("Failed to map pick readback buffer: {error}");
                        0
                    }
                });
            }
        }

        None
    }

    pub fn get_device(&self) -> &wgpu::Device {
        &self.context.device
    }

    pub fn get_queue(&self) -> &wgpu::Queue {
        &self.context.queue
    }

    pub fn reserve_object_capacity(&mut self, count: u32) {
        self.objects_buffer.ensure_capacity(&self.context, count);
    }

    pub fn objects_buffer(&self) -> &wgpu::Buffer {
        self.objects_buffer.buffer()
    }

    pub fn camera_bind_layout(&self) -> &wgpu::BindGroupLayout {
        &self.camera_buffer.bind_layout
    }

    pub fn camera_bind_group(&self) -> &wgpu::BindGroup {
        &self.camera_buffer.bind_group
    }

    pub fn lights_bind_layout(&self) -> &wgpu::BindGroupLayout {
        &self.lights_buffer.bind_layout
    }

    pub fn lights_bind_group(&self) -> &wgpu::BindGroup {
        &self.lights_buffer.bind_group
    }

    pub fn textures_bind_layout(&self) -> &wgpu::BindGroupLayout {
        self.texture_binder.bind_layout()
    }

    pub fn textures_bind_group(&self) -> &wgpu::BindGroup {
        self.texture_binder
            .global_bind_group()
            .expect("GPU-driven particles require bindless textures")
    }

    pub fn depth_view(&self) -> &wgpu::TextureView {
        &self.context.depth.view
    }

    /// Returns the depth texture view configured for sampling during
    /// post-processing passes. This view is multisample-resolved when MSAA is
    /// enabled, making it suitable for screen-space effects.
    pub fn depth_sample_view(&self) -> &wgpu::TextureView {
        &self.context.depth.sampled_view
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
        );
        self.postprocess
            .set_depth_view(&self.context.depth.sampled_view);
    }

    pub fn aspect_ratio(&self) -> f32 {
        self.context.config.width as f32 / self.context.config.height.max(1) as f32
    }

    pub fn set_camera(&mut self, camera: &Camera, aspect: f32) {
        self.camera_position = camera.position(); // Store it
        self.camera_target = camera.target;
        self.camera_up = camera.up;
        self.camera_projection = camera.projection();
        self.camera_fov_y = camera.fov_y_radians();
        self.camera_aspect = aspect;
        let view = camera.view();
        let view_inv = view.inverse();
        let proj = camera.proj(aspect);
        let vp = proj * view;
        let inv_vp = vp.inverse();
        let camera_pos = camera.position();
        let mut forward = (camera.target - camera_pos)
            .try_normalize()
            .unwrap_or(Vec3::NEG_Z);
        if forward.length_squared() < 1e-6 {
            forward = Vec3::NEG_Z;
        }
        let mut up = camera.up.try_normalize().unwrap_or(Vec3::Y);
        if up.length_squared() < 1e-6 {
            up = Vec3::Y;
        }
        let uni = CameraUniform::from_matrices(vp, inv_vp, camera_pos, forward, up);
        self.context
            .queue
            .write_buffer(&self.camera_buffer.buffer, 0, bytemuck::bytes_of(&uni));
        self.postprocess.update_camera(
            &self.context.queue,
            PostProcessCamera {
                proj,
                view,
                view_inv,
                view_proj: vp,
                view_proj_inv: inv_vp,
                position: camera.position(),
                near: camera.near(),
                far: camera.far(),
            },
        );
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

    pub fn camera_projection(&self) -> CameraProjection {
        self.camera_projection
    }

    pub fn camera_fov_y(&self) -> f32 {
        self.camera_fov_y
    }

    pub fn camera_aspect(&self) -> f32 {
        self.camera_aspect
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
        scene: &Scene,
        assets: &Assets,
        batcher: &RenderBatcher,
        lights: &LightsData,
        environment: &Environment,
        custom_render: &mut Option<CustomRenderRequest<'_>>,
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

        let mut prepared_batches = PreparedBatches::from_batcher(batcher, self.camera_position);

        let batch_count = prepared_batches.all().len() as u32;
        let instance_count = prepared_batches
            .all()
            .iter()
            .map(|batch| batch.instances.len() as u32)
            .sum();

        let mut frame_stats = RendererStats {
            batch_count,
            instance_count,
            ..RendererStats::default()
        };

        let env_texture_changed =
            self.environment
                .update(&self.context.device, &self.context.queue, environment);

        self.postprocess
            .set_color_grading(&self.context.queue, environment.color_grading());

        if env_texture_changed {
            self.lights_buffer.rebuild_bind_group(
                &self.context.device,
                &self.shadows,
                &self.environment,
            );
        }

        self.objects_buffer.update(
            &self.context,
            prepared_batches.all(),
            prepared_batches.materials(),
        )?;
        self.lights_buffer.update(&self.context.queue, lights);

        let custom_shadow_enabled = custom_render
            .as_ref()
            .is_some_and(|req| req.render_in_shadow_pass);
        let mut shadow_invocations: Vec<ShadowInvocation> = Vec::new();

        self.shadows.render(
            &self.context,
            &mut encoder,
            assets,
            prepared_batches.all(),
            lights,
            &self.objects_buffer,
            prepared_batches.materials(),
            custom_shadow_enabled,
            &mut shadow_invocations,
        );

        if custom_shadow_enabled {
            if let Some(request) = custom_render.as_mut() {
                for invocation in &shadow_invocations {
                    let mut ctx = CustomRenderContext::new(
                        &mut encoder,
                        self,
                        scene,
                        &invocation.view,
                        &invocation.view,
                        CustomRenderStage::Shadow(invocation.stage),
                        Some(invocation.view_proj),
                        None,
                    );
                    (request.callback)(&mut ctx);
                }
            }
        }

        let depth_view = self.context.depth.view.clone();
        let surface_width = self.context.config.width;
        let surface_height = self.context.config.height;
        let render_region = self
            .render_region
            .or_else(|| RenderRegion::full(surface_width, surface_height));

        self.postprocess
            .update_viewport(&self.context.queue, render_region);

        let pick_active = self.pick_active;
        if pick_active {
            self.postprocess
                .ensure_pick_attachment(&self.context.device);
        }

        let (scene_view, resolve_target) = {
            let (view, resolve) = self.postprocess.scene_color_views();
            (view.clone(), resolve.cloned())
        };
        let scene_format = self.context.scene_texture_format();

        {
            let mut frame_graph = FrameGraph::new(&mut encoder);
            self.render_depth_prepass(
                &mut frame_graph,
                &depth_view,
                &mut prepared_batches,
                assets,
                render_region,
                &mut frame_stats,
            );
        }

        {
            let mut frame_graph = FrameGraph::new(&mut encoder);
            self.render_main_color_pass(
                &mut frame_graph,
                &scene_view,
                resolve_target.as_ref(),
                &depth_view,
                environment,
                assets,
                &prepared_batches,
                scene_format,
                render_region,
                pick_active,
                &mut frame_stats,
            );
        }

        if let Some(request) = custom_render.as_mut() {
            if request.stage == CustomRenderStage::BeforePostprocess {
                let mut ctx = CustomRenderContext::new(
                    &mut encoder,
                    self,
                    scene,
                    &scene_view,
                    &depth_view,
                    CustomRenderStage::BeforePostprocess,
                    None,
                    render_region,
                );
                (request.callback)(&mut ctx);
            }
        }

        // Resolve scene → swapchain
        self.postprocess
            .execute(&mut encoder, &self.context.device, &view, render_region);

        if let Some(request) = custom_render.as_mut() {
            if request.stage == CustomRenderStage::AfterPostprocess {
                let resolved_depth_view = self.postprocess.after_postprocess_depth_view().cloned();
                let depth_view_after = resolved_depth_view.as_ref().unwrap_or(&depth_view);
                let mut ctx = CustomRenderContext::new(
                    &mut encoder,
                    self,
                    scene,
                    &view,
                    depth_view_after,
                    CustomRenderStage::AfterPostprocess,
                    None,
                    render_region,
                );
                (request.callback)(&mut ctx);
            }
        }

        {
            let mut frame_graph = FrameGraph::new(&mut encoder);
            self.render_transparent_pass(
                &mut frame_graph,
                &view,
                &depth_view,
                assets,
                &prepared_batches,
                render_region,
                pick_active,
                &mut frame_stats,
            );
        }

        {
            let mut frame_graph = FrameGraph::new(&mut encoder);
            self.render_overlay_pass(
                &mut frame_graph,
                &view,
                &depth_view,
                assets,
                &prepared_batches,
                render_region,
                pick_active,
                &mut frame_stats,
            );
        }

        let materials = prepared_batches.materials();

        if !prepared_batches.gizmos().is_empty() {
            let mut frame_graph = FrameGraph::new(&mut encoder);
            self.execute_gizmo_pass(
                &mut frame_graph,
                "GizmoPass",
                prepared_batches.gizmos(),
                &view,
                assets,
                render_region,
                pick_active,
                &mut frame_stats,
                materials,
            );
        }

        if !prepared_batches.gizmo_solids().is_empty() {
            let mut frame_graph = FrameGraph::new(&mut encoder);
            self.execute_gizmo_pass(
                &mut frame_graph,
                "GizmoSolidPass",
                prepared_batches.gizmo_solids(),
                &view,
                assets,
                render_region,
                pick_active,
                &mut frame_stats,
                materials,
            );
        }

        self.process_pick_request(&mut encoder);

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

        frame_stats.shadow_draw_calls = estimate_shadow_draw_calls(
            prepared_batches.all(),
            prepared_batches.materials(),
            lights,
        );

        self.stats = frame_stats;

        self.context.queue.submit(Some(encoder.finish()));

        if let Some(readback) = self.pick_state.pending_readback.as_mut() {
            readback.ensure_mapping_requested();
        }
        Ok(RenderFrame { frame })
    }

    pub fn surface_format(&self) -> wgpu::TextureFormat {
        self.context.config.format
    }

    pub fn scene_texture_format(&self) -> wgpu::TextureFormat {
        self.context.scene_texture_format()
    }

    pub fn color_format_for_stage(&self, stage: CustomRenderStage) -> wgpu::TextureFormat {
        match stage {
            CustomRenderStage::BeforePostprocess => self.scene_texture_format(),
            CustomRenderStage::AfterPostprocess => self.surface_format(),
            CustomRenderStage::Shadow(_) => wgpu::TextureFormat::Depth32Float,
        }
    }

    pub fn surface_size(&self) -> PhysicalSize<u32> {
        self.context.size
    }

    pub fn sample_count(&self) -> u32 {
        self.context.sample_count
    }

    pub fn sample_count_for_stage(&self, stage: CustomRenderStage) -> u32 {
        match stage {
            CustomRenderStage::BeforePostprocess => self.sample_count(),
            CustomRenderStage::AfterPostprocess => 1,
            CustomRenderStage::Shadow(_) => 1,
        }
    }

    pub fn set_postprocess_effects(&mut self, effects: PostProcessEffects) {
        self.postprocess.set_effects(&self.context.queue, effects);
    }

    pub fn postprocess_effects(&self) -> PostProcessEffects {
        self.postprocess.effects()
    }

    pub fn last_frame_stats(&self) -> RendererStats {
        self.stats
    }

    fn render_depth_prepass<'a>(
        &'a mut self,
        frame_graph: &'a mut FrameGraph<'a>,
        depth_view: &'a wgpu::TextureView,
        prepared_batches: &'a mut PreparedBatches,
        assets: &'a Assets,
        render_region: Option<RenderRegion>,
        frame_stats: &mut RendererStats,
    ) {
        let plan = PassPlan::new("DepthPrepass")
            .set_depth(FrameGraphDepthAttachment::clear(depth_view, 1.0));

        let pipeline = self.pipeline.depth_prepass();
        let camera_bind_group = &self.camera_buffer.bind_group;
        let object_bind_group = &self.objects_buffer.bind_group;
        let opaque_batches = prepared_batches.opaque_mut();

        frame_graph.execute_pass(plan, render_region, |pass| {
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, camera_bind_group, &[]);
            pass.set_bind_group(1, object_bind_group, &[]);

            for batch in opaque_batches.iter_mut() {
                if batch.alpha_blend
                    || !batch.depth_state.depth_write
                    || !batch.depth_state.depth_test
                {
                    continue;
                }

                let Some(mesh) = mesh_for_batch(assets, batch) else {
                    continue;
                };

                self.draw_full_batch(pass, mesh, batch);
                frame_stats.depth_prepass_draw_calls += 1;
                batch.depth_state.depth_write = false;
            }
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn render_main_color_pass<'a>(
        &'a mut self,
        frame_graph: &'a mut FrameGraph<'a>,
        scene_view: &'a wgpu::TextureView,
        resolve_target: Option<&'a wgpu::TextureView>,
        depth_view: &'a wgpu::TextureView,
        environment: &Environment,
        assets: &'a Assets,
        prepared_batches: &'a PreparedBatches,
        scene_format: wgpu::TextureFormat,
        render_region: Option<RenderRegion>,
        pick_active: bool,
        frame_stats: &mut RendererStats,
    ) {
        let (normal_view, normal_resolve, position_view, position_resolve, pick_targets) = {
            let gbuffer_views = self.postprocess.gbuffer_views();
            (
                gbuffer_views.normal.0.clone(),
                gbuffer_views.normal.1.cloned(),
                gbuffer_views.position.0.clone(),
                gbuffer_views.position.1.cloned(),
                if pick_active {
                    gbuffer_views
                        .id
                        .map(|views| (views.multisample.clone(), views.resolve.cloned()))
                } else {
                    None
                },
            )
        };

        let mut plan = PassPlan::new("MainPass")
            .add_color(FrameGraphColorAttachment::clear(
                scene_view,
                resolve_target,
                environment.clear_color(),
            ))
            .add_color(FrameGraphColorAttachment::clear(
                &normal_view,
                normal_resolve.as_ref(),
                wgpu::Color::TRANSPARENT,
            ))
            .add_color(FrameGraphColorAttachment::clear(
                &position_view,
                position_resolve.as_ref(),
                wgpu::Color::TRANSPARENT,
            ))
            .set_depth(FrameGraphDepthAttachment::load(depth_view));

        if let Some((pick_view, pick_resolve)) = pick_targets.as_ref() {
            plan = plan.add_color(FrameGraphColorAttachment::clear(
                pick_view,
                pick_resolve.as_ref(),
                wgpu::Color::TRANSPARENT,
            ));
        }

        let hdr_enabled = environment.is_hdr_enabled();
        let sample_count = self.context.sample_count;
        let materials = prepared_batches.materials();
        let opaque_batches = prepared_batches.opaque();

        frame_graph.execute_pass(plan, render_region, |pass| {
            if hdr_enabled {
                self.draw_environment_background(pass, pick_active);
            }

            frame_stats.opaque_draw_calls += self.record_batches(
                pass,
                assets,
                opaque_batches,
                materials,
                scene_format,
                sample_count,
                true,
                pick_active,
            );
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn render_transparent_pass<'a>(
        &'a mut self,
        frame_graph: &'a mut FrameGraph<'a>,
        surface_view: &'a wgpu::TextureView,
        depth_view: &'a wgpu::TextureView,
        assets: &'a Assets,
        prepared_batches: &'a PreparedBatches,
        render_region: Option<RenderRegion>,
        pick_active: bool,
        frame_stats: &mut RendererStats,
    ) {
        let batches = prepared_batches.transparent();
        if batches.is_empty() {
            return;
        }

        let pick_view = if pick_active {
            self.postprocess
                .pick_attachment_views()
                .map(|views| views.single_sample().clone())
        } else {
            None
        };

        let mut plan = PassPlan::new("TransparentPass")
            .add_color(FrameGraphColorAttachment::load(surface_view, None))
            .set_depth(FrameGraphDepthAttachment::load(depth_view));

        if let Some(view) = pick_view.as_ref() {
            plan = plan.add_color(FrameGraphColorAttachment::load(view, None));
        }

        let materials = prepared_batches.materials();

        frame_graph.execute_pass(plan, render_region, |pass| {
            frame_stats.transparent_draw_calls += self.record_batches(
                pass,
                assets,
                batches,
                materials,
                self.context.config.format,
                1,
                false,
                pick_active,
            );
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn render_overlay_pass<'a>(
        &'a mut self,
        frame_graph: &'a mut FrameGraph<'a>,
        surface_view: &'a wgpu::TextureView,
        depth_view: &'a wgpu::TextureView,
        assets: &'a Assets,
        prepared_batches: &'a PreparedBatches,
        render_region: Option<RenderRegion>,
        pick_active: bool,
        frame_stats: &mut RendererStats,
    ) {
        let batches = prepared_batches.overlay();
        if batches.is_empty() {
            return;
        }

        let overlay_needs_depth = batches
            .iter()
            .any(|batch| batch.depth_state.depth_test || batch.depth_state.depth_write);

        let pick_view = if pick_active {
            self.postprocess
                .pick_attachment_views()
                .map(|views| views.single_sample().clone())
        } else {
            None
        };

        let mut plan = PassPlan::new("OverlayPass")
            .add_color(FrameGraphColorAttachment::load(surface_view, None));

        if let Some(view) = pick_view.as_ref() {
            plan = plan.add_color(FrameGraphColorAttachment::load(view, None));
        }

        if overlay_needs_depth {
            plan = plan.set_depth(FrameGraphDepthAttachment::load(depth_view));
        }

        let materials = prepared_batches.materials();

        frame_graph.execute_pass(plan, render_region, |pass| {
            frame_stats.overlay_draw_calls += self.record_batches(
                pass,
                assets,
                batches,
                materials,
                self.context.config.format,
                1,
                false,
                pick_active,
            );
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn execute_gizmo_pass<'a>(
        &'a mut self,
        frame_graph: &'a mut FrameGraph<'a>,
        label: &'a str,
        batches: &'a [OrderedBatch],
        surface_view: &'a wgpu::TextureView,
        assets: &'a Assets,
        render_region: Option<RenderRegion>,
        pick_active: bool,
        frame_stats: &mut RendererStats,
        materials: &'a [Material],
    ) {
        if batches.is_empty() {
            return;
        }

        let pick_view = if pick_active {
            self.postprocess
                .pick_attachment_views()
                .map(|views| views.single_sample().clone())
        } else {
            None
        };

        let mut plan =
            PassPlan::new(label).add_color(FrameGraphColorAttachment::load(surface_view, None));

        if let Some(view) = pick_view.as_ref() {
            plan = plan.add_color(FrameGraphColorAttachment::load(view, None));
        }

        frame_graph.execute_pass(plan, render_region, |pass| {
            frame_stats.gizmo_draw_calls += self.record_batches(
                pass,
                assets,
                batches,
                materials,
                self.context.config.format,
                1,
                false,
                pick_active,
            );
        });
    }

    fn process_pick_request(&mut self, encoder: &mut wgpu::CommandEncoder) {
        let Some(request) = self.pick_state.pending_request.take() else {
            return;
        };

        let Some(texture) = self.postprocess.pick_texture() else {
            log::warn!("Pick request issued without an available pick attachment");
            self.pick_state.ready_value = Some(0);
            return;
        };

        let Some(extent) = self.postprocess.pick_texture_extent() else {
            self.pick_state.ready_value = Some(0);
            return;
        };

        if extent.width == 0 || extent.height == 0 {
            self.pick_state.ready_value = Some(0);
            return;
        }

        let origin = wgpu::Origin3d {
            x: request.x.min(extent.width.saturating_sub(1)),
            y: request.y.min(extent.height.saturating_sub(1)),
            z: 0,
        };

        let buffer = self.context.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("PickReadbackBuffer"),
            size: PICK_COPY_BYTES_PER_ROW as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let source = wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin,
            aspect: wgpu::TextureAspect::All,
        };

        let destination = wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(PICK_COPY_BYTES_PER_ROW),
                rows_per_image: Some(1),
            },
        };

        encoder.copy_texture_to_buffer(
            source,
            destination,
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );

        self.pick_state.pending_readback = Some(PendingPickReadback::new(buffer));
    }

    #[allow(clippy::too_many_arguments)]
    fn record_batches(
        &mut self,
        rpass: &mut wgpu::RenderPass<'_>,
        assets: &Assets,
        batches: &[OrderedBatch],
        materials: &[Material],
        color_format: wgpu::TextureFormat,
        color_sample_count: u32,
        use_gbuffer: bool,
        write_pick: bool,
    ) -> u32 {
        if batches.is_empty() {
            return 0;
        }

        let mut draw_calls = 0u32;

        if let Some(bindless_group) = self.texture_binder.global_bind_group() {
            for batch in batches {
                let Some(mesh) = self.setup_batch_state(
                    rpass,
                    assets,
                    batch,
                    color_format,
                    color_sample_count,
                    use_gbuffer,
                    write_pick,
                ) else {
                    continue;
                };
                rpass.set_bind_group(3, bindless_group, &[]);
                self.draw_full_batch(rpass, mesh, batch);
                draw_calls += 1;
            }
        } else {
            for batch in batches {
                let Some(mesh) = self.setup_batch_state(
                    rpass,
                    assets,
                    batch,
                    color_format,
                    color_sample_count,
                    use_gbuffer,
                    write_pick,
                ) else {
                    continue;
                };
                draw_calls += self.draw_classic_batch(rpass, assets, mesh, batch, materials) as u32;
            }
        }
        draw_calls
    }

    #[allow(clippy::too_many_arguments)]
    fn setup_batch_state<'a>(
        &self,
        rpass: &mut wgpu::RenderPass<'_>,
        assets: &'a Assets,
        batch: &OrderedBatch,
        color_format: wgpu::TextureFormat,
        color_sample_count: u32,
        use_gbuffer: bool,
        write_pick: bool,
    ) -> Option<&'a Mesh> {
        let mesh = mesh_for_batch(assets, batch)?;
        let wants_wireframe = matches!(batch.pass, RenderPass::Gizmo);
        let wireframe = wants_wireframe && self.context.supports_wireframe;
        let pipeline_key = PipelineKey::new(
            batch.depth_state.depth_test,
            batch.depth_state.depth_write,
            batch.alpha_blend,
            color_format,
            color_sample_count,
            batch.sampler_filtering,
            batch.cull_mode,
            use_gbuffer,
            write_pick,
            wireframe,
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
        materials: &[Material],
    ) -> usize {
        self.set_geometry_buffers(pass, mesh);

        let instances = &batch.instances;
        let mut local_offset = 0usize;
        let mut draw_calls = 0usize;

        while local_offset < instances.len() {
            let material_index = instances[local_offset].material_index as usize;
            let Some(material) = materials.get(material_index) else {
                log::warn!(
                    "Material index {} out of bounds ({} materials)",
                    material_index,
                    materials.len()
                );
                local_offset += 1;
                continue;
            };
            let Some(bind_group) = self.texture_binder.bind_group_for_material(
                &self.context.device,
                assets,
                *material,
            ) else {
                local_offset += 1;
                continue;
            };

            let run_length = material_run_length(instances, local_offset);
            let start_instance = batch.first_instance + local_offset as u32;
            let end_instance = start_instance + run_length as u32;

            pass.set_bind_group(3, bind_group, &[]);
            pass.draw_indexed(0..mesh.index_count(), 0, start_instance..end_instance);

            local_offset += run_length;
            draw_calls += 1;
        }
        draw_calls
    }

    fn set_geometry_buffers(&self, pass: &mut wgpu::RenderPass<'_>, mesh: &Mesh) {
        pass.set_vertex_buffer(0, mesh.vertex_buffer().slice(..));
        pass.set_index_buffer(mesh.index_buffer().slice(..), mesh.index_format());
    }

    fn draw_environment_background(&self, pass: &mut wgpu::RenderPass<'_>, write_pick: bool) {
        pass.set_pipeline(self.pipeline.background(write_pick));
        pass.set_bind_group(0, &self.camera_buffer.bind_group, &[]);
        pass.set_bind_group(1, &self.lights_buffer.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

fn material_run_length(instances: &[InstanceData], start: usize) -> usize {
    let material = instances[start].material_index;
    let mut length = 1usize;
    while start + length < instances.len() && instances[start + length].material_index == material {
        length += 1;
    }
    length
}

fn estimate_shadow_draw_calls(
    batches: &[OrderedBatch],
    materials: &[Material],
    lights: &LightsData,
) -> u32 {
    if batches.is_empty() {
        return 0;
    }

    let per_pass_draws: u32 = batches
        .iter()
        .map(|batch| count_shadow_draws_for_batch(batch, materials))
        .sum();

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

fn count_shadow_draws_for_batch(batch: &OrderedBatch, materials: &[Material]) -> u32 {
    if matches!(
        batch.pass,
        RenderPass::Transparent | RenderPass::Overlay | RenderPass::Gizmo | RenderPass::GizmoSolid
    ) {
        return 0;
    }

    let mut draws = 0u32;
    let mut run_active = false;

    for instance in &batch.instances {
        let material_index = instance.material_index as usize;
        let Some(material) = materials.get(material_index) else {
            log::warn!(
                "Material index {} out of bounds while counting shadows ({} materials)",
                material_index,
                materials.len()
            );
            if run_active {
                draws += 1;
                run_active = false;
            }
            continue;
        };
        if material.is_unlit() {
            if run_active {
                draws += 1;
                run_active = false;
            }
        } else if !run_active {
            run_active = true;
        }
    }

    if run_active {
        draws += 1;
    }

    draws
}

fn mesh_for_batch<'a>(assets: &'a Assets, batch: &OrderedBatch) -> Option<&'a Mesh> {
    let mesh = assets.meshes.get(batch.mesh);
    if mesh.is_none() {
        log::warn!("Skipping batch with invalid mesh handle");
    }
    mesh
}
