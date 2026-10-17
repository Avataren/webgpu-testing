//! Light probes that follow the time of day: a volume baked at several hours keeps each
//! hour's irradiance (a keyframe), and while the procedural sky's clock runs the renderer
//! blends the two keyframes around the hour into the volume's region of the irradiance
//! atlas, so surfaces read it as any other probe light (light adds up, so a blend of two
//! hours' light is close to the light between them). The distances and the probes'
//! offsets come from the geometry and are kept once.
//!
//! Keyframes are kept as shared-exponent RGB (`Rgb9e5Ufloat`, 4 bytes a texel), in the
//! `.probes` file and on the GPU, one layer per hour.

use crate::renderer::shader_source::wgsl;
use crate::renderer::{PipelineBuilder, ShaderBuilder};

const KEYFRAME_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgb9e5Ufloat;
const ATLAS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// A volume's keyframes on the GPU.
#[derive(Clone, Debug)]
pub(crate) struct ProbeKeyframes {
    /// Hours of the day (0-24, sorted) each layer was baked at.
    pub hours: Vec<f32>,
    view: wgpu::TextureView,
    _texture: wgpu::Texture,
    /// Its blend inputs, made against the atlas it was last blended into.
    group: Option<wgpu::BindGroup>,
    /// The keyframes and weight last blended in (so an unchanged hour costs nothing).
    shown: Option<(u32, u32, u16)>,
}

impl ProbeKeyframes {
    /// `data`: one layer per hour of `size` texels (the region's tile rows), each texel a
    /// little-endian RGB9E5 word.
    pub(crate) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        hours: Vec<f32>,
        size: (u32, u32),
        data: &[u8],
    ) -> Result<Self, String> {
        let layers = hours.len() as u32;
        let expected = (size.0 * size.1 * 4 * layers) as usize;
        if layers == 0 || data.len() != expected {
            return Err(format!(
                "{} bytes of light probe keyframes, expected {expected}",
                data.len()
            ));
        }
        let extent = wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: layers,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("LightProbeKeyframes"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: KEYFRAME_FORMAT,
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
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size.0 * 4),
                rows_per_image: Some(size.1),
            },
            extent,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        Ok(Self {
            hours,
            view,
            _texture: texture,
            group: None,
            shown: None,
        })
    }

    /// The atlas was made anew: blend into it afresh.
    pub(crate) fn forget_atlas(&mut self) {
        self.group = None;
        self.shown = None;
    }
}

/// The two keyframes around `hour` and how far it is from the first to the second (the
/// day wraps round: after the last keyframe comes the first of the next day).
pub(crate) fn bracket(hours: &[f32], hour: f32) -> (u32, u32, f32) {
    let count = hours.len();
    if count <= 1 {
        return (0, 0, 0.0);
    }
    let hour = hour.rem_euclid(24.0);
    let next = hours.iter().position(|&h| h > hour).unwrap_or(count);
    let (a, b) = if next == 0 || next == count {
        (count - 1, 0)
    } else {
        (next - 1, next)
    };
    let start = hours[a];
    let span = (hours[b] - start).rem_euclid(24.0);
    let weight = if span > 0.0 {
        ((hour - start).rem_euclid(24.0) / span).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (a as u32, b as u32, weight)
}

/// Mirrors `KeyframeBlend` in light_probe_keyframes.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct BlendUniform {
    /// x, y: the region's first texel in the atlas; z, w: the keyframes blended.
    region: [u32; 4],
    /// x: the share of the second keyframe.
    weight: [f32; 4],
}

/// The pass blending keyframes into the irradiance atlas (made on first use: GPUs without
/// ray tracing run it too).
pub(crate) struct KeyframeBlender {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    uniform: wgpu::Buffer,
    slot: u64,
}

impl KeyframeBlender {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let uniform_size = std::mem::size_of::<BlendUniform>() as u64;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("LightProbeKeyframeInputs"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(uniform_size),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
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
            label: Some("LightProbeKeyframes"),
            source: wgpu::ShaderSource::Wgsl(
                ShaderBuilder::new()
                    .with_module(wgsl!("postprocess/fullscreen.wgsl"))
                    .build(wgsl!("light_probe_keyframes.wgsl"))
                    .into(),
            ),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("LightProbeKeyframeLayout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = PipelineBuilder::new(device, &pipeline_layout, &shader)
            .with_label("LightProbeKeyframeBlend")
            .with_vertex_entry("vs_fullscreen")
            .with_fragment_entry("fs_keyframe_blend")
            .with_color_target(ATLAS_FORMAT, None)
            .with_no_culling()
            .build();
        let alignment = (device.limits().min_uniform_buffer_offset_alignment as u64).max(1);
        let slot = uniform_size.div_ceil(alignment) * alignment;
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("LightProbeKeyframeBlend"),
            size: slot * crate::renderer::MAX_LIGHT_PROBE_VOLUMES as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            layout,
            uniform,
            slot,
        }
    }

    /// Blend `keyframes`' light for `hour` into `tiles` (x, y, width, height: the
    /// volume's tile rows) of the atlas: drawn into `next` (reading the probes' open/inside
    /// flags from `current`), then copied into `current`. `slot` picks the volume's
    /// uniform slot. Does nothing while the blend would come out as last time.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn blend(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        keyframes: &mut ProbeKeyframes,
        hour: f32,
        slot: usize,
        tiles: [u32; 4],
        current: (&wgpu::Texture, &wgpu::TextureView),
        next: (&wgpu::Texture, &wgpu::TextureView),
    ) {
        let (a, b, weight) = bracket(&keyframes.hours, hour);
        let shown = (a, b, (weight * 1024.0).round() as u16);
        if keyframes.shown == Some(shown) {
            return;
        }
        keyframes.shown = Some(shown);
        let offset = slot as u64 * self.slot;
        let uniform = BlendUniform {
            region: [tiles[0], tiles[1], a, b],
            weight: [weight, 0.0, 0.0, 0.0],
        };
        queue.write_buffer(&self.uniform, offset, bytemuck::bytes_of(&uniform));
        let group = keyframes.group.get_or_insert_with(|| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("LightProbeKeyframeInputs"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &self.uniform,
                            offset: 0,
                            size: wgpu::BufferSize::new(std::mem::size_of::<BlendUniform>() as u64),
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&keyframes.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(current.1),
                    },
                ],
            })
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("LightProbeKeyframeBlend"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: next.1,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_scissor_rect(tiles[0], tiles[1], tiles[2], tiles[3]);
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &*group, &[offset as u32]);
            pass.draw(0..3, 0..1);
        }
        let at = |texture| wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: tiles[0],
                y: tiles[1],
                z: 0,
            },
            aspect: wgpu::TextureAspect::All,
        };
        encoder.copy_texture_to_texture(
            at(next.0),
            at(current.0),
            wgpu::Extent3d {
                width: tiles[2],
                height: tiles[3],
                depth_or_array_layers: 1,
            },
        );
    }
}

/// Packs linear RGB into the shared-exponent RGB9E5 word (EXT_texture_shared_exponent).
#[cfg(any(test, not(target_arch = "wasm32")))]
pub(crate) fn pack_rgb9e5(rgb: [f32; 3]) -> u32 {
    const MANTISSA_BITS: i32 = 9;
    const BIAS: i32 = 15;
    const MAX_EXPONENT: i32 = 31;
    let max_value = ((1 << MANTISSA_BITS) - 1) as f32 / (1 << MANTISSA_BITS) as f32
        * 2f32.powi(MAX_EXPONENT - BIAS);
    let clamped = rgb.map(|c| {
        if c.is_finite() {
            c.clamp(0.0, max_value)
        } else {
            0.0
        }
    });
    let max_channel = clamped[0].max(clamped[1]).max(clamped[2]);
    if max_channel <= 0.0 {
        return 0;
    }
    let mut exponent = (max_channel.log2().floor() as i32).max(-BIAS - 1) + 1 + BIAS;
    let scale = |exponent: i32| 2f32.powi(exponent - BIAS - MANTISSA_BITS);
    if (max_channel / scale(exponent) + 0.5).floor() as i32 == 1 << MANTISSA_BITS {
        exponent += 1;
    }
    let [r, g, b] = clamped.map(|c| ((c / scale(exponent) + 0.5).floor() as u32).min(511));
    r | (g << 9) | (b << 18) | ((exponent as u32) << 27)
}

#[cfg(test)]
fn unpack_rgb9e5(word: u32) -> [f32; 3] {
    let exponent = (word >> 27) as i32;
    let scale = 2f32.powi(exponent - 15 - 9);
    [
        (word & 511) as f32 * scale,
        ((word >> 9) & 511) as f32 * scale,
        ((word >> 18) & 511) as f32 * scale,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb9e5_keeps_light_to_its_precision() {
        for rgb in [
            [0.0, 0.0, 0.0],
            [1.0, 0.5, 0.25],
            [12.5, 0.003, 7.0],
            [0.0001, 0.0002, 0.00005],
            [60000.0, 1.0, 0.0],
        ] {
            let back = unpack_rgb9e5(pack_rgb9e5(rgb));
            let max = rgb.iter().cloned().fold(0.0f32, f32::max);
            for (a, b) in rgb.iter().zip(back) {
                // A channel is good to the shared exponent's step (1/512 of the largest).
                assert!((a - b).abs() <= max / 256.0 + 1e-9, "{rgb:?} -> {back:?}");
            }
        }
        assert_eq!(pack_rgb9e5([-1.0, f32::NAN, 0.0]), 0);
    }

    #[test]
    fn hours_bracket_round_the_clock() {
        let hours = [0.0, 6.0, 12.0, 18.0];
        assert_eq!(bracket(&hours, 3.0), (0, 1, 0.5));
        assert_eq!(bracket(&hours, 12.0), (2, 3, 0.0));
        let (a, b, w) = bracket(&hours, 21.0);
        assert_eq!((a, b), (3, 0));
        assert!((w - 0.5).abs() < 1e-6);
        // Keyframes that do not start at midnight wrap too.
        let late = [5.0, 20.0];
        let (a, b, w) = bracket(&late, 1.0);
        assert_eq!((a, b), (1, 0));
        assert!((w - 5.0 / 9.0).abs() < 1e-6);
        assert_eq!(bracket(&[7.0], 15.0), (0, 0, 0.0));
    }
}
