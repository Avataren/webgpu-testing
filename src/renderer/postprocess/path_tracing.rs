//! Path tracing with a spatiotemporal denoiser: the lighting of every opaque pixel traced
//! against the ray tracing scene (direct light from every light with soft shadows,
//! multi-bounce diffuse and specular light, emissive surfaces, glass), denoised and
//! written over the main pass's colour before particles and transparent surfaces are
//! drawn. Reflections, ambient occlusion and the environment's light come out of the
//! paths, so the screen-space and ray-traced reflection and occlusion passes stand down
//! while it runs.
//!
//! Frame order (all over the scene region):
//! 1. trace (`pt_trace.wgsl`, fragment): demodulated diffuse and specular light, hit
//!    distances, the surface's colours;
//! 2. caustics (`pt_caustics.wgsl`, `pt_caustic_resolve.wgsl`): photons from the lights
//!    through glass and off mirrors, splatted where they land and added to the diffuse
//!    light;
//! 3. denoise (`pt_denoise.wgsl`, compute): temporal accumulation, history fix and
//!    anti-lag, then five a-trous iterations;
//! 4. composite (`pt_composite.wgsl`, fragment): the light times the surface's colours,
//!    into the scene colour.
//!
//! The targets are the scene region's size and exist only while path tracing is on
//! (about 125 bytes a pixel: 260 MB at 1080p).

use crate::renderer::shader_source::wgsl;
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};

use super::settings::PathTracingSettings;
use super::{begin_color_pass, ColorAttachment, PostProcessCamera, HDR_FORMAT};
use crate::renderer::raytracing::RayTracingFrame;
use crate::renderer::{PipelineBuilder, RenderRegion, ShaderBuilder};

/// Demodulated light (rgb) with hit distance, frame count or variance (a).
const SIGNAL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Base colour and roughness.
const MATERIAL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const ATROUS_ITERATIONS: usize = 5;
const WORKGROUP_SIZE: u32 = 8;
/// Storage textures the temporal pass writes at once (the default limit is 4).
pub(crate) const STORAGE_TEXTURES_NEEDED: u32 = 5;

/// Mirrors `PtUniform` in pt_common.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
struct PtUniform {
    view_proj: [[f32; 4]; 4],
    prev_view_proj: [[f32; 4]; 4],
    prev_view_proj_inv: [[f32; 4]; 4],
    view: [[f32; 4]; 4],
    prev_view: [[f32; 4]; 4],
    camera_position: [f32; 4],
    prev_camera_position: [f32; 4],
    region: [f32; 4],
    frame: [u32; 4],
    params: [f32; 4],
    footprint: [f32; 4],
}

/// `PtUniform.frame.w` flags (PT_FLAG_* in pt_common.wgsl).
const FLAG_HISTORY_VALID: u32 = 1;
const FLAG_DENOISE: u32 = 2;
const FLAG_CAUSTICS: u32 = 4;

use crate::renderer::raytracing::MAX_CAUSTIC_CASTERS;
/// Format of the photon grid's (unused) colour target: one fragment per photon.
const PHOTON_TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;

/// Mirrors `PtCaustics` in pt_caustics.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct PtCaustics {
    casters: [[f32; 4]; MAX_CAUSTIC_CASTERS],
    /// Each light-caster pair's first photon (pair `light * casters + caster`), then the
    /// total: pairs get photons in proportion to the light that reaches the caster.
    pair_starts: [[u32; 4]; PAIR_START_VECTORS],
    counts: [u32; 4],
}

const MAX_LIGHTS: usize = crate::renderer::lights::MAX_DIRECTIONAL_LIGHTS
    + crate::renderer::lights::MAX_POINT_LIGHTS
    + crate::renderer::lights::MAX_SPOT_LIGHTS;
const MAX_PAIRS: usize = MAX_LIGHTS * MAX_CAUSTIC_CASTERS;
const PAIR_START_VECTORS: usize = (MAX_PAIRS + 1).div_ceil(4);

/// What focuses light this frame and the lights that shine on it.
#[derive(Clone, Copy)]
pub(crate) struct CausticSources<'a> {
    /// Bounding spheres of glass and mirror surfaces (centre, radius).
    pub casters: &'a [[f32; 4]],
    pub lights: &'a crate::renderer::LightsData,
}

impl CausticSources<'_> {
    /// Light power reaching each caster's bounding sphere from each light, unshadowed,
    /// in the lights buffer's order (directional, point, spot) times the casters.
    fn pair_flux(&self) -> Vec<f32> {
        use crate::renderer::lights::{MAX_DIRECTIONAL_LIGHTS, MAX_POINT_LIGHTS, MAX_SPOT_LIGHTS};
        let casters = &self.casters[..self.casters.len().min(MAX_CAUSTIC_CASTERS)];
        let luminance =
            |c: [f32; 4]| (0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]).max(0.0) * c[3].max(0.0);
        // Light from a lamp at `position` into the sphere: intensity times the solid angle
        // it subtends, with the main pass's range falloff; `cone` gives a spot's factor at
        // the sphere's nearest direction.
        let lamp = |position: [f32; 4],
                    power: f32,
                    sphere: [f32; 4],
                    cone: Option<([f32; 4], [f32; 4])>| {
            let to = Vec3::new(sphere[0], sphere[1], sphere[2])
                - Vec3::new(position[0], position[1], position[2]);
            let distance = to.length();
            let radius = sphere[3].max(1e-3);
            let range = position[3];
            if range > 0.0 && distance - radius > range {
                return 0.0;
            }
            let (solid_angle, half_angle) = if distance > radius {
                let sin = radius / distance;
                (
                    std::f32::consts::TAU * (1.0 - (1.0 - sin * sin).sqrt()),
                    sin.asin(),
                )
            } else {
                (4.0 * std::f32::consts::PI, std::f32::consts::PI)
            };
            let mut flux = power * solid_angle;
            if range > 0.0 {
                let falloff = (1.0 - (distance - radius).max(0.0) / range).clamp(0.0, 1.0);
                flux *= falloff * falloff;
            }
            if let Some((direction, cone_params)) = cone {
                let axis = Vec3::new(direction[0], direction[1], direction[2]).normalize_or_zero();
                let angle = axis.dot(to / distance.max(1e-6)).clamp(-1.0, 1.0).acos();
                let nearest = (angle - half_angle).max(0.0).cos();
                let (inner, outer) = (cone_params[0], cone_params[1]);
                let factor = ((nearest - outer) / (inner - outer).max(1e-4)).clamp(0.0, 1.0);
                flux *= factor * factor;
            }
            flux
        };
        let mut flux = Vec::new();
        for light in self
            .lights
            .directional_lights()
            .iter()
            .take(MAX_DIRECTIONAL_LIGHTS)
        {
            let irradiance = luminance(light.color_intensity);
            flux.extend(
                casters
                    .iter()
                    .map(|sphere| irradiance * std::f32::consts::PI * sphere[3] * sphere[3]),
            );
        }
        for light in self.lights.point_lights().iter().take(MAX_POINT_LIGHTS) {
            let power = luminance(light.color_intensity);
            flux.extend(
                casters
                    .iter()
                    .map(|sphere| lamp(light.position_range, power, *sphere, None)),
            );
        }
        for light in self.lights.spot_lights().iter().take(MAX_SPOT_LIGHTS) {
            let power = luminance(light.color_intensity);
            let cone = Some((light.direction, light.cone_params));
            flux.extend(
                casters
                    .iter()
                    .map(|sphere| lamp(light.position_range, power, *sphere, cone)),
            );
        }
        flux
    }
}

/// Each pair's first photon, then the total, sharing `photons` by the square root of each
/// pair's `flux`: brighter pairs get more, but a small caster (a ring, a lens) is not
/// starved by a large one (a mirror wall) whose light spreads thin anyway. Each photon
/// carries its pair's power divided by its pair's count.
fn photon_starts(flux: &[f32], photons: u32) -> Vec<u32> {
    let weight = |f: f32| {
        if f.is_finite() && f > 0.0 {
            f.sqrt()
        } else {
            0.0
        }
    };
    let total: f32 = flux.iter().map(|f| weight(*f)).sum();
    let mut starts = Vec::with_capacity(flux.len() + 1);
    let mut next = 0u32;
    for f in flux {
        starts.push(next);
        if total > 0.0 {
            next += (photons as f64 * (weight(*f) / total) as f64).floor() as u32;
        }
    }
    starts.push(next.min(photons));
    starts
}

/// Mirrors `PtAtrousStep` in pt_denoise.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
struct AtrousStep {
    step: i32,
    iteration: u32,
    _padding: [u32; 2],
}

/// The camera as last frame saw it, for reprojection.
#[derive(Clone, Copy)]
struct CameraSnapshot {
    view_proj: Mat4,
    view: Mat4,
    position: Vec3,
}

/// The trace shader: the ray tracing helpers the reflection pass uses, with the path
/// tracer. With `textures`, hits sample material textures from the bindless array
/// (group 4).
pub(crate) fn pt_trace_shader_source(textures: bool) -> String {
    let textures_module = if textures {
        wgsl!("postprocess/rt_textures_bindless.wgsl")
    } else {
        wgsl!("postprocess/rt_textures_none.wgsl")
    };
    let modules = ShaderBuilder::new()
        .with_module(wgsl!("postprocess/fullscreen.wgsl"))
        .with_module(wgsl!("postprocess/common.wgsl"))
        .with_constants()
        .with_material_system()
        .with_lighting()
        .with_environment()
        .with_module(textures_module)
        .with_module(wgsl!("postprocess/rt_scene.wgsl"))
        .with_module(wgsl!("postprocess/rt_alpha.wgsl"))
        .with_module(wgsl!("postprocess/rt_common.wgsl"))
        .with_module(wgsl!("postprocess/pt_common.wgsl"))
        .with_module(wgsl!("postprocess/pt_trace.wgsl"))
        .with_module(wgsl!("postprocess/pt_caustics.wgsl"))
        .build_modules_only();
    format!("enable wgpu_ray_query;\n\n{modules}")
}

pub(crate) fn pt_denoise_shader_source() -> String {
    ShaderBuilder::new()
        .with_module(wgsl!("postprocess/pt_common.wgsl"))
        .with_module(wgsl!("postprocess/pt_denoise.wgsl"))
        .build_modules_only()
}

pub(crate) fn pt_composite_shader_source() -> String {
    ShaderBuilder::new()
        .with_module(wgsl!("postprocess/fullscreen.wgsl"))
        .with_module(wgsl!("postprocess/pt_common.wgsl"))
        .with_module(wgsl!("postprocess/pt_composite.wgsl"))
        .with_module(wgsl!("postprocess/pt_caustic_resolve.wgsl"))
        .build_modules_only()
}

/// A target texture and its view.
struct Target {
    // The view drops before the texture.
    view: wgpu::TextureView,
    texture: wgpu::Texture,
}

impl Target {
    fn new(
        device: &wgpu::Device,
        size: (u32, u32),
        format: wgpu::TextureFormat,
        label: &str,
        render: bool,
    ) -> Self {
        let mut usage = wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::COPY_SRC;
        if render {
            usage |= wgpu::TextureUsages::RENDER_ATTACHMENT;
        }
        Self::with_usage(device, size, format, label, usage)
    }

    fn with_usage(
        device: &wgpu::Device,
        size: (u32, u32),
        format: wgpu::TextureFormat,
        label: &str,
        usage: wgpu::TextureUsages,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self { view, texture }
    }
}

/// One frame's history.
struct History {
    diffuse: Target,
    specular: Target,
    /// Luminance moments: diffuse m2, specular m2, diffuse fast, specular fast.
    moments: Target,
    /// Octahedral normal, view depth, roughness (0: nothing path traced).
    geometry: Target,
    /// Hit distances: diffuse, specular.
    hit: Target,
}

impl History {
    fn new(device: &wgpu::Device, size: (u32, u32), index: usize) -> Self {
        let target = |name: &str| {
            Target::new(
                device,
                size,
                SIGNAL_FORMAT,
                &format!("PtHistory{name}{index}"),
                false,
            )
        };
        Self {
            diffuse: target("Diffuse"),
            specular: target("Specular"),
            moments: target("Moments"),
            geometry: target("Geometry"),
            hit: target("Hit"),
        }
    }
}

struct Targets {
    size: (u32, u32),
    /// The trace's light, then the a-trous iterations' ping (and the composite's input).
    diffuse_a: Target,
    specular_a: Target,
    material: Target,
    emission: Target,
    /// Temporal accumulation's output, then the a-trous iterations' pong.
    diffuse_b: Target,
    specular_b: Target,
    history: [History; 2],
    /// Caustic photons' sums: red, green, blue per pixel, in fixed point.
    caustic_sums: wgpu::Buffer,
    /// Last frame's finished (denoised, composited) light: bounce rays that hit what the
    /// screen showed take it from here instead of tracing on (pt_cached_radiance).
    radiance: Target,
}

impl Targets {
    fn new(device: &wgpu::Device, size: (u32, u32)) -> Self {
        let render = |format, label| Target::new(device, size, format, label, true);
        let storage = |label| Target::new(device, size, SIGNAL_FORMAT, label, false);
        Self {
            size,
            diffuse_a: render(SIGNAL_FORMAT, "PtDiffuseA"),
            specular_a: render(SIGNAL_FORMAT, "PtSpecularA"),
            material: render(MATERIAL_FORMAT, "PtMaterial"),
            emission: render(SIGNAL_FORMAT, "PtEmission"),
            diffuse_b: storage("PtDiffuseB"),
            specular_b: storage("PtSpecularB"),
            history: [History::new(device, size, 0), History::new(device, size, 1)],
            radiance: Target::with_usage(
                device,
                size,
                crate::renderer::postprocess::HDR_FORMAT,
                "PtRadianceCache",
                wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::COPY_SRC,
            ),
            caustic_sums: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("PtCausticSums"),
                size: (size.0 as u64 * size.1 as u64 * 3 * 4).max(16),
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
        }
    }
}

/// The views the passes read from the scene targets.
pub struct PathTracingInputs<'a> {
    pub normal: &'a wgpu::TextureView,
    pub position: &'a wgpu::TextureView,
    pub scene: &'a wgpu::TextureView,
}

/// Bind groups for the current targets and inputs; indexed by history parity where it
/// matters (the history written this frame).
struct BindGroups {
    trace: [wgpu::BindGroup; 2],
    temporal: [wgpu::BindGroup; 2],
    history_fix: [wgpu::BindGroup; 2],
    atrous: [[wgpu::BindGroup; ATROUS_ITERATIONS]; 2],
    composite: wgpu::BindGroup,
    photons: wgpu::BindGroup,
    caustic_resolve: wgpu::BindGroup,
}

/// What a frame of path tracing uses besides the stage itself.
pub struct PathTracingFrame<'a> {
    pub uniform_bind_group: &'a wgpu::BindGroup,
    pub ray_tracing: &'a RayTracingFrame<'a>,
    pub caustics: CausticSources<'a>,
    pub inputs: PathTracingInputs<'a>,
    pub camera: &'a PostProcessCamera,
    pub settings: &'a PathTracingSettings,
    /// Ray-traced shadows' sun size (degrees) and lamp radius (m): lights have the same
    /// size for the path tracer.
    pub sun_size_degrees: f32,
    pub lamp_radius: f32,
    /// The scene region in the scene targets.
    pub region: RenderRegion,
    /// The output's height in pixels (the region's while not upscaling).
    pub output_height: f32,
    /// The scene colour to write the result into, and its texture (copied from as the
    /// radiance cache).
    pub scene: &'a wgpu::TextureView,
    pub scene_texture: &'a wgpu::Texture,
}

pub struct PathTracingStage {
    trace_inputs_layout: wgpu::BindGroupLayout,
    trace_pipeline: wgpu::RenderPipeline,
    temporal_layout: wgpu::BindGroupLayout,
    temporal_pipeline: wgpu::ComputePipeline,
    history_fix_layout: wgpu::BindGroupLayout,
    history_fix_pipeline: wgpu::ComputePipeline,
    atrous_layout: wgpu::BindGroupLayout,
    atrous_pipeline: wgpu::ComputePipeline,
    /// The compute passes' uniform (group 0), also the composite's.
    uniform_bind_group: wgpu::BindGroup,
    composite_layout: wgpu::BindGroupLayout,
    composite_pipeline: wgpu::RenderPipeline,
    photon_layout: wgpu::BindGroupLayout,
    photon_pipeline: wgpu::RenderPipeline,
    caustic_resolve_layout: wgpu::BindGroupLayout,
    caustic_resolve_pipeline: wgpu::RenderPipeline,
    caustic_uniform: wgpu::Buffer,
    /// The photon grid's target and its width.
    photon_target: Option<(u32, Target)>,
    uniform: wgpu::Buffer,
    atrous_steps: Vec<wgpu::Buffer>,
    /// Whether hits sample material textures (group 4 is the bindless texture array).
    textures: bool,
    targets: Option<Targets>,
    bind_groups: Option<BindGroups>,
    previous: Option<CameraSnapshot>,
    /// Last frame's history may be reprojected.
    history_valid: bool,
    /// The region last frame traced (a different one starts the history over).
    last_region: Option<RenderRegion>,
    frame: u32,
}

impl PathTracingStage {
    /// Build the pipelines. Fails (instead of raising an uncaptured device error) when the
    /// driver or wgpu rejects them, so the caller can go on without path tracing.
    pub fn new(
        device: &wgpu::Device,
        post_uniform_layout: &wgpu::BindGroupLayout,
        lights_layout: &wgpu::BindGroupLayout,
        scene_layout: &wgpu::BindGroupLayout,
        textures_layout: Option<&wgpu::BindGroupLayout>,
    ) -> Result<Self, String> {
        let limits = device.limits();
        if limits.max_storage_textures_per_shader_stage < STORAGE_TEXTURES_NEEDED {
            return Err(format!(
                "the device allows {} storage textures per shader stage, the denoiser needs {}",
                limits.max_storage_textures_per_shader_stage, STORAGE_TEXTURES_NEEDED
            ));
        }
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);

        let uniform_entry = |binding, visibility| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let texture_entry = |binding, visibility| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let storage_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format: SIGNAL_FORMAT,
                view_dimension: wgpu::TextureViewDimension::D2,
            },
            count: None,
        };
        let fragment = wgpu::ShaderStages::FRAGMENT;
        let compute = wgpu::ShaderStages::COMPUTE;
        let buffer_entry = |binding, read_only| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };

        // Trace.
        let trace_inputs_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("PtTraceInputsLayout"),
                entries: &[
                    uniform_entry(190, fragment),
                    texture_entry(191, fragment),
                    texture_entry(192, fragment),
                    texture_entry(193, fragment),
                    texture_entry(196, fragment),
                    texture_entry(197, fragment),
                ],
            });
        let trace_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("PtTraceShader"),
            source: wgpu::ShaderSource::Wgsl(
                pt_trace_shader_source(textures_layout.is_some()).into(),
            ),
        });
        let trace_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("PtTracePipelineLayout"),
            bind_group_layouts: &[
                Some(post_uniform_layout),
                Some(&trace_inputs_layout),
                Some(lights_layout),
                Some(scene_layout),
                textures_layout,
            ],
            immediate_size: 0,
        });
        let trace_pipeline = PipelineBuilder::new(device, &trace_layout, &trace_shader)
            .with_label("PtTracePipeline")
            .with_vertex_entry("vs_fullscreen")
            .with_fragment_entry("fs_pt_trace")
            .with_color_target(SIGNAL_FORMAT, None)
            .with_color_target(SIGNAL_FORMAT, None)
            .with_color_target(MATERIAL_FORMAT, None)
            .with_color_target(SIGNAL_FORMAT, None)
            .with_no_culling()
            .build();

        // Caustic photons: the trace module's photon entry, with the caster list and the
        // sums to splat into.
        let photon_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("PtPhotonLayout"),
            entries: &[
                uniform_entry(190, fragment),
                texture_entry(191, fragment),
                texture_entry(192, fragment),
                uniform_entry(194, fragment),
                buffer_entry(195, false),
            ],
        });
        let photon_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("PtPhotonPipelineLayout"),
                bind_group_layouts: &[
                    Some(post_uniform_layout),
                    Some(&photon_layout),
                    Some(lights_layout),
                    Some(scene_layout),
                    textures_layout,
                ],
                immediate_size: 0,
            });
        let photon_pipeline = PipelineBuilder::new(device, &photon_pipeline_layout, &trace_shader)
            .with_label("PtPhotonPipeline")
            .with_vertex_entry("vs_fullscreen")
            .with_fragment_entry("fs_pt_photons")
            .with_color_target(PHOTON_TARGET_FORMAT, None)
            .with_no_culling()
            .build();

        // Denoise.
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("PtUniformLayout"),
            entries: &[uniform_entry(0, compute | fragment)],
        });
        let temporal_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("PtTemporalLayout"),
            entries: &[
                texture_entry(0, compute),
                texture_entry(1, compute),
                texture_entry(2, compute),
                texture_entry(3, compute),
                texture_entry(4, compute),
                texture_entry(5, compute),
                texture_entry(6, compute),
                texture_entry(7, compute),
                texture_entry(8, compute),
                texture_entry(9, compute),
                storage_entry(10),
                storage_entry(11),
                storage_entry(12),
                storage_entry(13),
                storage_entry(14),
            ],
        });
        let history_fix_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("PtHistoryFixLayout"),
                entries: &[
                    texture_entry(20, compute),
                    texture_entry(21, compute),
                    texture_entry(22, compute),
                    texture_entry(23, compute),
                    storage_entry(24),
                    storage_entry(25),
                ],
            });
        let atrous_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("PtAtrousLayout"),
            entries: &[
                texture_entry(40, compute),
                texture_entry(41, compute),
                texture_entry(42, compute),
                texture_entry(43, compute),
                texture_entry(44, compute),
                texture_entry(45, compute),
                uniform_entry(46, compute),
                storage_entry(47),
                storage_entry(48),
            ],
        });
        let denoise_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("PtDenoiseShader"),
            source: wgpu::ShaderSource::Wgsl(pt_denoise_shader_source().into()),
        });
        let compute_pipeline = |label, layout: &wgpu::BindGroupLayout, entry| {
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[Some(&uniform_layout), Some(layout)],
                immediate_size: 0,
            });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                module: &denoise_shader,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let temporal_pipeline =
            compute_pipeline("PtTemporalPipeline", &temporal_layout, "cs_pt_temporal");
        let history_fix_pipeline = compute_pipeline(
            "PtHistoryFixPipeline",
            &history_fix_layout,
            "cs_pt_history_fix",
        );
        let atrous_pipeline = compute_pipeline("PtAtrousPipeline", &atrous_layout, "cs_pt_atrous");

        // Composite: replaces the colour, keeps the alpha (transmissive marks).
        let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("PtCompositeLayout"),
            entries: &[
                texture_entry(50, fragment),
                texture_entry(51, fragment),
                texture_entry(52, fragment),
                texture_entry(53, fragment),
                texture_entry(54, fragment),
                texture_entry(55, fragment),
            ],
        });
        let composite_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("PtCompositeShader"),
            source: wgpu::ShaderSource::Wgsl(pt_composite_shader_source().into()),
        });
        let composite_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("PtCompositePipelineLayout"),
                bind_group_layouts: &[Some(&uniform_layout), Some(&composite_layout)],
                immediate_size: 0,
            });
        let keep_alpha = wgpu::BlendState {
            color: wgpu::BlendComponent::REPLACE,
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let composite_pipeline =
            PipelineBuilder::new(device, &composite_pipeline_layout, &composite_shader)
                .with_label("PtCompositePipeline")
                .with_vertex_entry("vs_fullscreen")
                .with_fragment_entry("fs_pt_composite")
                .with_color_target(HDR_FORMAT, Some(keep_alpha))
                .with_no_culling()
                .build();

        // Caustic resolve: adds to the trace's diffuse light, keeping its hit distance.
        let caustic_resolve_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("PtCausticResolveLayout"),
                entries: &[
                    buffer_entry(60, true),
                    texture_entry(61, fragment),
                    texture_entry(62, fragment),
                ],
            });
        let caustic_resolve_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("PtCausticResolvePipelineLayout"),
                bind_group_layouts: &[Some(&uniform_layout), Some(&caustic_resolve_layout)],
                immediate_size: 0,
            });
        let add_keep_alpha = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: keep_alpha.alpha,
        };
        let caustic_resolve_pipeline =
            PipelineBuilder::new(device, &caustic_resolve_pipeline_layout, &composite_shader)
                .with_label("PtCausticResolvePipeline")
                .with_vertex_entry("vs_fullscreen")
                .with_fragment_entry("fs_pt_caustic_resolve")
                .with_color_target(SIGNAL_FORMAT, Some(add_keep_alpha))
                .with_no_culling()
                .build();
        let caustic_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("PtCaustics"),
            size: std::mem::size_of::<PtCaustics>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("PtUniform"),
            size: std::mem::size_of::<PtUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("PtUniformBindGroup"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let atrous_steps = (0..ATROUS_ITERATIONS)
            .map(|iteration| {
                use wgpu::util::DeviceExt;
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("PtAtrousStep"),
                    contents: bytemuck::bytes_of(&AtrousStep {
                        step: 1 << iteration,
                        iteration: iteration as u32,
                        _padding: [0; 2],
                    }),
                    usage: wgpu::BufferUsages::UNIFORM,
                })
            })
            .collect();

        let internal = pollster::block_on(internal.pop());
        let validation = pollster::block_on(validation.pop());
        if let Some(error) = validation.or(internal) {
            return Err(error.to_string());
        }
        Ok(Self {
            trace_inputs_layout,
            trace_pipeline,
            temporal_layout,
            temporal_pipeline,
            history_fix_layout,
            history_fix_pipeline,
            atrous_layout,
            atrous_pipeline,
            uniform_bind_group,
            composite_layout,
            composite_pipeline,
            photon_layout,
            photon_pipeline,
            caustic_resolve_layout,
            caustic_resolve_pipeline,
            caustic_uniform,
            photon_target: None,
            uniform,
            atrous_steps,
            textures: textures_layout.is_some(),
            targets: None,
            bind_groups: None,
            previous: None,
            history_valid: false,
            last_region: None,
            frame: 0,
        })
    }

    /// The path tracer's targets, for reading back (see [`crate::renderer::readback`]).
    /// They are the scene region's size (pixel (0, 0) is its corner).
    pub(crate) fn fetchable_textures<'a>(
        &'a self,
        out: &mut Vec<crate::renderer::readback::FetchableTexture<'a>>,
    ) {
        use crate::renderer::readback::{FetchSource, FetchableTexture};
        let Some(targets) = self.targets.as_ref() else {
            return;
        };
        // The history last written: `frame` has moved on since.
        let last = &targets.history[(self.frame.wrapping_sub(1) & 1) as usize];
        let mut add = |name: &str, description, target: &'a Target| {
            out.push(FetchableTexture::new(
                format!("pt.{name}"),
                description,
                &target.texture,
            ));
        };
        add(
            "diffuse",
            "Path tracing, final diffuse light (demodulated: before the base colour multiplies \
             it). With the denoiser: rgb filtered, a its variance; without: the raw trace with \
             caustics added, a the rays' mean hit distance",
            &targets.diffuse_a,
        );
        add(
            "specular",
            "Path tracing, final specular light (demodulated: divided by the specular albedo); \
             a as for pt.diffuse",
            &targets.specular_a,
        );
        add(
            "material",
            "Path tracing, the primary surface: rgb base colour, a roughness (0 where nothing \
             was path traced)",
            &targets.material,
        );
        add(
            "emission",
            "Path tracing, the primary surface: rgb emitted light, a metallic (-1 where nothing \
             was path traced and the main pass's colour is kept)",
            &targets.emission,
        );
        add(
            "history.diffuse",
            "Denoiser history, diffuse: rgb accumulated light (after anti-lag and history fix), \
             a the frames accumulated",
            &last.diffuse,
        );
        add(
            "history.specular",
            "Denoiser history, specular: rgb accumulated light, a the frames accumulated",
            &last.specular,
        );
        add(
            "history.moments",
            "Denoiser history: diffuse luminance^2, specular luminance^2, diffuse fast-history \
             luminance, specular fast-history luminance",
            &last.moments,
        );
        add(
            "history.geometry",
            "Denoiser history: xy octahedral normal, z view depth (m), w roughness (0: not path traced)",
            &last.geometry,
        );
        add(
            "history.hit",
            "Denoiser history: accumulated hit distance of diffuse (r) and specular (g) rays (m)",
            &last.hit,
        );
        add(
            "radiance_cache",
            "Last frame's composited path-traced light (HDR), which bounce rays reuse where it \
             shows what they hit",
            &targets.radiance,
        );
        out.push(FetchableTexture {
            name: "pt.caustic_sums".into(),
            description: "Caustic photons' splats this frame: red, green, blue sums per pixel in \
                          fixed point (divide by 256 for demodulated diffuse light), before the \
                          resolve gathers them",
            source: FetchSource::Buffer {
                buffer: &targets.caustic_sums,
                width: targets.size.0,
                height: targets.size.1,
                channels: 3,
            },
        });
    }

    /// The scene targets were recreated: bind them anew.
    pub fn invalidate(&mut self) {
        self.bind_groups = None;
    }

    /// Start accumulating anew (camera cut, a frame without path tracing).
    pub fn reset_history(&mut self) {
        self.history_valid = false;
    }

    /// Free the targets while path tracing is off.
    pub fn release(&mut self) {
        self.targets = None;
        self.bind_groups = None;
        self.history_valid = false;
        self.previous = None;
    }

    fn ensure_targets(&mut self, device: &wgpu::Device, size: (u32, u32)) {
        if self
            .targets
            .as_ref()
            .is_none_or(|targets| targets.size != size)
        {
            self.targets = Some(Targets::new(device, size));
            self.bind_groups = None;
            self.history_valid = false;
        }
    }

    fn ensure_bind_groups(&mut self, device: &wgpu::Device, inputs: &PathTracingInputs<'_>) {
        if self.bind_groups.is_some() {
            return;
        }
        let Some(targets) = self.targets.as_ref() else {
            return;
        };
        // Last frame's geometry validates the radiance cache.
        let trace = [0, 1].map(|current| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("PtTraceInputs"),
                layout: &self.trace_inputs_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 190,
                        resource: self.uniform.as_entire_binding(),
                    },
                    view(191, inputs.normal),
                    view(192, inputs.position),
                    view(193, inputs.scene),
                    view(196, &targets.radiance.view),
                    view(197, &targets.history[1 - current].geometry.view),
                ],
            })
        });
        let temporal = [0, 1].map(|current| {
            let (now, prev) = (&targets.history[current], &targets.history[1 - current]);
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("PtTemporal"),
                layout: &self.temporal_layout,
                entries: &[
                    view(0, &targets.diffuse_a.view),
                    view(1, &targets.specular_a.view),
                    view(2, &targets.material.view),
                    view(3, inputs.normal),
                    view(4, inputs.position),
                    view(5, &prev.diffuse.view),
                    view(6, &prev.specular.view),
                    view(7, &prev.moments.view),
                    view(8, &prev.geometry.view),
                    view(9, &prev.hit.view),
                    view(10, &targets.diffuse_b.view),
                    view(11, &targets.specular_b.view),
                    view(12, &now.moments.view),
                    view(13, &now.geometry.view),
                    view(14, &now.hit.view),
                ],
            })
        });
        let history_fix = [0, 1].map(|current| {
            let now = &targets.history[current];
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("PtHistoryFix"),
                layout: &self.history_fix_layout,
                entries: &[
                    view(20, &targets.diffuse_b.view),
                    view(21, &targets.specular_b.view),
                    view(22, &now.moments.view),
                    view(23, &now.geometry.view),
                    view(24, &now.diffuse.view),
                    view(25, &now.specular.view),
                ],
            })
        });
        // Iteration 0 reads the history and writes A; then B, A, B, A.
        let atrous = [0, 1].map(|current| {
            let now = &targets.history[current];
            std::array::from_fn(|iteration| {
                let (input, output) = match iteration {
                    0 => (
                        (&now.diffuse, &now.specular),
                        (&targets.diffuse_a, &targets.specular_a),
                    ),
                    i if i % 2 == 1 => (
                        (&targets.diffuse_a, &targets.specular_a),
                        (&targets.diffuse_b, &targets.specular_b),
                    ),
                    _ => (
                        (&targets.diffuse_b, &targets.specular_b),
                        (&targets.diffuse_a, &targets.specular_a),
                    ),
                };
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("PtAtrous"),
                    layout: &self.atrous_layout,
                    entries: &[
                        view(40, &input.0.view),
                        view(41, &input.1.view),
                        view(42, &now.moments.view),
                        view(43, &now.geometry.view),
                        view(44, &now.hit.view),
                        view(45, inputs.position),
                        wgpu::BindGroupEntry {
                            binding: 46,
                            resource: self.atrous_steps[iteration].as_entire_binding(),
                        },
                        view(47, &output.0.view),
                        view(48, &output.1.view),
                    ],
                })
            })
        });
        const _: () = assert!(ATROUS_ITERATIONS % 2 == 1, "the composite reads target A");
        let composite = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("PtComposite"),
            layout: &self.composite_layout,
            entries: &[
                view(50, &targets.diffuse_a.view),
                view(51, &targets.specular_a.view),
                view(52, &targets.material.view),
                view(53, &targets.emission.view),
                view(54, inputs.normal),
                view(55, inputs.position),
            ],
        });
        let photons = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("PtPhotons"),
            layout: &self.photon_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 190,
                    resource: self.uniform.as_entire_binding(),
                },
                view(191, inputs.normal),
                view(192, inputs.position),
                wgpu::BindGroupEntry {
                    binding: 194,
                    resource: self.caustic_uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 195,
                    resource: targets.caustic_sums.as_entire_binding(),
                },
            ],
        });
        let caustic_resolve = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("PtCausticResolve"),
            layout: &self.caustic_resolve_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 60,
                    resource: targets.caustic_sums.as_entire_binding(),
                },
                view(61, inputs.normal),
                view(62, inputs.position),
            ],
        });
        self.bind_groups = Some(BindGroups {
            trace,
            temporal,
            history_fix,
            atrous,
            composite,
            photons,
            caustic_resolve,
        });
    }

    fn write_uniform(&self, queue: &wgpu::Queue, frame: &PathTracingFrame<'_>, valid: bool) {
        let camera = frame.camera;
        let previous = self.previous.unwrap_or(CameraSnapshot {
            view_proj: camera.unjittered_view_proj,
            view: camera.view,
            position: camera.position,
        });
        let region = frame.region;
        let height = region.height().max(1) as f32;
        let scale_y = camera.proj.y_axis.y.abs().max(1e-6);
        let perspective = camera.proj.w_axis.w == 0.0;
        // World size of a pixel: per meter of depth in perspective, fixed in orthographic.
        let pixel = 2.0 / (scale_y * height);
        let footprint = if perspective {
            [pixel, 0.0]
        } else {
            [0.0, pixel]
        };
        let output_pixel_angle = 2.0 / (scale_y * frame.output_height.max(1.0));
        let settings = frame.settings;
        let mut flags = 0;
        if valid {
            flags |= FLAG_HISTORY_VALID;
        }
        if settings.denoiser {
            flags |= FLAG_DENOISE;
        }
        if settings.caustics && !frame.caustics.casters.is_empty() {
            flags |= FLAG_CAUSTICS;
        }
        let uniform = PtUniform {
            view_proj: camera.unjittered_view_proj.to_cols_array_2d(),
            prev_view_proj: previous.view_proj.to_cols_array_2d(),
            prev_view_proj_inv: previous.view_proj.inverse().to_cols_array_2d(),
            view: camera.view.to_cols_array_2d(),
            prev_view: previous.view.to_cols_array_2d(),
            camera_position: camera
                .position
                .extend(if perspective { 1.0 } else { 0.0 })
                .to_array(),
            prev_camera_position: previous.position.extend(0.0).to_array(),
            region: [
                region.x() as f32,
                region.y() as f32,
                region.width() as f32,
                region.height() as f32,
            ],
            frame: [self.frame, settings.bounces, settings.samples, flags],
            params: [
                settings.max_history as f32,
                settings.max_history as f32,
                (0.5 * frame.sun_size_degrees).to_radians(),
                frame.lamp_radius,
            ],
            footprint: [
                footprint[0],
                footprint[1],
                // Orthographic rays do not spread.
                if perspective { output_pixel_angle } else { 0.0 },
                0.0,
            ],
        };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniform));
    }

    /// Path trace the scene region and write the result into `frame.scene`. Returns false,
    /// drawing nothing, when the texture array the trace was built for is missing.
    pub fn run(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        frame: PathTracingFrame<'_>,
        timer: &mut crate::renderer::gpu_timer::GpuTimer,
    ) -> bool {
        let textures = match (self.textures, frame.ray_tracing.textures) {
            (true, Some(textures)) => Some(textures),
            (true, None) => return false,
            (false, _) => None,
        };
        let region = frame.region;
        let size = (region.width(), region.height());
        self.ensure_targets(device, size);
        self.ensure_bind_groups(device, &frame.inputs);
        if self.last_region != Some(region) {
            self.history_valid = false;
            self.last_region = Some(region);
        }
        let valid = self.history_valid && self.previous.is_some();
        self.write_uniform(queue, &frame, valid);
        // Caustic photons: shared evenly between every light and every caster.
        let casters = frame.caustics.casters.len().min(MAX_CAUSTIC_CASTERS) as u32;
        let flux = frame.caustics.pair_flux();
        let pairs = flux.len() as u32;
        let photon_grid = if frame.settings.caustics && casters > 0 && pairs > 0 {
            let width = (frame.settings.caustic_photons as f32)
                .sqrt()
                .ceil()
                .max(1.0) as u32;
            let starts = photon_starts(&flux, width * width);
            let photons = *starts.last().unwrap_or(&0);
            (photons > 0).then(|| {
                let mut uniform = PtCaustics {
                    casters: [[0.0; 4]; MAX_CAUSTIC_CASTERS],
                    pair_starts: [[0; 4]; PAIR_START_VECTORS],
                    counts: [casters, pairs, photons, width],
                };
                uniform.casters[..casters as usize]
                    .copy_from_slice(&frame.caustics.casters[..casters as usize]);
                for (index, start) in starts.iter().enumerate() {
                    uniform.pair_starts[index / 4][index % 4] = *start;
                }
                queue.write_buffer(&self.caustic_uniform, 0, bytemuck::bytes_of(&uniform));
                if self.photon_target.as_ref().is_none_or(|(w, _)| *w != width) {
                    let target = Target::new(
                        device,
                        (width, width),
                        PHOTON_TARGET_FORMAT,
                        "PtPhotonGrid",
                        true,
                    );
                    self.photon_target = Some((width, target));
                }
                width
            })
        } else {
            None
        };
        let (Some(targets), Some(groups)) = (self.targets.as_ref(), self.bind_groups.as_ref())
        else {
            return false;
        };
        let current = (self.frame & 1) as usize;

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("PtTrace"),
                color_attachments: &[
                    attachment(&targets.diffuse_a),
                    attachment(&targets.specular_a),
                    attachment(&targets.material),
                    attachment(&targets.emission),
                ],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.trace_pipeline);
            pass.set_bind_group(0, frame.uniform_bind_group, &[]);
            pass.set_bind_group(1, &groups.trace[current], &[]);
            pass.set_bind_group(2, frame.ray_tracing.lights, &[]);
            pass.set_bind_group(3, frame.ray_tracing.scene, &[]);
            if let Some(textures) = textures {
                pass.set_bind_group(4, textures, &[]);
            }
            pass.draw(0..3, 0..1);
        }
        timer.mark(encoder, "path_tracing");

        if let (Some(_), Some((_, grid))) = (photon_grid, self.photon_target.as_ref()) {
            encoder.clear_buffer(&targets.caustic_sums, 0, None);
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("PtPhotons"),
                    color_attachments: &[attachment(grid)],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&self.photon_pipeline);
                pass.set_bind_group(0, frame.uniform_bind_group, &[]);
                pass.set_bind_group(1, &groups.photons, &[]);
                pass.set_bind_group(2, frame.ray_tracing.lights, &[]);
                pass.set_bind_group(3, frame.ray_tracing.scene, &[]);
                if let Some(textures) = textures {
                    pass.set_bind_group(4, textures, &[]);
                }
                pass.draw(0..3, 0..1);
            }
            {
                let mut pass = begin_color_pass(
                    encoder,
                    "PtCausticResolve",
                    ColorAttachment::single(&targets.diffuse_a.view),
                    wgpu::LoadOp::Load,
                    None,
                );
                pass.set_pipeline(&self.caustic_resolve_pipeline);
                pass.set_bind_group(0, &self.uniform_bind_group, &[]);
                pass.set_bind_group(1, &groups.caustic_resolve, &[]);
                pass.draw(0..3, 0..1);
            }
            timer.mark(encoder, "caustics");
        }

        if frame.settings.denoiser {
            let workgroups = (
                size.0.div_ceil(WORKGROUP_SIZE),
                size.1.div_ceil(WORKGROUP_SIZE),
            );
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("PtDenoise"),
                timestamp_writes: None,
            });
            pass.set_bind_group(0, &self.uniform_bind_group, &[]);
            pass.set_pipeline(&self.temporal_pipeline);
            pass.set_bind_group(1, &groups.temporal[current], &[]);
            pass.dispatch_workgroups(workgroups.0, workgroups.1, 1);
            pass.set_pipeline(&self.history_fix_pipeline);
            pass.set_bind_group(1, &groups.history_fix[current], &[]);
            pass.dispatch_workgroups(workgroups.0, workgroups.1, 1);
            pass.set_pipeline(&self.atrous_pipeline);
            for group in &groups.atrous[current] {
                pass.set_bind_group(1, group, &[]);
                pass.dispatch_workgroups(workgroups.0, workgroups.1, 1);
            }
        }

        {
            let mut pass = begin_color_pass(
                encoder,
                "PtComposite",
                ColorAttachment::single(frame.scene),
                wgpu::LoadOp::Load,
                Some(region),
            );
            pass.set_pipeline(&self.composite_pipeline);
            pass.set_bind_group(0, &self.uniform_bind_group, &[]);
            pass.set_bind_group(1, &groups.composite, &[]);
            pass.draw(0..3, 0..1);
        }
        // Next frame's radiance cache.
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: frame.scene_texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: region.x(),
                    y: region.y(),
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: &targets.radiance.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
        );
        timer.mark(encoder, "denoise");

        // The history only carries over while the denoiser keeps writing it.
        self.history_valid = frame.settings.denoiser;
        self.previous = Some(CameraSnapshot {
            view_proj: frame.camera.unjittered_view_proj,
            view: frame.camera.view,
            position: frame.camera.position,
        });
        self.frame = self.frame.wrapping_add(1);
        true
    }
}

fn view(binding: u32, view: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::TextureView(view),
    }
}

/// A trace target, cleared.
fn attachment(target: &Target) -> Option<wgpu::RenderPassColorAttachment<'_>> {
    Some(wgpu::RenderPassColorAttachment {
        view: &target.view,
        resolve_target: None,
        depth_slice: None,
        ops: wgpu::Operations {
            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
            store: wgpu::StoreOp::Store,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn validate(source: &str, capabilities: naga::valid::Capabilities) {
        let module = naga::front::wgsl::parse_str(source)
            .unwrap_or_else(|err| panic!("{}", err.emit_to_string(source)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), capabilities)
            .validate(&module)
            .unwrap_or_else(|err| panic!("{}", err.emit_to_string(source)));
    }

    #[test]
    fn path_tracing_shaders_validate() {
        for textures in [false, true] {
            validate(
                &pt_trace_shader_source(textures),
                naga::valid::Capabilities::RAY_QUERY
                    | naga::valid::Capabilities::TEXTURE_AND_SAMPLER_BINDING_ARRAY
                    | naga::valid::Capabilities::TEXTURE_AND_SAMPLER_BINDING_ARRAY_NON_UNIFORM_INDEXING,
            );
        }
        validate(
            &pt_denoise_shader_source(),
            naga::valid::Capabilities::empty(),
        );
        validate(
            &pt_composite_shader_source(),
            naga::valid::Capabilities::empty(),
        );
    }

    #[test]
    fn uniforms_match_the_shader_layout() {
        assert_eq!(std::mem::size_of::<PtUniform>(), 416);
        assert_eq!(std::mem::size_of::<AtrousStep>(), 16);
        assert_eq!(
            std::mem::size_of::<PtCaustics>(),
            16 * (MAX_CAUSTIC_CASTERS + PAIR_START_VECTORS + 1)
        );
    }

    #[test]
    fn photons_follow_the_light_reaching_each_caster() {
        let starts = photon_starts(&[9.0, 0.0, 1.0], 1000);
        assert_eq!(starts, vec![0, 750, 750, 1000]);
        assert_eq!(photon_starts(&[0.0, 0.0], 1000), vec![0, 0, 0]);
    }
}
