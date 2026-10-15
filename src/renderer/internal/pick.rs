//! GPU picking: on a frame with a pick request, a pass of its own draws every pickable
//! batch's entity id into a single-sample target at output resolution (whatever the
//! scene's MSAA sample count or upscaling), and one texel is read back asynchronously.

use std::mem::size_of;
use std::sync::{Arc, Mutex};

use crate::renderer::postprocess::GBUFFER_PICK_FORMAT;

/// Pick ids are 8 bytes; a copied row is padded to the copy alignment.
const PICK_COPY_BYTES_PER_ROW: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;

/// The id target and its depth buffer, sized to the surface.
pub(crate) struct PickTarget {
    texture: wgpu::Texture,
    pub(crate) view: wgpu::TextureView,
    pub(crate) depth_view: wgpu::TextureView,
}

impl PickTarget {
    fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let size = wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("PickIds"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: GBUFFER_PICK_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("PickDepth"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        Self {
            view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
            depth_view: depth.create_view(&wgpu::TextureViewDescriptor::default()),
            texture,
        }
    }

    pub(crate) fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }
}

#[derive(Default)]
pub(crate) struct PickState {
    pending_request: Option<[u32; 2]>,
    pending_readback: Option<PendingPickReadback>,
    ready_value: Option<u64>,
    /// Created by the first pick, then kept at the surface's size.
    target: Option<PickTarget>,
}

impl PickState {
    /// Queue a pick at output pixel `coords`; false while another is in flight.
    pub(crate) fn request(&mut self, coords: [u32; 2]) -> bool {
        if self.pending_request.is_some() || self.pending_readback.is_some() {
            return false;
        }
        self.ready_value = None;
        self.pending_request = Some(coords);
        true
    }

    /// Whether this frame has a pick to draw.
    pub(crate) fn has_request(&self) -> bool {
        self.pending_request.is_some()
    }

    /// The id target at `width` x `height`, (re)created as needed.
    pub(crate) fn target(&mut self, device: &wgpu::Device, width: u32, height: u32) -> &PickTarget {
        let stale = self.target.as_ref().is_none_or(|target| {
            target.texture.width() != width.max(1) || target.texture.height() != height.max(1)
        });
        if stale {
            self.target = Some(PickTarget::new(device, width, height));
        }
        self.target.as_ref().expect("pick target was just created")
    }

    /// The target the last pick pass drew, for reading back.
    pub(crate) fn last_target(&self) -> Option<&PickTarget> {
        self.target.as_ref()
    }

    /// Copy the requested texel (after the pick pass drew it) for reading back.
    pub(crate) fn copy_request(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        let Some([x, y]) = self.pending_request.take() else {
            return;
        };
        let Some(target) = self.target.as_ref() else {
            self.ready_value = Some(0);
            return;
        };
        let texture = &target.texture;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("PickReadbackBuffer"),
            size: PICK_COPY_BYTES_PER_ROW as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: x.min(texture.width() - 1),
                    y: y.min(texture.height() - 1),
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(PICK_COPY_BYTES_PER_ROW),
                    rows_per_image: Some(1),
                },
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        self.pending_readback = Some(PendingPickReadback::new(buffer));
    }

    /// Start mapping the readback once its copy is submitted.
    pub(crate) fn after_submit(&mut self) {
        if let Some(readback) = self.pending_readback.as_mut() {
            readback.ensure_mapping_requested();
        }
    }

    /// The picked id (0: nothing) once it is back from the GPU.
    pub(crate) fn poll(&mut self, device: &wgpu::Device) -> Option<u64> {
        if let Some(value) = self.ready_value.take() {
            return Some(value);
        }
        let result = self.pending_readback.as_mut()?.poll(device)?;
        self.pending_readback = None;
        Some(match result {
            Ok(value) => value,
            Err(error) => {
                log::error!("Failed to map pick readback buffer: {error}");
                0
            }
        })
    }
}

struct PendingPickReadback {
    buffer: wgpu::Buffer,
    status: Arc<Mutex<Option<Result<(), wgpu::BufferAsyncError>>>>,
    mapping_requested: bool,
}

impl PendingPickReadback {
    fn new(buffer: wgpu::Buffer) -> Self {
        Self {
            buffer,
            status: Arc::new(Mutex::new(None)),
            mapping_requested: false,
        }
    }

    fn ensure_mapping_requested(&mut self) {
        if self.mapping_requested {
            return;
        }
        let status = Arc::clone(&self.status);
        self.buffer
            .slice(..PICK_COPY_BYTES_PER_ROW as u64)
            .map_async(wgpu::MapMode::Read, move |result| {
                if let Ok(mut guard) = status.lock() {
                    *guard = Some(result);
                }
            });
        self.mapping_requested = true;
    }

    fn poll(&mut self, device: &wgpu::Device) -> Option<Result<u64, Box<dyn std::error::Error>>> {
        self.ensure_mapping_requested();
        let done = self
            .status
            .lock()
            .expect("pick readback status poisoned")
            .is_some();
        if !done {
            let _ = device.poll(wgpu::PollType::Poll);
        }
        let status = self
            .status
            .lock()
            .expect("pick readback status poisoned")
            .take()?;
        let value = status.map_err(Into::into).and_then(|()| {
            let data = self
                .buffer
                .slice(..size_of::<u64>() as u64)
                .get_mapped_range()?;
            let mut bytes = [0u8; 8];
            bytes.copy_from_slice(&data[..8]);
            Ok(u64::from_le_bytes(bytes))
        });
        self.buffer.unmap();
        self.mapping_requested = false;
        Some(value)
    }
}

impl Drop for PendingPickReadback {
    fn drop(&mut self) {
        if self.mapping_requested {
            self.buffer.unmap();
        }
    }
}
