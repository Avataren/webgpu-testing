//! Presenting to HDR displays.
//!
//! In HDR mode the post-processed scene, the overlays drawn after it and the egui layer
//! do not go straight to the surface: the scene and overlays render into a half-float
//! texture (linear light, 1.0 = paper white) and egui into its own 8-bit layer. A present
//! pass then composites the two and encodes for the display: scRGB (linear, 1.0 = 80 nits)
//! or HDR10 (BT.2020 primaries, PQ). The simulated mode runs the same path on an SDR
//! surface, compressing highlights to fit.

use crate::settings::{HdrOutputSetting, RenderSettings};
use bytemuck::{Pod, Zeroable};
use half::f16;

/// Format of the texture the scene and overlays are composed in before presenting.
pub const PRESENT_SOURCE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Format of the layer egui draws into in HDR mode (egui writes sRGB-encoded values).
pub const UI_LAYER_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// How frames reach the display.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HdrOutputMode {
    /// Standard dynamic range straight to the surface.
    Sdr,
    /// The HDR path, shown on an SDR surface.
    Simulated,
    /// Extended linear sRGB (scRGB) on an `Rgba16Float` surface.
    ScRgb,
    /// HDR10: BT.2020 primaries and the PQ curve on an `Rgb10a2Unorm` surface.
    Pq,
}

impl HdrOutputMode {
    fn shader_index(self) -> f32 {
        match self {
            HdrOutputMode::Sdr | HdrOutputMode::Simulated => 0.0,
            HdrOutputMode::ScRgb => 1.0,
            HdrOutputMode::Pq => 2.0,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            HdrOutputMode::Sdr => "sdr",
            HdrOutputMode::Simulated => "simulated",
            HdrOutputMode::ScRgb => "scrgb",
            HdrOutputMode::Pq => "hdr10",
        }
    }
}

/// The display's HDR mode and brightness.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HdrDisplay {
    pub mode: HdrOutputMode,
    pub paper_white_nits: f32,
    pub peak_nits: f32,
}

impl HdrDisplay {
    pub fn sdr() -> Self {
        Self {
            mode: HdrOutputMode::Sdr,
            paper_white_nits: 80.0,
            peak_nits: 80.0,
        }
    }

    /// Whether frames take the HDR path (composed, then presented by the present pass).
    pub fn is_hdr_path(&self) -> bool {
        self.mode != HdrOutputMode::Sdr
    }

    /// The brightest value post-processing should produce, relative to paper white.
    pub fn peak_ratio(&self) -> f32 {
        if self.is_hdr_path() {
            (self.peak_nits / self.paper_white_nits).max(1.0)
        } else {
            1.0
        }
    }
}

/// Pick the surface format, colour space and mode for `settings`, given the formats the
/// surface offers (`sdr_format` is the one SDR rendering would use).
pub fn choose_surface(
    settings: &RenderSettings,
    capabilities: &wgpu::SurfaceCapabilities,
    sdr_format: wgpu::TextureFormat,
) -> (wgpu::TextureFormat, wgpu::SurfaceColorSpace, HdrDisplay) {
    let display = |mode| HdrDisplay {
        mode,
        paper_white_nits: settings.hdr_paper_white_nits,
        peak_nits: settings.hdr_peak_nits,
    };
    match settings.hdr_output {
        HdrOutputSetting::Off => (sdr_format, wgpu::SurfaceColorSpace::Auto, HdrDisplay::sdr()),
        HdrOutputSetting::Simulate => (
            sdr_format,
            wgpu::SurfaceColorSpace::Auto,
            display(HdrOutputMode::Simulated),
        ),
        HdrOutputSetting::Auto => {
            let offers = |format, space: wgpu::SurfaceColorSpaces| {
                capabilities.color_spaces(format).contains(space)
            };
            if offers(
                wgpu::TextureFormat::Rgba16Float,
                wgpu::SurfaceColorSpaces::EXTENDED_SRGB_LINEAR,
            ) {
                log::info!("HDR output: scRGB (Rgba16Float, extended linear sRGB)");
                return (
                    wgpu::TextureFormat::Rgba16Float,
                    wgpu::SurfaceColorSpace::ExtendedSrgbLinear,
                    display(HdrOutputMode::ScRgb),
                );
            }
            if offers(
                wgpu::TextureFormat::Rgb10a2Unorm,
                wgpu::SurfaceColorSpaces::BT2100_PQ,
            ) {
                log::info!("HDR output: HDR10 (Rgb10a2Unorm, BT.2100 PQ)");
                return (
                    wgpu::TextureFormat::Rgb10a2Unorm,
                    wgpu::SurfaceColorSpace::Bt2100Pq,
                    display(HdrOutputMode::Pq),
                );
            }
            log::warn!(
                "HDR output was requested but the display offers no HDR colour space \
                 (is HDR enabled in the system's display settings?); using SDR"
            );
            (sdr_format, wgpu::SurfaceColorSpace::Auto, HdrDisplay::sdr())
        }
    }
}

#[repr(C, align(16))]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PresentUniform {
    mode: f32,
    paper_white: f32,
    peak: f32,
    encode_srgb: f32,
}

/// The textures frames are composed in and the pass that presents them.
pub struct PresentStage {
    // CRITICAL: views must drop before their textures
    scene_view: wgpu::TextureView,
    _scene: wgpu::Texture,
    ui_view: wgpu::TextureView,
    _ui: wgpu::Texture,
    uniform: wgpu::Buffer,
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
}

impl PresentStage {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
        display: &HdrDisplay,
    ) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("PresentLayout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                texture_entry(1),
                texture_entry(2),
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("PresentHdrShader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shader/present_hdr.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("PresentPipelineLayout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("PresentPipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_present"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_present"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("PresentUniform"),
            size: std::mem::size_of::<PresentUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(
            &uniform,
            0,
            bytemuck::bytes_of(&PresentUniform {
                mode: display.mode.shader_index(),
                paper_white: display.paper_white_nits,
                peak: display.peak_ratio(),
                encode_srgb: if surface_format.is_srgb() { 0.0 } else { 1.0 },
            }),
        );
        let (scene, scene_view, ui, ui_view) = create_targets(device, width, height);
        let bind_group = create_bind_group(device, &layout, &uniform, &scene_view, &ui_view);
        Self {
            scene_view,
            _scene: scene,
            ui_view,
            _ui: ui,
            uniform,
            layout,
            pipeline,
            bind_group,
        }
    }

    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let (scene, scene_view, ui, ui_view) = create_targets(device, width, height);
        self.bind_group =
            create_bind_group(device, &self.layout, &self.uniform, &scene_view, &ui_view);
        self.scene_view = scene_view;
        self._scene = scene;
        self.ui_view = ui_view;
        self._ui = ui;
    }

    /// Where the scene and overlays are drawn instead of the surface.
    pub fn scene_view(&self) -> &wgpu::TextureView {
        &self.scene_view
    }

    /// Where egui is drawn instead of the surface.
    pub fn ui_view(&self) -> &wgpu::TextureView {
        &self.ui_view
    }

    /// Clear the UI layer for a new frame.
    pub fn clear_ui(&self, encoder: &mut wgpu::CommandEncoder) {
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ClearUiLayer"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.ui_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    }

    /// Composite scene and UI into `surface` in the display's encoding.
    pub fn present(&self, encoder: &mut wgpu::CommandEncoder, surface: &wgpu::TextureView) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("PresentHdr"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: surface,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn create_targets(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> (
    wgpu::Texture,
    wgpu::TextureView,
    wgpu::Texture,
    wgpu::TextureView,
) {
    let make = |label, format| {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, view)
    };
    let (scene, scene_view) = make("PresentScene", PRESENT_SOURCE_FORMAT);
    let (ui, ui_view) = make("PresentUiLayer", UI_LAYER_FORMAT);
    (scene, scene_view, ui, ui_view)
}

fn create_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniform: &wgpu::Buffer,
    scene: &wgpu::TextureView,
    ui: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("PresentBindGroup"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(scene),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(ui),
            },
        ],
    })
}

// --- CPU-side decoding, so screenshots of HDR surfaces come out as SDR images ---

const PQ_M1: f32 = 0.159_301_76;
const PQ_M2: f32 = 78.843_75;
const PQ_C1: f32 = 0.835_937_5;
const PQ_C2: f32 = 18.851_563;
const PQ_C3: f32 = 18.6875;

/// PQ (SMPTE ST 2084) signal 0-1 to absolute luminance in nits.
pub fn pq_decode(signal: f32) -> f32 {
    let e = signal.clamp(0.0, 1.0).powf(1.0 / PQ_M2);
    let y = ((e - PQ_C1).max(0.0) / (PQ_C2 - PQ_C3 * e)).powf(1.0 / PQ_M1);
    y * 10000.0
}

/// Absolute luminance in nits to a PQ signal (the inverse of [`pq_decode`]).
pub fn pq_encode(nits: f32) -> f32 {
    let y = (nits / 10000.0).clamp(0.0, 1.0).powf(PQ_M1);
    ((PQ_C1 + PQ_C2 * y) / (1.0 + PQ_C3 * y)).powf(PQ_M2)
}

/// BT.2020 to BT.709 primaries (linear), the inverse of the shader's matrix.
const BT2020_TO_BT709: [[f32; 3]; 3] = [
    [1.660_491, -0.587_641_1, -0.072_849_9],
    [-0.124_550_5, 1.132_899_9, -0.008_349_4],
    [-0.018_150_8, -0.100_578_9, 1.118_729_7],
];

fn linear_to_srgb8(value: f32) -> u8 {
    let v = value.clamp(0.0, 1.0);
    let encoded = if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    };
    (encoded * 255.0 + 0.5) as u8
}

/// Convert one HDR surface pixel to SDR sRGB bytes (RGBA), clipping above paper white.
pub fn hdr_pixel_to_srgb8(display: &HdrDisplay, raw: &[u8]) -> [u8; 4] {
    let linear = match display.mode {
        HdrOutputMode::ScRgb => {
            let channel = |i: usize| f16::from_le_bytes([raw[i * 2], raw[i * 2 + 1]]).to_f32();
            let scale = 80.0 / display.paper_white_nits;
            [channel(0) * scale, channel(1) * scale, channel(2) * scale]
        }
        HdrOutputMode::Pq => {
            let packed = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
            let channel = |shift: u32| ((packed >> shift) & 0x3ff) as f32 / 1023.0;
            let bt2020 = [channel(0), channel(10), channel(20)]
                .map(|signal| pq_decode(signal) / display.paper_white_nits);
            BT2020_TO_BT709.map(|row| row[0] * bt2020[0] + row[1] * bt2020[1] + row[2] * bt2020[2])
        }
        HdrOutputMode::Sdr | HdrOutputMode::Simulated => [0.0; 3],
    };
    [
        linear_to_srgb8(linear[0]),
        linear_to_srgb8(linear[1]),
        linear_to_srgb8(linear[2]),
        255,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pq_round_trips_and_hits_reference_points() {
        for nits in [0.0, 0.1, 80.0, 203.0, 1000.0, 4000.0, 10000.0] {
            let back = pq_decode(pq_encode(nits));
            assert!(
                (back - nits).abs() <= nits * 1e-3 + 1e-3,
                "{nits} -> {back}"
            );
        }
        // 100 nits is about 0.508 and 1000 nits about 0.752 in PQ.
        assert!((pq_encode(100.0) - 0.508).abs() < 2e-3);
        assert!((pq_encode(1000.0) - 0.752).abs() < 2e-3);
    }

    #[test]
    fn primaries_matrices_are_inverse() {
        // Shader matrix (BT.709 -> BT.2020), rows.
        let forward = [
            [0.627_404, 0.329_282, 0.043_313_6],
            [0.069_097, 0.919_54, 0.011_361_2],
            [0.016_391_6, 0.088_013_2, 0.895_595],
        ];
        for (i, row) in BT2020_TO_BT709.iter().enumerate() {
            for (j, expected_column) in [0usize, 1, 2].into_iter().enumerate() {
                let value: f32 = (0..3).map(|k| row[k] * forward[k][expected_column]).sum();
                let expected = if i == j { 1.0 } else { 0.0 };
                assert!((value - expected).abs() < 1e-4, "[{i}][{j}] = {value}");
            }
        }
    }

    #[test]
    fn hdr_pixels_decode_to_paper_white() {
        let display = HdrDisplay {
            mode: HdrOutputMode::ScRgb,
            paper_white_nits: 200.0,
            peak_nits: 1000.0,
        };
        // Paper white in scRGB is 200 / 80 = 2.5.
        let half = f16::from_f32(2.5).to_le_bytes();
        let raw = [half[0], half[1], half[0], half[1], half[0], half[1], 0, 0];
        assert_eq!(hdr_pixel_to_srgb8(&display, &raw), [255, 255, 255, 255]);

        let display = HdrDisplay {
            mode: HdrOutputMode::Pq,
            ..display
        };
        // Half of paper white (100 nits) on all channels.
        let signal = (pq_encode(100.0) * 1023.0).round() as u32;
        let packed = signal | (signal << 10) | (signal << 20) | (3 << 30);
        let pixel = hdr_pixel_to_srgb8(&display, &packed.to_le_bytes());
        // 0.5 linear is 188 in sRGB.
        for channel in &pixel[..3] {
            assert!((*channel as i32 - 188).abs() <= 2, "{pixel:?}");
        }
        assert_eq!(display.peak_ratio(), 5.0);
        assert_eq!(HdrDisplay::sdr().peak_ratio(), 1.0);
    }

    #[test]
    fn present_shader_validates() {
        let source = include_str!("../shader/present_hdr.wgsl");
        let module = naga::front::wgsl::parse_str(source)
            .unwrap_or_else(|err| panic!("{}", err.emit_to_string(source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .unwrap_or_else(|err| panic!("{}", err.emit_to_string(source)));
    }
}
