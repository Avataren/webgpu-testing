// renderer/renderer.rs
use crate::asset::{Assets, Handle, MaterialAsset, Mesh};
use crate::environment::Environment;
#[path = "passes/mod.rs"]
pub(crate) mod passes;
use crate::renderer::internal::shadows::ShadowInvocation;
use crate::renderer::internal::{
    CameraBuffer, DynamicObjectsBuffer, EnvironmentResources, LightsBuffer, MaterialPipelineKey,
    OrderedBatch, PipelineKey, PreparedBatches, RenderContext, RenderPipeline, ShadowResources,
    TextureBindingModel,
};
use crate::renderer::{
    lights::{MAX_DIRECTIONAL_LIGHTS, MAX_POINT_LIGHTS, MAX_SPOT_LIGHTS},
    postprocess::{PostProcess, PostProcessCamera, PostProcessSettings},
    CameraUniform, CustomRenderRequest, CustomRenderStage, LightsData, Material, RenderBatcher,
    RenderPass, RenderRegion, SamplerFilterMode, Vertex,
};
use crate::scene::{Camera, CameraProjection, Scene};
use crate::settings::RenderSettings;

use glam::{Mat4, Vec2, Vec3};
use std::mem::size_of;
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

    fn poll(&mut self, device: &wgpu::Device) -> Option<Result<u64, Box<dyn std::error::Error>>> {
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

        result.map(|status| {
            let value = status.map_err(Into::into).and_then(|()| {
                let slice = self.buffer.slice(..size_of::<u64>() as u64);
                let data = slice.get_mapped_range()?;
                let mut bytes = [0u8; 8];
                bytes.copy_from_slice(&data[..8]);
                Ok(u64::from_le_bytes(bytes))
            });
            self.buffer.unmap();
            self.mapping_requested = false;
            value
        })
    }
}
/// Why a frame could not be acquired from the surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceError {
    /// Acquiring the next frame timed out; try again next frame.
    Timeout,
    /// The window is occluded (e.g. minimized); skip rendering until it is visible.
    Occluded,
    /// The surface configuration no longer matches; reconfigure and retry.
    Outdated,
    /// The surface was lost; reconfigure (or recreate) and retry.
    Lost,
    /// A validation error was raised while acquiring the frame.
    Validation,
}

impl std::fmt::Display for SurfaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::Timeout => "timed out acquiring the next surface texture",
            Self::Occluded => "the surface is occluded",
            Self::Outdated => "the surface configuration is outdated",
            Self::Lost => "the surface was lost",
            Self::Validation => "validation error acquiring the surface texture",
        };
        f.write_str(message)
    }
}

impl std::error::Error for SurfaceError {}

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
    /// In HDR mode, whether the present pass has already written this frame's surface.
    pub(crate) output_resolved: std::cell::Cell<bool>,
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

struct FrameState {
    prepared_batches: PreparedBatches,
    encoder: wgpu::CommandEncoder,
    stats: RendererStats,
    render_region: Option<RenderRegion>,
    pick_active: bool,
    surface_view: wgpu::TextureView,
    /// Where egui draws: the surface, or the HDR UI layer.
    #[cfg_attr(not(feature = "egui"), allow(dead_code))]
    ui_view: wgpu::TextureView,
    scene_view: wgpu::TextureView,
    scene_resolve_view: Option<wgpu::TextureView>,
    depth_view: wgpu::TextureView,
    resolved_depth_view: Option<wgpu::TextureView>,
    shadow_invocations: Vec<ShadowInvocation>,
}

pub struct Renderer {
    // CRITICAL: context must be first so it drops LAST (after all GPU resources)
    // The Device in context must outlive all buffers, textures, pipelines, etc.
    context: RenderContext,

    // GPU resources that depend on the Device (must drop before context)
    texture_binder: TextureBindingModel,
    objects_buffer: DynamicObjectsBuffer,
    camera_buffer: CameraBuffer,
    lights_buffer: LightsBuffer,
    environment: EnvironmentResources,
    shadows: ShadowResources,
    postprocess: PostProcess,
    pipeline: RenderPipeline,
    /// Composes and presents frames in HDR mode; `None` for plain SDR output.
    present_stage: Option<crate::renderer::hdr_output::PresentStage>,

    // Non-GPU state (no Drop impl, drop order doesn't matter)
    camera_position: Vec3,
    camera_target: Vec3,
    camera_up: Vec3,
    camera_projection: CameraProjection,
    camera_fov_y: f32,
    camera_aspect: f32,
    /// Whether editor-only overlays (the viewport grid) are drawn this frame. Off while a
    /// screenshot renders through a game camera.
    editor_overlays: bool,
    settings: RenderSettings,
    pick_active: bool,
    pick_state: PickState,
    #[cfg(feature = "egui")]
    ui_hook: Option<UiHook>,
    stats: RendererStats,
    render_region: Option<RenderRegion>,
    /// Last frame's unjittered view-projection, for motion vectors.
    prev_view_proj: Option<Mat4>,
    /// Position in the TAA jitter sequence.
    jitter_index: u32,
}

impl Renderer {
    pub async fn new(window: Arc<Window>, settings: RenderSettings) -> Self {
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
        let hdr_display = context.hdr_display;
        let present_stage = hdr_display.is_hdr_path().then(|| {
            crate::renderer::hdr_output::PresentStage::new(
                &context.device,
                &context.queue,
                context.config.format,
                context.config.width,
                context.config.height,
                &hdr_display,
            )
        });
        // Post-processing writes the composed image when presenting goes through HDR.
        let mut post_config = context.config.clone();
        if present_stage.is_some() {
            post_config.format = crate::renderer::hdr_output::PRESENT_SOURCE_FORMAT;
        }
        let mut postprocess =
            PostProcess::new(&context.device, &context.queue, &post_config, sample_count);
        postprocess.set_display_peak(&context.queue, hdr_display.peak_ratio());
        postprocess.set_depth_view(&context.depth.sampled_view);

        Self {
            context,
            pipeline,
            present_stage,
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
            editor_overlays: true,
            settings,
            pick_active: false,
            pick_state: PickState::default(),
            #[cfg(feature = "egui")]
            ui_hook: None,
            stats: RendererStats::default(),
            render_region: None,
            prev_view_proj: None,
            jitter_index: 0,
        }
    }

    // Setter to install the per-frame hook (only compiled with egui feature)
    #[cfg(feature = "egui")]
    pub fn set_ui_hook(&mut self, hook: UiHook) {
        self.ui_hook = Some(hook);
    }

    /// The part of the surface the scene was last drawn into, if it is not all of it.
    pub fn render_region(&self) -> Option<RenderRegion> {
        self.render_region
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

    pub fn invalidate_material_shader_modules(
        &mut self,
        handle: Handle<MaterialAsset>,
        filter: Option<SamplerFilterMode>,
    ) {
        self.pipeline
            .invalidate_material_shader_modules(handle, filter);
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

    /// Presents a rendered frame to the window.
    pub fn present(&self, frame: RenderFrame) {
        self.resolve_output(&frame);
        self.context.queue.present(frame.frame);
    }

    /// In HDR mode, composite the frame and its UI onto the surface (once per frame);
    /// screenshots call this before reading the surface. Nothing to do in SDR mode.
    pub fn resolve_output(&self, frame: &RenderFrame) {
        let Some(stage) = self.present_stage.as_ref() else {
            return;
        };
        if frame.output_resolved.replace(true) {
            return;
        }
        let view = frame
            .frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder =
            self.context
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("PresentHdrEncoder"),
                });
        stage.present(&mut encoder, &view);
        self.context.queue.submit(Some(encoder.finish()));
    }

    /// How frames reach the display.
    pub fn hdr_display(&self) -> crate::renderer::hdr_output::HdrDisplay {
        self.context.hdr_display
    }

    /// Format of what the scene, overlays and after-post-processing passes draw into:
    /// the surface, or the HDR composition texture.
    pub fn output_format(&self) -> wgpu::TextureFormat {
        if self.present_stage.is_some() {
            crate::renderer::hdr_output::PRESENT_SOURCE_FORMAT
        } else {
            self.context.config.format
        }
    }

    pub fn get_device(&self) -> &wgpu::Device {
        &self.context.device
    }

    pub fn get_queue(&self) -> &wgpu::Queue {
        &self.context.queue
    }

    pub(crate) fn pipeline_for_material(
        &mut self,
        assets: &Assets,
        key: PipelineKey,
        material_key: MaterialPipelineKey,
    ) -> &wgpu::RenderPipeline {
        self.pipeline
            .pipeline_for_material(&self.context.device, assets, key, material_key)
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
        if let Some(stage) = self.present_stage.as_mut() {
            stage.resize(
                &self.context.device,
                self.context.config.width,
                self.context.config.height,
            );
        }
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
        let unjittered_vp = proj * view;
        let prev_vp = self
            .prev_view_proj
            .replace(unjittered_vp)
            .unwrap_or(unjittered_vp);
        // TAA: shift the projection by a different sub-pixel offset every frame.
        let jitter = if self.postprocess.settings().anti_aliasing
            == crate::renderer::postprocess::AntiAliasing::Taa
        {
            self.jitter_index = self.jitter_index.wrapping_add(1);
            let (width, height) = self
                .render_region
                .map(|region| (region.width(), region.height()))
                .unwrap_or((self.context.config.width, self.context.config.height));
            taa_jitter(self.jitter_index, width.max(1), height.max(1))
        } else {
            Vec2::ZERO
        };
        let vp = Mat4::from_translation(jitter.extend(0.0)) * unjittered_vp;
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
        let uni = CameraUniform::from_matrices(
            vp,
            inv_vp,
            camera_pos,
            forward,
            up,
            unjittered_vp,
            prev_vp,
        );
        self.context
            .queue
            .write_buffer(&self.camera_buffer.buffer, 0, bytemuck::bytes_of(&uni));
        let stable = CameraUniform::from_matrices(
            unjittered_vp,
            unjittered_vp.inverse(),
            camera_pos,
            forward,
            up,
            unjittered_vp,
            prev_vp,
        );
        self.context.queue.write_buffer(
            &self.camera_buffer.stable_buffer,
            0,
            bytemuck::bytes_of(&stable),
        );
        self.postprocess.update_camera(
            &self.context.queue,
            PostProcessCamera {
                proj,
                view,
                view_inv,
                view_proj: unjittered_vp,
                view_proj_inv: unjittered_vp.inverse(),
                position: camera.position(),
                near: camera.near(),
                far: camera.far(),
            }
            .with_projection_jitter(jitter),
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

    pub fn editor_overlays(&self) -> bool {
        self.editor_overlays
    }

    pub fn set_editor_overlays(&mut self, visible: bool) {
        self.editor_overlays = visible;
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

    #[allow(clippy::needless_option_as_deref)]
    pub fn render<'scene, 'custom>(
        &mut self,
        scene: &'scene Scene,
        assets: &'scene Assets,
        batcher: &RenderBatcher,
        lights: LightsData,
        environment: &'scene Environment,
        custom_render: Option<&'custom mut CustomRenderRequest<'custom>>,
    ) -> Result<RenderFrame, SurfaceError>
    where
        'scene: 'custom,
    {
        let frame = match self.context.surface.get_current_texture() {
            // A suboptimal frame is still presentable; the next resize reconfigures.
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Timeout => return Err(SurfaceError::Timeout),
            wgpu::CurrentSurfaceTexture::Occluded => return Err(SurfaceError::Occluded),
            wgpu::CurrentSurfaceTexture::Outdated => return Err(SurfaceError::Outdated),
            wgpu::CurrentSurfaceTexture::Lost => return Err(SurfaceError::Lost),
            wgpu::CurrentSurfaceTexture::Validation => return Err(SurfaceError::Validation),
        };
        let surface_view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut custom_render = custom_render;
        let custom_shadow_enabled = custom_render
            .as_ref()
            .is_some_and(|request| request.render_in_shadow_pass);

        // In HDR mode everything meant for the surface goes to the composition texture,
        // and egui to its own layer; the present pass combines them.
        let (target_view, ui_view) = match self.present_stage.as_ref() {
            Some(stage) => (stage.scene_view().clone(), stage.ui_view().clone()),
            None => (surface_view.clone(), surface_view),
        };
        let mut frame_state =
            self.prepare_frame_state(batcher, environment, &lights, target_view, ui_view)?;
        if let Some(stage) = self.present_stage.as_ref() {
            stage.clear_ui(&mut frame_state.encoder);
        }

        self.run_shadow_pass(
            &mut frame_state,
            scene,
            assets,
            &lights,
            custom_render.as_deref_mut(),
            custom_shadow_enabled,
        );
        self.run_depth_prepass(&mut frame_state, assets);
        self.run_main_color_pass(&mut frame_state, environment, assets);
        self.run_custom_stage(
            &mut frame_state,
            scene,
            CustomRenderStage::BeforePostprocess,
            custom_render.as_deref_mut(),
        );
        // Transparent surfaces are part of the lit HDR scene, so they get exposure,
        // bloom and tonemapping like everything else.
        self.run_transparent_pass(&mut frame_state, assets);
        self.execute_postprocess(&mut frame_state);
        self.run_custom_stage(
            &mut frame_state,
            scene,
            CustomRenderStage::AfterPostprocess,
            custom_render.as_deref_mut(),
        );
        self.run_surface_passes(&mut frame_state, assets);
        self.handle_pick_requests(&mut frame_state);
        self.run_ui_hook(&mut frame_state);
        self.finalize_stats(&mut frame_state, &lights);

        let FrameState { encoder, stats, .. } = frame_state;
        self.stats = stats;

        Ok(passes::finish_frame(self, encoder, frame))
    }

    pub fn surface_format(&self) -> wgpu::TextureFormat {
        self.context.config.format
    }

    /// Format UI overlays (egui) should render with: the surface format without its
    /// sRGB suffix. Use [`Self::ui_view`] to get a matching view of a surface texture.
    pub fn ui_format(&self) -> wgpu::TextureFormat {
        if self.present_stage.is_some() {
            return crate::renderer::hdr_output::UI_LAYER_FORMAT;
        }
        self.context.config.format.remove_srgb_suffix()
    }

    /// A view of `surface_texture` in [`Self::ui_format`].
    pub fn ui_view(&self, surface_texture: &wgpu::Texture) -> wgpu::TextureView {
        if let Some(stage) = self.present_stage.as_ref() {
            return stage.ui_view().clone();
        }
        surface_texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("ui_surface_view"),
            format: Some(self.ui_format()),
            ..Default::default()
        })
    }

    pub fn scene_texture_format(&self) -> wgpu::TextureFormat {
        self.context.scene_texture_format()
    }

    pub fn color_format_for_stage(&self, stage: CustomRenderStage) -> wgpu::TextureFormat {
        match stage {
            CustomRenderStage::BeforePostprocess => self.scene_texture_format(),
            CustomRenderStage::AfterPostprocess => self.output_format(),
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

    /// Post-processing for the next frames, usually the scene's volumes blended at the
    /// camera (see [`crate::scene::PostProcessVolume`]).
    pub fn set_postprocess_settings(&mut self, settings: &PostProcessSettings) {
        self.postprocess
            .set_settings(&self.context.device, &self.context.queue, settings);
    }

    /// The post-processing settings in effect (after clamping).
    pub fn postprocess_settings(&self) -> &PostProcessSettings {
        self.postprocess.settings()
    }

    /// Make auto exposure measure the next frame afresh instead of easing from the last
    /// adapted value.
    pub fn reset_auto_exposure(&self) {
        self.postprocess.reset_exposure(&self.context.queue);
    }

    pub fn last_frame_stats(&self) -> RendererStats {
        self.stats
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

    fn draw_full_batch(&self, pass: &mut wgpu::RenderPass<'_>, mesh: &Mesh, batch: &OrderedBatch) {
        self.set_geometry_buffers(pass, mesh);
        let instance_count = batch.instances.len() as u32;
        pass.draw_indexed(
            0..mesh.index_count(),
            0,
            batch.first_instance..(batch.first_instance + instance_count),
        );
    }

    fn material_bind_group(
        &mut self,
        assets: &Assets,
        material: Material,
    ) -> Option<&wgpu::BindGroup> {
        self.texture_binder
            .bind_group_for_material(&self.context.device, assets, material)
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

    fn prepare_frame_state(
        &mut self,
        batcher: &RenderBatcher,
        environment: &Environment,
        lights: &LightsData,
        surface_view: wgpu::TextureView,
        ui_view: wgpu::TextureView,
    ) -> Result<FrameState, SurfaceError> {
        let encoder = self
            .context
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Encoder"),
            });

        let prepared_batches = PreparedBatches::from_batcher(batcher, self.camera_position);
        let batch_count = prepared_batches.all().len() as u32;
        let instance_count = prepared_batches
            .all()
            .iter()
            .map(|batch| batch.instances.len() as u32)
            .sum();
        let stats = RendererStats {
            batch_count,
            instance_count,
            ..RendererStats::default()
        };

        let env_texture_changed =
            self.environment
                .update(&self.context.device, &self.context.queue, environment);

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

        let render_region = self
            .render_region
            .or_else(|| RenderRegion::full(self.context.config.width, self.context.config.height));

        self.postprocess
            .update_viewport(&self.context.queue, render_region);

        let pick_active = self.pick_active;
        if pick_active {
            self.postprocess
                .ensure_pick_attachment(&self.context.device);
        }

        let (scene_view, scene_resolve_view) = {
            let (view, resolve) = self.postprocess.scene_color_views();
            (view.clone(), resolve.cloned())
        };

        Ok(FrameState {
            prepared_batches,
            encoder,
            stats,
            render_region,
            pick_active,
            surface_view,
            ui_view,
            scene_view,
            scene_resolve_view,
            depth_view: self.context.depth.view.clone(),
            resolved_depth_view: None,
            shadow_invocations: Vec::new(),
        })
    }

    fn run_shadow_pass(
        &mut self,
        frame: &mut FrameState,
        scene: &Scene,
        assets: &Assets,
        lights: &LightsData,
        custom_render: Option<&mut CustomRenderRequest<'_>>,
        custom_shadow_enabled: bool,
    ) {
        passes::shadow_pass(
            self,
            scene,
            assets,
            &frame.prepared_batches,
            lights,
            &mut frame.encoder,
            custom_render,
            custom_shadow_enabled,
            &mut frame.shadow_invocations,
        );
    }

    fn run_depth_prepass(&mut self, frame: &mut FrameState, assets: &Assets) {
        let draw_calls = passes::depth_prepass(
            self,
            assets,
            &mut frame.prepared_batches,
            &mut frame.encoder,
            &frame.depth_view,
            frame.render_region,
        );
        frame.stats.depth_prepass_draw_calls += draw_calls;
    }

    fn run_main_color_pass(
        &mut self,
        frame: &mut FrameState,
        environment: &Environment,
        assets: &Assets,
    ) {
        let draw_calls = passes::main_color_pass(
            self,
            environment,
            assets,
            &frame.prepared_batches,
            &mut frame.encoder,
            &frame.scene_view,
            frame.scene_resolve_view.as_ref(),
            &frame.depth_view,
            frame.render_region,
            self.context.scene_texture_format(),
            frame.pick_active,
        );
        frame.stats.opaque_draw_calls += draw_calls;
    }

    fn run_custom_stage(
        &mut self,
        frame: &mut FrameState,
        scene: &Scene,
        stage: CustomRenderStage,
        request: Option<&mut CustomRenderRequest<'_>>,
    ) {
        let (color_view, depth_view) = match stage {
            CustomRenderStage::BeforePostprocess => {
                (frame.scene_view.clone(), frame.depth_view.clone())
            }
            CustomRenderStage::AfterPostprocess => (
                frame.surface_view.clone(),
                frame
                    .resolved_depth_view
                    .as_ref()
                    .cloned()
                    .unwrap_or_else(|| frame.depth_view.clone()),
            ),
            CustomRenderStage::Shadow(_) => return,
        };

        passes::custom_pass(
            self,
            scene,
            &mut frame.encoder,
            &color_view,
            &depth_view,
            stage,
            frame.render_region,
            request,
        );
    }

    fn execute_postprocess(&mut self, frame: &mut FrameState) {
        self.postprocess.execute(
            &mut frame.encoder,
            &self.context.device,
            &frame.surface_view,
            frame.render_region,
        );
        frame.resolved_depth_view = self.postprocess.after_postprocess_depth_view().cloned();
    }

    fn run_transparent_pass(&mut self, frame: &mut FrameState, assets: &Assets) {
        let transparent_draw_calls = passes::transparent_pass(
            self,
            assets,
            &frame.prepared_batches,
            &mut frame.encoder,
            &frame.scene_view,
            frame.scene_resolve_view.as_ref(),
            &frame.depth_view,
            frame.render_region,
            frame.pick_active,
            self.context.scene_texture_format(),
        );
        frame.stats.transparent_draw_calls += transparent_draw_calls;
    }

    /// Editor overlays and gizmos, drawn on the finished (tonemapped) image.
    fn run_surface_passes(&mut self, frame: &mut FrameState, assets: &Assets) {
        let render_region = frame.render_region;
        let pick_active = frame.pick_active;
        let surface_format = self.output_format();
        let surface_view = frame.surface_view.clone();
        let depth_view = frame.depth_view.clone();

        let overlay_draw_calls = passes::overlay_pass(
            self,
            assets,
            &frame.prepared_batches,
            &mut frame.encoder,
            &surface_view,
            &depth_view,
            render_region,
            pick_active,
            surface_format,
        );
        frame.stats.overlay_draw_calls += overlay_draw_calls;

        let prepared_batches = &frame.prepared_batches;
        let materials = prepared_batches.materials();
        let material_handles = prepared_batches.material_handles();
        let material_pipeline_keys = prepared_batches.material_pipeline_keys();

        let gizmo_draws = passes::gizmo_pass(
            self,
            assets,
            prepared_batches.gizmos(),
            materials,
            material_handles,
            material_pipeline_keys,
            &mut frame.encoder,
            &surface_view,
            render_region,
            pick_active,
            surface_format,
            "GizmoPass",
        );
        let gizmo_solid_draws = passes::gizmo_pass(
            self,
            assets,
            prepared_batches.gizmo_solids(),
            materials,
            material_handles,
            material_pipeline_keys,
            &mut frame.encoder,
            &surface_view,
            render_region,
            pick_active,
            surface_format,
            "GizmoSolidPass",
        );
        frame.stats.gizmo_draw_calls += gizmo_draws + gizmo_solid_draws;
    }

    fn handle_pick_requests(&mut self, frame: &mut FrameState) {
        passes::process_pick(self, &mut frame.encoder);
    }

    #[cfg(feature = "egui")]
    fn run_ui_hook(&mut self, frame: &mut FrameState) {
        if let Some(hook) = self.ui_hook.take() {
            hook(
                &self.context.device,
                &self.context.queue,
                &mut frame.encoder,
                &frame.ui_view,
            );
        }
    }

    #[cfg(not(feature = "egui"))]
    fn run_ui_hook(&mut self, _frame: &mut FrameState) {}

    fn finalize_stats(&mut self, frame: &mut FrameState, lights: &LightsData) {
        let prepared_batches = &frame.prepared_batches;
        frame.stats.shadow_draw_calls = estimate_shadow_draw_calls(
            prepared_batches.all(),
            prepared_batches.materials(),
            lights,
        );
    }
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

/// Clip-space offset for TAA frame `index`: a Halton(2, 3) point inside the pixel,
/// converted to NDC for a `width` x `height` view.
fn taa_jitter(index: u32, width: u32, height: u32) -> Vec2 {
    // 8 points cover the pixel evenly; longer sequences only add noise at 0.1 blend.
    let sample = index % 8 + 1;
    let offset = Vec2::new(halton(sample, 2), halton(sample, 3)) - Vec2::splat(0.5);
    Vec2::new(
        offset.x * 2.0 / width as f32,
        offset.y * 2.0 / height as f32,
    )
}

fn halton(mut index: u32, base: u32) -> f32 {
    let mut result = 0.0;
    let mut fraction = 1.0;
    while index > 0 {
        fraction /= base as f32;
        result += fraction * (index % base) as f32;
        index /= base;
    }
    result
}

#[cfg(test)]
mod taa_tests {
    use super::*;

    #[test]
    fn jitter_stays_within_half_a_pixel() {
        for index in 0..32 {
            let jitter = taa_jitter(index, 100, 50);
            assert!(jitter.x.abs() <= 1.0 / 100.0 + 1e-6, "{jitter:?}");
            assert!(jitter.y.abs() <= 1.0 / 50.0 + 1e-6, "{jitter:?}");
        }
        assert_eq!(halton(1, 2), 0.5);
        assert_eq!(halton(2, 3), 2.0 / 3.0);
    }
}
