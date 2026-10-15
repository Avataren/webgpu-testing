//! Post-processing on the HDR scene: SSAO, bloom, auto exposure, colour correction,
//! tonemapping, vignette, chromatic aberration, film grain, sharpening and FXAA.
//!
//! With path tracing on, [`PostProcess::path_trace`] runs first, right after the main
//! pass (see `path_tracing`): it replaces the opaque lighting, and the reflection and
//! ambient occlusion passes below stand down for that frame.
//!
//! Frame order: depth resolve (MSAA only) -> screen-space reflections (added to the scene)
//! -> TAA / motion blur -> SSAO -> bloom -> depth of field -> auto exposure -> composite
//! (HDR to display, into an intermediate target) -> final (anti-aliasing, sharpening,
//! grain and dither, onto the surface).
//!
//! With upscaling the scene is drawn into a smaller rectangle of the same targets (the
//! scene region, at the display region's origin). The passes up to the TAA read it through
//! a second uniform describing that rectangle; the TAA upscales into the display region,
//! and everything after it runs at output resolution, mapping into the scene region where
//! it reads the G-buffer or occlusion. Depth and pick ids are upscaled too, for what is
//! drawn after post-processing.

mod bloom;
mod dof;
mod exposure;
mod fsr;
mod lut;
mod path_tracing;
mod pipelines;
mod resources;
mod rt_ao;
mod rt_reflections;
mod settings;
mod ssao;
mod ssr;
mod temporal;
mod upscale;

pub use lut::CubeLut;
pub use settings::{
    setting_overridden, AntiAliasing, BloomSettings, ChromaticAberrationSettings, ColorAdjustments,
    DepthOfFieldSettings, ExposureMode, ExposureSettings, FilmGrainSettings, FocusMode,
    LegacyBloomSettings, LegacyColorGrading, LegacySsaoSettings, LutSettings, MotionBlurSettings,
    PathTracingSettings, PostProcessEffects, PostProcessSettings, ReflectionSettings,
    SharpenSettings, SsaoSettings, Tonemapper, Upscaler, UpscalingSettings, VignetteSettings,
    TRACKBALL_NEUTRAL,
};

use bloom::{BloomBindGroupInputs, BloomStage};
use dof::DofStage;
use exposure::ExposureStage;
use fsr::{FsrFrame, FsrUpscaler, ReactiveFrame};
use lut::LutTexture;
pub(crate) use path_tracing::CausticSources;
use path_tracing::{PathTracingFrame, PathTracingInputs, PathTracingStage};
use pipelines::{
    build_composite, build_depth_resolve, build_final, CompositePipeline, DepthResolvePipeline,
    FinalPipeline,
};
use resources::{BloomMip, LazyPickTarget, MsaaTarget, TextureBundle};
use rt_ao::RtAoStage;
use rt_reflections::{RtReflectionInputs, RtReflectionStage};
use ssao::{SsaoBindGroupInputs, SsaoStage, SsaoTargets};
use ssr::{SsrInputs, SsrStage, SsrTargets};
use temporal::{TemporalInputs, TemporalStage};
use upscale::UpscaledTargets;

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
    /// A copy of `scene_source` the refraction pass reads while adding to it.
    scene_copy: TextureBundle,
    scene_msaa: Option<MsaaTarget>,
    normal_source: TextureBundle,
    normal_msaa: Option<MsaaTarget>,
    position_source: TextureBundle,
    position_msaa: Option<MsaaTarget>,
    pick_target: LazyPickTarget,
    /// Output-resolution copies of depth and pick ids while upscaling.
    upscaled: UpscaledTargets,
    ssao: TextureBundle,
    ssao_ping: TextureBundle,
    /// Reflection textures, at half or full resolution (`ssr.full_resolution`).
    reflection: ReflectionTargets,
    bloom_chain: Vec<BloomMip>,
    dof_half: TextureBundle,
    dof_blur: TextureBundle,
    taa_output: TextureBundle,
    taa_history: TextureBundle,
    motion_output: TextureBundle,
    display: TextureBundle,
    /// The flat ambient light the main pass adds (reflections replace it on metals).
    ambient: [f32; 3],
    sampler_linear: wgpu::Sampler,
    sampler_noise: wgpu::Sampler,
    _noise_texture: wgpu::Texture,
    noise_view: wgpu::TextureView,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    /// The uniform for passes on the scene before it is upscaled: the same, with the
    /// viewport of the scene region.
    scene_uniform_buffer: wgpu::Buffer,
    scene_uniform_bind_group: wgpu::BindGroup,
    depth_resolve_pipeline: Option<DepthResolvePipeline>,
    composite_pipeline: CompositePipeline,
    final_pipeline: FinalPipeline,
    ssao_stage: SsaoStage,
    ssr_stage: SsrStage,
    /// Ray-traced reflections, when the device has ray queries and the pass built.
    rt_reflection_stage: Option<RtReflectionStage>,
    /// Ray-traced ambient occlusion, likewise.
    rt_ao_stage: Option<RtAoStage>,
    /// Path tracing with its denoiser, likewise.
    path_tracing_stage: Option<PathTracingStage>,
    /// This frame's lighting was path traced (reflection and occlusion passes stand down).
    path_traced: bool,
    /// Kept to build stages added after construction (ray-traced reflections).
    uniform_layout: wgpu::BindGroupLayout,
    bloom_stage: BloomStage,
    exposure_stage: ExposureStage,
    dof_stage: DofStage,
    temporal_stage: TemporalStage,
    /// FSR 3.1 upscaling, built the first time it runs.
    fsr: Option<FsrUpscaler>,
    /// The scene as the opaque pass left it, before particles and transparent surfaces:
    /// FSR's reactive mask compares it with the finished scene.
    opaque_copy: Option<TextureBundle>,
    /// The scene with particles drawn over the opaque pass, before transparent surfaces,
    /// while particles are ray traced (`particles_traced`).
    particle_copy: Option<TextureBundle>,
    particles_traced: bool,
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
    /// The scene region's size in pixels and share of the targets; the display region's
    /// when not upscaling.
    scene_resolution: Vec2,
    scene_scale: Vec2,
    /// The scene renders smaller than the display region and the TAA pass upscales it.
    upscaling: bool,
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
    /// `view_proj` without the TAA jitter (what motion vectors are measured with).
    pub unjittered_view_proj: Mat4,
    pub view_proj_inv: Mat4,
    pub view: Mat4,
    pub view_inv: Mat4,
    pub position: Vec3,
    pub near: f32,
    pub far: f32,
    /// How far the TAA jitter moves the image, in scene pixels (+y down).
    pub jitter_pixels: Vec2,
}

impl Default for PostProcessCamera {
    fn default() -> Self {
        Self {
            proj: Mat4::IDENTITY,
            view_proj: Mat4::IDENTITY,
            unjittered_view_proj: Mat4::IDENTITY,
            view_proj_inv: Mat4::IDENTITY,
            view: Mat4::IDENTITY,
            view_inv: Mat4::IDENTITY,
            position: Vec3::ZERO,
            near: 0.01,
            far: 100.0,
            jitter_pixels: Vec2::ZERO,
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
        let scene_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("PostProcessSceneUniformBuffer"),
            size: std::mem::size_of::<PostProcessUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let scene_uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("PostProcessSceneUniformBindGroup"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: scene_uniform_buffer.as_entire_binding(),
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
            scene_copy: targets.scene_copy,
            scene_msaa: targets.scene_msaa,
            normal_source: targets.normal_source,
            normal_msaa: targets.normal_msaa,
            position_source: targets.position_source,
            position_msaa: targets.position_msaa,
            pick_target: LazyPickTarget::new(),
            upscaled: UpscaledTargets::new(device, &uniform_layout),
            ssao: targets.ssao,
            ssao_ping: targets.ssao_ping,
            reflection: ReflectionTargets::new(device, &size, false),
            bloom_chain: targets.bloom_chain,
            dof_half: targets.dof_half,
            dof_blur: targets.dof_blur,
            taa_output: targets.taa_output,
            taa_history: targets.taa_history,
            motion_output: targets.motion_output,
            display: targets.display,
            ambient: [0.0; 3],
            sampler_linear,
            sampler_noise,
            _noise_texture: noise_texture,
            noise_view,
            uniform_buffer,
            uniform_bind_group,
            scene_uniform_buffer,
            scene_uniform_bind_group,
            depth_resolve_pipeline,
            composite_pipeline,
            final_pipeline,
            ssao_stage,
            ssr_stage,
            rt_reflection_stage: None,
            rt_ao_stage: None,
            path_tracing_stage: None,
            path_traced: false,
            uniform_layout,
            bloom_stage,
            exposure_stage,
            dof_stage,
            temporal_stage,
            fsr: None,
            opaque_copy: None,
            particle_copy: None,
            particles_traced: false,
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
            scene_resolution: Vec2::new(size.width as f32, size.height as f32),
            scene_scale: Vec2::ONE,
            upscaling: false,
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
        self.scene_copy = targets.scene_copy;
        self.scene_msaa = targets.scene_msaa;
        self.normal_source = targets.normal_source;
        self.normal_msaa = targets.normal_msaa;
        self.position_source = targets.position_source;
        self.position_msaa = targets.position_msaa;
        self.ssao = targets.ssao;
        self.ssao_ping = targets.ssao_ping;
        self.reflection = ReflectionTargets::new(device, &self.size, self.reflection.full);
        self.bloom_chain = targets.bloom_chain;
        self.dof_half = targets.dof_half;
        self.dof_blur = targets.dof_blur;
        self.taa_output = targets.taa_output;
        self.taa_history = targets.taa_history;
        self.motion_output = targets.motion_output;
        self.taa_history_valid = false;
        if let Some(fsr) = self.fsr.as_mut() {
            fsr.resize();
        }
        self.opaque_copy = None;
        self.particle_copy = None;
        self.particles_traced = false;
        self.reflection_history = None;
        self.display = targets.display;
        self.resolved_depth = targets.resolved_depth;
        self.pick_target.invalidate();
        self.upscaled.invalidate();
        self.viewport_resolution = Vec2::new(width as f32, height as f32);
        self.viewport_offset = Vec2::ZERO;
        self.viewport_scale = Vec2::ONE;
        self.scene_resolution = self.viewport_resolution;
        self.scene_scale = Vec2::ONE;
        self.mark_bind_groups_dirty();
        self.upload_uniform(queue);
    }

    /// The part of the targets post-processing outputs to (`region`, all of them when
    /// `None`) and the part the scene was drawn into: the same, or a smaller rectangle at
    /// its origin when upscaling.
    pub fn update_viewport(
        &mut self,
        queue: &wgpu::Queue,
        region: Option<RenderRegion>,
        scene_region: Option<RenderRegion>,
    ) {
        let full = Vec2::new(
            self.size.width.max(1) as f32,
            self.size.height.max(1) as f32,
        );
        let rect = |region: Option<RenderRegion>| match region {
            Some(region) => {
                let size = Vec2::new(region.width().max(1) as f32, region.height().max(1) as f32);
                let offset = Vec2::new(region.x() as f32, region.y() as f32) / full;
                (size, offset, size / full)
            }
            None => (full, Vec2::ZERO, Vec2::ONE),
        };
        let (resolution, offset, scale) = rect(region);
        let (scene_resolution, _, scene_scale) = rect(scene_region.or(region));
        let upscaling = scene_resolution != resolution;

        if self.viewport_resolution != resolution
            || self.viewport_offset != offset
            || self.viewport_scale != scale
            || self.scene_resolution != scene_resolution
        {
            self.viewport_resolution = resolution;
            self.viewport_offset = offset;
            self.viewport_scale = scale;
            self.scene_resolution = scene_resolution;
            self.scene_scale = scene_scale;
            // The history no longer lines up with the view.
            self.taa_history_valid = false;
            self.reflection_history = None;
            self.upload_uniform(queue);
        }
        if upscaling != self.upscaling {
            self.upscaling = upscaling;
            self.refresh_temporal_routing();
            self.upload_uniform(queue);
        }
    }

    /// Which temporal passes run: the TAA pass when TAA is on or it upscales.
    fn refresh_temporal_routing(&mut self) {
        let routing = (
            self.settings.taa_active() || self.upscaling,
            self.settings.motion_blur_active(),
        );
        if routing != self.temporal_routing {
            if routing.0 && !self.temporal_routing.0 {
                self.taa_history_valid = false;
            }
            self.temporal_routing = routing;
            self.mark_bind_groups_dirty();
        }
    }

    /// Called once per rendered frame with the camera it is seen through; also advances
    /// the clock animated effects and exposure adaptation run on.
    pub fn update_camera(&mut self, queue: &wgpu::Queue, camera: PostProcessCamera) {
        if is_camera_cut(&self.camera, &camera) {
            // Reprojected histories would show the old view for a few frames.
            self.reflection_history = None;
            self.taa_history_valid = false;
            if let Some(stage) = self.path_tracing_stage.as_mut() {
                stage.reset_history();
            }
        }
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

    /// Single-sample depth at output resolution, for what draws after post-processing.
    pub fn after_postprocess_depth_view(&self) -> Option<&wgpu::TextureView> {
        if let Some(upscaled) = self.upscaled.depth_view().filter(|_| self.upscaling) {
            Some(upscaled)
        } else if let Some(resolved) = self.resolved_depth.as_ref() {
            Some(&resolved.view)
        } else {
            self.cached_depth_view.as_ref()
        }
    }

    /// Whether the scene renders below the output resolution and is upscaled.
    pub fn is_upscaling(&self) -> bool {
        self.upscaling
    }

    /// Whether this frame upscales with FSR: it is chosen, and the camera is a perspective
    /// one (FSR derives view depth from a perspective projection; TAAU handles the rest).
    fn fsr_active(&self) -> bool {
        self.upscaling
            && self.settings.upscaling.method == Upscaler::Fsr
            && self.camera.proj.w_axis.w == 0.0
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
        if settings.ssr.full_resolution != self.reflection.full {
            self.reflection =
                ReflectionTargets::new(device, &self.size, settings.ssr.full_resolution);
            self.reflection_history = None;
            self.mark_bind_groups_dirty();
            // The uniform's reflection texel scale changes with it.
            self.upload_uniform(queue);
        }
        if self.settings != *settings {
            let was_auto = self.settings.auto_exposure_active();
            let was_taa = self.settings.taa_active();
            let previous_upscaler = self.settings.upscaling.method;
            self.settings = settings.clone().sanitized();
            if self.settings.auto_exposure_active() && !was_auto {
                // Start from a fresh measurement instead of whatever was adapted to
                // when auto exposure was last on.
                self.exposure_stage.reset(queue);
            }
            let was_upscaler = previous_upscaler;
            if self.settings.taa_active() != was_taa
                || self.settings.upscaling.method != was_upscaler
            {
                // The upscale pass starts (or stops) accumulating, or another one takes
                // over the history.
                self.taa_history_valid = false;
            }
            self.upload_uniform(queue);
        }
        if !self.settings.path_tracing.enabled {
            if let Some(stage) = self.path_tracing_stage.as_mut() {
                stage.release();
            }
        }
        self.refresh_temporal_routing();
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

    /// Build the ray-traced ambient occlusion pass (`scene_layout` is the ray tracing
    /// scene's). On error occlusion stays screen-space.
    pub(crate) fn enable_ray_traced_ao(
        &mut self,
        device: &wgpu::Device,
        scene_layout: &wgpu::BindGroupLayout,
    ) -> Result<(), String> {
        self.rt_ao_stage = Some(RtAoStage::new(device, &self.uniform_layout, scene_layout)?);
        self.mark_bind_groups_dirty();
        Ok(())
    }

    /// Build the path tracer and its denoiser (layouts as for
    /// [`Self::enable_ray_traced_reflections`]). On error the main pass keeps lighting.
    pub(crate) fn enable_path_tracing(
        &mut self,
        device: &wgpu::Device,
        lights_layout: &wgpu::BindGroupLayout,
        scene_layout: &wgpu::BindGroupLayout,
        textures_layout: Option<&wgpu::BindGroupLayout>,
    ) -> Result<(), String> {
        self.path_tracing_stage = Some(PathTracingStage::new(
            device,
            &self.uniform_layout,
            lights_layout,
            scene_layout,
            textures_layout,
        )?);
        Ok(())
    }

    /// Whether path tracing can run: it built, and the targets are single-sample.
    pub(crate) fn path_tracing_available(&self) -> bool {
        self.path_tracing_stage.is_some() && self.sample_count == 1
    }

    /// Whether this frame's lighting would be path traced if the scene is ready (it needs
    /// single-sample targets).
    pub(crate) fn wants_path_tracing(&self) -> bool {
        self.path_tracing_available() && self.settings.path_tracing.enabled
    }

    /// Path trace the opaque scene and write it over the main pass's colour (call after
    /// the main pass, before particles and transparent surfaces). `lamp_radius` and
    /// `sun_size_degrees` size the lights. Returns whether it ran.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn path_trace(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        ray_tracing: &RayTracingFrame<'_>,
        caustics: CausticSources<'_>,
        scene_region: Option<RenderRegion>,
        timer: &mut crate::renderer::gpu_timer::GpuTimer,
    ) -> bool {
        if !self.wants_path_tracing() {
            return false;
        }
        let Some(region) = scene_region
            .or_else(|| RenderRegion::full(self.size.width, self.size.height))
            .and_then(|region| region.clamp(self.size.width, self.size.height))
        else {
            return false;
        };
        let Some(stage) = self.path_tracing_stage.as_mut() else {
            return false;
        };
        let frame = PathTracingFrame {
            uniform_bind_group: &self.scene_uniform_bind_group,
            ray_tracing,
            caustics,
            inputs: PathTracingInputs {
                normal: &self.normal_source.view,
                position: &self.position_source.view,
                scene: &self.scene_source.view,
            },
            camera: &self.camera,
            settings: &self.settings.path_tracing,
            sun_size_degrees: self.settings.rt_shadows.softness,
            lamp_radius: self.settings.rt_shadows.light_radius,
            region,
            output_height: self.viewport_resolution.y,
            scene: &self.scene_source.view,
            scene_texture: self.scene_source.texture(),
        };
        self.path_traced = stage.run(device, queue, encoder, frame, timer);
        self.path_traced
    }

    /// Whether transmissive surfaces are refracted with ray tracing (otherwise they are
    /// drawn see-through).
    pub(crate) fn ray_traced_refraction(&self) -> bool {
        self.rt_reflection_stage.is_some() && self.settings.refraction.ray_traced
    }

    /// Whether this frame's post-processing would trace rays (reflections or occlusion)
    /// if the scene is ready.
    pub(crate) fn wants_ray_tracing(&self) -> bool {
        self.wants_path_tracing()
            || self.wants_ray_traced_reflections()
            || (self.rt_ao_stage.is_some()
                && self.settings.ssao_active()
                && self.settings.ssao.ray_traced)
    }

    /// Whether this frame's reflections would be ray traced if the scene is ready.
    pub(crate) fn wants_ray_traced_reflections(&self) -> bool {
        self.rt_reflection_stage.is_some()
            && self.settings.ssr_active()
            && self.settings.ssr.ray_traced
    }

    /// The post-processing targets (and the path tracer's), for reading back (see
    /// [`crate::renderer::readback`]). MSAA targets are listed by their resolved copies.
    pub(crate) fn fetchable_textures<'a>(
        &'a self,
        out: &mut Vec<crate::renderer::readback::FetchableTexture<'a>>,
    ) {
        use crate::renderer::readback::FetchableTexture;
        out.push(FetchableTexture::new(
            "scene",
            "The lit HDR scene (linear, before exposure and tonemapping) as post-processing left \
             it: main pass or path tracing, particles, transparent surfaces, refraction and \
             reflections. Alpha: the surface's alpha, or -transmission on transmissive surfaces. \
             While upscaling the scene fills only the scene region",
            self.scene_source.texture(),
        ));
        out.push(FetchableTexture::new(
            "gbuffer.normal",
            "G-buffer normal and motion: xy the octahedral-encoded world normal (shading normal, \
             normal maps included), zw the screen motion since last frame in viewport uv (+y down)",
            self.normal_source.texture(),
        ));
        out.push(FetchableTexture::new(
            "gbuffer.position",
            "G-buffer world position (xyz, metres, half precision). w: below 0.5 no surface; \
             otherwise a code for smoothness and specular colour (encode_surface_w in common.wgsl)",
            self.position_source.texture(),
        ));
        if let Some(pick) = self.pick_target.texture() {
            out.push(FetchableTexture::new(
                "gbuffer.pick",
                "Pick ids (the two halves of each pixel's entity pick id; 0 where nothing pickable)",
                pick,
            ));
        }
        out.push(FetchableTexture::new(
            "ssao",
            "Ambient occlusion (r: 1 open, 0 occluded), screen-space or ray-traced, after the blur",
            self.ssao.texture(),
        ));
        out.push(FetchableTexture::new(
            "reflections.trace",
            "Reflections (half or full resolution): rgb the traced reflection after the blur, \
             compressed as c / (1 + luminance), a its weight",
            self.reflection.trace.texture(),
        ));
        out.push(FetchableTexture::new(
            "reflections.accumulated",
            "Reflections accumulated over frames (same encoding as reflections.trace)",
            self.reflection.accumulated.texture(),
        ));
        out.push(FetchableTexture::new(
            "taa.output",
            "Temporal anti-aliasing (or upscaling) output: HDR colour at output resolution",
            self.taa_output.texture(),
        ));
        out.push(FetchableTexture::new(
            "display",
            "The tonemapped image before the final pass (anti-aliasing, sharpening, grain)",
            self.display.texture(),
        ));
        if let Some(copy) = self.opaque_copy.as_ref() {
            out.push(FetchableTexture::new(
                "scene.opaque",
                "The HDR scene as the opaque pass left it, before particles and transparent \
                 surfaces (kept while FSR upscales or particles are ray traced)",
                copy.texture(),
            ));
        }
        if let Some(depth) = self.resolved_depth.as_ref() {
            out.push(FetchableTexture::new(
                "depth.resolved",
                "Depth resolved from the MSAA depth buffer (0 near to 1 far)",
                depth.texture(),
            ));
        }
        if let Some(stage) = self.path_tracing_stage.as_ref() {
            stage.fetchable_textures(out);
        }
    }

    pub fn settings(&self) -> &PostProcessSettings {
        &self.settings
    }

    /// The flat ambient light the main pass adds to every lit surface this frame (linear
    /// RGB); where a reflection is found on a metal it replaces that light.
    pub fn set_ambient(&mut self, queue: &wgpu::Queue, ambient: [f32; 3]) {
        if self.ambient != ambient {
            self.ambient = ambient;
            self.upload_uniform(queue);
        }
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

    /// The pick target the scene passes write ids into (at the scene's resolution).
    pub fn scene_pick_views(&self) -> Option<PickAttachmentViews<'_>> {
        self.pick_target.views()
    }

    /// The pick target at output resolution, for passes after post-processing (overlays,
    /// gizmos): the upscaled copy of the scene's ids while upscaling.
    pub fn pick_attachment_views(&self) -> Option<PickAttachmentViews<'_>> {
        match self.upscaled.pick.as_ref().filter(|_| self.upscaling) {
            Some(pick) => Some(PickAttachmentViews {
                multisample: &pick.view,
                resolve: None,
            }),
            None => self.pick_target.views(),
        }
    }

    /// The pick ids clicks are read from, at output resolution.
    pub fn pick_texture(&self) -> Option<&wgpu::Texture> {
        match self.upscaled.pick.as_ref().filter(|_| self.upscaling) {
            Some(pick) => Some(pick.texture()),
            None => self.pick_target.texture(),
        }
    }

    pub fn pick_texture_extent(&self) -> Option<wgpu::Extent3d> {
        self.pick_target.extent()
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn execute(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        region: Option<RenderRegion>,
        scene_region: Option<RenderRegion>,
        ray_tracing: Option<RayTracingFrame<'_>>,
        timer: &mut crate::renderer::gpu_timer::GpuTimer,
    ) {
        self.ensure_cached_bind_groups(device);
        let scene_uniform = &self.scene_uniform_bind_group.clone();
        // Before reflections and refraction change the scene.
        if self.temporal_routing.0 && self.fsr_active() {
            self.run_fsr_reactive(encoder, device, queue);
        }

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

        // Refraction first: reflections then see glass with what shows through it.
        if let (Some(stage), Some(frame)) =
            (self.rt_reflection_stage.as_ref(), ray_tracing.as_ref())
        {
            if frame.refraction && self.settings.refraction.ray_traced {
                self.scene_source.copy_to(encoder, &self.scene_copy);
                stage.refract(
                    encoder,
                    scene_uniform,
                    frame,
                    &self.scene_source.view,
                    scene_region,
                );
                timer.mark(encoder, "refraction");
            }
        }

        // Path traced lighting has its reflections and occlusion already.
        let path_traced = std::mem::take(&mut self.path_traced);
        if !path_traced {
            if let Some(stage) = self.path_tracing_stage.as_mut() {
                stage.reset_history();
            }
        }
        if self.settings.ssr_active() && !path_traced {
            let traced = match (self.rt_reflection_stage.as_ref(), ray_tracing.as_ref()) {
                (Some(stage), Some(frame)) if self.settings.ssr.ray_traced => {
                    stage.trace(encoder, scene_uniform, frame, &self.reflection.trace.view)
                }
                _ => false,
            };
            let targets = SsrTargets {
                reflection: &self.reflection.trace,
                ping: &self.reflection.ping.view,
                accumulated: &self.reflection.accumulated,
                history: &self.reflection.history,
                history_valid: self.reflection_history == Some(traced),
            };
            if traced {
                self.ssr_stage.resolve(
                    encoder,
                    scene_uniform,
                    &targets,
                    &self.scene_source.view,
                    scene_region,
                );
            } else {
                self.ssr_stage.render(
                    encoder,
                    scene_uniform,
                    &targets,
                    &self.scene_source.view,
                    scene_region,
                );
            }
            self.reflection_history = Some(traced);
            timer.mark(encoder, "reflections");
        } else {
            self.reflection_history = None;
        }

        let (taa, motion_blur) = self.temporal_routing;
        if taa && self.fsr_active() {
            self.run_fsr(encoder, device, queue);
            self.taa_history_valid = true;
        } else if taa {
            self.temporal_stage.render_taa(
                encoder,
                &self.uniform_bind_group,
                &self.taa_output,
                &self.taa_history,
                self.upscaling,
                region,
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

        timer.mark(encoder, "temporal");
        let ssao_targets = SsaoTargets {
            output: &self.ssao.view,
            ping: &self.ssao_ping.view,
        };
        if self.settings.ssao_active() && !path_traced {
            let traced = match (self.rt_ao_stage.as_ref(), ray_tracing.as_ref()) {
                (Some(stage), Some(frame)) if self.settings.ssao.ray_traced => {
                    stage.trace(encoder, scene_uniform, frame.scene, ssao_targets.output)
                }
                _ => false,
            };
            if traced {
                self.ssao_stage.blur(encoder, scene_uniform, ssao_targets);
            } else {
                self.ssao_stage.render(encoder, scene_uniform, ssao_targets);
            }
            timer.mark(encoder, "ambient_occlusion");
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
        drop(pass);

        if self.upscaling {
            if let Some(depth) = self.cached_depth_view.as_ref() {
                self.upscaled.render(
                    device,
                    encoder,
                    &self.uniform_bind_group,
                    &self.size,
                    depth,
                    self.pick_target.views().map(|views| views.single_sample()),
                    region,
                );
            }
        }
    }

    /// Keep the scene as the opaque pass left it (call before particles and transparent
    /// surfaces are drawn), when FSR upscales this frame (its reactive mask comes from
    /// what changes after) or `particles` are ray traced this frame (reflections tell
    /// where particles cover the screen from what [`Self::capture_particle_scene`] adds).
    pub(crate) fn capture_opaque_scene(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        scene_region: Option<RenderRegion>,
        particles: bool,
    ) {
        let particles =
            particles && (self.wants_ray_traced_reflections() || self.ray_traced_refraction());
        if particles != self.particles_traced {
            // The ray-traced passes bind the copies only while particles are traced.
            self.particles_traced = particles;
            if !particles {
                self.particle_copy = None;
            }
            self.mark_bind_groups_dirty();
        }
        if !self.fsr_active() && !particles {
            return;
        }
        if self.opaque_copy.is_none() {
            self.opaque_copy = Some(TextureBundle::copyable(
                device,
                &self.size,
                HDR_FORMAT,
                "SceneOpaqueCopy",
            ));
            if let Some(fsr) = self.fsr.as_mut() {
                fsr.invalidate_bind_groups();
            }
            self.mark_bind_groups_dirty();
        }
        if let Some(copy) = self.opaque_copy.as_ref() {
            self.copy_scene(encoder, copy, scene_region);
        }
    }

    /// Keep the scene with particles drawn over it, before transparent surfaces are (call
    /// between the two), while particles are ray traced: where it differs from the opaque
    /// copy, the screen shows particles, which reflections trace instead of taking from it.
    pub(crate) fn capture_particle_scene(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        scene_region: Option<RenderRegion>,
    ) {
        if !self.particles_traced {
            return;
        }
        if self.particle_copy.is_none() {
            self.particle_copy = Some(TextureBundle::copyable(
                device,
                &self.size,
                HDR_FORMAT,
                "SceneParticleCopy",
            ));
            self.mark_bind_groups_dirty();
        }
        if let Some(copy) = self.particle_copy.as_ref() {
            self.copy_scene(encoder, copy, scene_region);
        }
    }

    /// Copy the scene (its `scene_region`, or all of it) into `copy`.
    fn copy_scene(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        copy: &TextureBundle,
        scene_region: Option<RenderRegion>,
    ) {
        let (origin, extent) = match scene_region {
            Some(region) => (
                wgpu::Origin3d {
                    x: region.x(),
                    y: region.y(),
                    z: 0,
                },
                wgpu::Extent3d {
                    width: region.width(),
                    height: region.height(),
                    depth_or_array_layers: 1,
                },
            ),
            None => (wgpu::Origin3d::ZERO, self.size),
        };
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: self.scene_source.texture(),
                mip_level: 0,
                origin,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: copy.texture(),
                mip_level: 0,
                origin,
                aspect: wgpu::TextureAspect::All,
            },
            extent,
        );
    }

    /// FSR's reactive mask: what was blended over the opaque scene, before reflections and
    /// refraction are added to it.
    fn run_fsr_reactive(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) {
        let Some(opaque) = self.opaque_copy.as_ref() else {
            return;
        };
        let full = Vec2::new(self.size.width as f32, self.size.height as f32);
        let origin = (self.viewport_offset * full).round();
        let frame = ReactiveFrame {
            opaque: &opaque.view,
            color: &self.scene_source.view,
            origin: [origin.x as i32, origin.y as i32],
            render_size: [
                self.scene_resolution.x as i32,
                self.scene_resolution.y as i32,
            ],
        };
        self.fsr
            .get_or_insert_with(|| FsrUpscaler::new(device))
            .generate_reactive(
                device,
                queue,
                encoder,
                [self.size.width, self.size.height],
                frame,
            );
    }

    /// Upscale the scene region into the output region of the TAA target with FSR.
    fn run_fsr(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) {
        let Some(depth) = self.cached_depth_view.as_ref() else {
            return;
        };
        let full = Vec2::new(self.size.width as f32, self.size.height as f32);
        let origin = (self.viewport_offset * full).round();
        let frame = FsrFrame {
            color: &self.scene_source.view,
            depth,
            motion: &self.normal_source.view,
            output: &self.taa_output.view,
            sampler: &self.sampler_linear,
            origin: [origin.x as i32, origin.y as i32],
            render_size: [
                self.scene_resolution.x as i32,
                self.scene_resolution.y as i32,
            ],
            upscale_size: [
                self.viewport_resolution.x as i32,
                self.viewport_resolution.y as i32,
            ],
            projection: self.camera.proj,
            jitter: self.camera.jitter_pixels,
            delta_time: self.clock.delta,
            reset: !self.taa_history_valid,
            sharpness: self.settings.upscaling.sharpness,
        };
        self.fsr
            .get_or_insert_with(|| FsrUpscaler::new(device))
            .dispatch(
                device,
                queue,
                encoder,
                [self.size.width, self.size.height],
                frame,
            );
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
        if let Some(stage) = self.path_tracing_stage.as_mut() {
            stage.invalidate();
        }
        if let Some(stage) = self.rt_ao_stage.as_mut() {
            stage.invalidate();
        }
        self.bloom_stage.invalidate();
        self.exposure_stage.invalidate();
        self.dof_stage.invalidate();
        self.temporal_stage.invalidate();
        self.upscaled.invalidate_bind_group();
        if let Some(fsr) = self.fsr.as_mut() {
            fsr.invalidate_bind_groups();
        }
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
                reflection: &self.reflection.trace.view,
                ping: &self.reflection.ping.view,
                accumulated: &self.reflection.accumulated.view,
                history: &self.reflection.history.view,
            },
        );
        if let Some(stage) = self.rt_ao_stage.as_mut() {
            stage.ensure_bind_groups(device, &self.normal_source.view, &self.position_source.view);
        }
        if let Some(stage) = self.rt_reflection_stage.as_mut() {
            // Without traced particles one texture stands in for both, so nothing differs
            // (not the scene, which refraction draws into while reading these).
            let (opaque, particles) = match (&self.opaque_copy, &self.particle_copy) {
                (Some(opaque), Some(particles)) if self.particles_traced => {
                    (&opaque.view, &particles.view)
                }
                _ => (&self.scene_copy.view, &self.scene_copy.view),
            };
            stage.ensure_bind_groups(
                device,
                RtReflectionInputs {
                    scene: &self.scene_source.view,
                    scene_copy: &self.scene_copy.view,
                    opaque,
                    particles,
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
        let mut inputs = UniformInputs {
            camera: &self.camera,
            viewport_resolution: self.viewport_resolution,
            viewport_offset: self.viewport_offset,
            viewport_scale: self.viewport_scale,
            scene_resolution: self.scene_resolution,
            output_resolution: self.viewport_resolution,
            settings: &self.settings,
            lut_loaded: self.lut.is_loaded(),
            taa_history_valid: self.taa_history_valid,
            display_peak: self.display_peak,
            encode_srgb: self.encode_srgb,
            seconds: self.clock.seconds,
            delta: self.clock.delta,
            frame: self.clock.frame,
            reflection_scale: self.reflection.scale(),
            ambient: self.ambient,
        };
        let uniform = PostProcessUniform::new(&inputs);
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniform));
        // The scene passes see the scene region as their viewport.
        inputs.viewport_resolution = self.scene_resolution;
        inputs.viewport_scale = self.scene_scale;
        let scene_uniform = PostProcessUniform::new(&inputs);
        queue.write_buffer(
            &self.scene_uniform_buffer,
            0,
            bytemuck::bytes_of(&scene_uniform),
        );
    }
}

/// Whether the camera jumped between two frames (a cut, a teleport, or a turn too fast
/// for last frame's image to help): more than 2 m or 15 degrees.
fn is_camera_cut(previous: &PostProcessCamera, next: &PostProcessCamera) -> bool {
    let forward =
        |camera: &PostProcessCamera| -camera.view_inv.z_axis.truncate().normalize_or_zero();
    previous.position.distance(next.position) > 2.0
        || forward(previous).dot(forward(next)) < 15f32.to_radians().cos()
}

/// The textures reflections go through, at half or full resolution.
struct ReflectionTargets {
    /// The trace, then the blurred result.
    trace: TextureBundle,
    /// The horizontal blur's output.
    ping: TextureBundle,
    /// The trace accumulated over frames.
    accumulated: TextureBundle,
    /// Last frame's `accumulated`.
    history: TextureBundle,
    full: bool,
}

impl ReflectionTargets {
    fn new(device: &wgpu::Device, size: &wgpu::Extent3d, full: bool) -> Self {
        let size = if full { *size } else { half_size(size) };
        let copyable = |label| TextureBundle::copyable(device, &size, HDR_FORMAT, label);
        Self {
            trace: copyable("ReflectionTrace"),
            ping: TextureBundle::color(device, &size, HDR_FORMAT, "ReflectionPing"),
            accumulated: copyable("ReflectionAccumulated"),
            history: copyable("ReflectionHistory"),
            full,
        }
    }

    /// Full-resolution pixels per reflection texel.
    fn scale(&self) -> f32 {
        if self.full {
            1.0
        } else {
            2.0
        }
    }
}

/// Every render target whose size follows the surface.
struct SizedTargets {
    scene_source: TextureBundle,
    /// A copy of `scene_source` the refraction pass reads while adding to it.
    scene_copy: TextureBundle,
    scene_msaa: Option<MsaaTarget>,
    normal_source: TextureBundle,
    normal_msaa: Option<MsaaTarget>,
    position_source: TextureBundle,
    position_msaa: Option<MsaaTarget>,
    ssao: TextureBundle,
    ssao_ping: TextureBundle,
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
            scene_copy: TextureBundle::copyable(device, size, HDR_FORMAT, "SceneColorCopy"),
            scene_source,
            scene_msaa,
            normal_source,
            normal_msaa,
            position_source,
            position_msaa,
            ssao: TextureBundle::ssao(device, size),
            ssao_ping: TextureBundle::ssao(device, size),
            bloom_chain: create_bloom_chain(device, size),
            dof_half: TextureBundle::color(device, &half_size(size), DOF_FORMAT, "DofHalf"),
            dof_blur: TextureBundle::color(device, &half_size(size), DOF_FORMAT, "DofBlur"),
            // Storage too: FSR writes the upscaled image from a compute pass.
            taa_output: TextureBundle::storage(device, size, HDR_FORMAT, "TaaOutput"),
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
    /// The size of the region the scene was drawn into (the upscale's source), and of the
    /// output region it is upscaled to.
    scene_resolution: Vec2,
    output_resolution: Vec2,
    settings: &'a PostProcessSettings,
    lut_loaded: bool,
    taa_history_valid: bool,
    display_peak: f32,
    encode_srgb: bool,
    seconds: f32,
    delta: f32,
    frame: u32,
    /// Full-resolution pixels per reflection texel.
    reflection_scale: f32,
    ambient: [f32; 3],
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
    reflection: [f32; 4],
    upscale: [f32; 4],
    upscale_source: [f32; 4],
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
            reflection: {
                let [r, g, b] = inputs.ambient;
                [inputs.reflection_scale, r, g, b]
            },
            // Scene pixels per output pixel, and the jitter (only the upscale pass and the
            // passes reading the scene region at output resolution need it; 0 otherwise so
            // those read the same pixel they always did).
            upscale: if inputs.scene_resolution != resolution {
                let ratio = inputs.scene_resolution / resolution.max(Vec2::ONE);
                [
                    ratio.x,
                    ratio.y,
                    camera.jitter_pixels.x,
                    camera.jitter_pixels.y,
                ]
            } else {
                [1.0, 1.0, 0.0, 0.0]
            },
            upscale_source: [
                inputs.scene_resolution.x,
                inputs.scene_resolution.y,
                // TAAU accumulates whenever the camera jitters: TAA, or FSR chosen (the
                // fallback for orthographic cameras).
                flag(settings.taa_active() || settings.upscaling.method == Upscaler::Fsr),
                inputs.scene_resolution.y / inputs.output_resolution.y.max(1.0),
            ],
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
            unjittered_view_proj: stable_vp,
            position: eye,
            near: 0.1,
            far: 1200.0,
            jitter_pixels: Vec2::ZERO,
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
        // 6 matrices, camera position, 8 vec2 pairs and 23 vec4 blocks.
        assert_eq!(std::mem::size_of::<PostProcessUniform>(), 848);
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
    fn camera_cuts_are_big_jumps_only() {
        let camera = |position: Vec3, target: Vec3| {
            let view = Mat4::look_at_rh(position, target, Vec3::Y);
            PostProcessCamera {
                view,
                view_inv: view.inverse(),
                position,
                ..PostProcessCamera::default()
            }
        };
        let start = camera(Vec3::ZERO, Vec3::NEG_Z);
        assert!(!is_camera_cut(
            &start,
            &camera(Vec3::new(0.1, 0.0, 0.0), Vec3::NEG_Z)
        ));
        assert!(!is_camera_cut(
            &start,
            &camera(Vec3::ZERO, Vec3::new(0.1, 0.0, -1.0))
        ));
        assert!(is_camera_cut(
            &start,
            &camera(Vec3::new(5.0, 0.0, 0.0), Vec3::NEG_Z)
        ));
        assert!(is_camera_cut(&start, &camera(Vec3::ZERO, Vec3::X)));
    }

    #[test]
    fn rt_ao_shader_validates() {
        let source = rt_ao::rt_ao_shader_source();
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|err| panic!("{}", err.emit_to_string(&source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::RAY_QUERY,
        )
        .validate(&module)
        .unwrap_or_else(|err| panic!("{}", err.emit_to_string(&source)));
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
