//! Reading the presented frame back from the GPU.

use std::sync::{Arc, Mutex};

use crate::renderer::{RenderFrame, Renderer};

/// An RGBA8 (sRGB-encoded) copy of a rendered frame.
#[derive(Clone)]
pub struct FrameCapture {
    pub width: u32,
    pub height: u32,
    /// Tightly packed RGBA rows, top to bottom.
    pub rgba: Vec<u8>,
}

/// Copy the swapchain image of `frame` to the CPU, including any UI drawn into it.
///
/// Call after everything has been rendered and before the frame is presented. This
/// blocks until the GPU finishes the copy, so it is meant for occasional captures,
/// not every frame. The surface must have been configured with `COPY_SRC`.
pub fn capture_surface(renderer: &Renderer, frame: &RenderFrame) -> Result<FrameCapture, String> {
    // In HDR mode the surface is only written when the frame is presented.
    renderer.resolve_output(frame);
    let texture = &frame.frame.texture;
    if !texture.usage().contains(wgpu::TextureUsages::COPY_SRC) {
        return Err("the surface does not support copies (no COPY_SRC usage)".into());
    }

    let format = texture.format();
    // HDR surfaces are converted to an SDR image, clipped at paper white.
    let (swap_red_blue, bytes_per_pixel, hdr) = match format {
        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb => (true, 4, false),
        wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Rgba8UnormSrgb => (false, 4, false),
        wgpu::TextureFormat::Rgba16Float => (false, 8, true),
        wgpu::TextureFormat::Rgb10a2Unorm => (false, 4, true),
        other => return Err(format!("unsupported surface format for capture: {other:?}")),
    };
    let display = renderer.hdr_display();

    let width = texture.width();
    let height = texture.height();
    let unpadded_bytes_per_row = width * bytes_per_pixel;
    let alignment = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded_bytes_per_row = unpadded_bytes_per_row.div_ceil(alignment) * alignment;

    let device = renderer.get_device();
    let queue = renderer.get_queue();
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("FrameCaptureBuffer"),
        size: u64::from(padded_bytes_per_row) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("FrameCaptureEncoder"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bytes_per_row),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));

    let slice = buffer.slice(..);
    let status = Arc::new(Mutex::new(None));
    let status_writer = Arc::clone(&status);
    slice.map_async(wgpu::MapMode::Read, move |result| {
        if let Ok(mut status) = status_writer.lock() {
            *status = Some(result);
        }
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|err| format!("waiting for the capture copy failed: {err}"))?;

    let mapped = status
        .lock()
        .map_err(|_| "capture status lock poisoned".to_string())?
        .take();
    match mapped {
        Some(Ok(())) => {}
        Some(Err(err)) => return Err(format!("failed to map the capture buffer: {err}")),
        None => return Err("the capture buffer was not mapped".into()),
    }

    let mut rgba = Vec::with_capacity((width * 4 * height) as usize);
    {
        let data = slice
            .get_mapped_range()
            .map_err(|err| format!("failed to read the capture buffer: {err}"))?;
        for row in data.chunks_exact(padded_bytes_per_row as usize) {
            let row = &row[..unpadded_bytes_per_row as usize];
            if hdr {
                for pixel in row.chunks_exact(bytes_per_pixel as usize) {
                    rgba.extend_from_slice(&crate::renderer::hdr_output::hdr_pixel_to_srgb8(
                        &display, pixel,
                    ));
                }
            } else {
                rgba.extend_from_slice(row);
            }
        }
    }
    buffer.unmap();

    for pixel in rgba.as_chunks_mut::<4>().0 {
        if swap_red_blue {
            pixel.swap(0, 2);
        }
        // The swapchain alpha is meaningless once composited; keep the PNG opaque.
        pixel[3] = 255;
    }

    Ok(FrameCapture {
        width,
        height,
        rgba,
    })
}
