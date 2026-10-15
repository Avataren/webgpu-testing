//! GPU-to-CPU readback that never blocks.
//!
//! A browser cannot wait for the GPU: `Device::poll(Wait)` does nothing there, so a
//! `map_async` followed by a blocking poll never completes and leaves the buffer stuck in
//! a pending map. Instead the readback is a small state machine advanced once per frame:
//! the copy is recorded into a frame's command buffer, the buffer is mapped on the next
//! call, and the data is handed over on the first call after the mapping has completed
//! (natively `Device::poll(Poll)` runs the callback; in a browser it arrives between frames).

use std::sync::{Arc, Mutex};

type MapStatus = Arc<Mutex<Option<Result<(), wgpu::BufferAsyncError>>>>;

enum State {
    Idle,
    /// The copy is in a command buffer that has been (or is about to be) submitted; the
    /// next `poll` maps the buffer.
    Copied {
        size: u64,
    },
    /// `map_async` was requested and has not called back yet.
    Mapping {
        size: u64,
        status: MapStatus,
    },
}

pub(crate) struct AsyncReadback {
    buffer: wgpu::Buffer,
    state: State,
    label: &'static str,
}

impl AsyncReadback {
    pub(crate) fn new(device: &wgpu::Device, size: u64, label: &'static str) -> Self {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Self {
            buffer,
            state: State::Idle,
            label,
        }
    }

    /// Whether a new copy can be scheduled (nothing is in flight).
    pub(crate) fn is_idle(&self) -> bool {
        matches!(self.state, State::Idle)
    }

    /// Records a copy of the first `size` bytes of `source` and returns true, unless a
    /// readback is still in flight, in which case nothing is recorded and it returns false.
    /// The copy only counts once the encoder is submitted.
    pub(crate) fn schedule(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::Buffer,
        size: u64,
    ) -> bool {
        if !self.is_idle() {
            return false;
        }
        encoder.copy_buffer_to_buffer(source, 0, &self.buffer, 0, size);
        self.state = State::Copied { size };
        true
    }

    /// Advances the readback. Returns true, after calling `on_ready` with the copied bytes,
    /// when a copy has arrived; false while it is still on its way (or nothing is scheduled).
    pub(crate) fn poll(&mut self, device: &wgpu::Device, on_ready: impl FnOnce(&[u8])) -> bool {
        if let State::Copied { size } = self.state {
            let status: MapStatus = Arc::new(Mutex::new(None));
            let callback_status = Arc::clone(&status);
            self.buffer
                .slice(..size)
                .map_async(wgpu::MapMode::Read, move |result| {
                    *callback_status.lock().unwrap() = Some(result);
                });
            self.state = State::Mapping { size, status };
        }

        // Natively this runs the callback of a finished mapping; on the web it does nothing
        // and the browser delivers the callback between frames.
        let _ = device.poll(wgpu::PollType::Poll);

        let State::Mapping { size, status } = &self.state else {
            return false;
        };
        let size = *size;
        let Some(result) = status.lock().unwrap().take() else {
            return false;
        };

        self.state = State::Idle;
        match result {
            Ok(()) => {
                match self.buffer.slice(..size).get_mapped_range() {
                    Ok(data) => on_ready(&data),
                    Err(error) => log::warn!("Failed to read {}: {error}", self.label),
                }
                self.buffer.unmap();
                true
            }
            Err(error) => {
                log::warn!("Failed to map {}: {error}", self.label);
                false
            }
        }
    }

    /// Drops whatever is in flight (a pending mapping is cancelled) so the next
    /// `schedule` starts fresh.
    pub(crate) fn reset(&mut self) {
        if matches!(self.state, State::Mapping { .. }) {
            self.buffer.unmap();
        }
        self.state = State::Idle;
    }
}
