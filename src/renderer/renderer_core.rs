// renderer/renderer.rs
use crate::asset::{Assets, Handle, MaterialAsset, Mesh};
use crate::environment::Environment;
#[path = "passes/mod.rs"]
pub(crate) mod passes;
use crate::renderer::internal::culling::Frustum;
use crate::renderer::internal::pick::PickState;
use crate::renderer::internal::shadows::ShadowInvocation;
use crate::renderer::internal::{
    CameraBuffer, DynamicObjectsBuffer, EnvironmentResources, LightsBuffer, MaterialPipelineKey,
    PipelineKey, PreparedBatches, RenderContext, RenderPipeline, ShadowResources,
    TextureBindingModel,
};
use crate::renderer::{
    postprocess::{PostProcess, PostProcessCamera, PostProcessSettings, TracedLighting},
    CameraUniform, CustomRenderRequest, CustomRenderStage, LightsData, Material, RenderBatcher,
    RenderRegion, SamplerFilterMode, Vertex,
};
use crate::scene::{Camera, CameraProjection, Scene};
use crate::settings::RenderSettings;

use glam::{Mat4, Vec2, Vec3};
use std::sync::Arc;
use winit::{dpi::PhysicalSize, window::Window};

const INITIAL_OBJECTS_CAPACITY: u32 = 1024 * 100;

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
    /// Shadow views (one per directional or spot light, six per point light) rendered
    /// this frame, and those kept from an earlier frame because nothing they show
    /// changed.
    pub shadow_views_rendered: u32,
    pub shadow_views_cached: u32,
    /// Instances outside the camera's view this frame, left out of the scene passes.
    pub culled_instances: u32,
    /// The pick pass's, on frames that resolve a pick.
    pub pick_draw_calls: u32,
    /// Skinned mesh instances deformed this frame, and their vertices.
    pub skinned_instances: u32,
    pub skinned_vertices: u32,
    /// Acceleration structure counters when ray-traced effects ran this frame.
    pub ray_tracing: Option<crate::renderer::raytracing::RayTracingStats>,
    /// The frame's lighting was path traced.
    pub path_traced: bool,
    /// The frame's indirect diffuse light was ray traced (global illumination).
    pub global_illumination: bool,
}

impl RendererStats {
    pub fn total_draw_calls(&self) -> u32 {
        self.depth_prepass_draw_calls
            + self.opaque_draw_calls
            + self.transparent_draw_calls
            + self.overlay_draw_calls
            + self.gizmo_draw_calls
            + self.shadow_draw_calls
            + self.pick_draw_calls
    }
}

struct FrameState {
    prepared_batches: PreparedBatches,
    encoder: wgpu::CommandEncoder,
    stats: RendererStats,
    /// The part of the output the frame fills.
    render_region: Option<RenderRegion>,
    /// The part of the scene targets the 3D passes draw into: `render_region`, or a
    /// smaller rectangle at its origin while upscaling.
    scene_region: Option<RenderRegion>,
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

/// What a frame runs, decided once before its passes are recorded (and `ray_traced`
/// once the acceleration structures are built). Later passes read these instead of
/// re-deriving them, or leaving flags for each other.
#[derive(Clone, Copy, Debug, Default)]
struct FramePlan {
    /// Post-processing traces rays: reflections, ambient occlusion or path tracing.
    post_ray_tracing: bool,
    /// Directional shadows come from the ray-traced mask (single-sample targets only).
    rt_shadows: bool,
    /// Glass is on screen and refracted with ray tracing (it was batched as opaque).
    refraction: bool,
    /// The opaque lighting is path traced.
    path_tracing: bool,
    /// The opaque surfaces' indirect diffuse light is ray traced (global illumination).
    global_illumination: bool,
    /// Nothing but the main pass's opaque PBR surfaces reads the shadow maps: no
    /// transparent surfaces, shader materials, custom shadow passes or lit particles.
    /// Maps the ray-traced mask replaces can then be skipped.
    only_main_pass_reads_shadow_maps: bool,
    /// The light probe grid traces its update.
    light_probes: bool,
    /// The acceleration structures for this frame are built.
    ray_traced: bool,
    /// Ray-traced reflections and refraction see GPU particle systems.
    particles_traced: bool,
}

impl FramePlan {
    fn needs_acceleration_structures(&self) -> bool {
        self.post_ray_tracing || self.rt_shadows || self.refraction || self.light_probes
    }

    fn trace_rt_shadows(&self) -> bool {
        self.ray_traced && self.rt_shadows
    }

    fn path_trace(&self) -> bool {
        self.ray_traced && self.path_tracing
    }

    fn trace_global_illumination(&self) -> bool {
        self.ray_traced && self.global_illumination
    }

    fn may_skip_shadow_maps(&self) -> bool {
        self.trace_rt_shadows() && self.only_main_pass_reads_shadow_maps
    }
}

/// What [`Renderer::reload_shaders`] did.
#[derive(Debug, Clone, Default)]
pub struct ShaderReload {
    /// Shader files (relative to `src/shader`) whose text changed.
    pub changed: Vec<String>,
    /// Changed files no rebuilt pass uses: they apply after a restart.
    pub not_reloaded: Vec<String>,
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
    light_probes: crate::renderer::light_probes::LightProbes,
    environment: EnvironmentResources,
    shadows: ShadowResources,
    postprocess: PostProcess,
    pipeline: RenderPipeline,
    skinning: crate::renderer::skinning::SkinningStage,
    /// Acceleration structures for ray-traced effects; `None` without hardware ray
    /// tracing (or when its passes failed to build).
    ray_tracing: Option<crate::renderer::raytracing::RayTracingScene>,
    /// GPU particle systems rays see, as last handed in.
    ray_traced_particles: Vec<crate::renderer::raytracing::RayTracedParticles>,
    /// Ray-traced directional shadows; `None` without hardware ray tracing (or when the
    /// pass failed to build).
    rt_shadows: Option<crate::renderer::raytracing::RtShadowStage>,
    /// Whether something outside the renderer took the lights group (and so may read the
    /// shadow maps) since the last frame started.
    lights_group_shared: std::cell::Cell<bool>,
    /// Composes and presents frames in HDR mode; `None` for plain SDR output.
    present_stage: Option<crate::renderer::hdr_output::PresentStage>,
    gpu_timer: crate::renderer::gpu_timer::GpuTimer,
    /// Screen-space gizmo shapes drawn over the editor overlays.
    gizmo_shapes: crate::renderer::gizmo_shapes::GizmoShapeRenderer,
    debug_draw: crate::renderer::debug_draw::DebugDrawStage,
    /// The light probe debug view, drawn with the editor overlays.
    light_probe_debug: crate::renderer::light_probe_debug::LightProbeDebugStage,
    /// How the last light probe bake ended, as the app reported it.
    light_probe_bake_outcome: Option<crate::renderer::LightProbeBakeOutcome>,

    // Non-GPU state (no Drop impl, drop order doesn't matter)
    camera_position: Vec3,
    /// The scene camera's clip planes (with this frame's TAA jitter), for culling.
    camera_frustum: Option<Frustum>,
    /// Counts the texture set's updates ([`Self::update_texture_bind_group`]), so passes
    /// that cache work depending on texture contents (cut-out shadows) notice a change.
    texture_generation: u64,
    camera_target: Vec3,
    camera_up: Vec3,
    camera_projection: CameraProjection,
    camera_fov_y: f32,
    camera_aspect: f32,
    /// Whether editor-only overlays (the viewport grid) are drawn this frame. Off while a
    /// screenshot renders through a game camera.
    editor_overlays: bool,
    settings: RenderSettings,
    /// GPU picking: requests, the pick pass's target and the readback.
    pick_state: PickState,
    #[cfg(feature = "egui")]
    ui_hook: Option<UiHook>,
    stats: RendererStats,
    render_region: Option<RenderRegion>,
    /// Last rendered frame's unjittered view-projection, for motion vectors.
    prev_view_proj: Option<Mat4>,
    /// This frame's, set by `set_camera`; it becomes `prev_view_proj` once rendered.
    pending_view_proj: Option<Mat4>,
    /// Position in the TAA jitter sequence.
    jitter_index: u32,
    /// Hold every per-frame noise source still (TAA jitter, grain and dither phase, live
    /// light probe updates), so an unchanged view renders the same image every frame:
    /// for captures compared across builds. See [`Self::set_frozen_noise`].
    frozen_noise: bool,
    /// Whether ray-traced effects can run, or why not.
    ray_tracing_support: crate::renderer::raytracing::RayTracingSupport,
    /// The last frames' scene submissions, oldest first, for [`Self::wait_for_frame_slot`].
    frames_in_flight: std::collections::VecDeque<wgpu::SubmissionIndex>,
}

/// Frames the CPU may have submitted that the GPU has not finished, when a new one starts.
/// Two keeps the GPU busy (it draws one frame while the CPU prepares the next) without
/// letting the CPU run further ahead.
const MAX_FRAMES_IN_FLIGHT: usize = 2;

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
        let mut light_probes = crate::renderer::light_probes::LightProbes::new(&context.device);
        let lights_buffer = LightsBuffer::new(
            &context.device,
            &context.queue,
            &shadows,
            &environment,
            &light_probes,
        );
        let (pipeline, texture_binder) = RenderPipeline::new(
            &context,
            &camera_buffer,
            &objects_buffer,
            &lights_buffer,
            sample_count,
        );
        let mut shadows = shadows;
        shadows.enable_alpha_masking(
            &context.device,
            &objects_buffer,
            texture_binder.bind_layout(),
            context.supports_bindless_textures,
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
        let skinning = crate::renderer::skinning::SkinningStage::new(&context.device);
        let gpu_timer = crate::renderer::gpu_timer::GpuTimer::new(&context.device, &context.queue);
        let mut postprocess =
            PostProcess::new(&context.device, &context.queue, &post_config, sample_count);
        postprocess.set_display_peak(&context.queue, hdr_display.peak_ratio());
        postprocess.set_depth_view(&context.depth.sampled_view);

        let mut ray_tracing_support = context.ray_tracing.clone();
        let ray_tracing = if ray_tracing_support.is_available() {
            let scene = crate::renderer::raytracing::RayTracingScene::new(&context.device);
            let textures_layout = ray_traced_textures_layout(&texture_binder, &context.device);
            if textures_layout.is_none() {
                log::info!("Ray-traced reflections shade hits without material textures");
            }
            match postprocess.enable_ray_traced_reflections(
                &context.device,
                &lights_buffer.bind_layout,
                scene.layout(),
                textures_layout,
            ) {
                Ok(()) => Some(scene),
                Err(error) => {
                    log::warn!("Ray-traced reflections unavailable, using screen space: {error}");
                    ray_tracing_support =
                        crate::renderer::raytracing::RayTracingSupport::Unavailable(format!(
                            "reflection pass failed to build: {error}"
                        ));
                    None
                }
            }
        } else {
            None
        };
        if let Some(scene) = ray_tracing.as_ref() {
            if let Err(error) = postprocess.enable_ray_traced_ao(
                &context.device,
                scene.layout(),
                cutout_textures_layout(&texture_binder),
            ) {
                log::warn!("Ray-traced ambient occlusion unavailable, using screen space: {error}");
            }
            let textures_layout = ray_traced_textures_layout(&texture_binder, &context.device);
            if let Err(error) = postprocess.enable_path_tracing(
                &context.device,
                &lights_buffer.bind_layout,
                scene.layout(),
                textures_layout,
            ) {
                log::warn!("Path tracing unavailable: {error}");
            }
            if let Err(error) = light_probes.enable_ray_tracing(
                &context.device,
                &lights_buffer.bind_layout,
                scene.layout(),
                textures_layout,
            ) {
                log::warn!("Light probes cannot update by ray tracing: {error}");
            }
        }
        let rt_shadows = ray_tracing.as_ref().and_then(|scene| {
            crate::renderer::raytracing::RtShadowStage::new(
                &context.device,
                &camera_buffer.bind_layout,
                scene.layout(),
                cutout_textures_layout(&texture_binder),
            )
            .inspect_err(|error| {
                log::warn!("Ray-traced shadows unavailable, using shadow maps: {error}")
            })
            .ok()
        });

        Self {
            context,
            pipeline,
            present_stage,
            texture_binder,
            objects_buffer,
            camera_buffer,
            lights_buffer,
            light_probes,
            environment,
            shadows,
            postprocess,
            skinning,
            ray_tracing,
            ray_traced_particles: Vec::new(),
            rt_shadows,
            lights_group_shared: std::cell::Cell::new(false),
            gpu_timer,
            gizmo_shapes: crate::renderer::gizmo_shapes::GizmoShapeRenderer::new(),
            debug_draw: crate::renderer::debug_draw::DebugDrawStage::default(),
            light_probe_debug: crate::renderer::light_probe_debug::LightProbeDebugStage::new(),
            light_probe_bake_outcome: None,
            camera_position: Vec3::ZERO,
            camera_frustum: None,
            texture_generation: 0,
            camera_target: Vec3::ZERO,
            camera_up: Vec3::Y,
            camera_projection: CameraProjection::default(),
            camera_fov_y: 60f32.to_radians(),
            camera_aspect: aspect,
            editor_overlays: true,
            settings,
            pick_state: PickState::default(),
            #[cfg(feature = "egui")]
            ui_hook: None,
            stats: RendererStats::default(),
            render_region: None,
            prev_view_proj: None,
            pending_view_proj: None,
            jitter_index: 0,
            frozen_noise: false,
            ray_tracing_support,
            frames_in_flight: std::collections::VecDeque::new(),
        }
    }

    /// Call before a frame samples the clock and runs its update: waits until the GPU has
    /// finished all but the last frame. When the GPU is the bottleneck, the CPU then
    /// waits here, at the start of the frame, instead of for a swapchain image in the
    /// middle of it, after the game has already advanced to the time the frame shows. A
    /// wait there varies from frame to frame, so frames reached the display at uneven
    /// delays after the moment they show: judder that the frame rate did not reveal.
    pub fn wait_for_frame_slot(&mut self) {
        while self.frames_in_flight.len() >= MAX_FRAMES_IN_FLIGHT {
            let Some(index) = self.frames_in_flight.pop_front() else {
                break;
            };
            // WebGPU cannot block; the browser paces frames itself.
            #[cfg(not(target_arch = "wasm32"))]
            if let Err(err) = self.context.device.poll(wgpu::PollType::Wait {
                submission_index: Some(index),
                timeout: Some(std::time::Duration::from_secs(1)),
            }) {
                log::warn!("Waiting for the GPU to finish a frame failed: {err}");
            }
            #[cfg(target_arch = "wasm32")]
            let _ = index;
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

    /// How skinned meshes are deformed on this device.
    /// Whether transmissive materials (glass) are drawn as opaque surfaces whose refraction
    /// is ray traced, rather than see-through in the transparent pass. The batcher asks
    /// before each frame.
    pub fn ray_traced_transmission(&self) -> bool {
        self.ray_tracing.is_some() && self.postprocess.ray_traced_refraction()
    }

    /// The GPU particle systems ray-traced reflections and refraction see, replacing the
    /// last list: hand in every system that draws, each frame they change (after their
    /// simulation step, before the frame renders). Ignored without ray tracing.
    pub fn set_ray_traced_particles(
        &mut self,
        particles: Vec<crate::renderer::raytracing::RayTracedParticles>,
    ) {
        self.ray_traced_particles = particles;
    }

    /// GPU time per frame stage, from a frame or two ago; `None` without timestamp
    /// queries inside command encoders.
    pub fn gpu_timings(&self) -> Option<&crate::renderer::GpuTimings> {
        self.gpu_timer.timings()
    }

    pub fn skinning_backend(&self) -> crate::renderer::skinning::SkinningBackend {
        self.skinning.backend()
    }

    /// Whether hardware ray tracing is available for ray-traced effects, or why not.
    pub fn ray_tracing_support(&self) -> &crate::renderer::raytracing::RayTracingSupport {
        &self.ray_tracing_support
    }

    /// Read the WGSL sources from disk again (see [`crate::renderer::shader_source`]) and
    /// rebuild the passes that use them: the main pass (PBR, shader materials, depth
    /// prepass, background), every post-processing pass including the ray-traced ones and
    /// the path tracer, and ray-traced shadows. A shader that fails to compile leaves every
    /// pipeline as it was and returns the compiler's message; the previous sources stay in
    /// effect. Temporal histories start over.
    ///
    /// Shadow maps, skinning, particles, the environment and HDR presentation keep their
    /// pipelines until a restart; the result lists changed files that only they use.
    pub fn reload_shaders(&mut self) -> Result<ShaderReload, String> {
        use crate::renderer::shader_source;

        let staged = shader_source::stage_from_disk()?;
        let device = self.context.device.clone();
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let rebuilt = self.rebuild_shader_pipelines();
        let device_error = pollster::block_on(validation.pop());
        let rebuilt = match (rebuilt, device_error) {
            (Ok(rebuilt), None) => rebuilt,
            (Err(error), _) => {
                shader_source::restore(staged);
                return Err(error);
            }
            (Ok(_), Some(error)) => {
                shader_source::restore(staged);
                return Err(error.to_string());
            }
        };
        let used = shader_source::finish();
        let (main, post, rt_shadows) = rebuilt;
        self.pipeline.install_shaders(main);
        self.postprocess.install_pipelines(post);
        if let Some(stage) = rt_shadows {
            self.rt_shadows = Some(stage);
        }
        self.prev_view_proj = None;
        let not_reloaded = staged
            .changed
            .iter()
            .filter(|file| !used.contains(*file))
            .cloned()
            .collect();
        Ok(ShaderReload {
            changed: staged.changed,
            not_reloaded,
        })
    }

    #[allow(clippy::type_complexity)]
    fn rebuild_shader_pipelines(
        &self,
    ) -> Result<
        (
            crate::renderer::internal::pipeline::RebuiltShaders,
            crate::renderer::postprocess::RebuiltPostProcess,
            Option<crate::renderer::raytracing::RtShadowStage>,
        ),
        String,
    > {
        let context = &self.context;
        let main = self
            .pipeline
            .rebuild_shaders(
                context,
                &self.camera_buffer,
                &self.objects_buffer,
                &self.lights_buffer,
                context.sample_count,
            )
            .map_err(|error| format!("main pass: {error}"))?;
        let scene_layout = self.ray_tracing.as_ref().map(|scene| scene.layout());
        let textures = ray_traced_textures_layout(&self.texture_binder, &context.device);
        let ray_traced = scene_layout.map(|scene| crate::renderer::postprocess::RayTracedLayouts {
            lights: &self.lights_buffer.bind_layout,
            scene,
            textures,
            cutout_textures: cutout_textures_layout(&self.texture_binder),
        });
        let post = self
            .postprocess
            .rebuild_pipelines(&context.device, ray_traced.as_ref())?;
        let rt_shadows = match (&self.rt_shadows, scene_layout) {
            (Some(_), Some(scene)) => Some(
                crate::renderer::raytracing::RtShadowStage::new(
                    &context.device,
                    &self.camera_buffer.bind_layout,
                    scene,
                    cutout_textures_layout(&self.texture_binder),
                )
                .map_err(|error| format!("ray-traced shadows: {error}"))?,
            ),
            _ => None,
        };
        Ok((main, post, rt_shadows))
    }

    /// Every render target that can be read back by name, with what its channels hold
    /// (see [`crate::renderer::readback`]). Targets hold what the last frame left in them.
    pub fn fetchable_textures(&self) -> Vec<crate::renderer::readback::FetchableTexture<'_>> {
        use crate::renderer::readback::FetchableTexture;
        let mut out = Vec::new();
        if self.context.depth.texture.sample_count() == 1 {
            out.push(FetchableTexture::new(
                "depth",
                "Depth buffer (0 near to 1 far; perspective, so not linear in distance)",
                &self.context.depth.texture,
            ));
        }
        if let Some(mask) = self
            .rt_shadows
            .as_ref()
            .and_then(|stage| stage.mask_texture())
        {
            out.push(FetchableTexture::new(
                "rt_shadows.mask",
                "Ray-traced shadow mask: one bit per light a pixel's shadow ray reached (bits 0-3 \
                 directional, 4-7 point, 8-11 spot lights); only lights traced this frame count",
                mask,
            ));
        }
        if let Some(target) = self.pick_state.last_target() {
            out.push(FetchableTexture::new(
                "pick",
                "Entity pick ids from the last pick pass (the two halves of each pixel's pick \
                 id; 0 where nothing pickable), at output resolution",
                target.texture(),
            ));
        }
        self.shadows.fetchable_textures(&mut out);
        self.light_probes.fetchable_textures(&mut out);
        self.postprocess.fetchable_textures(&mut out);
        out
    }

    /// Copy `rect` ([x, y, width, height]) of array layer `layer` of `target` to the CPU,
    /// waiting for the GPU (native only).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn read_texture(
        &self,
        target: &crate::renderer::readback::FetchableTexture<'_>,
        rect: [u32; 4],
        layer: u32,
    ) -> Result<crate::renderer::readback::Readback, String> {
        crate::renderer::readback::read(
            &self.context.device,
            &self.context.queue,
            target,
            rect,
            layer,
        )
    }

    /// Whether lighting can be path traced (hardware ray tracing, no MSAA).
    pub fn path_tracing_available(&self) -> bool {
        self.ray_tracing.is_some() && self.postprocess.path_tracing_available()
    }

    pub fn supports_bindless_textures(&self) -> bool {
        self.context.supports_bindless_textures
    }

    pub fn invalidate_material_shader_modules(
        &mut self,
        handle: Handle<MaterialAsset>,
        filter: Option<SamplerFilterMode>,
    ) {
        self.pipeline
            .invalidate_material_shader_modules(handle, filter);
    }

    /// Pick the entity at output pixel `coords` with the next frame (a pick pass draws
    /// entity ids then). False while an earlier pick is still in flight; the result
    /// arrives through [`Self::poll_pick_result`].
    pub fn request_pick(&mut self, coords: [u32; 2]) -> bool {
        self.pick_state.request(coords)
    }

    /// The pick id under the last requested pixel (0: nothing pickable) once the GPU has
    /// it.
    pub fn poll_pick_result(&mut self) -> Option<u64> {
        self.pick_state.poll(&self.context.device)
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

    /// The lights group (lights, shadow maps, environment) for passes outside the
    /// renderer, such as lit GPU particles. Taking it keeps next frame's shadow maps
    /// rendered even where ray-traced shadows replace them for the main pass.
    pub fn lights_bind_group(&self) -> &wgpu::BindGroup {
        self.lights_group_shared.set(true);
        &self.lights_buffer.bind_group
    }

    pub fn textures_bind_layout(&self) -> &wgpu::BindGroupLayout {
        self.texture_binder.bind_layout()
    }

    /// The one bind group holding every texture, with bindless textures; `None` on adapters
    /// without them, where textures are bound per material instead.
    pub fn bindless_textures_bind_group(&self) -> Option<&wgpu::BindGroup> {
        self.texture_binder.global_bind_group()
    }

    /// The group-3 bind group for drawing with `material` outside the batched passes (GPU
    /// particles) on adapters without bindless textures; `None` with them.
    pub fn material_textures_bind_group(
        &self,
        assets: &Assets,
        material: Material,
    ) -> Option<wgpu::BindGroup> {
        self.texture_binder
            .create_material_bind_group(&self.context.device, assets, material)
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
        // Last *rendered* frame's: the view becomes history only once a frame is drawn
        // with it (see `render`), so a skipped frame does not break motion vectors.
        let prev_vp = self.prev_view_proj.unwrap_or(unjittered_vp);
        self.pending_view_proj = Some(unjittered_vp);
        // TAA: shift the projection by a different sub-pixel offset every frame (a scene
        // pixel's worth, when the scene renders smaller and is upscaled).
        let scene_size = self
            .display_region()
            .map(|region| region.scaled(self.render_scale()))
            .map(|region| (region.width(), region.height()))
            .unwrap_or((1, 1));
        let jitter = if self.jitters() {
            if !self.frozen_noise {
                self.jitter_index = self.jitter_index.wrapping_add(1);
            }
            taa_jitter(
                self.jitter_index,
                self.jitter_period(),
                scene_size.0,
                scene_size.1,
            )
        } else {
            Vec2::ZERO
        };
        // The same offset in scene pixels (+y down, as textures are addressed).
        let jitter_pixels = Vec2::new(
            jitter.x * scene_size.0 as f32 * 0.5,
            -jitter.y * scene_size.1 as f32 * 0.5,
        );
        let vp = Mat4::from_translation(jitter.extend(0.0)) * unjittered_vp;
        self.camera_frustum = Some(Frustum::from_view_proj(vp));
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
        let mut uni = CameraUniform::from_matrices(
            vp,
            inv_vp,
            camera_pos,
            forward,
            up,
            unjittered_vp,
            prev_vp,
        );
        // Sample material textures at the mip the output resolution would pick.
        uni.mip_bias = self.render_scale().log2();
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
                unjittered_view_proj: unjittered_vp,
                position: camera.position(),
                near: camera.near(),
                far: camera.far(),
                jitter_pixels,
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

    /// Screen-space shapes (transform gizmos) drawn on top of the next frame, after
    /// the gizmo meshes. Each frame replaces the last frame's.
    pub fn set_gizmo_shapes(&mut self, shapes: Vec<crate::renderer::gizmo_shapes::GizmoShape>) {
        self.gizmo_shapes.set_shapes(shapes);
    }

    /// World-space debug geometry (navmeshes, paths) drawn over the next frame, depth
    /// tested against the scene. Each frame replaces the last frame's.
    pub fn set_debug_geometry(&mut self, geometry: crate::renderer::debug_draw::DebugGeometry) {
        self.debug_draw.set_geometry(geometry);
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
        self.texture_generation = self.texture_generation.wrapping_add(1);
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
        if let Some(view_proj) = self.pending_view_proj.take() {
            self.prev_view_proj = Some(view_proj);
        }

        // Shadow maps that follow the camera stay on one texel grid.
        let mut lights = lights;
        lights.snap_directional_shadows(self.shadows.directional_map_size());

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
            self.prepare_frame_state(batcher, assets, environment, &lights, target_view, ui_view)?;
        self.gpu_timer.begin_frame(&mut frame_state.encoder);
        if let Some(stage) = self.present_stage.as_ref() {
            stage.clear_ui(&mut frame_state.encoder);
        }

        // Skinned vertices must exist before the first pass draws them.
        self.skinning.run(
            &self.context.device,
            &self.context.queue,
            &mut frame_state.encoder,
            assets,
            batcher.skin_jobs(),
        );
        frame_state.stats.skinned_instances = self.skinning.instance_count;
        frame_state.stats.skinned_vertices = self.skinning.vertex_count;
        self.gpu_timer.mark(&mut frame_state.encoder, "skinning");

        let mut plan = self.plan_frame(&frame_state.prepared_batches, custom_shadow_enabled);
        // Acceleration structures before the passes that trace against them.
        plan.ray_traced = self.update_ray_tracing(
            &mut frame_state,
            assets,
            plan.needs_acceleration_structures(),
        );
        plan.particles_traced = plan.ray_traced && !self.ray_traced_particles.is_empty();
        self.gpu_timer
            .mark(&mut frame_state.encoder, "acceleration_structures");

        // Shadow maps the ray-traced mask replaces, when nothing else reads them.
        let skip_maps = match self.rt_shadows.as_ref() {
            Some(stage) if plan.may_skip_shadow_maps() => stage.traced_bits(&lights),
            _ => 0,
        };
        self.run_shadow_pass(
            &mut frame_state,
            scene,
            assets,
            &lights,
            custom_render.as_deref_mut(),
            custom_shadow_enabled,
            skip_maps,
        );
        self.gpu_timer.mark(&mut frame_state.encoder, "shadows");
        self.run_depth_prepass(&mut frame_state, assets);
        self.gpu_timer
            .mark(&mut frame_state.encoder, "depth_prepass");
        self.run_rt_shadows(&mut frame_state, &lights, plan.trace_rt_shadows());
        if plan.ray_traced && plan.light_probes {
            self.run_light_probes(&mut frame_state);
        }
        // Traced global illumination brings the diffuse light the main pass would take
        // from the environment, and ray-traced reflections the sky's reflection.
        let gi = plan.trace_global_illumination();
        let traced_reflections = plan.ray_traced && self.postprocess.wants_ray_traced_reflections();
        self.environment
            .set_traced(&self.context.queue, gi, traced_reflections);
        self.run_main_color_pass(&mut frame_state, environment, assets);
        // Path traced lighting replaces the main pass's on opaque surfaces (traced global
        // illumination adds to it), before particles and transparent surfaces are drawn
        // over them.
        let lighting = if plan.path_trace() || plan.trace_global_illumination() {
            self.gpu_timer.mark(&mut frame_state.encoder, "main");
            self.run_path_tracing(&mut frame_state, &lights)
        } else {
            TracedLighting::Raster
        };
        let path_traced = lighting == TracedLighting::PathTraced;
        frame_state.stats.path_traced = path_traced;
        frame_state.stats.global_illumination = lighting == TracedLighting::GlobalIllumination;
        // Shader materials draw their own colour (after path tracing, which lights only
        // the PBR surfaces).
        self.run_shader_material_pass(&mut frame_state, assets, path_traced);
        // What particles and transparent surfaces change is reactive for FSR; where
        // particles cover the screen, ray-traced reflections trace them instead.
        self.postprocess.capture_opaque_scene(
            &self.context.device,
            &mut frame_state.encoder,
            frame_state.scene_region,
            plan.particles_traced,
        );
        self.gpu_timer.mark(&mut frame_state.encoder, "main");
        self.run_custom_stage(
            &mut frame_state,
            scene,
            CustomRenderStage::BeforePostprocess,
            custom_render.as_deref_mut(),
        );
        self.postprocess.capture_particle_scene(
            &self.context.device,
            &mut frame_state.encoder,
            frame_state.scene_region,
        );
        // Transparent surfaces are part of the lit HDR scene, so they get exposure,
        // bloom and tonemapping like everything else.
        self.run_transparent_pass(&mut frame_state, assets);
        self.gpu_timer.mark(&mut frame_state.encoder, "transparent");
        self.execute_postprocess(&mut frame_state, &plan, lighting);
        self.gpu_timer
            .mark(&mut frame_state.encoder, "post_process");
        self.run_custom_stage(
            &mut frame_state,
            scene,
            CustomRenderStage::AfterPostprocess,
            custom_render.as_deref_mut(),
        );
        self.run_surface_passes(&mut frame_state, assets);
        self.run_pick_pass(&mut frame_state, assets);
        self.gpu_timer.mark(&mut frame_state.encoder, "overlays");
        self.run_ui_hook(&mut frame_state);
        self.gpu_timer.mark(&mut frame_state.encoder, "ui");
        self.gpu_timer.end_frame(&mut frame_state.encoder);

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

    /// Wait for the display's refresh, or present as soon as a frame is ready (the
    /// configured present mode, see [`RenderSettings`]).
    pub fn set_vsync(&mut self, vsync: bool) {
        self.context.set_vsync(vsync);
    }

    /// Whether frames can be presented without waiting for the display.
    pub fn vsync_optional(&self) -> bool {
        self.context.unsynced_present_mode != wgpu::PresentMode::Fifo
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

    /// Position in the TAA jitter sequence of the last frame's camera set-up.
    pub fn jitter_index(&self) -> u32 {
        self.jitter_index
    }

    /// Hold the per-frame noise sources still (or let them run again): the TAA jitter
    /// stays at its current phase, film grain and dither keep their pattern, and light
    /// probes stop tracing new rays (a bake in progress pauses too). With the scene and
    /// camera unchanged, every frame then renders the same image, so captures from
    /// different builds or settings can be compared exactly. Temporal histories keep
    /// accumulating, so give them a few frames to settle after freezing. Capture-only: it
    /// slightly reduces anti-aliasing quality while on.
    pub fn set_frozen_noise(&mut self, frozen: bool) {
        self.frozen_noise = frozen;
        self.postprocess.set_frozen_clock(frozen);
    }

    pub fn frozen_noise(&self) -> bool {
        self.frozen_noise
    }

    /// Frames after which the TAA jitter repeats: [`TAA_JITTER_FRAMES`] at native
    /// resolution, more when upscaling (as FSR: 8 x the pixel ratio), so the jittered
    /// samples cover each scene pixel's share of output pixels.
    pub fn jitter_period(&self) -> u32 {
        jitter_period(self.render_scale())
    }

    /// The share of the output resolution (per axis) the scene renders at this frame:
    /// the upscaling setting, or 1 under MSAA, which upscaling does not support.
    pub fn render_scale(&self) -> f32 {
        let upscaling = &self.postprocess.settings().upscaling;
        if self.context.sample_count == 1 && upscaling.active() {
            upscaling.render_scale
        } else {
            1.0
        }
    }

    /// Whether the camera jitters: for TAA, or for FSR upscaling (its own anti-aliasing).
    fn jitters(&self) -> bool {
        let settings = self.postprocess.settings();
        settings.taa_active()
            || (self.render_scale() < 1.0
                && settings.upscaling.method == crate::renderer::postprocess::Upscaler::Fsr)
    }

    /// The part of the surface the frame fills: the render region, or all of it.
    /// Height in pixels of the view the scene is drawn into: the render region, or
    /// the whole surface.
    pub fn viewport_height(&self) -> u32 {
        self.display_region()
            .map(|region| region.height())
            .unwrap_or(self.context.config.height)
            .max(1)
    }

    fn display_region(&self) -> Option<RenderRegion> {
        self.render_region
            .or_else(|| RenderRegion::full(self.context.config.width, self.context.config.height))
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

    fn material_bind_group(
        &mut self,
        assets: &Assets,
        material: Material,
    ) -> Option<&wgpu::BindGroup> {
        self.texture_binder
            .bind_group_for_material(&self.context.device, assets, material)
    }

    fn draw_environment_background(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(self.pipeline.background());
        pass.set_bind_group(0, &self.camera_buffer.bind_group, &[]);
        pass.set_bind_group(1, &self.lights_buffer.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    fn prepare_frame_state(
        &mut self,
        batcher: &RenderBatcher,
        assets: &Assets,
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

        let mut prepared_batches =
            PreparedBatches::from_batcher(batcher, self.camera_position, &|mesh| {
                assets.meshes.get(mesh).map(Mesh::local_bounds)
            });
        let culled_instances = match self.camera_frustum.as_ref() {
            Some(frustum) => prepared_batches.cull_for_camera(frustum),
            None => 0,
        };
        let batch_count = prepared_batches.all().len() as u32;
        let instance_count = prepared_batches
            .all()
            .iter()
            .map(|batch| batch.instances.len() as u32)
            .sum();
        let stats = RendererStats {
            batch_count,
            instance_count,
            culled_instances,
            ..RendererStats::default()
        };

        let env_texture_changed =
            self.environment
                .update(&self.context.device, &self.context.queue, environment);
        self.postprocess
            .set_ambient(&self.context.queue, self.environment.flat_ambient());

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

        let render_region = self.display_region();
        let scale = self.render_scale();
        let scene_region = render_region.map(|region| region.scaled(scale));

        self.postprocess
            .update_viewport(&self.context.queue, render_region, scene_region);

        let (scene_view, scene_resolve_view) = {
            let (view, resolve) = self.postprocess.scene_color_views();
            (view.clone(), resolve.cloned())
        };

        Ok(FrameState {
            prepared_batches,
            encoder,
            stats,
            render_region,
            scene_region,
            surface_view,
            ui_view,
            scene_view,
            scene_resolve_view,
            depth_view: self.context.depth.view.clone(),
            resolved_depth_view: None,
            shadow_invocations: Vec::new(),
        })
    }

    /// Render the shadow maps, except those of lights in `skip` (bits as the ray-traced
    /// shadow mask), whose maps nothing reads this frame.
    #[allow(clippy::too_many_arguments)]
    fn run_shadow_pass(
        &mut self,
        frame: &mut FrameState,
        scene: &Scene,
        assets: &Assets,
        lights: &LightsData,
        custom_render: Option<&mut CustomRenderRequest<'_>>,
        custom_shadow_enabled: bool,
        skip: u32,
    ) {
        let outcome = passes::shadow_pass(
            self,
            scene,
            assets,
            &frame.prepared_batches,
            lights,
            &mut frame.encoder,
            custom_render,
            custom_shadow_enabled,
            &mut frame.shadow_invocations,
            skip,
        );
        frame.stats.shadow_draw_calls += outcome.draw_calls;
        frame.stats.shadow_views_rendered += outcome.views_rendered;
        frame.stats.shadow_views_cached += outcome.views_cached;
    }

    fn run_depth_prepass(&mut self, frame: &mut FrameState, assets: &Assets) {
        let draw_calls = passes::depth_prepass(
            self,
            assets,
            &mut frame.prepared_batches,
            &mut frame.encoder,
            &frame.depth_view,
            frame.scene_region,
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
            frame.scene_region,
            self.context.scene_texture_format(),
        );
        frame.stats.opaque_draw_calls += draw_calls;
    }

    fn run_shader_material_pass(
        &mut self,
        frame: &mut FrameState,
        assets: &Assets,
        path_traced: bool,
    ) {
        frame.stats.opaque_draw_calls += passes::shader_material_pass(
            self,
            assets,
            &frame.prepared_batches,
            &mut frame.encoder,
            &frame.scene_view,
            frame.scene_resolve_view.as_ref(),
            &frame.depth_view,
            frame.scene_region,
            path_traced,
        );
    }

    fn run_custom_stage(
        &mut self,
        frame: &mut FrameState,
        scene: &Scene,
        stage: CustomRenderStage,
        request: Option<&mut CustomRenderRequest<'_>>,
    ) {
        let (color_view, depth_view, region) = match stage {
            CustomRenderStage::BeforePostprocess => (
                frame.scene_view.clone(),
                frame.depth_view.clone(),
                frame.scene_region,
            ),
            CustomRenderStage::AfterPostprocess => (
                frame.surface_view.clone(),
                frame
                    .resolved_depth_view
                    .as_ref()
                    .cloned()
                    .unwrap_or_else(|| frame.depth_view.clone()),
                frame.render_region,
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
            region,
            request,
        );
    }

    /// What this frame will run (see [`FramePlan`]); `ray_traced` and
    /// `particles_traced` are filled in once the acceleration structures are built.
    fn plan_frame(&self, batches: &PreparedBatches, custom_shadow_enabled: bool) -> FramePlan {
        let settings = self.postprocess.settings();
        // Lit GPU particles draw after the shadow pass, so the lights group taken last
        // frame stands for this frame's.
        let particles_lit = self.lights_group_shared.replace(false);
        let shader_materials = batches
            .material_pipeline_keys
            .iter()
            .any(|key| matches!(key, MaterialPipelineKey::Shader(_)));
        FramePlan {
            post_ray_tracing: self.postprocess.wants_ray_tracing(),
            rt_shadows: self.rt_shadows.is_some()
                && settings.rt_shadows.enabled
                && self.context.sample_count == 1,
            // Transmissive surfaces were batched as opaque for ray-traced refraction.
            refraction: self.ray_traced_transmission()
                && batches.materials.iter().any(Material::is_transmissive),
            path_tracing: self.postprocess.wants_path_tracing(),
            global_illumination: self.postprocess.wants_global_illumination(),
            // Frozen noise also holds the probes' random rays still.
            light_probes: !self.frozen_noise && self.light_probes.wants_update(),
            only_main_pass_reads_shadow_maps: !custom_shadow_enabled
                && !particles_lit
                && !shader_materials
                && batches.transparent_range.is_empty(),
            ray_traced: false,
            particles_traced: false,
        }
    }

    /// Build this frame's acceleration structures when a ray-traced effect will use
    /// them. Returns whether they are ready.
    fn update_ray_tracing(
        &mut self,
        frame: &mut FrameState,
        assets: &Assets,
        wanted: bool,
    ) -> bool {
        let Some(scene) = self.ray_tracing.as_mut() else {
            return false;
        };
        if !wanted {
            return false;
        }
        // Only reflection and refraction rays see particles.
        let particles = if self.postprocess.wants_ray_traced_reflections()
            || self.postprocess.ray_traced_refraction()
        {
            &self.ray_traced_particles[..]
        } else {
            &[]
        };
        let camera = crate::renderer::raytracing::RtCamera {
            forward: (self.camera_target - self.camera_position)
                .try_normalize()
                .unwrap_or(Vec3::NEG_Z),
            up: self.camera_up.try_normalize().unwrap_or(Vec3::Y),
        };
        scene.update(
            &self.context.device,
            &self.context.queue,
            &mut frame.encoder,
            assets,
            &frame.prepared_batches,
            &self.skinning,
            &self.objects_buffer.materials,
            particles,
            camera,
        );
        frame.stats.ray_tracing = Some(scene.stats());
        scene.bind_group().is_some()
    }

    /// Trace the directional shadow mask after the depth prepass (when `active`) and tell
    /// the main pass which lights read it; otherwise they all use shadow maps.
    fn run_rt_shadows(&mut self, frame: &mut FrameState, lights: &LightsData, active: bool) {
        let mut bits = 0;
        if let (true, Some(stage), Some(scene)) = (
            active,
            self.rt_shadows.as_mut(),
            self.ray_tracing
                .as_ref()
                .and_then(|scene| scene.bind_group()),
        ) {
            let depth = &self.context.depth;
            if stage.ensure_targets(
                &self.context.device,
                depth.texture.size(),
                &depth.sampled_view,
            ) {
                self.lights_buffer.set_rt_shadow_mask(
                    &self.context.device,
                    &self.shadows,
                    &self.environment,
                    stage.mask_view(),
                );
            }
            bits = stage.trace(
                &mut frame.encoder,
                &self.context.queue,
                &self.camera_buffer.bind_group,
                scene,
                self.texture_binder.global_bind_group(),
                lights,
                self.postprocess.settings().rt_shadows.softness,
                self.postprocess.settings().rt_shadows.light_radius,
                self.jitter_index,
                frame.scene_region,
            );
            self.gpu_timer.mark(&mut frame.encoder, "rt_shadows");
        }
        self.lights_buffer
            .set_rt_shadow_bits(&self.context.queue, bits);
    }

    /// Trace this frame's slice of the light probe grid (before the main pass reads it).
    fn run_light_probes(&mut self, frame: &mut FrameState) {
        let Some(scene) = self
            .ray_tracing
            .as_ref()
            .and_then(|scene| scene.bind_group())
        else {
            return;
        };
        self.light_probes.update(
            &self.context.device,
            &self.context.queue,
            &mut frame.encoder,
            &self.lights_buffer.bind_group,
            scene,
            self.texture_binder.global_bind_group(),
        );
        self.gpu_timer.mark(&mut frame.encoder, "light_probes");
    }

    /// Light surfaces inside `grids` from their probes, the first one's light where it
    /// covers a surface, the next one's where it does not, and so on (at most
    /// [`crate::renderer::MAX_LIGHT_PROBE_VOLUMES`]; none: surfaces take the
    /// environment's light). Call every frame with the scene's light probe volumes; a
    /// volume that moved starts its probes over, the others keep theirs.
    pub fn set_light_probe_grids(&mut self, grids: &[crate::renderer::LightProbeGrid]) {
        self.light_probes
            .set_grids(&self.context.device, &self.context.queue, grids);
        if self.light_probes.take_bindings_changed() {
            self.lights_buffer.set_light_probes(
                &self.context.device,
                &self.shadows,
                &self.environment,
                &self.light_probes,
            );
        }
    }

    /// Bake light probe volumes `volumes` (their grid ids; every volume when `None`) at
    /// high quality, one after another (needs hardware ray tracing): `warmup` sweeps over
    /// every probe for the bounced light to build up, then the mean of `sweeps` more with
    /// `rays` rays per probe, one update per frame. Volumes asked for during a bake wait
    /// their turn. Returns the frames the bakes take;
    /// [`Self::take_finished_light_probe_bake`] then gives each volume's `.probes` file.
    pub fn start_light_probe_bake(
        &mut self,
        volumes: Option<&[u64]>,
        warmup: u32,
        sweeps: u32,
        rays: u32,
    ) -> Result<u32, String> {
        self.light_probes.start_bake(volumes, warmup, sweeps, rays)
    }

    pub fn light_probe_bake_progress(
        &self,
    ) -> Option<crate::renderer::light_probes::LightProbeBakeProgress> {
        self.light_probes.bake_progress()
    }

    /// A finished bake: the volume's grid id and its `.probes` file (read back from the
    /// GPU), once per bake.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn take_finished_light_probe_bake(&mut self) -> Option<(u64, Result<Vec<u8>, String>)> {
        use crate::renderer::readback::FetchableTexture;
        let finished = self.light_probes.finished_bake()?;
        let read = |name: &str, texture: &wgpu::Texture, region: [u32; 4]| {
            self.read_texture(
                &FetchableTexture::new(name.to_owned(), "", texture),
                region,
                0,
            )
        };
        let result = read(
            "probes.irradiance",
            finished.irradiance,
            finished.irradiance_region,
        )
        .and_then(|irradiance| {
            Ok((
                irradiance,
                read(
                    "probes.distance",
                    finished.distance,
                    finished.distance_region,
                )?,
            ))
        })
        .and_then(|(irradiance, distance)| {
            crate::renderer::light_probes::encode_bake(&finished.header, &irradiance, &distance)
        });
        let volume = finished.volume;
        self.light_probes.end_bake();
        Some((volume, result))
    }

    /// Show the baked light probe data in `bytes` (a `.probes` file read from `source`)
    /// in volume `volume` (its grid id).
    pub fn load_light_probe_bake(
        &mut self,
        volume: u64,
        bytes: &[u8],
        source: &str,
    ) -> Result<(), String> {
        self.light_probes
            .load_bake(&self.context.queue, volume, bytes, source)
    }

    /// The `.probes` file light probe volume `volume` (its grid id) shows, if it was
    /// loaded from one.
    pub fn light_probe_baked_source(&self, volume: u64) -> Option<&str> {
        self.light_probes.baked_source(volume)
    }

    /// Where light probe volume `volume`'s regions of `probes.irradiance` and
    /// `probes.distance` are ([x, y, width, height] each).
    pub fn light_probe_regions(&self, volume: u64) -> Option<([u32; 4], [u32; 4])> {
        self.light_probes.regions(volume)
    }

    /// Whether the light probe grid can trace (hardware ray tracing).
    pub fn light_probes_can_trace(&self) -> bool {
        self.light_probes.can_trace()
    }

    /// Record how the last light probe bake (of volume `volume`) ended (the app saves it):
    /// the project path of the saved file, or what went wrong.
    pub fn set_light_probe_bake_outcome(&mut self, volume: u64, result: Result<String, String>) {
        let serial = self
            .light_probe_bake_outcome
            .as_ref()
            .map_or(1, |outcome| outcome.serial + 1);
        self.light_probe_bake_outcome = Some(crate::renderer::LightProbeBakeOutcome {
            serial,
            volume,
            result,
        });
    }

    pub fn light_probe_bake_outcome(&self) -> Option<&crate::renderer::LightProbeBakeOutcome> {
        self.light_probe_bake_outcome.as_ref()
    }

    /// Draw the light probes as spheres over the image (`None`: don't).
    pub fn set_light_probe_debug(&mut self, view: Option<crate::renderer::LightProbeDebugView>) {
        self.light_probe_debug.set_view(view);
    }

    pub fn light_probe_debug(&self) -> Option<crate::renderer::LightProbeDebugView> {
        self.light_probe_debug.view()
    }

    fn run_path_tracing(&mut self, frame: &mut FrameState, lights: &LightsData) -> TracedLighting {
        let Some(rt_scene) = self.ray_tracing.as_ref() else {
            return TracedLighting::Raster;
        };
        let Some(scene) = rt_scene.bind_group() else {
            return TracedLighting::Raster;
        };
        let ray_tracing = crate::renderer::raytracing::RayTracingFrame {
            lights: &self.lights_buffer.bind_group,
            scene,
            textures: self.texture_binder.global_bind_group(),
            refraction: false,
        };
        self.postprocess.path_trace(
            &mut frame.encoder,
            &self.context.device,
            &self.context.queue,
            &ray_tracing,
            crate::renderer::postprocess::CausticSources {
                casters: rt_scene.caustic_casters(),
                lights,
            },
            frame.scene_region,
            &mut self.gpu_timer,
        )
    }

    fn execute_postprocess(
        &mut self,
        frame: &mut FrameState,
        plan: &FramePlan,
        lighting: TracedLighting,
    ) {
        let refraction = plan.refraction;
        let ray_tracing = self
            .ray_tracing
            .as_ref()
            .filter(|_| plan.ray_traced)
            .and_then(|scene| scene.bind_group())
            .map(|scene| crate::renderer::raytracing::RayTracingFrame {
                lights: &self.lights_buffer.bind_group,
                scene,
                textures: self.texture_binder.global_bind_group(),
                refraction,
            });
        self.postprocess.execute(
            &mut frame.encoder,
            &self.context.device,
            &self.context.queue,
            &frame.surface_view,
            frame.render_region,
            frame.scene_region,
            ray_tracing,
            lighting,
            &mut self.gpu_timer,
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
            frame.scene_region,
            self.context.scene_texture_format(),
        );
        frame.stats.transparent_draw_calls += transparent_draw_calls;
    }

    /// Editor overlays and gizmos, drawn on the finished (tonemapped) image.
    fn run_surface_passes(&mut self, frame: &mut FrameState, assets: &Assets) {
        let render_region = frame.render_region;
        let surface_format = self.output_format();
        let surface_view = frame.surface_view.clone();
        // Single-sample, output-resolution depth (upscaled or MSAA-resolved).
        let depth_view = frame
            .resolved_depth_view
            .clone()
            .unwrap_or_else(|| frame.depth_view.clone());

        let overlay_draw_calls = passes::overlay_pass(
            self,
            assets,
            &frame.prepared_batches,
            &mut frame.encoder,
            &surface_view,
            &depth_view,
            render_region,
            surface_format,
        );
        frame.stats.overlay_draw_calls += overlay_draw_calls;

        let viewport = match render_region {
            Some(region) => (
                region.x() as f32,
                region.y() as f32,
                region.width() as f32,
                region.height() as f32,
            ),
            None => (
                0.0,
                0.0,
                self.context.config.width as f32,
                self.context.config.height as f32,
            ),
        };
        frame.stats.overlay_draw_calls += self.light_probe_debug.render(
            &self.context.device,
            &self.context.queue,
            &mut frame.encoder,
            &self.light_probes,
            &surface_view,
            surface_format,
            &depth_view,
            self.context.depth.format,
            &self.camera_buffer.bind_layout,
            &self.camera_buffer.stable_bind_group,
            viewport,
        );

        frame.stats.overlay_draw_calls += self.debug_draw.render(
            &self.context.device,
            &self.context.queue,
            &mut frame.encoder,
            &surface_view,
            surface_format,
            &depth_view,
            self.context.depth.format,
            &self.camera_buffer.bind_layout,
            &self.camera_buffer.stable_bind_group,
            viewport,
        );

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
            surface_format,
            "GizmoSolidPass",
        );
        let shape_draws = self.gizmo_shapes.render(
            &self.context.device,
            &self.context.queue,
            &mut frame.encoder,
            &surface_view,
            surface_format,
            &self.camera_buffer.bind_layout,
            &self.camera_buffer.stable_bind_group,
            viewport,
        );
        frame.stats.gizmo_draw_calls += gizmo_draws + gizmo_solid_draws + shape_draws;
    }

    /// Entity ids for a pending pick request (nothing otherwise).
    fn run_pick_pass(&mut self, frame: &mut FrameState, assets: &Assets) {
        frame.stats.pick_draw_calls += passes::pick_pass(
            self,
            assets,
            &frame.prepared_batches,
            &mut frame.encoder,
            frame.render_region,
        );
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
}

/// The bindless texture array's layout for ray-traced passes, whose hits sample material
/// textures from it as a fifth bind group; `None` without bindless textures or that many
/// bind groups (hits then use material factors only).
fn ray_traced_textures_layout<'a>(
    texture_binder: &'a TextureBindingModel,
    device: &wgpu::Device,
) -> Option<&'a wgpu::BindGroupLayout> {
    (texture_binder.global_bind_group().is_some() && device.limits().max_bind_groups >= 5)
        .then(|| texture_binder.bind_layout())
}

/// The bindless texture array's layout for ray-traced passes that only alpha-test
/// cut-out surfaces (bound as their fourth group); `None` without bindless textures.
fn cutout_textures_layout(texture_binder: &TextureBindingModel) -> Option<&wgpu::BindGroupLayout> {
    texture_binder
        .global_bind_group()
        .is_some()
        .then(|| texture_binder.bind_layout())
}

/// Frames after which the TAA jitter pattern repeats. 8 points cover the pixel evenly;
/// longer sequences only add noise at the 0.1 blend. Frame-to-frame differences of an
/// unchanged view repeat with this period, so comparisons of them use whole periods.
pub const TAA_JITTER_FRAMES: u32 = 8;

/// Jitter frames for a scene rendered at `render_scale` of the output (per axis): FSR's
/// 8 x (output pixels per scene pixel), at most 64.
fn jitter_period(render_scale: f32) -> u32 {
    if render_scale >= 0.999 {
        return TAA_JITTER_FRAMES;
    }
    let ratio = 1.0 / render_scale.max(0.01);
    ((TAA_JITTER_FRAMES as f32 * ratio * ratio).round() as u32).clamp(TAA_JITTER_FRAMES, 64)
}

/// Clip-space offset for TAA frame `index` of a `period`-frame sequence: a Halton(2, 3)
/// point inside the pixel, converted to NDC for a `width` x `height` view.
fn taa_jitter(index: u32, period: u32, width: u32, height: u32) -> Vec2 {
    let sample = index % period.max(1) + 1;
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
        for index in 0..64 {
            let jitter = taa_jitter(index, jitter_period(0.5), 100, 50);
            assert!(jitter.x.abs() <= 1.0 / 100.0 + 1e-6, "{jitter:?}");
            assert!(jitter.y.abs() <= 1.0 / 50.0 + 1e-6, "{jitter:?}");
        }
        assert_eq!(halton(1, 2), 0.5);
        assert_eq!(halton(2, 3), 2.0 / 3.0);
    }

    #[test]
    fn jitter_period_grows_with_the_upscale_ratio() {
        assert_eq!(jitter_period(1.0), TAA_JITTER_FRAMES);
        assert_eq!(jitter_period(0.5), 32);
        assert_eq!(jitter_period(2.0 / 3.0), 18);
        assert_eq!(jitter_period(0.25), 64);
    }
}
