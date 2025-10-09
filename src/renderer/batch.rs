// renderer/batch.rs (Smart version)
use super::material::Material;
use crate::{
    asset::{Handle, Mesh},
    scene::components::DepthState,
    scene::transform::Transform,
};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RenderPass {
    Opaque,      // Normal opaque geometry
    Transparent, // Alpha blended (needs sorting)
    Overlay,     // Draw last, typically with depth disabled
}

impl RenderPass {
    /// Returns true when instances in this pass should be sorted from back to
    /// front relative to the camera.  Transparent and overlay elements need
    /// back-to-front ordering so blending behaves as expected.
    pub fn requires_back_to_front_sort(self) -> bool {
        matches!(self, Self::Transparent | Self::Overlay)
    }

    /// Returns true when the pass intrinsically requires alpha blending.
    pub fn uses_alpha_blending(self) -> bool {
        matches!(self, Self::Transparent | Self::Overlay)
    }

    /// Sample count for the color attachment used by this pass.  Overlay
    /// passes are resolved directly into the swap chain, so MSAA is not used.
    pub fn color_sample_count(self, msaa_samples: u32) -> u32 {
        if matches!(self, Self::Overlay) {
            1
        } else {
            msaa_samples
        }
    }
}

/// A single renderable object instance
pub struct RenderObject {
    pub mesh: Handle<Mesh>,
    pub material: Material,
    pub transform: Transform, // Changed from Mat4
    pub depth_state: DepthState,
    pub force_overlay: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct InstanceData {
    pub transform: Transform, // Changed from Mat4
}

pub struct Batch<'a> {
    pub mesh: Handle<Mesh>,
    pub pass: RenderPass,
    pub depth_state: DepthState,
    pub material: Material,
    pub instances: &'a [InstanceData],
}

/// Batching key - only splits by what ACTUALLY requires different draw calls
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct BatchKey {
    mesh: Handle<Mesh>,
    material: Material,
    pass: RenderPass, // Only split if different pipeline needed
    depth_state: DepthState,
}

/// Collects objects and batches by pipeline requirements
pub struct RenderBatcher {
    batches: HashMap<BatchKey, Vec<InstanceData>>,
}

impl RenderBatcher {
    pub fn new() -> Self {
        Self {
            batches: HashMap::new(),
        }
    }

    /// Add an object to be rendered
    pub fn add(&mut self, obj: RenderObject) {
        // Determine which pass this object belongs to
        let pass = if obj.force_overlay {
            RenderPass::Overlay
        } else if obj.material.requires_separate_pass() {
            RenderPass::Transparent
        } else {
            RenderPass::Opaque
        };

        let key = BatchKey {
            mesh: obj.mesh,
            material: obj.material,
            pass,
            depth_state: obj.depth_state,
        };

        self.batches.entry(key).or_default().push(InstanceData {
            transform: obj.transform,
        });
    }

    /// Clear all batches
    pub fn clear(&mut self) {
        for batch in self.batches.values_mut() {
            batch.clear();
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = Batch<'_>> {
        self.batches.iter().map(|(key, instances)| Batch {
            mesh: key.mesh,
            pass: key.pass,
            depth_state: key.depth_state,
            material: key.material,
            instances: instances.as_slice(),
        })
    }

    pub fn iter_pass(&self, pass: RenderPass) -> impl Iterator<Item = Batch<'_>> {
        self.batches.iter().filter_map(move |(key, instances)| {
            if key.pass == pass {
                Some(Batch {
                    mesh: key.mesh,
                    pass: key.pass,
                    depth_state: key.depth_state,
                    material: key.material,
                    instances: instances.as_slice(),
                })
            } else {
                None
            }
        })
    }

    /// Get all instances for a pass (useful for sorting transparent objects)
    pub fn get_pass_instances(&self, pass: RenderPass) -> Vec<&InstanceData> {
        self.batches
            .iter()
            .filter(|(key, _)| key.pass == pass)
            .flat_map(|(_, instances)| instances.iter())
            .collect()
    }

    pub fn instance_count(&self) -> usize {
        self.batches.values().map(|v| v.len()).sum()
    }

    pub fn batch_count(&self) -> usize {
        self.batches.len()
    }
}

impl Default for RenderBatcher {
    fn default() -> Self {
        Self::new()
    }
}
