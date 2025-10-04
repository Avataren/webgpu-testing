// renderer/batch.rs (Smart version)
use crate::{asset::{Handle, Mesh}, scene::transform::Transform};
use super::material::Material;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RenderPass {
    Opaque,      // Normal opaque geometry
    Transparent, // Alpha blended (needs sorting)
}

/// A single renderable object instance
pub struct RenderObject {
    pub mesh: Handle<Mesh>,
    pub material: Material,
    pub transform: Transform,  // Changed from Mat4
}

pub struct InstanceData {
    pub transform: Transform,  // Changed from Mat4
    pub material: Material,
}

/// Batching key - only splits by what ACTUALLY requires different draw calls
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct BatchKey {
    mesh: Handle<Mesh>,
    pass: RenderPass,  // Only split if different pipeline needed
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
        let pass = if obj.material.requires_separate_pass() {
            RenderPass::Transparent
        } else {
            RenderPass::Opaque
        };

        let key = BatchKey {
            mesh: obj.mesh,
            pass,
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

    pub fn iter(&self) -> impl Iterator<Item = (Handle<Mesh>, &[InstanceData])> + '_ {
        self.batches
            .iter()
            .map(|(key, instances)| (key.mesh, instances.as_slice()))  // This works because Handle is Copy
    }

    pub fn iter_pass(&self, pass: RenderPass) -> impl Iterator<Item = (Handle<Mesh>, &[InstanceData])> + '_ {
        self.batches
            .iter()
            .filter(move |(key, _)| key.pass == pass)
            .map(|(key, instances)| (key.mesh, instances.as_slice()))  // This works because Handle is Copy
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