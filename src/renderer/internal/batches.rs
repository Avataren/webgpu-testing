use std::ops::Range;

use crate::asset::{Handle, Mesh};
use crate::renderer::batch::InstanceData;
use crate::renderer::RenderPass;
use crate::scene::components::DepthState;

#[derive(Debug, Clone)]
pub(crate) struct OrderedBatch {
    pub mesh: Handle<Mesh>,
    pub pass: RenderPass,
    pub depth_state: DepthState,
    pub instances: Vec<InstanceData>,
    pub alpha_blend: bool,
    pub first_instance: u32,
}

pub(crate) struct PreparedBatches {
    pub batches: Vec<OrderedBatch>,
    pub opaque_range: Range<usize>,
    pub transparent_range: Range<usize>,
    pub overlay_range: Range<usize>,
}

impl PreparedBatches {
    pub(crate) fn all(&self) -> &[OrderedBatch] {
        &self.batches
    }

    pub(crate) fn opaque(&self) -> &[OrderedBatch] {
        &self.batches[self.opaque_range.clone()]
    }

    pub(crate) fn transparent(&self) -> &[OrderedBatch] {
        &self.batches[self.transparent_range.clone()]
    }

    pub(crate) fn overlay(&self) -> &[OrderedBatch] {
        &self.batches[self.overlay_range.clone()]
    }
}
