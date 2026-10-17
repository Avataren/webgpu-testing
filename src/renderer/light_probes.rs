//! Volumes of light probes for indirect diffuse light (DDGI: Majercik et al., "Dynamic
//! Diffuse Global Illumination with Ray-Traced Irradiance Fields", JCGT 2019).
//!
//! Every probe keeps an octahedral tile of irradiance (the light arriving from each
//! direction, cosine weighted) and one of distances (the mean and mean square distance it
//! sees in each direction) in two atlases, which the main lights group binds (12-14, read
//! by `light_probes.wgsl`). Surfaces take the light of the eight probes around them,
//! weighted by whether those probes can see them, so light does not leak through walls.
//!
//! Up to [`MAX_LIGHT_PROBE_VOLUMES`] volumes share the atlases: each has its own region,
//! laid out as a lone volume's atlas would be (so its bake file maps straight onto it),
//! the regions one below the other at the atlases' left edge. A volume's grid may be
//! rotated; the tiles hold world directions. Surfaces take the volumes in the order given:
//! the first one's light where it covers them, the next one's where it does not, and so
//! on, so nested and neighbouring volumes blend.
//!
//! With hardware ray tracing the volumes update themselves (`light_probe_update.wgsl`):
//! each frame up to [`MAX_PROBES_PER_UPDATE`] probes trace their rays (direct light at the
//! hits plus the probes' own light there, so bounces add up over frames), taken from the
//! volumes in turn, and the rays are blended into the tiles of their volume's region.
//!
//! Probes inside or right against geometry move out of it (relocation, as in RTXGI): the
//! irradiance pass also keeps an offset per probe, in texel rows below its volume's
//! tiles, which every lookup and trace adds to the probe's grid position.
//!
//! A bake ([`LightProbes::start_bake`]) traces one volume at a time at high quality
//! instead: more rays, warm-up sweeps for the bounces to build up, then the exact mean of
//! many sweeps. Its region is saved as a `.probes` file ([`encode_bake`]); where the
//! probes cannot trace (the web, GPUs without ray tracing), loading that file is the only
//! light the volume gets.

use std::collections::VecDeque;

use bytemuck::{Pod, Zeroable};
use glam::{Mat3, Mat4, Quat, Vec3};

use crate::renderer::shader_source::wgsl;
use crate::renderer::{PipelineBuilder, ShaderBuilder};

/// Most volumes the renderer uses at once (`LIGHT_PROBE_MAX_VOLUMES` in the shader).
pub const MAX_LIGHT_PROBE_VOLUMES: usize = 8;
/// Irradiance tile side in texels: 6x6 interior plus a one-texel border.
pub(crate) const IRRADIANCE_TILE: u32 = 8;
/// Distance tile side in texels: 14x14 interior plus a one-texel border.
pub(crate) const DISTANCE_TILE: u32 = 16;
const IRRADIANCE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const DISTANCE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Float;
const RAY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Most probes traced per frame, over all volumes (and rows of the ray texture).
pub(crate) const MAX_PROBES_PER_UPDATE: u32 = 2048;
const MIN_RAYS: u32 = 32;
const MAX_RAYS: u32 = 256;
/// Bias of a lookup point off the surface, along its normal and toward the eye, in
/// spacings.
const NORMAL_BIAS: f32 = 0.1;
const VIEW_BIAS: f32 = 0.2;
/// The sky's blur for probe rays, as a share of the environment's mip levels.
const SKY_BLUR: f32 = 0.5;
/// A probe with more of its rays than this on backfaces is inside geometry.
const BACKFACE_SHARE: f32 = 0.25;
/// Distances beyond this many spacings are kept as this (they matter to no neighbour).
const MAX_DISTANCE_SPACINGS: f32 = 1.5;

/// A volume's grid, as a scene asks for it (from a `LightProbeVolume`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightProbeGrid {
    /// Who asks for it (the volume entity's bits): keeps its probes' light while the
    /// volumes around it change.
    pub id: u64,
    /// The first probe's position (the box's corner the grid steps away from).
    pub origin: Vec3,
    /// The grid's orientation: probes step along its x, y and z axes.
    pub rotation: Quat,
    /// Distance between neighbouring probes (m).
    pub spacing: f32,
    /// Probes along x, y and z.
    pub counts: [u32; 3],
    pub rays_per_probe: u32,
    /// The share of a probe's previous light kept per update.
    pub hysteresis: f32,
    pub intensity: f32,
    /// Update by ray tracing where possible (false: show baked data only).
    pub live: bool,
    /// Move probes out of geometry they sit in or against.
    pub relocate: bool,
}

impl LightProbeGrid {
    /// A grid of `counts` probes `spacing` apart, centred on `center` and turned by
    /// `rotation`.
    pub fn centered(
        center: Vec3,
        rotation: Quat,
        spacing: f32,
        counts: [u32; 3],
        rays_per_probe: u32,
        hysteresis: f32,
        intensity: f32,
    ) -> Self {
        let mut grid = Self {
            id: 0,
            origin: Vec3::ZERO,
            rotation,
            spacing,
            counts,
            rays_per_probe,
            hysteresis,
            intensity,
            live: true,
            relocate: true,
        };
        grid.origin = center - rotation * (grid.extent() * 0.5);
        grid
    }

    pub fn probe_count(&self) -> u32 {
        self.counts[0] * self.counts[1] * self.counts[2]
    }

    /// From the first probe to the last, along the grid's own axes (m).
    pub fn extent(&self) -> Vec3 {
        (Vec3::new(
            self.counts[0] as f32,
            self.counts[1] as f32,
            self.counts[2] as f32,
        ) - Vec3::ONE)
            * self.spacing
    }

    /// Where a point given along the grid's own axes, from the first probe, is.
    pub fn to_world(&self, local: Vec3) -> Vec3 {
        self.origin + self.rotation * local
    }

    fn rays(&self) -> u32 {
        self.rays_per_probe.clamp(MIN_RAYS, MAX_RAYS)
    }

    /// Probe tiles per region row: about square regions.
    fn columns(&self) -> u32 {
        (self.probe_count() as f32).sqrt().ceil().max(1.0) as u32
    }

    /// Rows of probe tiles in a region.
    fn rows(&self) -> u32 {
        self.probe_count().div_ceil(self.columns())
    }

    fn atlas_size(&self, tile: u32) -> (u32, u32) {
        (self.columns() * tile, self.rows() * tile)
    }

    /// The volume's irradiance region: its tiles, then the probes' offsets (one texel
    /// each, rows as wide as the tiles).
    fn irradiance_atlas_size(&self) -> (u32, u32) {
        let (width, height) = self.atlas_size(IRRADIANCE_TILE);
        (width, height + self.probe_count().div_ceil(width))
    }

    /// Whether `other` puts its probes in the same places.
    fn same_placement(&self, other: &Self) -> bool {
        self.counts == other.counts
            && self.origin.distance(other.origin) <= 1e-3
            && (self.spacing - other.spacing).abs() <= 1e-4
            && self.rotation.angle_between(other.rotation) <= 1e-4
    }
}

/// Mirrors `LightProbeVolume` in light_probes.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable, PartialEq)]
struct VolumeUniform {
    origin_spacing: [f32; 4],
    counts: [u32; 4],
    atlas: [u32; 4],
    params: [f32; 4],
    axes: [[f32; 4]; 3],
}

/// Mirrors `LightProbeVolumes` in light_probes.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable, PartialEq)]
struct VolumesUniform {
    header: [u32; 4],
    volumes: [VolumeUniform; MAX_LIGHT_PROBE_VOLUMES],
}

/// `LightProbeVolume.counts.w` flags.
const VOLUME_LIT: u32 = 1;
const VOLUME_RELOCATE: u32 = 2;

/// Mirrors `LightProbeUpdate` in light_probe_update.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
struct UpdateUniform {
    rotation: [[f32; 4]; 4],
    frame: [u32; 4],
    params: [f32; 4],
    volume: [u32; 4],
}

const FLAG_RESET: u32 = 1;
/// Move the probes this update traces out of geometry (else their offsets stay).
const FLAG_RELOCATE: u32 = 2;

struct Atlas {
    current: wgpu::Texture,
    current_view: wgpu::TextureView,
    next: wgpu::Texture,
    next_view: wgpu::TextureView,
}

impl Atlas {
    fn new(
        device: &wgpu::Device,
        label: &str,
        format: wgpu::TextureFormat,
        size: (u32, u32),
    ) -> Self {
        let create = |suffix: &str, usage: wgpu::TextureUsages| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(&format!("{label}{suffix}")),
                size: wgpu::Extent3d {
                    width: size.0.max(1),
                    height: size.1.max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let current = create(
            "",
            wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
        );
        let next = create(
            "Next",
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        Self {
            current_view: current.create_view(&Default::default()),
            next_view: next.create_view(&Default::default()),
            current,
            next,
        }
    }

    /// Make the next frame's texels in `region` (x, y, width, height) the current ones.
    fn swap_region(&self, encoder: &mut wgpu::CommandEncoder, region: [u32; 4]) {
        copy_region(encoder, &self.next, &self.current, region, region[1]);
    }
}

/// Copy `region` (x, y, width, height) of `from` to `to`, at the same column but from row
/// `to_y`.
fn copy_region(
    encoder: &mut wgpu::CommandEncoder,
    from: &wgpu::Texture,
    to: &wgpu::Texture,
    region: [u32; 4],
    to_y: u32,
) {
    if region[2] == 0 || region[3] == 0 {
        return;
    }
    let at = |texture, y| wgpu::TexelCopyTextureInfo {
        texture,
        mip_level: 0,
        origin: wgpu::Origin3d {
            x: region[0],
            y,
            z: 0,
        },
        aspect: wgpu::TextureAspect::All,
    };
    encoder.copy_texture_to_texture(
        at(from, region[1]),
        at(to, to_y),
        wgpu::Extent3d {
            width: region[2],
            height: region[3],
            depth_or_array_layers: 1,
        },
    );
}

/// One volume the renderer shows: its grid, its regions of the atlases, and how far its
/// probes have come.
#[derive(Clone, Debug)]
struct Volume {
    grid: LightProbeGrid,
    /// First rows of its irradiance and distance regions.
    irradiance_top: u32,
    distance_top: u32,
    /// The first probe the next update traces (a slice per frame, round robin).
    next_probe: u32,
    /// Its region holds nothing yet: the next update of each probe replaces its value.
    reset: bool,
    /// Probes whose tiles hold no update yet.
    probes_untouched: u32,
    /// The region holds light (traced or loaded): surfaces may read it.
    has_data: bool,
    /// The baked file the region was loaded from.
    baked_source: Option<String>,
    /// Live updates replaced (some of) the loaded bake's light since.
    traced_since_load: bool,
}

impl Volume {
    fn new(grid: LightProbeGrid) -> Self {
        Self {
            grid,
            irradiance_top: 0,
            distance_top: 0,
            next_probe: 0,
            reset: true,
            probes_untouched: grid.probe_count(),
            has_data: false,
            baked_source: None,
            traced_since_load: false,
        }
    }

    /// Use `grid` from now on (placed as before). A volume that stops tracing live after
    /// tracing over its bake forgets the bake, so the file is loaded again and shown.
    fn set_grid(&mut self, grid: LightProbeGrid) {
        if self.grid.live && !grid.live && self.traced_since_load {
            self.baked_source = None;
            self.traced_since_load = false;
        }
        self.grid = grid;
    }

    /// Start over: the probes moved, or the region is new.
    fn start_over(&mut self) {
        self.next_probe = 0;
        self.reset = true;
        self.probes_untouched = self.grid.probe_count();
        self.has_data = false;
        self.baked_source = None;
        self.traced_since_load = false;
    }

    fn irradiance_region(&self) -> [u32; 4] {
        let (width, height) = self.grid.irradiance_atlas_size();
        [0, self.irradiance_top, width, height]
    }

    fn distance_region(&self) -> [u32; 4] {
        let (width, height) = self.grid.atlas_size(DISTANCE_TILE);
        [0, self.distance_top, width, height]
    }
}

/// A bake of one volume in progress.
#[derive(Clone, Copy, Debug)]
struct Bake {
    volume: u64,
    settings: BakeSettings,
    /// Sweeps over every probe completed, warm-up included.
    done: u32,
    /// Probes of the current sweep already traced.
    into_sweep: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BakeSettings {
    warmup: u32,
    sweeps: u32,
    rays: u32,
}

impl BakeSettings {
    /// Frames a bake of `probes` probes takes (one update per frame).
    fn frames(&self, probes: u32) -> u32 {
        (self.warmup + self.sweeps) * probes.div_ceil(MAX_PROBES_PER_UPDATE)
    }
}

impl Bake {
    fn finished(&self) -> bool {
        self.done >= self.settings.warmup + self.settings.sweeps
    }
}

/// How far a bake has come.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LightProbeBakeProgress {
    /// The volume being baked (its id).
    pub volume: u64,
    /// Volumes waiting to be baked after it.
    pub queued: u32,
    /// Warm-up sweeps, for the bounced light to build up before it is averaged.
    pub warmup_sweeps: u32,
    /// Sweeps averaged after the warm-up.
    pub sweeps: u32,
    pub rays_per_probe: u32,
    /// Sweeps completed, warm-up included.
    pub sweeps_done: u32,
    pub finished: bool,
}

/// How a finished bake ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LightProbeBakeOutcome {
    /// Counts the bakes that ended, from 1.
    pub serial: u32,
    /// The volume baked (its id).
    pub volume: u64,
    /// The project path of the saved `.probes` file, or what went wrong.
    pub result: Result<String, String>,
}

/// A finished bake to read back: the volume, its file's header, and where its data is.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct FinishedBake<'a> {
    pub volume: u64,
    pub header: BakeHeader,
    pub irradiance: &'a wgpu::Texture,
    pub irradiance_region: [u32; 4],
    pub distance: &'a wgpu::Texture,
    pub distance_region: [u32; 4],
}

struct UpdateStage {
    /// One slot per slice traced in a frame (up to one per volume), `slot` bytes apart.
    uniform: wgpu::Buffer,
    slot: u64,
    trace_inputs: wgpu::BindGroup,
    blend_layout: wgpu::BindGroupLayout,
    trace: wgpu::RenderPipeline,
    irradiance: wgpu::RenderPipeline,
    distance: wgpu::RenderPipeline,
}

/// What one frame traces of one volume.
#[derive(Clone, Copy, Debug)]
struct Slice {
    volume: usize,
    first: u32,
    probes: u32,
    rays: u32,
    hysteresis: f32,
    flags: u32,
}

/// The light probe volumes' resources, and their update when ray tracing is available.
pub(crate) struct LightProbes {
    volumes: Vec<Volume>,
    grid_buffer: wgpu::Buffer,
    last_uniform: Option<VolumesUniform>,
    irradiance: Atlas,
    distance: Atlas,
    update: Option<UpdateStage>,
    /// The ray texture, as many rows as the largest slice.
    rays: Option<(wgpu::Texture, wgpu::TextureView)>,
    blend_inputs: Option<wgpu::BindGroup>,
    /// The volume the next frame's live update starts with.
    next_volume: usize,
    rng: u64,
    /// The atlases were recreated: the lights group must bind the new views.
    bindings_changed: bool,
    bake: Option<Bake>,
    /// Volumes to bake after the running one, with its settings.
    bake_queue: VecDeque<u64>,
}

/// The update shader: the probe passes with the ray tracing modules (`textures`: the
/// bindless material textures, as the path tracer has them).
pub(crate) fn light_probe_update_shader_source(textures: bool) -> String {
    let textures_module = if textures {
        wgsl!("postprocess/rt_textures_bindless.wgsl")
    } else {
        wgsl!("postprocess/rt_textures_none.wgsl")
    };
    let modules = ShaderBuilder::new()
        .with_module(wgsl!("postprocess/fullscreen.wgsl"))
        .with_constants()
        .with_material_system()
        .with_lighting()
        .with_environment()
        .with_module(textures_module)
        .with_module(wgsl!("postprocess/rt_scene.wgsl"))
        .with_module(wgsl!("postprocess/rt_alpha.wgsl"))
        .with_module(wgsl!("postprocess/rt_common.wgsl"))
        .with_module(wgsl!("light_probe_update.wgsl"))
        .build_modules_only();
    format!("enable wgpu_ray_query;\n\n{modules}")
}

/// The update uniform as each pass binds it: one slot, chosen by the dynamic offset.
fn update_uniform_binding(buffer: &wgpu::Buffer) -> wgpu::BindingResource<'_> {
    wgpu::BindingResource::Buffer(wgpu::BufferBinding {
        buffer,
        offset: 0,
        size: wgpu::BufferSize::new(std::mem::size_of::<UpdateUniform>() as u64),
    })
}

impl LightProbes {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let grid_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("LightProbeVolumes"),
            size: std::mem::size_of::<VolumesUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            volumes: Vec::new(),
            grid_buffer,
            last_uniform: None,
            irradiance: Atlas::new(device, "LightProbeIrradiance", IRRADIANCE_FORMAT, (1, 1)),
            distance: Atlas::new(device, "LightProbeDistance", DISTANCE_FORMAT, (1, 1)),
            update: None,
            rays: None,
            blend_inputs: None,
            next_volume: 0,
            rng: 0x9e37_79b9_7f4a_7c15,
            bindings_changed: false,
            bake: None,
            bake_queue: VecDeque::new(),
        }
    }

    pub(crate) fn grid_buffer(&self) -> &wgpu::Buffer {
        &self.grid_buffer
    }

    pub(crate) fn irradiance_view(&self) -> &wgpu::TextureView {
        &self.irradiance.current_view
    }

    pub(crate) fn distance_view(&self) -> &wgpu::TextureView {
        &self.distance.current_view
    }

    /// Whether the atlases were recreated since the last call (the lights group then
    /// binds the new views).
    pub(crate) fn take_bindings_changed(&mut self) -> bool {
        std::mem::take(&mut self.bindings_changed)
    }

    /// Build the ray-traced update. Without it the volumes give only baked light.
    pub(crate) fn enable_ray_tracing(
        &mut self,
        device: &wgpu::Device,
        lights_layout: &wgpu::BindGroupLayout,
        scene_layout: &wgpu::BindGroupLayout,
        textures_layout: Option<&wgpu::BindGroupLayout>,
    ) -> Result<(), String> {
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let uniform_size = std::mem::size_of::<UpdateUniform>() as u64;
        let uniform_entry = wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: true,
                min_binding_size: wgpu::BufferSize::new(uniform_size),
            },
            count: None,
        };
        let trace_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("LightProbeTraceInputs"),
            entries: &[uniform_entry],
        });
        let blend_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("LightProbeBlendInputs"),
            entries: &[
                uniform_entry,
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("LightProbeUpdateShader"),
            source: wgpu::ShaderSource::Wgsl(
                light_probe_update_shader_source(textures_layout.is_some()).into(),
            ),
        });
        let trace_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("LightProbeTraceLayout"),
                bind_group_layouts: &[
                    Some(&trace_layout),
                    None,
                    Some(lights_layout),
                    Some(scene_layout),
                    textures_layout,
                ],
                immediate_size: 0,
            });
        let blend_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("LightProbeBlendLayout"),
                bind_group_layouts: &[Some(&blend_layout), None, Some(lights_layout)],
                immediate_size: 0,
            });
        let trace = PipelineBuilder::new(device, &trace_pipeline_layout, &shader)
            .with_label("LightProbeTrace")
            .with_vertex_entry("vs_fullscreen")
            .with_fragment_entry("fs_probe_trace")
            .with_color_target(RAY_FORMAT, None)
            .with_no_culling()
            .build();
        let irradiance = PipelineBuilder::new(device, &blend_pipeline_layout, &shader)
            .with_label("LightProbeIrradiance")
            .with_vertex_entry("vs_fullscreen")
            .with_fragment_entry("fs_probe_irradiance")
            .with_color_target(IRRADIANCE_FORMAT, None)
            .with_no_culling()
            .build();
        let distance = PipelineBuilder::new(device, &blend_pipeline_layout, &shader)
            .with_label("LightProbeDistance")
            .with_vertex_entry("vs_fullscreen")
            .with_fragment_entry("fs_probe_distance")
            .with_color_target(DISTANCE_FORMAT, None)
            .with_no_culling()
            .build();
        let alignment = (device.limits().min_uniform_buffer_offset_alignment as u64).max(1);
        let slot = uniform_size.div_ceil(alignment) * alignment;
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("LightProbeUpdate"),
            size: slot * MAX_LIGHT_PROBE_VOLUMES as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let trace_inputs = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("LightProbeTraceInputs"),
            layout: &trace_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: update_uniform_binding(&uniform),
            }],
        });
        if let Some(error) = pollster::block_on(validation.pop()) {
            return Err(error.to_string());
        }
        self.update = Some(UpdateStage {
            uniform,
            slot,
            trace_inputs,
            blend_layout,
            trace,
            irradiance,
            distance,
        });
        self.blend_inputs = None;
        Ok(())
    }

    /// Whether any volume traces this frame (it needs the acceleration structures): a
    /// live one, or a bake.
    pub(crate) fn wants_update(&self) -> bool {
        if self.update.is_none() {
            return false;
        }
        match self.bake {
            Some(bake) => !bake.finished(),
            None => self.volumes.iter().any(|volume| volume.grid.live),
        }
    }

    /// Whether the probes can trace at all (hardware ray tracing).
    pub(crate) fn can_trace(&self) -> bool {
        self.update.is_some()
    }

    fn volume_index(&self, id: u64) -> Option<usize> {
        self.volumes.iter().position(|volume| volume.grid.id == id)
    }

    /// The baked file volume `id`'s region holds, if it was loaded from one.
    pub(crate) fn baked_source(&self, id: u64) -> Option<&str> {
        let index = self.volume_index(id)?;
        self.volumes[index].baked_source.as_deref()
    }

    /// The probes the debug view draws: every volume's, once one holds light (the shader
    /// skips the volumes that do not).
    pub(crate) fn debug_probe_count(&self) -> Option<u32> {
        self.volumes
            .iter()
            .any(|volume| volume.has_data)
            .then(|| self.volumes.iter().map(|v| v.grid.probe_count()).sum())
    }

    /// Bake volumes `ids` one after another (every volume when `None`) at bake quality:
    /// `warmup` sweeps over every probe for the bounces to build up, then the mean of
    /// `sweeps` more, each with `rays` rays per probe. While a bake runs, the volumes
    /// asked for wait their turn and take its settings. Returns the frames the bakes
    /// asked for take (one update per frame).
    pub(crate) fn start_bake(
        &mut self,
        ids: Option<&[u64]>,
        warmup: u32,
        sweeps: u32,
        rays: u32,
    ) -> Result<u32, String> {
        if self.update.is_none() {
            return Err("baking traces the probes and needs hardware ray tracing".into());
        }
        let ids: Vec<u64> = match ids {
            Some(ids) => ids.to_vec(),
            None => self.volumes.iter().map(|volume| volume.grid.id).collect(),
        };
        if ids.is_empty() {
            return Err("the scene has no enabled light probe volume".into());
        }
        let running = self.bake.filter(|bake| !bake.finished());
        let settings = running.map_or(
            BakeSettings {
                warmup: warmup.max(1),
                sweeps: sweeps.max(1),
                rays: rays.clamp(MIN_RAYS, MAX_RAYS),
            },
            |bake| bake.settings,
        );
        let mut frames = 0;
        for &id in &ids {
            let index = self
                .volume_index(id)
                .ok_or_else(|| format!("no light probe volume {id} is in use to bake"))?;
            frames += settings.frames(self.volumes[index].grid.probe_count());
        }
        for id in ids {
            let baking = running.is_some_and(|bake| bake.volume == id);
            if !baking && !self.bake_queue.contains(&id) {
                self.bake_queue.push_back(id);
            }
        }
        if running.is_none() {
            self.start_next_bake(settings);
        }
        Ok(frames)
    }

    /// Bake the next queued volume that is still in use with `settings`.
    fn start_next_bake(&mut self, settings: BakeSettings) {
        self.bake = None;
        while let Some(id) = self.bake_queue.pop_front() {
            if let Some(index) = self.volume_index(id) {
                self.volumes[index].next_probe = 0;
                self.bake = Some(Bake {
                    volume: id,
                    settings,
                    done: 0,
                    into_sweep: 0,
                });
                return;
            }
        }
    }

    pub(crate) fn bake_progress(&self) -> Option<LightProbeBakeProgress> {
        self.bake.map(|bake| LightProbeBakeProgress {
            volume: bake.volume,
            queued: self.bake_queue.len() as u32,
            warmup_sweeps: bake.settings.warmup,
            sweeps: bake.settings.sweeps,
            rays_per_probe: bake.settings.rays,
            sweeps_done: bake.done,
            finished: bake.finished(),
        })
    }

    /// The finished bake's volume, its file's header, and where to read its data.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn finished_bake(&self) -> Option<FinishedBake<'_>> {
        let bake = self.bake.filter(Bake::finished)?;
        let volume = &self.volumes[self.volume_index(bake.volume)?];
        Some(FinishedBake {
            volume: bake.volume,
            header: BakeHeader::new(&volume.grid, bake.settings.sweeps, bake.settings.rays),
            irradiance: &self.irradiance.current,
            irradiance_region: volume.irradiance_region(),
            distance: &self.distance.current,
            distance_region: volume.distance_region(),
        })
    }

    /// Done with a finished (or failed) bake: the next queued volume bakes, or the
    /// volumes go back to updating live (or keep the baked light).
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn end_bake(&mut self) {
        if let Some(bake) = self.bake {
            self.start_next_bake(bake.settings);
        }
    }

    /// Show the baked data in `bytes` (a `.probes` file from [`encode_bake`], read from
    /// `source`) in volume `id`. It must be for that volume's probes.
    pub(crate) fn load_bake(
        &mut self,
        queue: &wgpu::Queue,
        id: u64,
        bytes: &[u8],
        source: &str,
    ) -> Result<(), String> {
        let index = self
            .volume_index(id)
            .ok_or("no light probe volume to load into")?;
        let grid = self.volumes[index].grid;
        let bake = decode_bake(bytes)?;
        let mut expected = BakeHeader::new(&grid, bake.header.sweeps, bake.header.rays);
        let (tiles_width, tiles_height) = grid.atlas_size(IRRADIANCE_TILE);
        if bake.version == 1 {
            // Tiles only: the probes stay at their grid positions.
            expected.irradiance_size = [tiles_width, tiles_height];
        }
        if !bake.header.matches(&expected) {
            return Err(format!(
                "{source} was baked for other probes ({:?} from {:?}, spacing {}, rotation \
                 {:?}; the volume now has {:?} from {:?}, spacing {}, rotation {:?}): bake again",
                bake.header.counts,
                bake.header.origin,
                bake.header.spacing,
                bake.header.rotation,
                expected.counts,
                expected.origin,
                expected.spacing,
                expected.rotation,
            ));
        }
        let volume = &self.volumes[index];
        let irradiance_region = volume.irradiance_region();
        let mut irradiance = std::borrow::Cow::Borrowed(&bake.irradiance[..]);
        if bake.version == 1 {
            // No offsets: zero rows below the tiles.
            let mut padded = irradiance.into_owned();
            let rows = irradiance_region[3] - tiles_height;
            padded.resize(padded.len() + (rows * tiles_width * 8) as usize, 0);
            irradiance = std::borrow::Cow::Owned(padded);
        }
        for (texture, data, region, texel_bytes) in [
            (
                &self.irradiance.current,
                &irradiance[..],
                irradiance_region,
                8u32,
            ),
            (
                &self.distance.current,
                &bake.distance[..],
                volume.distance_region(),
                4u32,
            ),
        ] {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: region[0],
                        y: region[1],
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(region[2] * texel_bytes),
                    rows_per_image: Some(region[3]),
                },
                wgpu::Extent3d {
                    width: region[2],
                    height: region[3],
                    depth_or_array_layers: 1,
                },
            );
        }
        let volume = &mut self.volumes[index];
        volume.has_data = true;
        volume.reset = false;
        volume.probes_untouched = 0;
        volume.baked_source = Some(source.to_owned());
        volume.traced_since_load = false;
        self.write_uniform(queue);
        Ok(())
    }

    /// Use `grids` from now on, in blending order (at most [`MAX_LIGHT_PROBE_VOLUMES`];
    /// none: surfaces take the environment's light). Volumes that stay keep their probes'
    /// light; a changed layout makes new atlases.
    pub(crate) fn set_grids(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        grids: &[LightProbeGrid],
    ) {
        let grids = &grids[..grids.len().min(MAX_LIGHT_PROBE_VOLUMES)];
        let same_layout = grids.len() == self.volumes.len()
            && grids.iter().zip(&self.volumes).all(|(grid, volume)| {
                grid.id == volume.grid.id && grid.counts == volume.grid.counts
            });
        if same_layout {
            for (index, grid) in grids.iter().enumerate() {
                let moved = !self.volumes[index].grid.same_placement(grid);
                self.volumes[index].set_grid(*grid);
                if moved {
                    // Moved probes see something else: start over.
                    self.volumes[index].start_over();
                    self.cancel_bake_of(grid.id);
                }
            }
        } else {
            self.relayout(device, queue, grids);
        }
        self.write_uniform(queue);
    }

    /// New atlases for `grids`: each volume that stays where it was keeps its light,
    /// copied into its new region.
    fn relayout(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, grids: &[LightProbeGrid]) {
        let mut volumes: Vec<Volume> = Vec::with_capacity(grids.len());
        // (old irradiance region, old distance region) of the volumes that keep theirs.
        let mut kept: Vec<Option<([u32; 4], [u32; 4])>> = Vec::with_capacity(grids.len());
        for grid in grids {
            let old = self
                .volumes
                .iter()
                .find(|volume| volume.grid.id == grid.id && volume.grid.same_placement(grid));
            match old {
                Some(old) if old.has_data => {
                    kept.push(Some((old.irradiance_region(), old.distance_region())));
                    let mut volume = old.clone();
                    volume.set_grid(*grid);
                    volumes.push(volume);
                }
                _ => {
                    kept.push(None);
                    volumes.push(Volume::new(*grid));
                }
            }
        }
        let (mut irradiance_height, mut distance_height, mut width) = (0, 0, 1);
        for volume in &mut volumes {
            volume.irradiance_top = irradiance_height;
            volume.distance_top = distance_height;
            let irradiance = volume.grid.irradiance_atlas_size();
            let distance = volume.grid.atlas_size(DISTANCE_TILE);
            irradiance_height += irradiance.1;
            distance_height += distance.1;
            width = width.max(distance.0);
        }
        let irradiance = Atlas::new(
            device,
            "LightProbeIrradiance",
            IRRADIANCE_FORMAT,
            (width / DISTANCE_TILE * IRRADIANCE_TILE, irradiance_height),
        );
        let distance = Atlas::new(
            device,
            "LightProbeDistance",
            DISTANCE_FORMAT,
            (width, distance_height),
        );
        if kept.iter().any(Option::is_some) {
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("LightProbeRelayout"),
            });
            for (volume, kept) in volumes.iter().zip(&kept) {
                if let Some((old_irradiance, old_distance)) = kept {
                    copy_region(
                        &mut encoder,
                        &self.irradiance.current,
                        &irradiance.current,
                        *old_irradiance,
                        volume.irradiance_top,
                    );
                    copy_region(
                        &mut encoder,
                        &self.distance.current,
                        &distance.current,
                        *old_distance,
                        volume.distance_top,
                    );
                }
            }
            queue.submit([encoder.finish()]);
        }
        self.irradiance = irradiance;
        self.distance = distance;

        let rows = volumes
            .iter()
            .map(|volume| volume.grid.probe_count())
            .max()
            .unwrap_or(0)
            .min(MAX_PROBES_PER_UPDATE);
        self.rays = (rows > 0).then(|| {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("LightProbeRays"),
                size: wgpu::Extent3d {
                    width: MAX_RAYS,
                    height: rows,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: RAY_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            (texture, view)
        });
        self.volumes = volumes;
        self.blend_inputs = None;
        self.bindings_changed = true;
        self.next_volume = 0;
        if let Some(bake) = self.bake {
            if self.volume_index(bake.volume).is_none() {
                self.cancel_bake_of(bake.volume);
            }
        }
        let ids: Vec<u64> = self.volumes.iter().map(|volume| volume.grid.id).collect();
        self.bake_queue.retain(|id| ids.contains(id));
    }

    /// Stop baking volume `id` (it moved or went away); the next queued one bakes.
    fn cancel_bake_of(&mut self, id: u64) {
        if let Some(bake) = self.bake.filter(|bake| bake.volume == id) {
            log::warn!("Light probes: volume {id} changed during its bake; the bake stopped");
            self.start_next_bake(bake.settings);
        }
        self.bake_queue.retain(|queued| *queued != id);
    }

    fn volumes_uniform(&self) -> VolumesUniform {
        let mut uniform = VolumesUniform::zeroed();
        uniform.header[0] = self.volumes.len() as u32;
        for (slot, volume) in uniform.volumes.iter_mut().zip(&self.volumes) {
            let grid = &volume.grid;
            let axes = Mat3::from_quat(grid.rotation);
            let flags = if volume.has_data { VOLUME_LIT } else { 0 }
                | if grid.relocate { VOLUME_RELOCATE } else { 0 };
            *slot = VolumeUniform {
                origin_spacing: [grid.origin.x, grid.origin.y, grid.origin.z, grid.spacing],
                counts: [grid.counts[0], grid.counts[1], grid.counts[2], flags],
                atlas: [
                    grid.columns(),
                    volume.irradiance_top,
                    volume.distance_top,
                    grid.rays(),
                ],
                params: [grid.intensity.max(0.0), NORMAL_BIAS, VIEW_BIAS, 0.0],
                axes: [
                    axes.x_axis.extend(0.0).to_array(),
                    axes.y_axis.extend(0.0).to_array(),
                    axes.z_axis.extend(0.0).to_array(),
                ],
            };
        }
        uniform
    }

    fn write_uniform(&mut self, queue: &wgpu::Queue) {
        let uniform = self.volumes_uniform();
        if self.last_uniform != Some(uniform) {
            queue.write_buffer(&self.grid_buffer, 0, bytemuck::bytes_of(&uniform));
            self.last_uniform = Some(uniform);
        }
    }

    /// What this frame traces: the baking volume's next slice, or slices of the live
    /// volumes in turn up to [`MAX_PROBES_PER_UPDATE`] probes.
    fn plan_slices(&mut self) -> Vec<Slice> {
        if let Some(bake) = self.bake {
            let Some(index) = self.volume_index(bake.volume) else {
                return Vec::new();
            };
            let Some(bake) = self.bake.as_mut().filter(|bake| !bake.finished()) else {
                return Vec::new();
            };
            let total = self.volumes[index].grid.probe_count();
            // Sweeps start at the first probe, so every probe of a sweep is traced as
            // often as the others.
            let probes = (total - bake.into_sweep).min(MAX_PROBES_PER_UPDATE);
            let mut flags = 0;
            let hysteresis = if bake.done < bake.settings.warmup {
                if bake.done == 0 {
                    flags |= FLAG_RESET;
                }
                // Probes settle where they are during the warm-up; the averaged sweeps
                // all see them in one place.
                flags |= FLAG_RELOCATE;
                // Fast to follow, as bounces build on the last sweep's light.
                0.5
            } else {
                // The exact mean of the averaged sweeps (the first one replaces).
                let averaged = (bake.done - bake.settings.warmup) as f32;
                averaged / (averaged + 1.0)
            };
            let first = bake.into_sweep;
            bake.into_sweep += probes;
            if bake.into_sweep >= total {
                bake.into_sweep = 0;
                bake.done += 1;
            }
            self.volumes[index].has_data = true;
            return vec![Slice {
                volume: index,
                first,
                probes,
                rays: bake.settings.rays,
                hysteresis,
                flags,
            }];
        }

        let count = self.volumes.len();
        let mut budget = MAX_PROBES_PER_UPDATE;
        let mut slices = Vec::new();
        for step in 0..count {
            let index = (self.next_volume + step) % count;
            let volume = &mut self.volumes[index];
            if !volume.grid.live {
                continue;
            }
            let total = volume.grid.probe_count();
            let probes = total.min(budget);
            let first = volume.next_probe % total;
            let mut flags = FLAG_RELOCATE;
            if volume.reset {
                // Every probe is replaced once; then they blend.
                flags |= FLAG_RESET;
                volume.probes_untouched = volume.probes_untouched.saturating_sub(probes);
                if volume.probes_untouched == 0 {
                    volume.reset = false;
                }
            }
            volume.next_probe = (first + probes) % total;
            volume.has_data = true;
            volume.traced_since_load = true;
            slices.push(Slice {
                volume: index,
                first,
                probes,
                rays: volume.grid.rays(),
                hysteresis: volume.grid.hysteresis.clamp(0.0, 0.995),
                flags,
            });
            budget -= probes;
            if probes < total {
                // Partly traced: it goes on from here next frame.
                self.next_volume = index;
                break;
            }
            if budget == 0 {
                self.next_volume = (index + 1) % count;
                break;
            }
        }
        slices
    }

    /// Trace this frame's slices of probes and blend them in. `lights` is the main
    /// lights group (which binds the current atlases), `scene` and `textures` the ray
    /// tracing groups.
    pub(crate) fn update(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        lights: &wgpu::BindGroup,
        scene: &wgpu::BindGroup,
        textures: Option<&wgpu::BindGroup>,
    ) {
        if self.update.is_none() || self.rays.is_none() {
            return;
        }
        let slices = self.plan_slices();
        if slices.is_empty() {
            return;
        }
        let rotations: Vec<Quat> = slices
            .iter()
            .map(|_| random_rotation(&mut self.rng))
            .collect();
        self.write_uniform(queue);
        let (Some(stage), Some((_, rays_view))) = (self.update.as_ref(), self.rays.as_ref()) else {
            return;
        };
        let blend_inputs = self.blend_inputs.get_or_insert_with(|| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("LightProbeBlendInputs"),
                layout: &stage.blend_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: update_uniform_binding(&stage.uniform),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(rays_view),
                    },
                ],
            })
        });

        for (slot, (slice, rotation)) in slices.iter().zip(rotations).enumerate() {
            let volume = &self.volumes[slice.volume];
            let uniform = UpdateUniform {
                rotation: Mat4::from_quat(rotation).to_cols_array_2d(),
                frame: [slice.first, slice.probes, slice.flags, slice.rays],
                params: [
                    slice.hysteresis,
                    volume.grid.spacing * MAX_DISTANCE_SPACINGS,
                    SKY_BLUR,
                    BACKFACE_SHARE,
                ],
                volume: [slice.volume as u32, 0, 0, 0],
            };
            let offset = slot as u64 * stage.slot;
            queue.write_buffer(&stage.uniform, offset, bytemuck::bytes_of(&uniform));
            let offset = offset as u32;

            {
                let mut pass = fullscreen_pass(encoder, "LightProbeTrace", rays_view);
                pass.set_viewport(0.0, 0.0, slice.rays as f32, slice.probes as f32, 0.0, 1.0);
                pass.set_scissor_rect(0, 0, slice.rays, slice.probes);
                pass.set_pipeline(&stage.trace);
                pass.set_bind_group(0, &stage.trace_inputs, &[offset]);
                pass.set_bind_group(2, lights, &[]);
                pass.set_bind_group(3, scene, &[]);
                if let Some(textures) = textures {
                    pass.set_bind_group(4, textures, &[]);
                }
                pass.draw(0..3, 0..1);
            }
            for (label, pipeline, atlas, region) in [
                (
                    "LightProbeIrradiance",
                    &stage.irradiance,
                    &self.irradiance,
                    volume.irradiance_region(),
                ),
                (
                    "LightProbeDistance",
                    &stage.distance,
                    &self.distance,
                    volume.distance_region(),
                ),
            ] {
                {
                    let mut pass = fullscreen_pass(encoder, label, &atlas.next_view);
                    pass.set_scissor_rect(region[0], region[1], region[2], region[3]);
                    pass.set_pipeline(pipeline);
                    pass.set_bind_group(0, &*blend_inputs, &[offset]);
                    pass.set_bind_group(2, lights, &[]);
                    pass.draw(0..3, 0..1);
                }
                atlas.swap_region(encoder, region);
            }
        }
    }

    /// The textures `texture_fetch` can read.
    pub(crate) fn fetchable_textures<'a>(
        &'a self,
        out: &mut Vec<crate::renderer::readback::FetchableTexture<'a>>,
    ) {
        use crate::renderer::readback::FetchableTexture;
        if self.volumes.is_empty() {
            return;
        }
        out.push(FetchableTexture::new(
            "probes.irradiance",
            "Light probe irradiance: a region per volume (one below the other, in blending \
             order), each an octahedral tile per probe (8x8 with the border, probes in rows of \
             about the square root of their count) of the light arriving from each direction, \
             cosine weighted (rgb); a: 1 for a probe in the open, 0 inside geometry. Below a \
             volume's tiles, one texel per probe in probe order: its offset from its grid \
             position (rgb, m; relocation out of geometry). light_probes_status gives each \
             volume's region",
            &self.irradiance.current,
        ));
        out.push(FetchableTexture::new(
            "probes.distance",
            "Light probe distances: a region per volume, each an octahedral tile per probe \
             (16x16 with the border) of the mean distance (r) and mean squared distance (g) \
             the probe sees in each direction (m)",
            &self.distance.current,
        ));
        if let Some((rays, _)) = &self.rays {
            out.push(FetchableTexture::new(
                "probes.rays",
                "Light probe rays of the last slice traced: one texel per ray, one row per \
                 probe; rgb the light along the ray, a the distance to its hit (negative on a \
                 backface)",
                rays,
            ));
        }
    }

    /// Where volume `id`'s regions are (irradiance, distance: x, y, width, height).
    pub(crate) fn regions(&self, id: u64) -> Option<([u32; 4], [u32; 4])> {
        let volume = &self.volumes[self.volume_index(id)?];
        Some((volume.irradiance_region(), volume.distance_region()))
    }
}

/// What a `.probes` file is for: the probes' layout and placement, and how it was baked.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct BakeHeader {
    pub counts: [u32; 3],
    pub origin: [f32; 3],
    pub spacing: f32,
    /// The grid's orientation (x, y, z, w).
    pub rotation: [f32; 4],
    pub irradiance_tile: u32,
    pub distance_tile: u32,
    pub irradiance_size: [u32; 2],
    pub distance_size: [u32; 2],
    pub sweeps: u32,
    pub rays: u32,
}

impl BakeHeader {
    fn new(grid: &LightProbeGrid, sweeps: u32, rays: u32) -> Self {
        let irradiance = grid.irradiance_atlas_size();
        let distance = grid.atlas_size(DISTANCE_TILE);
        Self {
            counts: grid.counts,
            origin: grid.origin.to_array(),
            spacing: grid.spacing,
            rotation: grid.rotation.to_array(),
            irradiance_tile: IRRADIANCE_TILE,
            distance_tile: DISTANCE_TILE,
            irradiance_size: [irradiance.0, irradiance.1],
            distance_size: [distance.0, distance.1],
            sweeps,
            rays,
        }
    }

    /// Whether data baked under `self` fits probes laid out as `other` (bake quality
    /// aside).
    fn matches(&self, other: &Self) -> bool {
        self.counts == other.counts
            && self.irradiance_tile == other.irradiance_tile
            && self.distance_tile == other.distance_tile
            && self.irradiance_size == other.irradiance_size
            && self.distance_size == other.distance_size
            && Vec3::from(self.origin).distance(Vec3::from(other.origin)) <= 1e-3
            && (self.spacing - other.spacing).abs() <= 1e-4
            && Quat::from_array(self.rotation).angle_between(Quat::from_array(other.rotation))
                <= 1e-3
    }

    fn irradiance_bytes(&self) -> usize {
        self.irradiance_size[0] as usize * self.irradiance_size[1] as usize * 8
    }

    fn distance_bytes(&self) -> usize {
        self.distance_size[0] as usize * self.distance_size[1] as usize * 4
    }
}

const BAKE_MAGIC: &[u8; 8] = b"WGCPROBE";
/// 3: the grid's rotation follows the header's other words. 2: the irradiance atlas has
/// the probes' offsets below its tiles. 1: tiles only, no relocation. 1 and 2 are for
/// unrotated grids.
const BAKE_VERSION: u32 = 3;
/// Header words before version 3's rotation.
const BAKE_HEADER_WORDS_V2: usize = 17;
const BAKE_HEADER_WORDS: usize = BAKE_HEADER_WORDS_V2 + 4;

fn bake_header_bytes(version: u32) -> usize {
    8 + 4 * if version >= 3 {
        BAKE_HEADER_WORDS
    } else {
        BAKE_HEADER_WORDS_V2
    }
}

/// A decoded `.probes` file: the region's atlases as the textures hold them (half
/// floats; RGBA irradiance, RG distances).
pub(crate) struct DecodedBake {
    pub version: u32,
    pub header: BakeHeader,
    pub irradiance: Vec<u8>,
    pub distance: Vec<u8>,
}

/// A `.probes` file from a volume's read-back regions: a little-endian header, then the
/// irradiance region (RGBA, half floats) and the distance region (RG, half floats), row
/// by row.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn encode_bake(
    header: &BakeHeader,
    irradiance: &crate::renderer::readback::Readback,
    distance: &crate::renderer::readback::Readback,
) -> Result<Vec<u8>, String> {
    let check = |readback: &crate::renderer::readback::Readback, size: [u32; 2], channels| {
        if readback.width != size[0] || readback.height != size[1] || readback.channels != channels
        {
            Err(format!(
                "read back {}x{}x{}, expected {}x{}x{channels}",
                readback.width, readback.height, readback.channels, size[0], size[1]
            ))
        } else {
            Ok(())
        }
    };
    check(irradiance, header.irradiance_size, 4)?;
    check(distance, header.distance_size, 2)?;
    let mut out = Vec::with_capacity(
        bake_header_bytes(BAKE_VERSION) + header.irradiance_bytes() + header.distance_bytes(),
    );
    out.extend_from_slice(BAKE_MAGIC);
    let words: [u32; BAKE_HEADER_WORDS] = [
        BAKE_VERSION,
        header.counts[0],
        header.counts[1],
        header.counts[2],
        header.origin[0].to_bits(),
        header.origin[1].to_bits(),
        header.origin[2].to_bits(),
        header.spacing.to_bits(),
        header.irradiance_tile,
        header.distance_tile,
        header.irradiance_size[0],
        header.irradiance_size[1],
        header.distance_size[0],
        header.distance_size[1],
        header.sweeps,
        header.rays,
        0,
        header.rotation[0].to_bits(),
        header.rotation[1].to_bits(),
        header.rotation[2].to_bits(),
        header.rotation[3].to_bits(),
    ];
    for word in words {
        out.extend_from_slice(&word.to_le_bytes());
    }
    for value in irradiance.values.iter().chain(&distance.values) {
        out.extend_from_slice(&half::f16::from_f64(*value).to_le_bytes());
    }
    Ok(out)
}

pub(crate) fn decode_bake(bytes: &[u8]) -> Result<DecodedBake, String> {
    if bytes.len() < bake_header_bytes(1) || &bytes[..8] != BAKE_MAGIC {
        return Err("not a light probe bake (.probes) file".into());
    }
    let word = |index: usize| {
        let at = 8 + index * 4;
        u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
    };
    let version = word(0);
    if !(1..=BAKE_VERSION).contains(&version) {
        return Err(format!(
            "light probe bake version {version} (this engine reads 1 to {BAKE_VERSION})"
        ));
    }
    let header_bytes = bake_header_bytes(version);
    if bytes.len() < header_bytes {
        return Err("light probe bake is shorter than its header".into());
    }
    let rotation = if version >= 3 {
        let at = BAKE_HEADER_WORDS_V2;
        [word(at), word(at + 1), word(at + 2), word(at + 3)].map(f32::from_bits)
    } else {
        Quat::IDENTITY.to_array()
    };
    let header = BakeHeader {
        counts: [word(1), word(2), word(3)],
        origin: [
            f32::from_bits(word(4)),
            f32::from_bits(word(5)),
            f32::from_bits(word(6)),
        ],
        spacing: f32::from_bits(word(7)),
        rotation,
        irradiance_tile: word(8),
        distance_tile: word(9),
        irradiance_size: [word(10), word(11)],
        distance_size: [word(12), word(13)],
        sweeps: word(14),
        rays: word(15),
    };
    let irradiance_end = header_bytes + header.irradiance_bytes();
    let end = irradiance_end + header.distance_bytes();
    if bytes.len() != end {
        return Err(format!(
            "light probe bake is {} bytes, its header says {end}",
            bytes.len()
        ));
    }
    Ok(DecodedBake {
        version,
        header,
        irradiance: bytes[header_bytes..irradiance_end].to_vec(),
        distance: bytes[irradiance_end..end].to_vec(),
    })
}

/// A uniformly random rotation, so successive updates cover the sphere between the fixed
/// ray directions.
fn random_rotation(rng: &mut u64) -> Quat {
    let mut next = || {
        // xorshift64*
        *rng ^= *rng >> 12;
        *rng ^= *rng << 25;
        *rng ^= *rng >> 27;
        (rng.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 40) as f32 / (1u64 << 24) as f32
    };
    // Shoemake's uniform random quaternion.
    let (u1, u2, u3) = (next(), next(), next());
    let tau = std::f32::consts::TAU;
    let (a, b) = ((1.0 - u1).sqrt(), u1.sqrt());
    Quat::from_xyzw(
        a * (tau * u2).sin(),
        a * (tau * u2).cos(),
        b * (tau * u3).sin(),
        b * (tau * u3).cos(),
    )
    .normalize()
}

fn fullscreen_pass<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    label: &'static str,
    view: &'a wgpu::TextureView,
) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            resolve_target: None,
            depth_slice: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn validate(source: &str, capabilities: naga::valid::Capabilities) {
        let module = naga::front::wgsl::parse_str(source)
            .unwrap_or_else(|error| panic!("{}", error.emit_to_string(source)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), capabilities)
            .validate(&module)
            .unwrap_or_else(|error| panic!("{}", error.emit_to_string(source)));
    }

    fn grid(counts: [u32; 3]) -> LightProbeGrid {
        LightProbeGrid::centered(Vec3::ZERO, Quat::IDENTITY, 0.5, counts, 128, 0.97, 1.0)
    }

    fn readback(size: [u32; 2], channels: usize) -> crate::renderer::readback::Readback {
        crate::renderer::readback::Readback {
            width: size[0],
            height: size[1],
            channels,
            values: (0..size[0] as usize * size[1] as usize * channels)
                .map(|i| (i % 7) as f64 * 0.25)
                .collect(),
            integer: false,
        }
    }

    #[test]
    fn a_volume_that_stops_tracing_shows_its_bake_again() {
        let mut volume = Volume::new(grid([2, 2, 2]));
        volume.baked_source = Some("content/lighting/hall.probes".into());
        // Live without tracing yet: the loaded bake is still what it holds.
        volume.set_grid(LightProbeGrid {
            live: false,
            ..grid([2, 2, 2])
        });
        assert!(volume.baked_source.is_some());
        volume.set_grid(grid([2, 2, 2]));
        volume.traced_since_load = true;
        volume.set_grid(LightProbeGrid {
            live: false,
            ..grid([2, 2, 2])
        });
        assert_eq!(volume.baked_source, None, "the file is loaded again");
    }

    #[test]
    fn update_shader_validates() {
        for textures in [false, true] {
            validate(
                &light_probe_update_shader_source(textures),
                naga::valid::Capabilities::RAY_QUERY
                    | naga::valid::Capabilities::TEXTURE_AND_SAMPLER_BINDING_ARRAY
                    | naga::valid::Capabilities::TEXTURE_AND_SAMPLER_BINDING_ARRAY_NON_UNIFORM_INDEXING,
            );
        }
    }

    #[test]
    fn bakes_round_trip_and_check_their_grid() {
        let grid = grid([3, 2, 2]);
        let header = BakeHeader::new(&grid, 256, 256);
        let irradiance = readback(header.irradiance_size, 4);
        let distance = readback(header.distance_size, 2);
        let bytes = encode_bake(&header, &irradiance, &distance).unwrap();
        let decoded = decode_bake(&bytes).unwrap();
        assert_eq!(decoded.version, BAKE_VERSION);
        assert_eq!(decoded.header, header);
        assert_eq!(decoded.irradiance.len(), header.irradiance_bytes());
        let first = half::f16::from_le_bytes([decoded.irradiance[2], decoded.irradiance[3]]);
        assert_eq!(first.to_f64(), 0.25);

        let mut moved = grid;
        moved.origin.x += 0.5;
        assert!(!header.matches(&BakeHeader::new(&moved, 1, 1)));
        let mut turned = grid;
        turned.rotation = Quat::from_rotation_y(0.3);
        assert!(!header.matches(&BakeHeader::new(&turned, 1, 1)));
        assert!(header.matches(&BakeHeader::new(&grid, 1, 32)));
        assert!(decode_bake(&bytes[..bytes.len() - 1]).is_err());
        assert!(decode_bake(b"nonsense").is_err());
    }

    #[test]
    fn uniforms_are_vec4_aligned() {
        assert_eq!(std::mem::size_of::<VolumeUniform>(), 112);
        assert_eq!(
            std::mem::size_of::<VolumesUniform>(),
            16 + 112 * MAX_LIGHT_PROBE_VOLUMES
        );
        assert_eq!(std::mem::size_of::<UpdateUniform>(), 112);
    }

    #[test]
    fn grid_is_centred_and_atlases_fit_every_probe() {
        let grid = LightProbeGrid::centered(
            Vec3::new(0.0, 1.5, 0.0),
            Quat::IDENTITY,
            0.5,
            [11, 7, 11],
            128,
            0.97,
            1.0,
        );
        assert_eq!(grid.origin, Vec3::new(-2.5, 0.0, -2.5));
        let (width, height) = grid.atlas_size(IRRADIANCE_TILE);
        let columns = width / IRRADIANCE_TILE;
        assert!(columns * (height / IRRADIANCE_TILE) >= grid.probe_count());
        assert_eq!(grid.atlas_size(DISTANCE_TILE).0, columns * DISTANCE_TILE);
        // A texel per probe for its offset, in rows below the tiles.
        let (offsets_width, full_height) = grid.irradiance_atlas_size();
        assert_eq!(offsets_width, width);
        assert!((full_height - height) * width >= grid.probe_count());
        assert_eq!(full_height - height, 4);
    }

    #[test]
    fn rotated_grids_are_centred_on_their_box() {
        let rotation = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let grid = LightProbeGrid::centered(
            Vec3::new(1.0, 0.0, 0.0),
            rotation,
            1.0,
            [3, 1, 1],
            128,
            0.97,
            1.0,
        );
        // Turned a quarter about y, the grid's x axis runs along -z.
        assert!(grid.origin.distance(Vec3::new(1.0, 0.0, 1.0)) < 1e-5);
        assert!(
            grid.to_world(grid.extent())
                .distance(Vec3::new(1.0, 0.0, -1.0))
                < 1e-5
        );
    }

    #[test]
    fn older_bakes_still_decode_as_unrotated() {
        let grid = grid([3, 2, 2]);
        let header = BakeHeader::new(&grid, 8, 64);
        let bytes = encode_bake(
            &header,
            &readback(header.irradiance_size, 4),
            &readback(header.distance_size, 2),
        )
        .unwrap();
        // Version 2: the same without the rotation words.
        let v3_header = bake_header_bytes(3);
        let mut v2 = bytes[..bake_header_bytes(2)].to_vec();
        v2.extend_from_slice(&bytes[v3_header..]);
        v2[8..12].copy_from_slice(&2u32.to_le_bytes());
        let decoded = decode_bake(&v2).unwrap();
        assert_eq!(decoded.version, 2);
        assert_eq!(decoded.header, header);

        // Version 1: tiles only.
        let (width, height) = grid.atlas_size(IRRADIANCE_TILE);
        let mut tiles_only = header;
        tiles_only.irradiance_size = [width, height];
        let bytes = encode_bake(
            &tiles_only,
            &readback(tiles_only.irradiance_size, 4),
            &readback(tiles_only.distance_size, 2),
        )
        .unwrap();
        let mut v1 = bytes[..bake_header_bytes(1)].to_vec();
        v1.extend_from_slice(&bytes[v3_header..]);
        v1[8..12].copy_from_slice(&1u32.to_le_bytes());
        let decoded = decode_bake(&v1).unwrap();
        assert_eq!(decoded.version, 1);
        assert_eq!(decoded.header.irradiance_size, [width, height]);
        v1[8..12].copy_from_slice(&(BAKE_VERSION + 1).to_le_bytes());
        assert!(decode_bake(&v1).is_err());
    }
}
