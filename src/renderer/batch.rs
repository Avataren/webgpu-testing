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
    pub material: Material,
}

pub struct Batch<'a> {
    pub mesh: Handle<Mesh>,
    pub pass: RenderPass,
    pub depth_state: DepthState,
    pub instances: &'a [InstanceData],
}

/// Batching key - only splits by what ACTUALLY requires different draw calls
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct BatchKey {
    mesh: Handle<Mesh>,
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
            pass,
            depth_state: obj.depth_state,
        };

        self.batches
            .entry(key)
            .or_insert_with(Vec::new)
            .push(InstanceData {
                transform: obj.transform,
                material: obj.material,
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
