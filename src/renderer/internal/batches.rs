use std::{cmp::Ordering, ops::Range};

use crate::asset::{Handle, Mesh};
use crate::renderer::batch::{InstanceData, RenderBatcher, RenderPass};
use crate::scene::components::DepthState;
use glam::Vec3;

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
    pub(crate) fn from_batcher(batcher: &RenderBatcher, camera_pos: Vec3) -> Self {
        let mut opaque = Vec::new();
        let mut transparent = Vec::new();
        let mut overlay = Vec::new();

        for batch in batcher.iter() {
            let mut instances = batch.instances.to_vec();

            if batch.pass.requires_back_to_front_sort() {
                sort_instances_back_to_front(&mut instances, camera_pos);
            }

            let alpha_blend = batch.pass.uses_alpha_blending()
                || instances
                    .iter()
                    .any(|inst| inst.material.requires_separate_pass());

            let ordered = OrderedBatch {
                mesh: batch.mesh,
                pass: batch.pass,
                depth_state: batch.depth_state,
                instances,
                alpha_blend,
                first_instance: 0,
            };

            match ordered.pass {
                RenderPass::Opaque => opaque.push(ordered),
                RenderPass::Transparent => transparent.push(ordered),
                RenderPass::Overlay => overlay.push(ordered),
            }
        }

        sort_batches_back_to_front(&mut transparent, camera_pos);
        sort_batches_back_to_front(&mut overlay, camera_pos);

        let mut batches = Vec::with_capacity(opaque.len() + transparent.len() + overlay.len());
        let opaque_range = append_batches(&mut batches, opaque);
        let transparent_range = append_batches(&mut batches, transparent);
        let overlay_range = append_batches(&mut batches, overlay);

        let mut offset = 0u32;
        for batch in &mut batches {
            batch.first_instance = offset;
            offset += batch.instances.len() as u32;
        }

        Self {
            batches,
            opaque_range,
            transparent_range,
            overlay_range,
        }
    }

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

fn sort_instances_back_to_front(instances: &mut [InstanceData], camera_pos: Vec3) {
    instances.sort_by(|a, b| {
        let da = (a.transform.translation - camera_pos).length_squared();
        let db = (b.transform.translation - camera_pos).length_squared();
        db.partial_cmp(&da).unwrap_or(Ordering::Equal)
    });
}

fn sort_batches_back_to_front(batches: &mut Vec<OrderedBatch>, camera_pos: Vec3) {
    batches.sort_by(|a, b| {
        farthest_distance_sq(b, camera_pos)
            .partial_cmp(&farthest_distance_sq(a, camera_pos))
            .unwrap_or(Ordering::Equal)
    });
}

fn farthest_distance_sq(batch: &OrderedBatch, camera_pos: Vec3) -> f32 {
    batch
        .instances
        .iter()
        .map(|inst| (inst.transform.translation - camera_pos).length_squared())
        .fold(0.0, f32::max)
}

fn append_batches(dest: &mut Vec<OrderedBatch>, src: Vec<OrderedBatch>) -> Range<usize> {
    let start = dest.len();
    dest.extend(src);
    start..dest.len()
}
