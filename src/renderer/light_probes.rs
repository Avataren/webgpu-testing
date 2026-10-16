//! A grid of light probes for indirect diffuse light (DDGI: Majercik et al., "Dynamic
//! Diffuse Global Illumination with Ray-Traced Irradiance Fields", JCGT 2019).
//!
//! Every probe keeps an octahedral tile of irradiance (the light arriving from each
//! direction, cosine weighted) and one of distances (the mean and mean square distance it
//! sees in each direction) in two atlases, which the main lights group binds (12-14, read
//! by `light_probes.wgsl`). Surfaces take the light of the eight probes around them,
//! weighted by whether those probes can see them, so light does not leak through walls.
//!
//! With hardware ray tracing the grid updates itself (`light_probe_update.wgsl`): a slice
//! of the probes traces its rays each frame (direct light at the hits plus the grid's own
//! light there, so bounces add up over frames), then the rays are blended into the tiles.
//!
//! A bake ([`LightProbes::start_bake`]) traces the grid at high quality instead: more rays,
//! warm-up sweeps for the bounces to build up, then the exact mean of many sweeps. Its
//! atlases are saved as a `.probes` file ([`encode_bake`]); where the grid cannot trace
//! (the web, GPUs without ray tracing), loading that file is the only light it gets.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Quat, Vec3};

use crate::renderer::shader_source::wgsl;
use crate::renderer::{PipelineBuilder, ShaderBuilder};

/// Irradiance tile side in texels: 6x6 interior plus a one-texel border.
pub(crate) const IRRADIANCE_TILE: u32 = 8;
/// Distance tile side in texels: 14x14 interior plus a one-texel border.
pub(crate) const DISTANCE_TILE: u32 = 16;
const IRRADIANCE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const DISTANCE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Float;
const RAY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Most probes one update traces (rows of the ray texture).
const MAX_PROBES_PER_UPDATE: u32 = 2048;
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

/// The grid a scene asks for (from a `LightProbeVolume`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightProbeGrid {
    /// The first probe's position (the box's minimum corner).
    pub origin: Vec3,
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
}

impl LightProbeGrid {
    /// A grid filling a box of `size` around `center`.
    pub fn centered(
        center: Vec3,
        spacing: f32,
        counts: [u32; 3],
        rays_per_probe: u32,
        hysteresis: f32,
        intensity: f32,
    ) -> Self {
        let extent =
            (Vec3::new(counts[0] as f32, counts[1] as f32, counts[2] as f32) - Vec3::ONE) * spacing;
        Self {
            origin: center - extent * 0.5,
            spacing,
            counts,
            rays_per_probe,
            hysteresis,
            intensity,
            live: true,
        }
    }

    pub fn probe_count(&self) -> u32 {
        self.counts[0] * self.counts[1] * self.counts[2]
    }

    fn rays(&self) -> u32 {
        self.rays_per_probe.clamp(MIN_RAYS, MAX_RAYS)
    }

    /// Probe tiles per atlas row: about square atlases.
    fn columns(&self) -> u32 {
        (self.probe_count() as f32).sqrt().ceil().max(1.0) as u32
    }

    fn atlas_size(&self, tile: u32) -> (u32, u32) {
        let columns = self.columns();
        let rows = self.probe_count().div_ceil(columns);
        (columns * tile, rows * tile)
    }

    /// Whether `other` needs new atlases (a different layout of probes).
    fn same_layout(&self, other: &Self) -> bool {
        self.counts == other.counts
    }

    /// Whether `other` puts its probes in the same places.
    fn same_placement(&self, other: &Self) -> bool {
        self.same_layout(other)
            && self.origin.distance(other.origin) <= 1e-3
            && (self.spacing - other.spacing).abs() <= 1e-4
    }
}

/// Mirrors `LightProbeGrid` in light_probes.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable, PartialEq)]
struct GridUniform {
    origin_spacing: [f32; 4],
    counts: [u32; 4],
    atlas: [u32; 4],
    params: [f32; 4],
}

/// Mirrors `LightProbeUpdate` in light_probe_update.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
struct UpdateUniform {
    rotation: [[f32; 4]; 4],
    frame: [u32; 4],
    params: [f32; 4],
}

const FLAG_RESET: u32 = 1;

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

    /// Make the next frame's tiles the current ones.
    fn swap(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.copy_texture_to_texture(
            self.next.as_image_copy(),
            self.current.as_image_copy(),
            self.current.size(),
        );
    }
}

/// The probe grid's resources, and its update when ray tracing is available.
pub(crate) struct LightProbes {
    grid: Option<LightProbeGrid>,
    grid_buffer: wgpu::Buffer,
    last_uniform: Option<GridUniform>,
    irradiance: Atlas,
    distance: Atlas,
    update: Option<UpdateStage>,
    /// The ray texture, sized for the grid's rays and probes per update.
    rays: Option<(wgpu::Texture, wgpu::TextureView)>,
    blend_inputs: Option<wgpu::BindGroup>,
    /// The first probe the next update traces (a slice per frame, round robin).
    next_probe: u32,
    /// The atlases hold nothing yet: the next update of each probe replaces its value.
    reset: bool,
    /// Probes whose tiles hold no update yet (they count as inside geometry until then).
    probes_untouched: u32,
    rng: u64,
    /// The atlases were recreated: the lights group must bind the new views.
    bindings_changed: bool,
    /// The atlases hold light (traced or loaded): surfaces may read them.
    has_data: bool,
    /// The baked file the atlases were loaded from.
    baked_source: Option<String>,
    bake: Option<Bake>,
}

/// A bake in progress.
#[derive(Clone, Copy, Debug)]
struct Bake {
    warmup: u32,
    sweeps: u32,
    rays: u32,
    /// Sweeps over every probe completed, warm-up included.
    done: u32,
    /// Probes of the current sweep already traced.
    into_sweep: u32,
}

impl Bake {
    fn finished(&self) -> bool {
        self.done >= self.warmup + self.sweeps
    }
}

/// How far a bake has come.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LightProbeBakeProgress {
    /// Warm-up sweeps, for the bounced light to build up before it is averaged.
    pub warmup_sweeps: u32,
    /// Sweeps averaged after the warm-up.
    pub sweeps: u32,
    pub rays_per_probe: u32,
    /// Sweeps completed, warm-up included.
    pub sweeps_done: u32,
    pub finished: bool,
}

struct UpdateStage {
    uniform: wgpu::Buffer,
    trace_inputs: wgpu::BindGroup,
    blend_layout: wgpu::BindGroupLayout,
    trace: wgpu::RenderPipeline,
    irradiance: wgpu::RenderPipeline,
    distance: wgpu::RenderPipeline,
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

impl LightProbes {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let grid_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("LightProbeGrid"),
            size: std::mem::size_of::<GridUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            grid: None,
            grid_buffer,
            last_uniform: None,
            irradiance: Atlas::new(device, "LightProbeIrradiance", IRRADIANCE_FORMAT, (1, 1)),
            distance: Atlas::new(device, "LightProbeDistance", DISTANCE_FORMAT, (1, 1)),
            update: None,
            rays: None,
            blend_inputs: None,
            next_probe: 0,
            reset: true,
            probes_untouched: 0,
            rng: 0x9e37_79b9_7f4a_7c15,
            bindings_changed: false,
            has_data: false,
            baked_source: None,
            bake: None,
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

    /// Build the ray-traced update. Without it the grid gives no light.
    pub(crate) fn enable_ray_tracing(
        &mut self,
        device: &wgpu::Device,
        lights_layout: &wgpu::BindGroupLayout,
        scene_layout: &wgpu::BindGroupLayout,
        textures_layout: Option<&wgpu::BindGroupLayout>,
    ) -> Result<(), String> {
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let uniform_entry = wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
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
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("LightProbeUpdate"),
            size: std::mem::size_of::<UpdateUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let trace_inputs = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("LightProbeTraceInputs"),
            layout: &trace_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        if let Some(error) = pollster::block_on(validation.pop()) {
            return Err(error.to_string());
        }
        self.update = Some(UpdateStage {
            uniform,
            trace_inputs,
            blend_layout,
            trace,
            irradiance,
            distance,
        });
        self.blend_inputs = None;
        Ok(())
    }

    /// Whether a grid is set and traces this frame (it needs the acceleration
    /// structures): live, or baking.
    pub(crate) fn wants_update(&self) -> bool {
        let Some(grid) = self.grid else {
            return false;
        };
        if self.update.is_none() {
            return false;
        }
        match self.bake {
            Some(bake) => !bake.finished(),
            None => grid.live,
        }
    }

    /// Whether the grid can trace at all (hardware ray tracing).
    pub(crate) fn can_trace(&self) -> bool {
        self.update.is_some()
    }

    /// The baked file the atlases hold, if they were loaded from one.
    pub(crate) fn baked_source(&self) -> Option<&str> {
        self.baked_source.as_deref()
    }

    /// Trace the grid at bake quality: `warmup` sweeps over every probe for the bounces
    /// to build up, then the mean of `sweeps` more, each with `rays` rays per probe.
    /// Returns the frames it takes (one update per frame).
    pub(crate) fn start_bake(
        &mut self,
        warmup: u32,
        sweeps: u32,
        rays: u32,
    ) -> Result<u32, String> {
        let Some(grid) = self.grid else {
            return Err("the scene has no enabled light probe volume".into());
        };
        if self.update.is_none() {
            return Err("baking traces the probes and needs hardware ray tracing".into());
        }
        if self.bake.is_some_and(|bake| !bake.finished()) {
            return Err("a bake is already running".into());
        }
        let bake = Bake {
            warmup: warmup.max(1),
            sweeps: sweeps.max(1),
            rays: rays.clamp(MIN_RAYS, MAX_RAYS),
            done: 0,
            into_sweep: 0,
        };
        self.bake = Some(bake);
        self.next_probe = 0;
        let updates_per_sweep = grid.probe_count().div_ceil(MAX_PROBES_PER_UPDATE);
        Ok((bake.warmup + bake.sweeps) * updates_per_sweep)
    }

    pub(crate) fn bake_progress(&self) -> Option<LightProbeBakeProgress> {
        self.bake.map(|bake| LightProbeBakeProgress {
            warmup_sweeps: bake.warmup,
            sweeps: bake.sweeps,
            rays_per_probe: bake.rays,
            sweeps_done: bake.done,
            finished: bake.finished(),
        })
    }

    /// The finished bake's settings, and the atlas textures to read back and save.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn finished_bake(&self) -> Option<(BakeHeader, &wgpu::Texture, &wgpu::Texture)> {
        let bake = self.bake.filter(Bake::finished)?;
        let grid = self.grid?;
        Some((
            BakeHeader::new(&grid, bake.sweeps, bake.rays),
            &self.irradiance.current,
            &self.distance.current,
        ))
    }

    /// Done with a finished (or failed) bake: the grid goes back to updating live, or
    /// keeps the baked light.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn end_bake(&mut self) {
        self.bake = None;
    }

    /// Show the baked atlases in `bytes` (a `.probes` file from [`encode_bake`]), read
    /// from `source`. They must be for this grid's probes.
    pub(crate) fn load_bake(
        &mut self,
        queue: &wgpu::Queue,
        bytes: &[u8],
        source: &str,
    ) -> Result<(), String> {
        let Some(grid) = self.grid else {
            return Err("no light probe grid to load into".into());
        };
        let bake = decode_bake(bytes)?;
        let expected = BakeHeader::new(&grid, bake.header.sweeps, bake.header.rays);
        if !bake.header.matches(&expected) {
            return Err(format!(
                "{source} was baked for other probes ({:?} from {:?}, spacing {}; the volume now \
                 has {:?} from {:?}, spacing {}): bake again",
                bake.header.counts,
                bake.header.origin,
                bake.header.spacing,
                expected.counts,
                expected.origin,
                expected.spacing
            ));
        }
        for (texture, data, texel_bytes) in [
            (&self.irradiance.current, &bake.irradiance, 8u32),
            (&self.distance.current, &bake.distance, 4u32),
        ] {
            let size = texture.size();
            queue.write_texture(
                texture.as_image_copy(),
                data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(size.width * texel_bytes),
                    rows_per_image: Some(size.height),
                },
                size,
            );
        }
        self.has_data = true;
        self.reset = false;
        self.probes_untouched = 0;
        self.baked_source = Some(source.to_owned());
        self.last_uniform = None;
        Ok(())
    }

    /// Use `grid` from now on (`None`: no probes, surfaces take the environment's light).
    pub(crate) fn set_grid(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        grid: Option<LightProbeGrid>,
    ) {
        match (&self.grid, &grid) {
            (Some(old), Some(new)) if old.same_layout(new) => {
                if !old.same_placement(new) {
                    // Moved probes see something else: start over.
                    self.reset = true;
                    self.probes_untouched = new.probe_count();
                    self.has_data = false;
                    self.baked_source = None;
                    self.bake = None;
                }
            }
            (None, None) => {}
            (_, Some(new)) => {
                self.irradiance = Atlas::new(
                    device,
                    "LightProbeIrradiance",
                    IRRADIANCE_FORMAT,
                    new.atlas_size(IRRADIANCE_TILE),
                );
                self.distance = Atlas::new(
                    device,
                    "LightProbeDistance",
                    DISTANCE_FORMAT,
                    new.atlas_size(DISTANCE_TILE),
                );
                let rows = new.probe_count().min(MAX_PROBES_PER_UPDATE);
                let rays = device.create_texture(&wgpu::TextureDescriptor {
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
                let view = rays.create_view(&Default::default());
                self.rays = Some((rays, view));
                self.blend_inputs = None;
                self.reset = true;
                self.probes_untouched = new.probe_count();
                self.next_probe = 0;
                self.bindings_changed = true;
                self.has_data = false;
                self.baked_source = None;
                self.bake = None;
            }
            (Some(_), None) => {
                self.irradiance =
                    Atlas::new(device, "LightProbeIrradiance", IRRADIANCE_FORMAT, (1, 1));
                self.distance = Atlas::new(device, "LightProbeDistance", DISTANCE_FORMAT, (1, 1));
                self.rays = None;
                self.blend_inputs = None;
                self.bindings_changed = true;
                self.has_data = false;
                self.baked_source = None;
                self.bake = None;
            }
        }
        self.grid = grid;
        let uniform = self.grid_uniform();
        if self.last_uniform != Some(uniform) {
            queue.write_buffer(&self.grid_buffer, 0, bytemuck::bytes_of(&uniform));
            self.last_uniform = Some(uniform);
        }
    }

    fn grid_uniform(&self) -> GridUniform {
        let Some(grid) = self.grid else {
            return GridUniform::default();
        };
        // Nothing to give until traced or loaded.
        let lit = self.has_data;
        GridUniform {
            origin_spacing: [grid.origin.x, grid.origin.y, grid.origin.z, grid.spacing],
            counts: [grid.counts[0], grid.counts[1], grid.counts[2], lit as u32],
            atlas: [grid.columns(), IRRADIANCE_TILE, DISTANCE_TILE, grid.rays()],
            params: [grid.intensity.max(0.0), NORMAL_BIAS, VIEW_BIAS, 0.0],
        }
    }

    /// Trace this frame's slice of probes and blend it in. `lights` is the main lights
    /// group (which binds this grid's current atlases), `scene` and `textures` the ray
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
        let (Some(grid), Some(stage), Some((_, rays_view))) =
            (self.grid, self.update.as_ref(), self.rays.as_ref())
        else {
            return;
        };
        let total = grid.probe_count();
        let first = self.next_probe % total;
        let mut flags = 0;
        let (probes, rays, hysteresis) = match self.bake.as_mut() {
            Some(bake) => {
                if bake.finished() {
                    return;
                }
                // Sweeps start at the first probe, so every probe of a sweep is traced
                // as often as the others.
                let probes = (total - bake.into_sweep).min(MAX_PROBES_PER_UPDATE);
                let hysteresis = if bake.done < bake.warmup {
                    if bake.done == 0 {
                        flags |= FLAG_RESET;
                    }
                    // Fast to follow, as bounces build on the last sweep's light.
                    0.5
                } else {
                    // The exact mean of the averaged sweeps (the first one replaces).
                    let averaged = (bake.done - bake.warmup) as f32;
                    averaged / (averaged + 1.0)
                };
                bake.into_sweep += probes;
                if bake.into_sweep >= total {
                    bake.into_sweep = 0;
                    bake.done += 1;
                }
                (probes, bake.rays, hysteresis)
            }
            None => {
                if self.reset {
                    flags |= FLAG_RESET;
                    // Every probe is replaced once; then they blend.
                    let probes = total.min(MAX_PROBES_PER_UPDATE);
                    self.probes_untouched = self.probes_untouched.saturating_sub(probes);
                    if self.probes_untouched == 0 {
                        self.reset = false;
                    }
                }
                (
                    total.min(MAX_PROBES_PER_UPDATE),
                    grid.rays(),
                    grid.hysteresis.clamp(0.0, 0.995),
                )
            }
        };
        self.next_probe = (first + probes) % total;
        if !self.has_data {
            self.has_data = true;
            self.last_uniform = None;
        }
        let uniform = UpdateUniform {
            rotation: Mat4::from_quat(random_rotation(&mut self.rng)).to_cols_array_2d(),
            frame: [first, probes, flags, rays],
            params: [
                hysteresis,
                grid.spacing * MAX_DISTANCE_SPACINGS,
                SKY_BLUR,
                BACKFACE_SHARE,
            ],
        };
        queue.write_buffer(&stage.uniform, 0, bytemuck::bytes_of(&uniform));
        let blend_inputs = self.blend_inputs.get_or_insert_with(|| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("LightProbeBlendInputs"),
                layout: &stage.blend_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: stage.uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(rays_view),
                    },
                ],
            })
        });

        {
            let mut pass = fullscreen_pass(encoder, "LightProbeTrace", rays_view);
            pass.set_viewport(0.0, 0.0, rays as f32, probes as f32, 0.0, 1.0);
            pass.set_scissor_rect(0, 0, rays, probes);
            pass.set_pipeline(&stage.trace);
            pass.set_bind_group(0, &stage.trace_inputs, &[]);
            pass.set_bind_group(2, lights, &[]);
            pass.set_bind_group(3, scene, &[]);
            if let Some(textures) = textures {
                pass.set_bind_group(4, textures, &[]);
            }
            pass.draw(0..3, 0..1);
        }
        for (label, pipeline, view) in [
            (
                "LightProbeIrradiance",
                &stage.irradiance,
                &self.irradiance.next_view,
            ),
            (
                "LightProbeDistance",
                &stage.distance,
                &self.distance.next_view,
            ),
        ] {
            let mut pass = fullscreen_pass(encoder, label, view);
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &*blend_inputs, &[]);
            pass.set_bind_group(2, lights, &[]);
            pass.draw(0..3, 0..1);
        }
        self.irradiance.swap(encoder);
        self.distance.swap(encoder);
    }

    /// The textures `texture_fetch` can read.
    pub(crate) fn fetchable_textures<'a>(
        &'a self,
        out: &mut Vec<crate::renderer::readback::FetchableTexture<'a>>,
    ) {
        use crate::renderer::readback::FetchableTexture;
        if self.grid.is_none() {
            return;
        }
        out.push(FetchableTexture::new(
            "probes.irradiance",
            "Light probe irradiance: an octahedral tile per probe (8x8 with the border, probes \
             in rows of about the square root of their count) of the light arriving from each \
             direction, cosine weighted (rgb); a: 1 for a probe in the open, 0 inside geometry",
            &self.irradiance.current,
        ));
        out.push(FetchableTexture::new(
            "probes.distance",
            "Light probe distances: an octahedral tile per probe (16x16 with the border) of \
             the mean distance (r) and mean squared distance (g) the probe sees in each \
             direction (m)",
            &self.distance.current,
        ));
        if let Some((rays, _)) = &self.rays {
            out.push(FetchableTexture::new(
                "probes.rays",
                "Light probe rays of the last update: one texel per ray, one row per probe \
                 traced; rgb the light along the ray, a the distance to its hit (negative on a \
                 backface)",
                rays,
            ));
        }
    }
}

/// What a `.probes` file is for: the probes' layout and placement, and how it was baked.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct BakeHeader {
    pub counts: [u32; 3],
    pub origin: [f32; 3],
    pub spacing: f32,
    pub irradiance_tile: u32,
    pub distance_tile: u32,
    pub irradiance_size: [u32; 2],
    pub distance_size: [u32; 2],
    pub sweeps: u32,
    pub rays: u32,
}

impl BakeHeader {
    fn new(grid: &LightProbeGrid, sweeps: u32, rays: u32) -> Self {
        let irradiance = grid.atlas_size(IRRADIANCE_TILE);
        let distance = grid.atlas_size(DISTANCE_TILE);
        Self {
            counts: grid.counts,
            origin: grid.origin.to_array(),
            spacing: grid.spacing,
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
    }

    fn irradiance_bytes(&self) -> usize {
        self.irradiance_size[0] as usize * self.irradiance_size[1] as usize * 8
    }

    fn distance_bytes(&self) -> usize {
        self.distance_size[0] as usize * self.distance_size[1] as usize * 4
    }
}

const BAKE_MAGIC: &[u8; 8] = b"WGCPROBE";
const BAKE_VERSION: u32 = 1;
const BAKE_HEADER_BYTES: usize = 8 + 4 * 17;

/// A decoded `.probes` file: the atlases as the textures hold them (half floats; RGBA
/// irradiance, RG distances).
pub(crate) struct DecodedBake {
    pub header: BakeHeader,
    pub irradiance: Vec<u8>,
    pub distance: Vec<u8>,
}

/// A `.probes` file from read-back atlases: a little-endian header, then the irradiance
/// atlas (RGBA, half floats) and the distance atlas (RG, half floats), row by row.
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
    let mut out =
        Vec::with_capacity(BAKE_HEADER_BYTES + header.irradiance_bytes() + header.distance_bytes());
    out.extend_from_slice(BAKE_MAGIC);
    let words: [u32; 17] = [
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
    if bytes.len() < BAKE_HEADER_BYTES || &bytes[..8] != BAKE_MAGIC {
        return Err("not a light probe bake (.probes) file".into());
    }
    let word = |index: usize| {
        let at = 8 + index * 4;
        u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
    };
    if word(0) != BAKE_VERSION {
        return Err(format!(
            "light probe bake version {} (this engine reads {BAKE_VERSION})",
            word(0)
        ));
    }
    let header = BakeHeader {
        counts: [word(1), word(2), word(3)],
        origin: [
            f32::from_bits(word(4)),
            f32::from_bits(word(5)),
            f32::from_bits(word(6)),
        ],
        spacing: f32::from_bits(word(7)),
        irradiance_tile: word(8),
        distance_tile: word(9),
        irradiance_size: [word(10), word(11)],
        distance_size: [word(12), word(13)],
        sweeps: word(14),
        rays: word(15),
    };
    let irradiance_end = BAKE_HEADER_BYTES + header.irradiance_bytes();
    let end = irradiance_end + header.distance_bytes();
    if bytes.len() != end {
        return Err(format!(
            "light probe bake is {} bytes, its header says {end}",
            bytes.len()
        ));
    }
    Ok(DecodedBake {
        header,
        irradiance: bytes[BAKE_HEADER_BYTES..irradiance_end].to_vec(),
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
        use crate::renderer::readback::Readback;
        let grid = LightProbeGrid::centered(Vec3::ZERO, 0.5, [3, 2, 2], 128, 0.97, 1.0);
        let header = BakeHeader::new(&grid, 256, 256);
        let readback = |size: [u32; 2], channels: usize| Readback {
            width: size[0],
            height: size[1],
            channels,
            values: (0..size[0] as usize * size[1] as usize * channels)
                .map(|i| (i % 7) as f64 * 0.25)
                .collect(),
            integer: false,
        };
        let irradiance = readback(header.irradiance_size, 4);
        let distance = readback(header.distance_size, 2);
        let bytes = encode_bake(&header, &irradiance, &distance).unwrap();
        let decoded = decode_bake(&bytes).unwrap();
        assert_eq!(decoded.header, header);
        assert_eq!(decoded.irradiance.len(), header.irradiance_bytes());
        let first = half::f16::from_le_bytes([decoded.irradiance[2], decoded.irradiance[3]]);
        assert_eq!(first.to_f64(), 0.25);

        let mut moved = grid;
        moved.origin.x += 0.5;
        assert!(!header.matches(&BakeHeader::new(&moved, 1, 1)));
        assert!(header.matches(&BakeHeader::new(&grid, 1, 32)));
        assert!(decode_bake(&bytes[..bytes.len() - 1]).is_err());
        assert!(decode_bake(b"nonsense").is_err());
    }

    #[test]
    fn uniforms_are_vec4_aligned() {
        assert_eq!(std::mem::size_of::<GridUniform>(), 64);
        assert_eq!(std::mem::size_of::<UpdateUniform>(), 96);
    }

    #[test]
    fn grid_is_centred_and_atlases_fit_every_probe() {
        let grid =
            LightProbeGrid::centered(Vec3::new(0.0, 1.5, 0.0), 0.5, [11, 7, 11], 128, 0.97, 1.0);
        assert_eq!(grid.origin, Vec3::new(-2.5, 0.0, -2.5));
        let (width, height) = grid.atlas_size(IRRADIANCE_TILE);
        let columns = width / IRRADIANCE_TILE;
        assert!(columns * (height / IRRADIANCE_TILE) >= grid.probe_count());
        assert_eq!(grid.atlas_size(DISTANCE_TILE).0, columns * DISTANCE_TILE);
    }
}
