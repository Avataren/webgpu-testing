//! Post-processing on the HDR scene: SSAO, bloom, auto exposure, colour correction,
//! tonemapping, vignette, chromatic aberration, film grain, sharpening and FXAA.
//!
//! Frame order: depth resolve (MSAA only) -> screen-space reflections (added to the scene)
//! -> TAA / motion blur -> SSAO -> bloom -> depth of field -> auto exposure -> composite
//! (HDR to display, into an intermediate target) -> final (anti-aliasing, sharpening,
//! grain and dither, onto the surface).

mod bloom;
mod dof;
mod exposure;
mod lut;
mod pipelines;
mod resources;
mod rt_reflections;
mod settings;
mod ssao;
mod ssr;
mod temporal;

pub use lut::CubeLut;
pub use settings::{
    setting_overridden, AntiAliasing, BloomSettings, ChromaticAberrationSettings, ColorAdjustments,
    DepthOfFieldSettings, ExposureMode, ExposureSettings, FilmGrainSettings, FocusMode,
    LegacyBloomSettings, LegacyColorGrading, LegacySsaoSettings, LutSettings, MotionBlurSettings,
    PostProcessEffects, PostProcessSettings, ReflectionSettings, SharpenSettings, SsaoSettings,
    Tonemapper, VignetteSettings, TRACKBALL_NEUTRAL,
};

use bloom::{BloomBindGroupInputs, BloomStage};
use dof::DofStage;
use exposure::ExposureStage;
use lut::LutTexture;
use pipelines::{
    build_composite, build_depth_resolve, build_final, CompositePipeline, DepthResolvePipeline,
    FinalPipeline,
};
use resources::{BloomMip, LazyPickTarget, MsaaTarget, TextureBundle};
use rt_reflections::{RtReflectionInputs, RtReflectionStage};
use ssao::{SsaoBindGroupInputs, SsaoStage, SsaoTargets};
use ssr::{SsrInputs, SsrStage, SsrTargets};
use temporal::{TemporalInputs, TemporalStage};

use crate::renderer::raytracing::RayTracingFrame;
use crate::renderer::{RenderRegion, ShaderBuilder};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec2, Vec3};

const NOISE_TEXTURE_SIZE: u32 = 4;
pub(crate) const BLOOM_MIP_COUNT: usize = 6;
pub(crate) const BLOOM_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Format of the lit scene: half floats keep light above 1.0 for bloom, exposure and
/// tonemapping instead of clipping it.
pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Tonemapped image waiting for anti-aliasing; half floats avoid banding before dither.
const DISPLAY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Half-resolution depth of field colour, with the circle of confusion in alpha.
pub(crate) const DOF_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Octahedral-encoded world normal in xy and screen motion since the previous frame (UV
/// units, for motion blur and TAA) in zw. Packed together because WebGPU allows only 32
/// bytes of colour attachments per sample.
pub const GBUFFER_NORMAL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
pub const GBUFFER_POSITION_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
pub const GBUFFER_PICK_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg32Uint;
const SSAO_NOISE_DATA: [f32; (NOISE_TEXTURE_SIZE * NOISE_TEXTURE_SIZE * 4) as usize] = [
    -0.6401949,
    -0.76821256,
    0.0,
    0.0,
    0.98767775,
    0.1565012,
    0.0,
    0.0,
    -0.1566164,
    0.9876595,
    0.0,
    0.0,
    0.1675282,
    0.98586726,
    0.0,
    0.0,
    -0.08490153,
    -0.9963893,
    0.0,
    0.0,
    -0.44445047,
    -0.89580345,
    0.0,
    0.0,
    0.77917,
    -0.62681264,
    0.0,
    0.0,
    0.85447717,
    0.519489,
    0.0,
    0.0,
    -0.88205993,
    0.471_137_3,
    0.0,
    0.0,
    0.98252517,
    0.18612963,
    0.0,
    0.0,
    0.19578062,
    0.98064774,
    0.0,
    0.0,
    -0.99943393,
    -0.03364192,
    0.0,
    0.0,
    0.9861326,
    0.165959,
    0.0,
    0.0,
    0.3159545,
    0.94877434,
    0.0,
    0.0,
    -0.5883725,
    -0.80859,
    0.0,
    0.0,
    -0.96039623,
    -0.278638,
    0.0,
    0.0,
];

pub struct PostProcess {
    scene_source: TextureBundle,
    scene_msaa: Option<MsaaTarget>,
    normal_source: TextureBundle,
    normal_msaa: Option<MsaaTarget>,
    position_source: TextureBundle,
    position_msaa: Option<MsaaTarget>,
    pick_target: LazyPickTarget,
    ssao: TextureBundle,
    ssao_ping: TextureBundle,
    ssr_half: TextureBundle,
    ssr_ping: TextureBundle,
    ssr_accumulated: TextureBundle,
    ssr_history: TextureBundle,
    bloom_chain: Vec<BloomMip>,
    dof_half: TextureBundle,
    dof_blur: TextureBundle,
    taa_output: TextureBundle,
    taa_history: TextureBundle,
    motion_output: TextureBundle,
    display: TextureBundle,
    sampler_linear: wgpu::Sampler,
    sampler_noise: wgpu::Sampler,
    _noise_texture: wgpu::Texture,
    noise_view: wgpu::TextureView,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    depth_resolve_pipeline: Option<DepthResolvePipeline>,
    composite_pipeline: CompositePipeline,
    final_pipeline: FinalPipeline,
    ssao_stage: SsaoStage,
    ssr_stage: SsrStage,
    /// Ray-traced reflections, when the device has ray queries and the pass built.
    rt_reflection_stage: Option<RtReflectionStage>,
    /// Kept to build stages added after construction (ray-traced reflections).
    uniform_layout: wgpu::BindGroupLayout,
    bloom_stage: BloomStage,
    exposure_stage: ExposureStage,
    dof_stage: DofStage,
    temporal_stage: TemporalStage,
    /// The history holds last frame's TAA result (false after a resize or turning TAA on).
    taa_history_valid: bool,
    /// How last frame's reflections were traced (Some(true) = ray traced), None when
    /// none ran: the reflection history only carries over while this stays the same.
    reflection_history: Option<bool>,
    /// Which temporal passes the cached bind groups were built for (TAA, motion blur).
    temporal_routing: (bool, bool),
    lut: LutTexture,
    size: wgpu::Extent3d,
    settings: PostProcessSettings,
    depth_resolve_bind_group: Option<wgpu::BindGroup>,
    composite_bind_group: Option<wgpu::BindGroup>,
    final_bind_group: Option<wgpu::BindGroup>,
    resolved_depth: Option<TextureBundle>,
    cached_depth_view: Option<wgpu::TextureView>,
    bind_groups_dirty: bool,
    camera: PostProcessCamera,
    viewport_resolution: Vec2,
    viewport_offset: Vec2,
    viewport_scale: Vec2,
    sample_count: u32,
    /// The surface has no sRGB format, so the final pass encodes itself.
    encode_srgb: bool,
    /// Brightest value the display shows, relative to paper white (1 for SDR).
    display_peak: f32,
    clock: FrameClock,
}

#[derive(Clone, Copy)]
pub struct PostProcessCamera {
    pub proj: Mat4,
    pub view_proj: Mat4,
    pub view_proj_inv: Mat4,
    pub view: Mat4,
    pub view_inv: Mat4,
    pub position: Vec3,
    pub near: f32,
    pub far: f32,
}

impl Default for PostProcessCamera {
    fn default() -> Self {
        Self {
            proj: Mat4::IDENTITY,
            view_proj: Mat4::IDENTITY,
            view_proj_inv: Mat4::IDENTITY,
            view: Mat4::IDENTITY,
            view_inv: Mat4::IDENTITY,
            position: Vec3::ZERO,
            near: 0.01,
            far: 100.0,
        }
    }
}

impl PostProcessCamera {
    /// Screen-space passes must project into the same jittered pixels as the G-buffer.
    /// Motion vectors and overlays keep their separate, unjittered camera matrices.
    pub(crate) fn with_projection_jitter(mut self, jitter: Vec2) -> Self {
        let shift = Mat4::from_translation(jitter.extend(0.0));
        self.proj = shift * self.proj;
        self.view_proj = shift * self.view_proj;
        self.view_proj_inv = self.view_proj.inverse();
        self
    }
}

/// Time for animated effects (grain, dither) and the frame delta auto exposure adapts by.
struct FrameClock {
    start: instant::Instant,
    last: Option<instant::Instant>,
    seconds: f32,
    delta: f32,
    frame: u32,
}

impl FrameClock {
    fn new() -> Self {
        Self {
            start: instant::Instant::now(),
            last: None,
            seconds: 0.0,
            delta: 0.0,
            frame: 0,
        }
    }

    fn tick(&mut self) {
        let now = instant::Instant::now();
        self.delta = self
            .last
            .map(|last| now.duration_since(last).as_secs_f32())
            .unwrap_or(0.0);
        self.last = Some(now);
        // Wrapped so the value keeps its precision in long sessions.
        self.seconds = now.duration_since(self.start).as_secs_f32() % 3600.0;
        self.frame = self.frame.wrapping_add(1) % (1 << 20);
    }
}

pub struct GBufferViews<'a> {
    pub normal: (&'a wgpu::TextureView, Option<&'a wgpu::TextureView>),
    pub position: (&'a wgpu::TextureView, Option<&'a wgpu::TextureView>),
    pub id: Option<PickAttachmentViews<'a>>,
}

#[derive(Clone, Copy)]
pub struct PickAttachmentViews<'a> {
    pub multisample: &'a wgpu::TextureView,
    pub resolve: Option<&'a wgpu::TextureView>,
}

impl<'a> PickAttachmentViews<'a> {
    pub fn single_sample(self) -> &'a wgpu::TextureView {
        self.resolve.unwrap_or(self.multisample)
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ColorAttachment<'a> {
    view: &'a wgpu::TextureView,
    resolve: Option<&'a wgpu::TextureView>,
}

impl<'a> ColorAttachment<'a> {
    fn single(view: &'a wgpu::TextureView) -> Self {
        Self {
            view,
            resolve: None,
        }
    }
}

pub(crate) fn begin_color_pass<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    label: &'static str,
    attachment: ColorAttachment<'a>,
    load: wgpu::LoadOp<wgpu::Color>,
    region: Option<RenderRegion>,
) -> wgpu::RenderPass<'a> {
    let color_attachment = wgpu::RenderPassColorAttachment {
        view: attachment.view,
        resolve_target: attachment.resolve,
        depth_slice: None,
        ops: wgpu::Operations {
            load,
            store: wgpu::StoreOp::Store,
        },
    };

    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(color_attachment)],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });

    if let Some(region) = region {
        region.apply_to_pass(&mut pass);
    }

    pass
}

fn begin_depth_pass<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    label: &'static str,
    view: &'a wgpu::TextureView,
    load: wgpu::LoadOp<f32>,
) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view,
            depth_ops: Some(wgpu::Operations {
                load,
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    })
}

pub(crate) fn fullscreen_vertex(shader: &wgpu::ShaderModule) -> wgpu::VertexState<'_> {
    wgpu::VertexState {
        module: shader,
        entry_point: Some("vs_fullscreen"),
        buffers: &[],
        compilation_options: Default::default(),
    }
}

/// The post-processing shader source: every pass's entry points in one module.
pub(crate) fn postprocess_shader_source() -> String {
    ShaderBuilder::new()
        .with_module(include_str!("../../shader/postprocess/fullscreen.wgsl"))
        .with_module(include_str!("../../shader/postprocess/common.wgsl"))
        .with_module(include_str!("../../shader/postprocess/ssao.wgsl"))
        .with_module(include_str!("../../shader/postprocess/ssao_blur.wgsl"))
        .with_module(include_str!(
            "../../shader/postprocess/reflection_common.wgsl"
        ))
        .with_module(include_str!("../../shader/postprocess/ssr.wgsl"))
        .with_module(include_str!("../../shader/postprocess/bloom_common.wgsl"))
        .with_module(include_str!(
            "../../shader/postprocess/bloom_prefilter.wgsl"
        ))
        .with_module(include_str!(
            "../../shader/postprocess/bloom_downsample.wgsl"
        ))
        .with_module(include_str!("../../shader/postprocess/bloom_upsample.wgsl"))
        .with_module(include_str!("../../shader/postprocess/auto_exposure.wgsl"))
        .with_module(include_str!("../../shader/postprocess/dof.wgsl"))
        .with_module(include_str!("../../shader/postprocess/temporal.wgsl"))
        .with_module(include_str!("../../shader/postprocess/composite.wgsl"))
        .with_module(include_str!("../../shader/postprocess/final.wgsl"))
        .build_modules_only()
}

impl PostProcess {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        config: &wgpu::SurfaceConfiguration,
        sample_count: u32,
    ) -> Self {
        let size = wgpu::Extent3d {
            width: config.width.max(1),
            height: config.height.max(1),
            depth_or_array_layers: 1,
        };

        let sampler_linear = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("PostProcessLinearSampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        let sampler_noise = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("PostProcessNoiseSampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let targets = SizedTargets::new(device, &size, sample_count);

        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("PostProcessUniformLayout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: Some(
                        wgpu::BufferSize::new(std::mem::size_of::<PostProcessUniform>() as u64)
                            .expect("post process uniform must have non-zero size"),
                    ),
                },
                count: None,
            }],
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("PostProcessUniformBuffer"),
            size: std::mem::size_of::<PostProcessUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("PostProcessUniformBindGroup"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let noise_texture = create_noise_texture(device, queue);
        let noise_view = noise_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let postprocess_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("PostProcessShader"),
            source: wgpu::ShaderSource::Wgsl(postprocess_shader_source().into()),
        });

        let depth_resolve_pipeline = build_depth_resolve(device, &uniform_layout, sample_count);
        let ssao_stage = SsaoStage::new(device, &uniform_layout, &postprocess_shader);
        let ssr_stage = SsrStage::new(device, &uniform_layout, &postprocess_shader);
        let bloom_stage = BloomStage::new(device, &uniform_layout, &postprocess_shader);
        let exposure_stage = ExposureStage::new(device, &uniform_layout, &postprocess_shader);
        let dof_stage = DofStage::new(device, &uniform_layout, &postprocess_shader);
        let temporal_stage = TemporalStage::new(device, &uniform_layout, &postprocess_shader);
        let composite_pipeline =
            build_composite(device, &uniform_layout, &postprocess_shader, DISPLAY_FORMAT);
        let final_pipeline =
            build_final(device, &uniform_layout, &postprocess_shader, config.format);

        let post = Self {
            scene_source: targets.scene_source,
            scene_msaa: targets.scene_msaa,
            normal_source: targets.normal_source,
            normal_msaa: targets.normal_msaa,
            position_source: targets.position_source,
            position_msaa: targets.position_msaa,
            pick_target: LazyPickTarget::new(),
            ssao: targets.ssao,
            ssao_ping: targets.ssao_ping,
            ssr_half: targets.ssr_half,
            ssr_ping: targets.ssr_ping,
            ssr_accumulated: targets.ssr_accumulated,
            ssr_history: targets.ssr_history,
            bloom_chain: targets.bloom_chain,
            dof_half: targets.dof_half,
            dof_blur: targets.dof_blur,
            taa_output: targets.taa_output,
            taa_history: targets.taa_history,
            motion_output: targets.motion_output,
            display: targets.display,
            sampler_linear,
            sampler_noise,
            _noise_texture: noise_texture,
            noise_view,
            uniform_buffer,
            uniform_bind_group,
            depth_resolve_pipeline,
            composite_pipeline,
            final_pipeline,
            ssao_stage,
            ssr_stage,
            rt_reflection_stage: None,
            uniform_layout,
            bloom_stage,
            exposure_stage,
            dof_stage,
            temporal_stage,
            taa_history_valid: false,
            reflection_history: None,
            temporal_routing: (false, false),
            lut: LutTexture::new(device, queue),
            size,
            settings: PostProcessSettings::default(),
            depth_resolve_bind_group: None,
            composite_bind_group: None,
            final_bind_group: None,
            resolved_depth: targets.resolved_depth,
            cached_depth_view: None,
            bind_groups_dirty: true,
            camera: PostProcessCamera::default(),
            viewport_resolution: Vec2::new(size.width as f32, size.height as f32),
            viewport_offset: Vec2::ZERO,
            viewport_scale: Vec2::ONE,
            sample_count,
            // Float targets (the HDR composition texture) take linear values.
            encode_srgb: !config.format.is_srgb() && !is_float_format(config.format),
            display_peak: 1.0,
            clock: FrameClock::new(),
        };

        post.upload_uniform(queue);
        post
    }

    pub fn resize(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let targets = SizedTargets::new(device, &self.size, self.sample_count);
        self.scene_source = targets.scene_source;
        self.scene_msaa = targets.scene_msaa;
        self.normal_source = targets.normal_source;
        self.normal_msaa = targets.normal_msaa;
        self.position_source = targets.position_source;
        self.position_msaa = targets.position_msaa;
        self.ssao = targets.ssao;
        self.ssao_ping = targets.ssao_ping;
        self.ssr_half = targets.ssr_half;
        self.ssr_ping = targets.ssr_ping;
        self.ssr_accumulated = targets.ssr_accumulated;
        self.ssr_history = targets.ssr_history;
        self.bloom_chain = targets.bloom_chain;
        self.dof_half = targets.dof_half;
        self.dof_blur = targets.dof_blur;
        self.taa_output = targets.taa_output;
        self.taa_history = targets.taa_history;
        self.motion_output = targets.motion_output;
        self.taa_history_valid = false;
        self.reflection_history = None;
        self.display = targets.display;
        self.resolved_depth = targets.resolved_depth;
        self.pick_target.invalidate();
        self.viewport_resolution = Vec2::new(width as f32, height as f32);
        self.viewport_offset = Vec2::ZERO;
        self.viewport_scale = Vec2::ONE;
        self.mark_bind_groups_dirty();
        self.upload_uniform(queue);
    }

    pub fn update_viewport(&mut self, queue: &wgpu::Queue, region: Option<RenderRegion>) {
        let full_width = self.size.width.max(1) as f32;
        let full_height = self.size.height.max(1) as f32;

        let (resolution, offset, scale) = if let Some(region) = region {
            let region_width = region.width().max(1) as f32;
            let region_height = region.height().max(1) as f32;
            let offset = Vec2::new(
                region.x() as f32 / full_width,
                region.y() as f32 / full_height,
            );
            let scale = Vec2::new(region_width / full_width, region_height / full_height);
            (Vec2::new(region_width, region_height), offset, scale)
        } else {
            (Vec2::new(full_width, full_height), Vec2::ZERO, Vec2::ONE)
        };

        if self.viewport_resolution != resolution
            || self.viewport_offset != offset
            || self.viewport_scale != scale
        {
            self.viewport_resolution = resolution;
            self.viewport_offset = offset;
            self.viewport_scale = scale;
            // The history no longer lines up with the view.
            self.taa_history_valid = false;
            self.reflection_history = None;
            self.upload_uniform(queue);
        }
    }

    /// Called once per rendered frame with the camera it is seen through; also advances
    /// the clock animated effects and exposure adaptation run on.
    pub fn update_camera(&mut self, queue: &wgpu::Queue, camera: PostProcessCamera) {
        self.camera = camera;
        self.clock.tick();
        self.upload_uniform(queue);
    }

    pub fn scene_color_views(&self) -> (&wgpu::TextureView, Option<&wgpu::TextureView>) {
        match self.scene_msaa.as_ref() {
            Some(msaa) => (&msaa.view, Some(&self.scene_source.view)),
            None => (&self.scene_source.view, None),
        }
    }

    pub fn gbuffer_views(&self) -> GBufferViews<'_> {
        let normal = match self.normal_msaa.as_ref() {
            Some(msaa) => (&msaa.view, Some(&self.normal_source.view)),
            None => (&self.normal_source.view, None),
        };
        let position = match self.position_msaa.as_ref() {
            Some(msaa) => (&msaa.view, Some(&self.position_source.view)),
            None => (&self.position_source.view, None),
        };
        let id = self.pick_target.views();
        GBufferViews {
            normal,
            position,
            id,
        }
    }

    pub fn set_depth_view(&mut self, depth_view: &wgpu::TextureView) {
        self.cached_depth_view = Some(depth_view.clone());
        self.mark_bind_groups_dirty();
    }

    pub fn after_postprocess_depth_view(&self) -> Option<&wgpu::TextureView> {
        if let Some(resolved) = self.resolved_depth.as_ref() {
            Some(&resolved.view)
        } else {
            self.cached_depth_view.as_ref()
        }
    }

    /// Apply new settings; values outside the supported ranges are clamped. A changed
    /// LUT file is loaded here.
    pub fn set_settings(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        settings: &PostProcessSettings,
    ) {
        let lut_path = if settings.lut.active() {
            settings.lut.path.as_str()
        } else {
            ""
        };
        if self.lut.update(device, queue, lut_path) {
            self.composite_bind_group = None;
            self.bind_groups_dirty = true;
            self.upload_uniform(queue);
        }
        let routing = (settings.taa_active(), settings.motion_blur_active());
        if routing != self.temporal_routing {
            if routing.0 && !self.temporal_routing.0 {
                self.taa_history_valid = false;
            }
            self.temporal_routing = routing;
            self.mark_bind_groups_dirty();
        }
        if self.settings != *settings {
            let was_auto = self.settings.auto_exposure_active();
            self.settings = settings.clone().sanitized();
            if self.settings.auto_exposure_active() && !was_auto {
                // Start from a fresh measurement instead of whatever was adapted to
                // when auto exposure was last on.
                self.exposure_stage.reset(queue);
            }
            self.upload_uniform(queue);
        }
    }

    /// Build the ray-traced reflection pass. `lights_layout` is the main pass's lights
    /// group, `scene_layout` the ray tracing scene's and `textures_layout` the bindless
    /// texture array's (None: hits use material factors only). On error reflections stay
    /// screen-space.
    pub(crate) fn enable_ray_traced_reflections(
        &mut self,
        device: &wgpu::Device,
        lights_layout: &wgpu::BindGroupLayout,
        scene_layout: &wgpu::BindGroupLayout,
        textures_layout: Option<&wgpu::BindGroupLayout>,
    ) -> Result<(), String> {
        let stage = RtReflectionStage::new(
            device,
            &self.uniform_layout,
            lights_layout,
            scene_layout,
            textures_layout,
        )?;
        self.rt_reflection_stage = Some(stage);
        self.mark_bind_groups_dirty();
        Ok(())
    }

    /// Whether this frame's reflections would be ray traced if the scene is ready.
    pub(crate) fn wants_ray_traced_reflections(&self) -> bool {
        self.rt_reflection_stage.is_some()
            && self.settings.ssr_active()
            && self.settings.ssr.ray_traced
    }

    pub fn settings(&self) -> &PostProcessSettings {
        &self.settings
    }

    /// The display's headroom above paper white (HDR output); tonemapping uses it.
    pub fn set_display_peak(&mut self, queue: &wgpu::Queue, peak: f32) {
        self.display_peak = peak.max(1.0);
        self.upload_uniform(queue);
    }

    /// Restart auto exposure adaptation, e.g. after switching scenes.
    pub fn reset_exposure(&self, queue: &wgpu::Queue) {
        self.exposure_stage.reset(queue);
    }

    pub fn ensure_pick_attachment(&mut self, device: &wgpu::Device) {
        self.pick_target
            .ensure(device, &self.size, self.sample_count);
    }

    pub fn pick_attachment_views(&self) -> Option<PickAttachmentViews<'_>> {
        self.pick_target.views()
    }

    pub fn pick_texture(&self) -> Option<&wgpu::Texture> {
        self.pick_target.texture()
    }

    pub fn pick_texture_extent(&self) -> Option<wgpu::Extent3d> {
        self.pick_target.extent()
    }

    pub fn execute(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        device: &wgpu::Device,
        target: &wgpu::TextureView,
        region: Option<RenderRegion>,
        ray_tracing: Option<RayTracingFrame<'_>>,
    ) {
        self.ensure_cached_bind_groups(device);

        if let (Some(depth_pipeline), Some(bind_group), Some(resolved)) = (
            self.depth_resolve_pipeline.as_ref(),
            self.depth_resolve_bind_group.as_ref(),
            self.resolved_depth.as_ref(),
        ) {
            let mut pass = begin_depth_pass(
                encoder,
                "DepthResolvePass",
                &resolved.view,
                wgpu::LoadOp::Clear(1.0),
            );
            pass.set_pipeline(&depth_pipeline.pipeline);
            pass.set_bind_group(0, &self.uniform_bind_group, &[]);
            pass.set_bind_group(1, bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        if self.settings.ssr_active() {
            let traced = match (self.rt_reflection_stage.as_ref(), ray_tracing.as_ref()) {
                (Some(stage), Some(frame)) if self.settings.ssr.ray_traced => stage.trace(
                    encoder,
                    &self.uniform_bind_group,
                    frame,
                    &self.ssr_half.view,
                ),
                _ => false,
            };
            let targets = SsrTargets {
                reflection: &self.ssr_half,
                ping: &self.ssr_ping.view,
                accumulated: &self.ssr_accumulated,
                history: &self.ssr_history,
                history_valid: self.reflection_history == Some(traced),
            };
            if traced {
                self.ssr_stage.resolve(
                    encoder,
                    &self.uniform_bind_group,
                    &targets,
                    &self.scene_source.view,
                    region,
                );
            } else {
                self.ssr_stage.render(
                    encoder,
                    &self.uniform_bind_group,
                    &targets,
                    &self.scene_source.view,
                    region,
                );
            }
            self.reflection_history = Some(traced);
        } else {
            self.reflection_history = None;
        }

        let (taa, motion_blur) = self.temporal_routing;
        if taa {
            self.temporal_stage.render_taa(
                encoder,
                &self.uniform_bind_group,
                &self.taa_output,
                &self.taa_history,
            );
            self.taa_history_valid = true;
        }
        if motion_blur {
            self.temporal_stage.render_motion_blur(
                encoder,
                &self.uniform_bind_group,
                &self.motion_output.view,
            );
        }

        let ssao_targets = SsaoTargets {
            output: &self.ssao.view,
            ping: &self.ssao_ping.view,
        };
        if self.settings.ssao_active() {
            self.ssao_stage
                .render(encoder, &self.uniform_bind_group, ssao_targets);
        } else {
            self.ssao_stage.clear(encoder, ssao_targets);
        }

        if self.settings.bloom_active() {
            self.bloom_stage.render(
                encoder,
                &self.uniform_bind_group,
                &self.bloom_chain,
                self.settings.bloom.scatter,
            );
        }

        if self.settings.depth_of_field_active() {
            self.dof_stage.render(
                encoder,
                &self.uniform_bind_group,
                &self.dof_half.view,
                &self.dof_blur.view,
            );
        }

        if self.settings.auto_exposure_active() {
            self.exposure_stage.render(
                encoder,
                &self.uniform_bind_group,
                (
                    self.viewport_resolution.x as u32,
                    self.viewport_resolution.y as u32,
                ),
            );
        }

        let composite_bind_group = self
            .composite_bind_group
            .as_ref()
            .expect("Composite bind group not initialized");
        {
            let mut pass = begin_color_pass(
                encoder,
                "PostCompositePass",
                ColorAttachment::single(&self.display.view),
                wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                region,
            );
            pass.set_pipeline(&self.composite_pipeline.pipeline);
            pass.set_bind_group(0, &self.uniform_bind_group, &[]);
            pass.set_bind_group(1, composite_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        let final_bind_group = self
            .final_bind_group
            .as_ref()
            .expect("Final bind group not initialized");
        let mut pass = begin_color_pass(
            encoder,
            "PostFinalPass",
            ColorAttachment::single(target),
            wgpu::LoadOp::Clear(wgpu::Color::BLACK),
            region,
        );
        pass.set_pipeline(&self.final_pipeline.pipeline);
        pass.set_bind_group(0, &self.uniform_bind_group, &[]);
        pass.set_bind_group(1, final_bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    fn mark_bind_groups_dirty(&mut self) {
        self.depth_resolve_bind_group = None;
        self.composite_bind_group = None;
        self.final_bind_group = None;
        self.ssao_stage.invalidate();
        self.ssr_stage.invalidate();
        if let Some(stage) = self.rt_reflection_stage.as_mut() {
            stage.invalidate();
        }
        self.bloom_stage.invalidate();
        self.exposure_stage.invalidate();
        self.dof_stage.invalidate();
        self.temporal_stage.invalidate();
        self.bind_groups_dirty = true;
    }

    fn ensure_cached_bind_groups(&mut self, device: &wgpu::Device) {
        if !self.bind_groups_dirty {
            return;
        }

        let depth_view = self
            .cached_depth_view
            .as_ref()
            .expect("Depth view must be set before executing post process");

        self.depth_resolve_bind_group = match (
            self.depth_resolve_pipeline.as_ref(),
            self.resolved_depth.as_ref(),
        ) {
            (Some(depth_pipeline), Some(_)) => {
                Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("DepthResolveBindGroup"),
                    layout: &depth_pipeline.layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(depth_view),
                    }],
                }))
            }
            _ => None,
        };

        // SSAO reads single-sample depth: the resolved copy under MSAA.
        let ssao_depth = self
            .resolved_depth
            .as_ref()
            .map_or(depth_view, |resolved| &resolved.view);
        self.ssao_stage.ensure_bind_groups(
            device,
            SsaoBindGroupInputs {
                depth_view: ssao_depth,
                noise_view: &self.noise_view,
                sampler_noise: &self.sampler_noise,
                sampler_linear: &self.sampler_linear,
                normal_view: &self.normal_source.view,
                position_view: &self.position_source.view,
                targets: SsaoTargets {
                    output: &self.ssao.view,
                    ping: &self.ssao_ping.view,
                },
            },
        );

        self.ssr_stage.ensure_bind_groups(
            device,
            SsrInputs {
                scene: &self.scene_source.view,
                normal: &self.normal_source.view,
                position: &self.position_source.view,
                sampler: &self.sampler_linear,
                reflection: &self.ssr_half.view,
                ping: &self.ssr_ping.view,
                accumulated: &self.ssr_accumulated.view,
                history: &self.ssr_history.view,
            },
        );
        if let Some(stage) = self.rt_reflection_stage.as_mut() {
            stage.ensure_bind_groups(
                device,
                RtReflectionInputs {
                    scene: &self.scene_source.view,
                    normal: &self.normal_source.view,
                    position: &self.position_source.view,
                    sampler: &self.sampler_linear,
                },
            );
        }

        // Later passes read the temporal output when TAA or motion blur run.
        let (taa, motion_blur) = self.temporal_routing;
        let taa_or_scene = if taa {
            &self.taa_output.view
        } else {
            &self.scene_source.view
        };
        let post_input = if motion_blur {
            &self.motion_output.view
        } else {
            taa_or_scene
        };
        self.temporal_stage.ensure_bind_groups(
            device,
            TemporalInputs {
                scene: &self.scene_source.view,
                history: &self.taa_history.view,
                motion: &self.normal_source.view,
                position: &self.position_source.view,
                sampler: &self.sampler_linear,
                blur_input: taa_or_scene,
            },
        );

        self.bloom_stage.ensure_bind_groups(
            device,
            BloomBindGroupInputs {
                scene_view: post_input,
                sampler_linear: &self.sampler_linear,
                chain: &self.bloom_chain,
            },
        );

        self.exposure_stage.ensure_bind_group(device, post_input);
        self.dof_stage.ensure_bind_groups(
            device,
            post_input,
            &self.position_source.view,
            &self.dof_half.view,
            &self.sampler_linear,
        );

        self.composite_bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("CompositeBindGroup"),
            layout: &self.composite_pipeline.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 90,
                    resource: wgpu::BindingResource::TextureView(post_input),
                },
                wgpu::BindGroupEntry {
                    binding: 91,
                    resource: wgpu::BindingResource::TextureView(&self.ssao.view),
                },
                wgpu::BindGroupEntry {
                    binding: 92,
                    resource: wgpu::BindingResource::TextureView(&self.bloom_chain[0].view),
                },
                wgpu::BindGroupEntry {
                    binding: 93,
                    resource: wgpu::BindingResource::Sampler(&self.sampler_linear),
                },
                wgpu::BindGroupEntry {
                    binding: 94,
                    resource: self.exposure_stage.state_buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 95,
                    resource: wgpu::BindingResource::TextureView(self.lut.view()),
                },
                wgpu::BindGroupEntry {
                    binding: 96,
                    resource: wgpu::BindingResource::TextureView(&self.dof_blur.view),
                },
            ],
        }));

        self.final_bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("FinalBindGroup"),
            layout: &self.final_pipeline.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 100,
                    resource: wgpu::BindingResource::TextureView(&self.display.view),
                },
                wgpu::BindGroupEntry {
                    binding: 101,
                    resource: wgpu::BindingResource::Sampler(&self.sampler_linear),
                },
            ],
        }));

        self.bind_groups_dirty = false;
    }

    fn upload_uniform(&self, queue: &wgpu::Queue) {
        let uniform = PostProcessUniform::new(&UniformInputs {
            camera: &self.camera,
            viewport_resolution: self.viewport_resolution,
            viewport_offset: self.viewport_offset,
            viewport_scale: self.viewport_scale,
            settings: &self.settings,
            lut_loaded: self.lut.is_loaded(),
            taa_history_valid: self.taa_history_valid,
            display_peak: self.display_peak,
            encode_srgb: self.encode_srgb,
            seconds: self.clock.seconds,
            delta: self.clock.delta,
            frame: self.clock.frame,
        });
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniform));
    }
}

/// Every render target whose size follows the surface.
struct SizedTargets {
    scene_source: TextureBundle,
    scene_msaa: Option<MsaaTarget>,
    normal_source: TextureBundle,
    normal_msaa: Option<MsaaTarget>,
    position_source: TextureBundle,
    position_msaa: Option<MsaaTarget>,
    ssao: TextureBundle,
    ssao_ping: TextureBundle,
    ssr_half: TextureBundle,
    ssr_ping: TextureBundle,
    ssr_accumulated: TextureBundle,
    ssr_history: TextureBundle,
    bloom_chain: Vec<BloomMip>,
    dof_half: TextureBundle,
    dof_blur: TextureBundle,
    taa_output: TextureBundle,
    taa_history: TextureBundle,
    motion_output: TextureBundle,
    display: TextureBundle,
    resolved_depth: Option<TextureBundle>,
}

impl SizedTargets {
    fn new(device: &wgpu::Device, size: &wgpu::Extent3d, sample_count: u32) -> Self {
        let (scene_source, scene_msaa) =
            create_msaa_target(device, size, HDR_FORMAT, sample_count, "SceneColor");
        let (normal_source, normal_msaa) = create_msaa_target(
            device,
            size,
            GBUFFER_NORMAL_FORMAT,
            sample_count,
            "SceneNormal",
        );
        let (position_source, position_msaa) = create_msaa_target(
            device,
            size,
            GBUFFER_POSITION_FORMAT,
            sample_count,
            "ScenePosition",
        );
        Self {
            scene_source,
            scene_msaa,
            normal_source,
            normal_msaa,
            position_source,
            position_msaa,
            ssao: TextureBundle::ssao(device, size),
            ssao_ping: TextureBundle::ssao(device, size),
            ssr_half: TextureBundle::copyable(device, &half_size(size), HDR_FORMAT, "SsrHalf"),
            ssr_ping: TextureBundle::color(device, &half_size(size), HDR_FORMAT, "SsrPing"),
            ssr_accumulated: TextureBundle::copyable(
                device,
                &half_size(size),
                HDR_FORMAT,
                "SsrAccumulated",
            ),
            ssr_history: TextureBundle::copyable(
                device,
                &half_size(size),
                HDR_FORMAT,
                "SsrHistory",
            ),
            bloom_chain: create_bloom_chain(device, size),
            dof_half: TextureBundle::color(device, &half_size(size), DOF_FORMAT, "DofHalf"),
            dof_blur: TextureBundle::color(device, &half_size(size), DOF_FORMAT, "DofBlur"),
            taa_output: TextureBundle::copyable(device, size, HDR_FORMAT, "TaaOutput"),
            taa_history: TextureBundle::copyable(device, size, HDR_FORMAT, "TaaHistory"),
            motion_output: TextureBundle::color(device, size, HDR_FORMAT, "MotionBlurOutput"),
            display: TextureBundle::color(device, size, DISPLAY_FORMAT, "PostDisplay"),
            resolved_depth: (sample_count > 1)
                .then(|| TextureBundle::depth(device, size, "ResolvedDepth")),
        }
    }
}

fn create_msaa_target(
    device: &wgpu::Device,
    size: &wgpu::Extent3d,
    format: wgpu::TextureFormat,
    sample_count: u32,
    label: &str,
) -> (TextureBundle, Option<MsaaTarget>) {
    let source = TextureBundle::color(device, size, format, &format!("{label}Source"));
    let msaa = (sample_count > 1)
        .then(|| MsaaTarget::new(device, size, format, sample_count, &format!("{label}Msaa")));
    (source, msaa)
}

fn is_float_format(format: wgpu::TextureFormat) -> bool {
    matches!(
        format,
        wgpu::TextureFormat::Rgba16Float | wgpu::TextureFormat::Rgba32Float
    )
}

fn half_size(size: &wgpu::Extent3d) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width: (size.width / 2).max(1),
        height: (size.height / 2).max(1),
        depth_or_array_layers: 1,
    }
}

fn create_bloom_chain(device: &wgpu::Device, size: &wgpu::Extent3d) -> Vec<BloomMip> {
    let mut width = (size.width.max(2) / 2).max(1);
    let mut height = (size.height.max(2) / 2).max(1);
    (0..BLOOM_MIP_COUNT)
        .map(|level| {
            let mip = BloomMip::new(
                device,
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                &format!("Bloom{level}"),
            );
            width = (width / 2).max(1);
            height = (height / 2).max(1);
            mip
        })
        .collect()
}

fn create_noise_texture(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::Texture {
    let data_bytes = bytemuck::cast_slice(&SSAO_NOISE_DATA);

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("SsaoNoiseTexture"),
        size: wgpu::Extent3d {
            width: NOISE_TEXTURE_SIZE,
            height: NOISE_TEXTURE_SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });

    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        data_bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some((4 * std::mem::size_of::<f32>()) as u32 * NOISE_TEXTURE_SIZE),
            rows_per_image: Some(NOISE_TEXTURE_SIZE),
        },
        wgpu::Extent3d {
            width: NOISE_TEXTURE_SIZE,
            height: NOISE_TEXTURE_SIZE,
            depth_or_array_layers: 1,
        },
    );

    texture
}

struct UniformInputs<'a> {
    camera: &'a PostProcessCamera,
    viewport_resolution: Vec2,
    viewport_offset: Vec2,
    viewport_scale: Vec2,
    settings: &'a PostProcessSettings,
    lut_loaded: bool,
    taa_history_valid: bool,
    display_peak: f32,
    encode_srgb: bool,
    seconds: f32,
    delta: f32,
    frame: u32,
}

/// Mirrors `PostUniform` in shader/postprocess/common.wgsl.
#[repr(C, align(16))]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PostProcessUniform {
    view_proj: [[f32; 4]; 4],
    view_proj_inv: [[f32; 4]; 4],
    proj: [[f32; 4]; 4],
    proj_inv: [[f32; 4]; 4],
    view: [[f32; 4]; 4],
    view_inv: [[f32; 4]; 4],
    camera_position: [f32; 4],
    viewport_resolution: [f32; 2],
    viewport_offset: [f32; 2],
    viewport_scale: [f32; 2],
    radius_bias: [f32; 2],
    intensity_power: [f32; 2],
    noise_scale: [f32; 2],
    near_far: [f32; 2],
    time_delta: [f32; 2],
    effects: [f32; 4],
    bloom_params: [f32; 4],
    bloom_tint: [f32; 4],
    exposure_params: [f32; 4],
    exposure_adapt: [f32; 4],
    white_balance: [f32; 4],
    color_filter: [f32; 4],
    grading: [f32; 4],
    lift: [f32; 4],
    gamma: [f32; 4],
    gain: [f32; 4],
    vignette: [f32; 4],
    vignette_color: [f32; 4],
    vignette_center: [f32; 4],
    lens: [f32; 4],
    output: [f32; 4],
    dof_params: [f32; 4],
    dof_ranges: [f32; 4],
    misc: [f32; 4],
    display: [f32; 4],
    ssr: [f32; 4],
}

fn flag(on: bool) -> f32 {
    if on {
        1.0
    } else {
        0.0
    }
}

fn vec3_4(v: [f32; 3], w: f32) -> [f32; 4] {
    [v[0], v[1], v[2], w]
}

impl PostProcessUniform {
    fn new(inputs: &UniformInputs<'_>) -> Self {
        let camera = inputs.camera;
        let settings = inputs.settings;
        let resolution = inputs.viewport_resolution;
        let noise_scale = resolution / NOISE_TEXTURE_SIZE as f32;
        let ssao = &settings.ssao;
        let bloom = &settings.bloom;
        let exposure = &settings.exposure;
        let color = &settings.color;
        let vignette = &settings.vignette;
        let (lift, gamma, gain) = color.lift_gamma_gain();
        let aspect = resolution.x / resolution.y.max(1.0);
        let effect_on = |enabled: bool, amount: f32| if enabled { amount } else { 0.0 };

        Self {
            view_proj: camera.view_proj.to_cols_array_2d(),
            view_proj_inv: camera.view_proj_inv.to_cols_array_2d(),
            proj: camera.proj.to_cols_array_2d(),
            proj_inv: camera.proj.inverse().to_cols_array_2d(),
            view: camera.view.to_cols_array_2d(),
            view_inv: camera.view_inv.to_cols_array_2d(),
            camera_position: camera.position.extend(1.0).to_array(),
            viewport_resolution: resolution.to_array(),
            viewport_offset: inputs.viewport_offset.to_array(),
            viewport_scale: inputs.viewport_scale.to_array(),
            radius_bias: [ssao.radius, ssao.bias],
            intensity_power: [ssao.intensity, ssao.power],
            noise_scale: noise_scale.to_array(),
            near_far: [camera.near, camera.far],
            time_delta: [inputs.seconds, inputs.delta],
            effects: [
                flag(settings.ssao_active()),
                flag(settings.bloom_active()),
                flag(settings.anti_aliasing == AntiAliasing::Fxaa),
                flag(settings.auto_exposure_active()),
            ],
            bloom_params: [bloom.threshold, bloom.knee, bloom.scatter, bloom.intensity],
            bloom_tint: vec3_4(bloom.tint, 0.0),
            exposure_params: [
                exposure.manual_multiplier(),
                2f32.powf(color.post_exposure),
                exposure.min_ev,
                exposure.max_ev,
            ],
            exposure_adapt: [
                exposure.speed_up,
                exposure.speed_down,
                exposure.low_percent,
                exposure.high_percent,
            ],
            white_balance: vec3_4(color.white_balance_lms(), 0.0),
            color_filter: vec3_4(color.color_filter, color.hue_shift / 360.0),
            grading: [
                color.contrast,
                color.saturation,
                color.vibrance,
                settings.tonemapping.shader_index(),
            ],
            lift: vec3_4(lift, 0.0),
            gamma: vec3_4(gamma, 0.0),
            gain: vec3_4(gain, 0.0),
            vignette: [
                effect_on(vignette.enabled, vignette.intensity),
                vignette.smoothness,
                vignette.roundness,
                aspect,
            ],
            vignette_color: vec3_4(vignette.color, 0.0),
            vignette_center: [
                vignette.center[0],
                vignette.center[1],
                flag(vignette.rounded),
                0.0,
            ],
            lens: [
                effect_on(
                    settings.chromatic_aberration.enabled,
                    settings.chromatic_aberration.intensity,
                ),
                effect_on(settings.film_grain.enabled, settings.film_grain.intensity),
                settings.film_grain.response,
                effect_on(settings.sharpen.enabled, settings.sharpen.strength),
            ],
            dof_params: if settings.depth_of_field_active() {
                let dof = &settings.depth_of_field;
                [
                    match dof.mode {
                        FocusMode::Physical => 1.0,
                        FocusMode::Manual => 2.0,
                    },
                    dof.focus_distance,
                    dof.aperture,
                    // Authored at 1080p; keep the same share of the view at any size.
                    dof.max_blur * resolution.y / 1080.0,
                ]
            } else {
                [0.0; 4]
            },
            dof_ranges: {
                let dof = &settings.depth_of_field;
                [dof.near_start, dof.near_end, dof.far_start, dof.far_end]
            },
            display: [
                inputs.display_peak,
                flag(inputs.display_peak > 1.0),
                0.0,
                0.0,
            ],
            misc: [
                flag(settings.depth_of_field.autofocus),
                flag(inputs.taa_history_valid),
                if settings.motion_blur_active() {
                    settings.motion_blur.intensity
                } else {
                    0.0
                },
                settings.motion_blur.max_blur * resolution.y / 1080.0,
            ],
            ssr: {
                let ssr = &settings.ssr;
                [
                    effect_on(settings.ssr_active(), ssr.intensity),
                    ssr.max_distance,
                    ssr.thickness,
                    ssr.blur,
                ]
            },
            output: [
                flag(inputs.encode_srgb),
                1.0,
                inputs.frame as f32,
                if inputs.lut_loaded && settings.lut.active() {
                    settings.lut.intensity
                } else {
                    0.0
                },
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_space_camera_matches_jittered_gbuffer_at_the_horizon() {
        let eye = Vec3::new(10.6, 8.4, 17.2);
        let view = Mat4::look_at_rh(eye, Vec3::new(0.0, 3.7, 0.0), Vec3::Y);
        let proj = Mat4::perspective_rh(43.0_f32.to_radians(), 16.0 / 9.0, 0.1, 1200.0);
        let stable_vp = proj * view;
        let camera = PostProcessCamera {
            proj,
            view,
            view_inv: view.inverse(),
            view_proj: stable_vp,
            view_proj_inv: stable_vp.inverse(),
            position: eye,
            near: 0.1,
            far: 1200.0,
        };
        for jitter in [
            Vec2::ZERO,
            Vec2::new(0.5 / 1824.0, -0.5 / 1080.0),
            Vec2::new(-0.9 / 1824.0, 0.8 / 1080.0),
        ] {
            let post = camera.with_projection_jitter(jitter);
            let raster_vp = Mat4::from_translation(jitter.extend(0.0)) * stable_vp;
            for position in [Vec3::new(0.0, -0.15, 0.0), Vec3::new(-600.0, -0.15, -900.0)] {
                let world = position.extend(1.0);
                let raster = raster_vp * world;
                let reflection = post.view_proj * world;
                let occlusion = post.proj * (post.view * world);
                let pixel = raster.truncate() / raster.w;
                assert!((reflection.truncate() / reflection.w).abs_diff_eq(pixel, 1e-6));
                assert!((occlusion.truncate() / occlusion.w).abs_diff_eq(pixel, 1e-6));
                let restored = post.view_proj_inv * raster;
                assert!((restored.truncate() / restored.w).abs_diff_eq(position, 1.0));
            }
            assert_eq!(post.view, camera.view);
            assert_eq!(post.near, camera.near);
            assert_eq!(post.far, camera.far);
        }
    }

    #[test]
    fn uniform_matches_the_shader_layout() {
        // 6 matrices, camera position, 8 vec2 pairs and 21 vec4 blocks.
        assert_eq!(std::mem::size_of::<PostProcessUniform>(), 800);
        assert_eq!(std::mem::align_of::<PostProcessUniform>(), 16);
    }

    #[test]
    fn postprocess_shader_validates() {
        let source = postprocess_shader_source();
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|err| panic!("{}", err.emit_to_string(&source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .unwrap_or_else(|err| panic!("{}", err.emit_to_string(&source)));
    }

    /// `encode_surface_w` (common.wgsl) and `ssr_surface_code` (reflection_common.wgsl),
    /// mirrored here: every code must survive the half-float G-buffer target exactly and
    /// read as a surface.
    #[test]
    fn surface_w_codes_survive_the_half_float_target() {
        let encode =
            |code: u32| (2f32).powi((code >> 10) as i32) * (1.0 + (code & 1023) as f32 / 1024.0);
        let decode = |w: f32| {
            let bits = w.to_bits();
            let power = ((bits >> 23) & 0xff) as i32 - 127;
            if power < 0 {
                return 0;
            }
            (((power as u32) << 10) | ((bits >> 13) & 1023)).min(0x3fff)
        };
        for code in 0..0x4000u32 {
            let stored = half::f16::from_f32(encode(code)).to_f32();
            assert!(stored >= 0.5, "code {code} would read as no surface");
            assert_eq!(decode(stored), code);
        }
        assert_eq!(decode(0.0), 0);
    }

    #[test]
    fn rt_reflections_shader_validates() {
        for textures in [false, true] {
            let source = rt_reflections::rt_reflections_shader_source(textures);
            let module = naga::front::wgsl::parse_str(&source)
                .unwrap_or_else(|err| panic!("{}", err.emit_to_string(&source)));
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::RAY_QUERY
                    | naga::valid::Capabilities::TEXTURE_AND_SAMPLER_BINDING_ARRAY
                    | naga::valid::Capabilities::TEXTURE_AND_SAMPLER_BINDING_ARRAY_NON_UNIFORM_INDEXING,
            )
            .validate(&module)
            .unwrap_or_else(|err| panic!("{}", err.emit_to_string(&source)));
        }
    }

    #[test]
    fn depth_resolve_shader_validates() {
        let source = ShaderBuilder::new()
            .with_module(include_str!("../../shader/postprocess/fullscreen.wgsl"))
            .build(include_str!("../../shader/depth_resolve.wgsl"));
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|err| panic!("{}", err.emit_to_string(&source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|err| panic!("{}", err.emit_to_string(&source)));
    }
}
