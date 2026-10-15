//! Reading render targets back to the CPU, for inspection (remote `texture.fetch`): the
//! renderer lists its targets by name ([`crate::renderer::Renderer::fetchable_textures`]),
//! and [`read`] copies a rectangle of one into memory as numbers per channel.
//!
//! The targets hold what the last frame left in them. Reading blocks until the GPU has
//! copied, so it is for occasional inspection, not every frame.

/// What a [`FetchableTexture`] reads from.
pub enum FetchSource<'a> {
    Texture(&'a wgpu::Texture),
    /// A storage buffer laid out as `channels` u32 values per pixel, `width` per row.
    Buffer {
        buffer: &'a wgpu::Buffer,
        width: u32,
        height: u32,
        channels: u32,
    },
}

/// A render target that can be read back, with what its channels hold.
pub struct FetchableTexture<'a> {
    pub name: String,
    pub description: &'static str,
    pub source: FetchSource<'a>,
}

impl FetchableTexture<'_> {
    pub fn new<'a>(
        name: impl Into<String>,
        description: &'static str,
        texture: &'a wgpu::Texture,
    ) -> FetchableTexture<'a> {
        FetchableTexture {
            name: name.into(),
            description,
            source: FetchSource::Texture(texture),
        }
    }

    /// Width, height and array layers.
    pub fn size(&self) -> (u32, u32, u32) {
        match &self.source {
            FetchSource::Texture(texture) => (
                texture.width(),
                texture.height(),
                texture.depth_or_array_layers(),
            ),
            FetchSource::Buffer { width, height, .. } => (*width, *height, 1),
        }
    }

    /// The pixel format, as text.
    pub fn format(&self) -> String {
        match &self.source {
            FetchSource::Texture(texture) => format!("{:?}", texture.format()),
            FetchSource::Buffer { channels, .. } => format!("buffer, {channels} x u32 per pixel"),
        }
    }
}

/// How a format's texels decode.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Texel {
    F16,
    F32,
    U32,
    /// 8-bit unorm (0-1).
    Unorm8,
}

/// Bytes per texel, channels and channel type of the formats [`read`] understands, and
/// whether the stored channels are BGR(A) order.
fn texel_layout(format: wgpu::TextureFormat) -> Option<(u32, usize, Texel, bool)> {
    use wgpu::TextureFormat as F;
    Some(match format {
        F::R16Float => (2, 1, Texel::F16, false),
        F::Rg16Float => (4, 2, Texel::F16, false),
        F::Rgba16Float => (8, 4, Texel::F16, false),
        F::R32Float | F::Depth32Float => (4, 1, Texel::F32, false),
        F::Rg32Float => (8, 2, Texel::F32, false),
        F::Rgba32Float => (16, 4, Texel::F32, false),
        F::R32Uint => (4, 1, Texel::U32, false),
        F::Rg32Uint => (8, 2, Texel::U32, false),
        F::Rgba32Uint => (16, 4, Texel::U32, false),
        F::R8Unorm => (1, 1, Texel::Unorm8, false),
        F::Rg8Unorm => (2, 2, Texel::Unorm8, false),
        F::Rgba8Unorm | F::Rgba8UnormSrgb => (4, 4, Texel::Unorm8, false),
        F::Bgra8Unorm | F::Bgra8UnormSrgb => (4, 4, Texel::Unorm8, true),
        _ => return None,
    })
}

/// A rectangle of a target as numbers: `channels` values per pixel, rows top to bottom.
#[derive(Clone, Debug)]
pub struct Readback {
    pub width: u32,
    pub height: u32,
    pub channels: usize,
    pub values: Vec<f64>,
    /// The values are integers (ids, bit masks, fixed-point sums).
    pub integer: bool,
}

impl Readback {
    pub fn get(&self, x: u32, y: u32, channel: usize) -> f64 {
        self.values[(y as usize * self.width as usize + x as usize) * self.channels + channel]
    }
}

/// Decode `row` (tightly packed texels) into `out`.
fn decode_row(
    row: &[u8],
    bytes: u32,
    channels: usize,
    texel: Texel,
    bgr: bool,
    out: &mut Vec<f64>,
) {
    for pixel in row.chunks_exact(bytes as usize) {
        let start = out.len();
        for channel in 0..channels {
            let value = match texel {
                Texel::F16 => {
                    let raw = u16::from_le_bytes([pixel[channel * 2], pixel[channel * 2 + 1]]);
                    f64::from(half::f16::from_bits(raw).to_f32())
                }
                Texel::F32 => {
                    let raw: [u8; 4] = pixel[channel * 4..channel * 4 + 4]
                        .try_into()
                        .unwrap_or([0; 4]);
                    f64::from(f32::from_le_bytes(raw))
                }
                Texel::U32 => {
                    let raw: [u8; 4] = pixel[channel * 4..channel * 4 + 4]
                        .try_into()
                        .unwrap_or([0; 4]);
                    f64::from(u32::from_le_bytes(raw))
                }
                Texel::Unorm8 => f64::from(pixel[channel]) / 255.0,
            };
            out.push(value);
        }
        if bgr {
            out.swap(start, start + 2);
        }
    }
}

/// Copy `rect` ([x, y, width, height], clamped to the target) of array layer `layer` to
/// the CPU.
#[cfg(not(target_arch = "wasm32"))]
pub fn read(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &FetchableTexture<'_>,
    rect: [u32; 4],
    layer: u32,
) -> Result<Readback, String> {
    use std::sync::{Arc, Mutex};

    let (width, height, layers) = target.size();
    if layer >= layers {
        return Err(format!(
            "layer {layer} is out of range: {} has {layers}",
            target.name
        ));
    }
    let x = rect[0].min(width.saturating_sub(1));
    let y = rect[1].min(height.saturating_sub(1));
    let w = rect[2].min(width - x).max(1);
    let h = rect[3].min(height - y).max(1);

    // A copy the device rejects comes back as an error instead of ending the process.
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("TextureFetchEncoder"),
    });
    // What the staging buffer will hold, and how to decode it: rows of `row_stride` bytes,
    // the rectangle starting `skip_rows` rows and `row_offset` bytes in.
    let (staging, bytes, channels, texel, bgr, row_stride, row_offset, skip_rows) = match &target
        .source
    {
        FetchSource::Texture(texture) => {
            if texture.sample_count() > 1 {
                return Err(format!(
                    "{} is multisampled and cannot be read",
                    target.name
                ));
            }
            if !texture.usage().contains(wgpu::TextureUsages::COPY_SRC) {
                return Err(format!(
                    "{} cannot be copied (no COPY_SRC usage)",
                    target.name
                ));
            }
            let format = texture.format();
            let (bytes, channels, texel, bgr) = texel_layout(format)
                .ok_or_else(|| format!("{}: reading {format:?} is not supported", target.name))?;
            // Depth can only be copied whole; the rectangle is cut out afterwards.
            let depth = format.is_depth_stencil_format();
            let (copy_x, copy_y, copy_w, copy_h) = if depth {
                (0, 0, width, height)
            } else {
                (x, y, w, h)
            };
            let unpadded = copy_w * bytes;
            let padded = unpadded.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
                * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
            let staging = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("TextureFetchStaging"),
                size: u64::from(padded) * u64::from(copy_h),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let aspect = if depth {
                wgpu::TextureAspect::DepthOnly
            } else {
                wgpu::TextureAspect::All
            };
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: copy_x,
                        y: copy_y,
                        z: layer,
                    },
                    aspect,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &staging,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(padded),
                        rows_per_image: Some(copy_h),
                    },
                },
                wgpu::Extent3d {
                    width: copy_w,
                    height: copy_h,
                    depth_or_array_layers: 1,
                },
            );
            let (skip_x, skip_y) = (x - copy_x, y - copy_y);
            (
                staging,
                bytes,
                channels,
                texel,
                bgr,
                padded,
                skip_x * bytes,
                skip_y,
            )
        }
        FetchSource::Buffer {
            buffer, channels, ..
        } => {
            if !buffer.usage().contains(wgpu::BufferUsages::COPY_SRC) {
                return Err(format!(
                    "{} cannot be copied (no COPY_SRC usage)",
                    target.name
                ));
            }
            let bytes = channels * 4;
            let row = u64::from(width) * u64::from(bytes);
            let offset = u64::from(y) * row;
            let size = row * u64::from(h);
            if offset + size > buffer.size() {
                return Err(format!("{} is smaller than its size says", target.name));
            }
            let staging = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("TextureFetchStaging"),
                size,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.copy_buffer_to_buffer(buffer, offset, &staging, 0, size);
            // Whole rows were copied; each row's rectangle starts `x` texels in.
            (
                staging,
                bytes,
                *channels as usize,
                Texel::U32,
                false,
                row as u32,
                x * bytes,
                0,
            )
        }
    };
    queue.submit(Some(encoder.finish()));
    if let Some(error) = pollster::block_on(validation.pop()) {
        return Err(format!("{}: the copy was rejected: {error}", target.name));
    }

    let slice = staging.slice(..);
    let status = Arc::new(Mutex::new(None));
    let writer = Arc::clone(&status);
    slice.map_async(wgpu::MapMode::Read, move |result| {
        if let Ok(mut status) = writer.lock() {
            *status = Some(result);
        }
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|err| format!("waiting for the copy failed: {err}"))?;
    match status
        .lock()
        .map_err(|_| "readback status lock poisoned")?
        .take()
    {
        Some(Ok(())) => {}
        Some(Err(err)) => return Err(format!("failed to map the readback: {err}")),
        None => return Err("the readback was not mapped".into()),
    }

    let mut values = Vec::with_capacity(w as usize * h as usize * channels);
    {
        let data = slice
            .get_mapped_range()
            .map_err(|err| format!("failed to read the readback: {err}"))?;
        for row in data
            .chunks_exact(row_stride as usize)
            .skip(skip_rows as usize)
            .take(h as usize)
        {
            let start = row_offset as usize;
            decode_row(
                &row[start..start + (w * bytes) as usize],
                bytes,
                channels,
                texel,
                bgr,
                &mut values,
            );
        }
    }
    staging.unmap();
    Ok(Readback {
        width: w,
        height: h,
        channels,
        values,
        integer: texel == Texel::U32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_decode_per_format() {
        let mut out = Vec::new();
        let half_one = half::f16::from_f32(1.5).to_bits().to_le_bytes();
        let half_two = half::f16::from_f32(-2.0).to_bits().to_le_bytes();
        decode_row(
            &[half_one[0], half_one[1], half_two[0], half_two[1]],
            4,
            2,
            Texel::F16,
            false,
            &mut out,
        );
        assert_eq!(out, vec![1.5, -2.0]);

        out.clear();
        decode_row(&[10, 20, 30, 255], 4, 4, Texel::Unorm8, true, &mut out);
        assert_eq!(out, vec![30.0 / 255.0, 20.0 / 255.0, 10.0 / 255.0, 1.0]);

        out.clear();
        decode_row(&7u32.to_le_bytes(), 4, 1, Texel::U32, false, &mut out);
        assert_eq!(out, vec![7.0]);
    }

    #[test]
    fn depth_and_float_formats_are_supported() {
        assert_eq!(
            texel_layout(wgpu::TextureFormat::Depth32Float),
            Some((4, 1, Texel::F32, false))
        );
        assert_eq!(
            texel_layout(wgpu::TextureFormat::Rgba16Float),
            Some((8, 4, Texel::F16, false))
        );
        assert!(texel_layout(wgpu::TextureFormat::Depth24Plus).is_none());
    }
}
