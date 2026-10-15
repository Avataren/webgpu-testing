//! GPU time per frame stage, from timestamp queries written between the passes of the
//! frame's command encoder.
//!
//! Needs `TIMESTAMP_QUERY` and `TIMESTAMP_QUERY_INSIDE_ENCODERS` (Vulkan, DX12, Metal;
//! not the web); without them every call is a no-op and [`GpuTimer::timings`] stays
//! `None`. Results are read back without blocking: each frame resolves its timestamps
//! into one of a few readback buffers, which are mapped after the frame is submitted and
//! read a frame or two later, so the timings trail the frame being drawn slightly.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

/// Timestamps one frame may write.
const MAX_STAMPS: u32 = 32;
/// Frames in flight before a frame goes untimed for want of a free readback buffer.
const SLOTS: usize = 3;
const STAMP_BYTES: u64 = 8;

const SLOT_IDLE: u8 = 0;
const SLOT_MAPPING: u8 = 1;
const SLOT_MAPPED: u8 = 2;
const SLOT_FAILED: u8 = 3;

/// GPU milliseconds per stage of one frame, in frame order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GpuTimings {
    pub stages: Vec<(&'static str, f32)>,
    /// From the first timestamp to the last.
    pub total_ms: f32,
}

struct Slot {
    readback: wgpu::Buffer,
    state: Arc<AtomicU8>,
    /// Stage names of the frame resolved into `readback` (one per timestamp after the
    /// first), while it is being read.
    labels: Vec<&'static str>,
    /// Timestamps written, while it is being read.
    count: u32,
}

pub(crate) struct GpuTimer {
    inner: Option<Inner>,
}

struct Inner {
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    slots: Vec<Slot>,
    /// Nanoseconds per timestamp tick.
    period: f32,
    /// The slot this frame writes into, if one was free.
    current: Option<usize>,
    /// Stage names so far this frame: `labels[i]` ends at timestamp `i + 1`.
    labels: Vec<&'static str>,
    written: u32,
    /// A slot resolved this frame, to map once the frame is submitted.
    to_map: Option<usize>,
    latest: Option<GpuTimings>,
}

/// Device features the timer needs.
pub(crate) const GPU_TIMER_FEATURES: wgpu::Features =
    wgpu::Features::TIMESTAMP_QUERY.union(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS);

impl GpuTimer {
    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        if !device.features().contains(GPU_TIMER_FEATURES) {
            log::info!("GPU timings unavailable: no timestamp queries inside encoders");
            return Self { inner: None };
        }
        let queries = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("GpuTimerQueries"),
            ty: wgpu::QueryType::Timestamp,
            count: MAX_STAMPS * SLOTS as u32,
        });
        let slot_bytes = MAX_STAMPS as u64 * STAMP_BYTES;
        let resolve = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("GpuTimerResolve"),
            size: slot_bytes * SLOTS as u64,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let slots = (0..SLOTS)
            .map(|_| Slot {
                readback: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("GpuTimerReadback"),
                    size: slot_bytes,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                state: Arc::new(AtomicU8::new(SLOT_IDLE)),
                labels: Vec::new(),
                count: 0,
            })
            .collect();
        Self {
            inner: Some(Inner {
                queries,
                resolve,
                slots,
                period: queue.get_timestamp_period(),
                current: None,
                labels: Vec::new(),
                written: 0,
                to_map: None,
                latest: None,
            }),
        }
    }

    /// The most recent frame's timings that have come back (a few frames old).
    pub(crate) fn timings(&self) -> Option<&GpuTimings> {
        self.inner.as_ref().and_then(|inner| inner.latest.as_ref())
    }

    /// Collect finished readbacks and start timing a frame: writes its first timestamp.
    pub(crate) fn begin_frame(&mut self, encoder: &mut wgpu::CommandEncoder) {
        let Some(inner) = self.inner.as_mut() else {
            return;
        };
        inner.collect();
        inner.labels.clear();
        inner.written = 0;
        inner.to_map = None;
        inner.current = inner
            .slots
            .iter()
            .position(|slot| slot.state.load(Ordering::Acquire) == SLOT_IDLE);
        inner.write(encoder);
    }

    /// Mark the end of the stage `label`, which ran since the previous mark.
    pub(crate) fn mark(&mut self, encoder: &mut wgpu::CommandEncoder, label: &'static str) {
        let Some(inner) = self.inner.as_mut() else {
            return;
        };
        if inner.current.is_some() && inner.written < MAX_STAMPS {
            inner.labels.push(label);
            inner.write(encoder);
        }
    }

    /// Resolve this frame's timestamps into its readback buffer. Call last before the
    /// encoder is finished, then [`Self::after_submit`] once it is submitted.
    pub(crate) fn end_frame(&mut self, encoder: &mut wgpu::CommandEncoder) {
        let Some(inner) = self.inner.as_mut() else {
            return;
        };
        let Some(index) = inner.current.take() else {
            return;
        };
        if inner.written < 2 {
            return;
        }
        let first = index as u32 * MAX_STAMPS;
        let offset = first as u64 * STAMP_BYTES;
        let bytes = inner.written as u64 * STAMP_BYTES;
        encoder.resolve_query_set(
            &inner.queries,
            first..first + inner.written,
            &inner.resolve,
            offset,
        );
        let slot = &mut inner.slots[index];
        encoder.copy_buffer_to_buffer(&inner.resolve, offset, &slot.readback, 0, bytes);
        slot.labels = std::mem::take(&mut inner.labels);
        slot.count = inner.written;
        slot.state.store(SLOT_MAPPING, Ordering::Release);
        inner.to_map = Some(index);
    }

    /// Start reading back the frame just submitted.
    pub(crate) fn after_submit(&mut self) {
        let Some(inner) = self.inner.as_mut() else {
            return;
        };
        let Some(index) = inner.to_map.take() else {
            return;
        };
        let slot = &inner.slots[index];
        let state = Arc::clone(&slot.state);
        slot.readback
            .slice(..slot.count as u64 * STAMP_BYTES)
            .map_async(wgpu::MapMode::Read, move |result| {
                let next = if result.is_ok() {
                    SLOT_MAPPED
                } else {
                    SLOT_FAILED
                };
                state.store(next, Ordering::Release);
            });
    }
}

impl Inner {
    fn write(&mut self, encoder: &mut wgpu::CommandEncoder) {
        if let Some(index) = self.current {
            encoder.write_timestamp(&self.queries, index as u32 * MAX_STAMPS + self.written);
            self.written += 1;
        }
    }

    /// Read every mapped slot (keeping the newest frame's result) and free it.
    fn collect(&mut self) {
        for slot in &mut self.slots {
            match slot.state.load(Ordering::Acquire) {
                SLOT_MAPPED => {
                    let range = ..slot.count as u64 * STAMP_BYTES;
                    if let Ok(data) = slot.readback.slice(range).get_mapped_range() {
                        let stamps: &[u64] = bytemuck::cast_slice(&data);
                        if let Some(timings) = timings_from(stamps, &slot.labels, self.period) {
                            self.latest = Some(timings);
                        }
                    }
                    slot.readback.unmap();
                    slot.state.store(SLOT_IDLE, Ordering::Release);
                }
                SLOT_FAILED => slot.state.store(SLOT_IDLE, Ordering::Release),
                _ => {}
            }
        }
    }
}

/// Stage durations from consecutive timestamps (`labels[i]` ends at `stamps[i + 1]`).
/// None when the timestamps run backwards (a reset or a driver that did not write them).
fn timings_from(stamps: &[u64], labels: &[&'static str], period_ns: f32) -> Option<GpuTimings> {
    let (first, last) = (*stamps.first()?, *stamps.last()?);
    if last < first {
        return None;
    }
    let ms = |ticks: u64| ticks as f32 * period_ns / 1.0e6;
    // A stage marked more than once in a frame (work split around another stage) adds up.
    let mut stages: Vec<(&'static str, f32)> = Vec::new();
    for (label, pair) in labels.iter().zip(stamps.windows(2)) {
        let time = ms(pair[1].saturating_sub(pair[0]));
        match stages.iter_mut().find(|(name, _)| name == label) {
            Some((_, total)) => *total += time,
            None => stages.push((*label, time)),
        }
    }
    Some(GpuTimings {
        stages,
        total_ms: ms(last - first),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stages_are_the_gaps_between_timestamps() {
        let timings = timings_from(&[1_000, 3_000, 3_500, 9_500], &["a", "b", "c"], 2.0).unwrap();
        assert_eq!(
            timings.stages,
            vec![("a", 0.004), ("b", 0.001), ("c", 0.012)]
        );
        assert!((timings.total_ms - 0.017).abs() < 1e-6);
    }

    #[test]
    fn repeated_stages_add_up() {
        let timings = timings_from(&[0, 1_000, 3_000, 3_500], &["a", "b", "a"], 1.0).unwrap();
        assert_eq!(timings.stages, vec![("a", 0.0015), ("b", 0.002)]);
    }

    #[test]
    fn backwards_timestamps_are_dropped() {
        assert!(timings_from(&[5, 3], &["a"], 1.0).is_none());
        assert!(timings_from(&[], &[], 1.0).is_none());
    }
}
