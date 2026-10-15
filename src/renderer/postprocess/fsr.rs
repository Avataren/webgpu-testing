//! AMD FidelityFX Super Resolution 3.1 upscaling, ported to WGSL
//! (`shader/postprocess/fsr3_upscaler.wgsl`, which lists what differs from the original).
//! It replaces the TAA pass while the scene renders below the output resolution: nine
//! compute passes from the jittered scene colour, depth and motion vectors to the
//! upscaled image in the output region of the TAA target, after a tenth that marks what
//! was blended over the opaque scene (particles, transparent surfaces) as reactive.

use crate::renderer::shader_source::wgsl;
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec2};

use crate::renderer::ShaderBuilder;

/// Mirrors `FsrConstants` in fsr3_upscaler.wgsl.
#[repr(C, align(16))]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FsrConstants {
    render_size: [i32; 2],
    previous_render_size: [i32; 2],
    upscale_size: [i32; 2],
    previous_upscale_size: [i32; 2],
    max_render_size: [i32; 2],
    input_offset: [i32; 2],
    output_offset: [i32; 2],
    luma_groups: [i32; 2],
    device_to_view_depth: [f32; 4],
    jitter: [f32; 2],
    previous_jitter: [f32; 2],
    downscale_factor: [f32; 2],
    jitter_sequence_length: f32,
    delta_time: f32,
    delta_pre_exposure: f32,
    frame_index: f32,
    velocity_factor: f32,
    reactiveness_scale: f32,
    shading_change_scale: f32,
    accumulation_added_per_frame: f32,
    min_disocclusion_accumulation: f32,
    rcas_sharpness: f32,
    sharpen: u32,
    _pad: [u32; 3],
}

/// Mirrors `ReactiveConstants` in fsr3_upscaler.wgsl.
#[repr(C, align(16))]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ReactiveConstants {
    origin: [i32; 2],
    render_size: [i32; 2],
    scale: f32,
    threshold: f32,
    binary_value: f32,
    flags: u32,
}

/// The autogen-reactive settings AMD's samples use: tonemapped colours, the largest
/// channel's change, and a binary mask (0.9 where it changes by 0.2 or more).
const AUTOREACTIVE_APPLY_TONEMAP: u32 = 1;
const AUTOREACTIVE_APPLY_THRESHOLD: u32 = 4;
const AUTOREACTIVE_USE_COMPONENTS_MAX: u32 = 8;
const AUTOREACTIVE_SCALE: f32 = 1.0;
const AUTOREACTIVE_THRESHOLD: f32 = 0.2;
const AUTOREACTIVE_BINARY_VALUE: f32 = 0.9;

const LUMA_GROUP_SIZE: u32 = 16;
const GROUP_SIZE: u32 = 8;

/// What one frame of upscaling reads and writes, and how it was rendered.
pub(super) struct FsrFrame<'a> {
    /// The jittered HDR scene, the depth it was drawn with and the normal G-buffer (motion
    /// vectors in zw); the render region starts at `origin` in each.
    pub color: &'a wgpu::TextureView,
    pub depth: &'a wgpu::TextureView,
    pub motion: &'a wgpu::TextureView,
    /// Receives the upscaled image in the output region, also starting at `origin`.
    pub output: &'a wgpu::TextureView,
    pub sampler: &'a wgpu::Sampler,
    pub origin: [i32; 2],
    pub render_size: [i32; 2],
    pub upscale_size: [i32; 2],
    /// The (jittered) projection: its depth terms recover view depth.
    pub projection: Mat4,
    /// The jitter in render pixels (+y down).
    pub jitter: Vec2,
    /// Seconds since the last frame.
    pub delta_time: f32,
    /// Start accumulating afresh (camera cut, resize, first frame).
    pub reset: bool,
    /// RCAS sharpening, 0 (off) to 1.
    pub sharpness: f32,
}

/// The scene before and after what is blended over the opaque surfaces, for the reactive
/// mask; the render region starts at `origin` in both.
pub(super) struct ReactiveFrame<'a> {
    pub opaque: &'a wgpu::TextureView,
    pub color: &'a wgpu::TextureView,
    pub origin: [i32; 2],
    pub render_size: [i32; 2],
}

struct Pipelines {
    autogen_reactive: wgpu::ComputePipeline,
    prepare_inputs: wgpu::ComputePipeline,
    luma_pyramid: wgpu::ComputePipeline,
    luma_finalize: wgpu::ComputePipeline,
    shading_change_pyramid: wgpu::ComputePipeline,
    shading_change: wgpu::ComputePipeline,
    prepare_reactivity: wgpu::ComputePipeline,
    luma_instability: wgpu::ComputePipeline,
    accumulate: wgpu::ComputePipeline,
    rcas: wgpu::ComputePipeline,
}

struct Target {
    // CRITICAL: view must drop before texture
    view: wgpu::TextureView,
    _texture: wgpu::Texture,
}

impl Target {
    fn new(
        device: &wgpu::Device,
        size: [u32; 2],
        format: wgpu::TextureFormat,
        label: &str,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size[0].max(1),
                height: size[1].max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            view,
            _texture: texture,
        }
    }
}

/// FSR's own textures, sized for the largest render and output size (the post-process
/// targets'); each frame uses the top-left corner.
struct Targets {
    size: [u32; 2],
    dilated_depth: Target,
    dilated_motion: Target,
    farthest_depth: Target,
    farthest_depth_mip1: Target,
    /// This frame's and last frame's, swapping each frame (also below).
    luma: [Target; 2],
    accumulation: [Target; 2],
    luma_history: [Target; 2],
    history: [Target; 2],
    luma_instability: Target,
    dilated_reactive: Target,
    spd_mips: [Target; 3],
    shading_change: Target,
    new_locks: Target,
    /// This frame's reactive mask (autogen reactive).
    reactive: Target,
    reconstructed_depth: wgpu::Buffer,
    luma_partials: wgpu::Buffer,
    frame_info: wgpu::Buffer,
}

fn half(size: [u32; 2], shift: u32) -> [u32; 2] {
    [(size[0] >> shift).max(1), (size[1] >> shift).max(1)]
}

fn luma_groups(render_size: [u32; 2]) -> [u32; 2] {
    let half_size = [render_size[0].div_ceil(2), render_size[1].div_ceil(2)];
    [
        half_size[0].div_ceil(LUMA_GROUP_SIZE),
        half_size[1].div_ceil(LUMA_GROUP_SIZE),
    ]
}

impl Targets {
    fn new(device: &wgpu::Device, size: [u32; 2]) -> Self {
        use wgpu::TextureFormat::{R32Float, Rg32Float, Rgba16Float, Rgba8Unorm};
        let pair = |format, label: &str| {
            [
                Target::new(device, size, format, &format!("{label}0")),
                Target::new(device, size, format, &format!("{label}1")),
            ]
        };
        let buffer = |label, bytes: u64| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: bytes.max(16),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let groups = luma_groups(size);
        Self {
            size,
            dilated_depth: Target::new(device, size, R32Float, "FsrDilatedDepth"),
            dilated_motion: Target::new(device, size, Rg32Float, "FsrDilatedMotion"),
            farthest_depth: Target::new(device, size, R32Float, "FsrFarthestDepth"),
            farthest_depth_mip1: Target::new(
                device,
                half(size, 1),
                R32Float,
                "FsrFarthestDepthMip1",
            ),
            luma: pair(R32Float, "FsrLuma"),
            accumulation: pair(R32Float, "FsrAccumulation"),
            luma_history: pair(Rgba16Float, "FsrLumaHistory"),
            history: pair(Rgba16Float, "FsrHistory"),
            luma_instability: Target::new(device, size, R32Float, "FsrLumaInstability"),
            dilated_reactive: Target::new(device, size, Rgba8Unorm, "FsrDilatedReactive"),
            spd_mips: [
                Target::new(device, half(size, 1), Rg32Float, "FsrShadingMip0"),
                Target::new(device, half(size, 2), Rg32Float, "FsrShadingMip1"),
                Target::new(device, half(size, 3), Rg32Float, "FsrShadingMip2"),
            ],
            shading_change: Target::new(device, half(size, 1), R32Float, "FsrShadingChange"),
            new_locks: Target::new(device, size, R32Float, "FsrNewLocks"),
            reactive: Target::new(device, size, R32Float, "FsrReactive"),
            reconstructed_depth: buffer(
                "FsrReconstructedDepth",
                u64::from(size[0]) * u64::from(size[1]) * 4,
            ),
            luma_partials: buffer(
                "FsrLumaPartials",
                u64::from(groups[0]) * u64::from(groups[1]) * 16,
            ),
            frame_info: buffer("FsrFrameInfo", 16),
        }
    }
}

/// Per-frame bind groups; index 0 or 1 by which of each pair is this frame's.
struct BindGroups {
    prepare_inputs: wgpu::BindGroup,
    luma_pyramid: wgpu::BindGroup,
    luma_finalize: wgpu::BindGroup,
    shading_change_pyramid: wgpu::BindGroup,
    shading_change: wgpu::BindGroup,
    prepare_reactivity: wgpu::BindGroup,
    luma_instability: wgpu::BindGroup,
    accumulate: wgpu::BindGroup,
    rcas: wgpu::BindGroup,
}

/// History carried from frame to frame.
#[derive(Default)]
struct FrameState {
    frame_index: f32,
    /// Which of each texture pair this frame writes.
    current: usize,
    previous_render_size: Option<[i32; 2]>,
    previous_upscale_size: Option<[i32; 2]>,
    previous_jitter: Vec2,
    jitter_phase_count: f32,
}

pub(super) struct FsrUpscaler {
    pipelines: Pipelines,
    constants: wgpu::Buffer,
    reactive_constants: wgpu::Buffer,
    targets: Option<Targets>,
    bind_groups: Option<[BindGroups; 2]>,
    reactive_bind_group: Option<wgpu::BindGroup>,
    state: FrameState,
}

fn view(binding: u32, view: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::TextureView(view),
    }
}

fn buffer(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    }
}

pub(super) fn shader_source() -> String {
    ShaderBuilder::new()
        .with_module(wgsl!("postprocess/fsr3_upscaler.wgsl"))
        .build_modules_only()
}

impl FsrUpscaler {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Fsr3UpscalerShader"),
            source: wgpu::ShaderSource::Wgsl(shader_source().into()),
        });
        // Layouts come from what each entry point uses.
        let pipeline = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: None,
                module: &shader,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let pipelines = Pipelines {
            autogen_reactive: pipeline("cs_autogen_reactive"),
            prepare_inputs: pipeline("cs_prepare_inputs"),
            luma_pyramid: pipeline("cs_luma_pyramid"),
            luma_finalize: pipeline("cs_luma_finalize"),
            shading_change_pyramid: pipeline("cs_shading_change_pyramid"),
            shading_change: pipeline("cs_shading_change"),
            prepare_reactivity: pipeline("cs_prepare_reactivity"),
            luma_instability: pipeline("cs_luma_instability"),
            accumulate: pipeline("cs_accumulate"),
            rcas: pipeline("cs_rcas"),
        };
        let constants = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("FsrConstants"),
            size: std::mem::size_of::<FsrConstants>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let reactive_constants = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("FsrReactiveConstants"),
            size: std::mem::size_of::<ReactiveConstants>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipelines,
            constants,
            reactive_constants,
            targets: None,
            bind_groups: None,
            reactive_bind_group: None,
            state: FrameState::default(),
        }
    }

    /// Drop the textures (their size follows the post-process targets) and the history.
    pub fn resize(&mut self) {
        self.targets = None;
        self.bind_groups = None;
        self.reactive_bind_group = None;
        self.state = FrameState::default();
    }

    /// Rebuild the bind groups before the next frame (an input view changed).
    pub fn invalidate_bind_groups(&mut self) {
        self.bind_groups = None;
        self.reactive_bind_group = None;
    }

    fn ensure_targets(&mut self, device: &wgpu::Device, size: [u32; 2]) {
        if self
            .targets
            .as_ref()
            .is_none_or(|targets| targets.size != size)
        {
            self.targets = Some(Targets::new(device, size));
            self.bind_groups = None;
            self.reactive_bind_group = None;
            self.state = FrameState::default();
        }
    }

    /// Mark what was blended over the opaque scene this frame as reactive: FSR trusts
    /// history less there, so particles and transparent surfaces do not smear. Runs
    /// before anything else is added to the scene (reflections, refraction), which must
    /// keep accumulating.
    pub fn generate_reactive(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        size: [u32; 2],
        frame: ReactiveFrame<'_>,
    ) {
        self.ensure_targets(device, size);
        let targets = self.targets.as_ref().expect("targets were just made");
        let group = self.reactive_bind_group.get_or_insert_with(|| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("FsrAutogenReactive"),
                layout: &self.pipelines.autogen_reactive.get_bind_group_layout(0),
                entries: &[
                    view(1, frame.color),
                    buffer(80, &self.reactive_constants),
                    view(81, frame.opaque),
                    view(82, &targets.reactive.view),
                ],
            })
        });
        let constants = ReactiveConstants {
            origin: frame.origin,
            render_size: frame.render_size,
            scale: AUTOREACTIVE_SCALE,
            threshold: AUTOREACTIVE_THRESHOLD,
            binary_value: AUTOREACTIVE_BINARY_VALUE,
            flags: AUTOREACTIVE_APPLY_TONEMAP
                | AUTOREACTIVE_APPLY_THRESHOLD
                | AUTOREACTIVE_USE_COMPONENTS_MAX,
        };
        queue.write_buffer(&self.reactive_constants, 0, bytemuck::bytes_of(&constants));
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("FsrAutogenReactive"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipelines.autogen_reactive);
        pass.set_bind_group(0, &*group, &[]);
        pass.dispatch_workgroups(
            (frame.render_size[0].max(1) as u32).div_ceil(GROUP_SIZE),
            (frame.render_size[1].max(1) as u32).div_ceil(GROUP_SIZE),
            1,
        );
    }

    pub fn dispatch(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        size: [u32; 2],
        frame: FsrFrame<'_>,
    ) {
        self.ensure_targets(device, size);
        if self.bind_groups.is_none() {
            let targets = self.targets.as_ref().expect("targets were just made");
            self.bind_groups = Some([
                self.create_bind_groups(device, targets, &frame, 0),
                self.create_bind_groups(device, targets, &frame, 1),
            ]);
        }

        let constants = self.next_constants(&frame, size);
        queue.write_buffer(&self.constants, 0, bytemuck::bytes_of(&constants));
        let targets = self.targets.as_ref().expect("targets were just made");
        // Nearest depth is kept with atomicMax of 1 - depth: zero is the far plane.
        encoder.clear_buffer(&targets.reconstructed_depth, 0, None);

        let groups = &self
            .bind_groups
            .as_ref()
            .expect("bind groups were just made")[self.state.current];
        let render = [frame.render_size[0] as u32, frame.render_size[1] as u32];
        let upscale = [frame.upscale_size[0] as u32, frame.upscale_size[1] as u32];
        let tiles =
            |extent: [u32; 2], tile: u32| [extent[0].div_ceil(tile), extent[1].div_ceil(tile)];
        let shading_size = [render[0] / 2, render[1] / 2];

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Fsr3Upscale"),
            timestamp_writes: None,
        });
        let mut run =
            |pipeline: &wgpu::ComputePipeline, group: &wgpu::BindGroup, count: [u32; 2]| {
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, group, &[]);
                pass.dispatch_workgroups(count[0].max(1), count[1].max(1), 1);
            };
        let p = &self.pipelines;
        run(
            &p.prepare_inputs,
            &groups.prepare_inputs,
            tiles(render, GROUP_SIZE),
        );
        run(&p.luma_pyramid, &groups.luma_pyramid, luma_groups(render));
        run(&p.luma_finalize, &groups.luma_finalize, [1, 1]);
        run(
            &p.shading_change_pyramid,
            &groups.shading_change_pyramid,
            tiles(shading_size, GROUP_SIZE),
        );
        run(
            &p.shading_change,
            &groups.shading_change,
            tiles(shading_size, GROUP_SIZE),
        );
        run(
            &p.prepare_reactivity,
            &groups.prepare_reactivity,
            tiles(render, GROUP_SIZE),
        );
        run(
            &p.luma_instability,
            &groups.luma_instability,
            tiles(render, GROUP_SIZE),
        );
        run(
            &p.accumulate,
            &groups.accumulate,
            tiles(upscale, GROUP_SIZE),
        );
        if constants.sharpen != 0 {
            run(&p.rcas, &groups.rcas, tiles(upscale, GROUP_SIZE));
        }
        drop(pass);

        self.state.current = 1 - self.state.current;
    }

    /// This frame's constants; advances the frame state (index, previous sizes, jitter).
    fn next_constants(&mut self, frame: &FsrFrame<'_>, size: [u32; 2]) -> FsrConstants {
        let state = &mut self.state;
        let reset = frame.reset || state.previous_render_size.is_none();
        state.frame_index = if reset { 0.0 } else { state.frame_index + 1.0 };

        // As ffxFsr3UpscalerGetJitterPhaseCount, eased toward a new value one per frame.
        let ratio = frame.upscale_size[0] as f32 / frame.render_size[0].max(1) as f32;
        let phase_count = (8.0 * ratio * ratio).trunc();
        if reset || state.jitter_phase_count == 0.0 {
            state.jitter_phase_count = phase_count;
        } else if phase_count > state.jitter_phase_count {
            state.jitter_phase_count += 1.0;
        } else if phase_count < state.jitter_phase_count {
            state.jitter_phase_count -= 1.0;
        }

        let previous_render_size = state.previous_render_size.unwrap_or(frame.render_size);
        let previous_upscale_size = state.previous_upscale_size.unwrap_or(frame.upscale_size);
        let previous_jitter = if reset {
            frame.jitter
        } else {
            state.previous_jitter
        };
        state.previous_render_size = Some(frame.render_size);
        state.previous_upscale_size = Some(frame.upscale_size);
        state.previous_jitter = frame.jitter;

        let render = [frame.render_size[0] as u32, frame.render_size[1] as u32];
        let groups = luma_groups(render);
        let proj = frame.projection;
        // RCAS sharpness: 0 = off, 1 = the strongest (0 stops of reduction).
        let sharpness = frame.sharpness.clamp(0.0, 1.0);
        let stops = -2.0 * sharpness + 2.0;
        FsrConstants {
            render_size: frame.render_size,
            previous_render_size,
            upscale_size: frame.upscale_size,
            previous_upscale_size,
            max_render_size: [size[0] as i32, size[1] as i32],
            input_offset: frame.origin,
            output_offset: frame.origin,
            luma_groups: [groups[0] as i32, (groups[0] * groups[1]) as i32],
            // View depth = [1] / (device depth - [0]) for a 0..1 depth projection; [2] and
            // [3] turn NDC into view x and y per unit depth.
            device_to_view_depth: [
                -proj.z_axis.z,
                proj.w_axis.z,
                1.0 / proj.x_axis.x,
                1.0 / proj.y_axis.y,
            ],
            jitter: frame.jitter.to_array(),
            previous_jitter: previous_jitter.to_array(),
            downscale_factor: [
                frame.render_size[0] as f32 / frame.upscale_size[0].max(1) as f32,
                frame.render_size[1] as f32 / frame.upscale_size[1].max(1) as f32,
            ],
            jitter_sequence_length: state.jitter_phase_count.max(1.0),
            delta_time: frame.delta_time.clamp(0.0, 1.0),
            delta_pre_exposure: 1.0,
            frame_index: state.frame_index,
            // FSR's defaults.
            velocity_factor: 1.0,
            reactiveness_scale: 1.0,
            shading_change_scale: 1.0,
            accumulation_added_per_frame: 1.0 / 3.0,
            min_disocclusion_accumulation: -1.0 / 3.0,
            rcas_sharpness: (-stops).exp2(),
            sharpen: u32::from(sharpness > 0.0),
            _pad: [0; 3],
        }
    }

    fn create_bind_groups(
        &self,
        device: &wgpu::Device,
        t: &Targets,
        frame: &FsrFrame<'_>,
        current: usize,
    ) -> BindGroups {
        let previous = 1 - current;
        let constants = buffer(0, &self.constants);
        let sampler = wgpu::BindGroupEntry {
            binding: 4,
            resource: wgpu::BindingResource::Sampler(frame.sampler),
        };
        let group =
            |label: &str, pipeline: &wgpu::ComputePipeline, entries: &[wgpu::BindGroupEntry]| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some(label),
                    layout: &pipeline.get_bind_group_layout(0),
                    entries,
                })
            };
        let p = &self.pipelines;
        BindGroups {
            prepare_inputs: group(
                "FsrPrepareInputs",
                &p.prepare_inputs,
                &[
                    constants.clone(),
                    view(1, frame.color),
                    view(2, frame.depth),
                    view(3, frame.motion),
                    view(10, &t.dilated_depth.view),
                    view(11, &t.dilated_motion.view),
                    view(12, &t.farthest_depth.view),
                    view(13, &t.luma[current].view),
                    buffer(14, &t.reconstructed_depth),
                ],
            ),
            luma_pyramid: group(
                "FsrLumaPyramid",
                &p.luma_pyramid,
                &[
                    constants.clone(),
                    view(20, &t.luma[current].view),
                    view(21, &t.farthest_depth.view),
                    view(22, &t.farthest_depth_mip1.view),
                    buffer(23, &t.luma_partials),
                ],
            ),
            luma_finalize: group(
                "FsrLumaFinalize",
                &p.luma_finalize,
                &[
                    constants.clone(),
                    buffer(23, &t.luma_partials),
                    buffer(24, &t.frame_info),
                ],
            ),
            shading_change_pyramid: group(
                "FsrShadingChangePyramid",
                &p.shading_change_pyramid,
                &[
                    constants.clone(),
                    view(20, &t.luma[current].view),
                    buffer(25, &t.frame_info),
                    view(30, &t.dilated_motion.view),
                    view(31, &t.luma[previous].view),
                    view(32, &t.spd_mips[0].view),
                    view(33, &t.spd_mips[1].view),
                    view(34, &t.spd_mips[2].view),
                ],
            ),
            shading_change: group(
                "FsrShadingChange",
                &p.shading_change,
                &[
                    constants.clone(),
                    view(35, &t.spd_mips[0].view),
                    view(36, &t.spd_mips[1].view),
                    view(37, &t.spd_mips[2].view),
                    view(38, &t.shading_change.view),
                ],
            ),
            prepare_reactivity: group(
                "FsrPrepareReactivity",
                &p.prepare_reactivity,
                &[
                    constants.clone(),
                    view(20, &t.luma[current].view),
                    buffer(25, &t.frame_info),
                    view(30, &t.dilated_motion.view),
                    view(40, &t.dilated_depth.view),
                    buffer(41, &t.reconstructed_depth),
                    view(42, &t.shading_change.view),
                    view(43, &t.accumulation[previous].view),
                    view(44, &t.accumulation[current].view),
                    view(45, &t.dilated_reactive.view),
                    view(46, &t.new_locks.view),
                    view(83, &t.reactive.view),
                ],
            ),
            luma_instability: group(
                "FsrLumaInstability",
                &p.luma_instability,
                &[
                    constants.clone(),
                    sampler.clone(),
                    view(20, &t.luma[current].view),
                    buffer(25, &t.frame_info),
                    view(30, &t.dilated_motion.view),
                    view(50, &t.dilated_reactive.view),
                    view(51, &t.luma_history[previous].view),
                    view(53, &t.luma_history[current].view),
                    view(54, &t.luma_instability.view),
                ],
            ),
            accumulate: group(
                "FsrAccumulate",
                &p.accumulate,
                &[
                    constants.clone(),
                    view(1, frame.color),
                    sampler,
                    buffer(25, &t.frame_info),
                    view(30, &t.dilated_motion.view),
                    view(50, &t.dilated_reactive.view),
                    view(52, &t.farthest_depth_mip1.view),
                    view(60, &t.history[previous].view),
                    view(61, &t.luma_instability.view),
                    view(62, &t.history[current].view),
                    view(63, frame.output),
                    view(64, &t.new_locks.view),
                ],
            ),
            rcas: group(
                "FsrRcas",
                &p.rcas,
                &[
                    constants,
                    buffer(25, &t.frame_info),
                    view(63, frame.output),
                    view(70, &t.history[current].view),
                ],
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn constants_match_the_shader_layout() {
        assert_eq!(std::mem::size_of::<super::FsrConstants>(), 160);
        assert_eq!(std::mem::size_of::<super::ReactiveConstants>(), 32);
    }

    #[test]
    fn shader_validates() {
        let source = super::shader_source();
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|err| panic!("{}", err.emit_to_string(&source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .unwrap_or_else(|err| panic!("{}", err.emit_to_string(&source)));
    }
}
