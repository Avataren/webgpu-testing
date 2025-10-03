// renderer/batch.rs
use super::assets::{Handle, Mesh};
use super::material::Material;
use glam::Mat4;
use std::collections::HashMap;

/// Uniquely identifies a batch (mesh + material combination)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct BatchKey {
    mesh: Handle<Mesh>,
    material: Material,
}

/// A single renderable object instance
pub struct RenderObject {
    pub mesh: Handle<Mesh>,
    pub material: Material,
    pub transform: Mat4,
}

/// Collects objects and automatically batches them by mesh+material
pub struct RenderBatcher {
    batches: HashMap<BatchKey, Vec<Mat4>>,
}

impl RenderBatcher {
    pub fn new() -> Self {
        Self {
            batches: HashMap::new(),
        }
    }

    /// Add an object to be rendered
    pub fn add(&mut self, obj: RenderObject) {
        let key = BatchKey {
            mesh: obj.mesh,
            material: obj.material,
        };
        self.batches
            .entry(key)
            .or_insert_with(Vec::new)
            .push(obj.transform);
    }

    /// Clear all batches (call at start of each frame)
    pub fn clear(&mut self) {
        for batch in self.batches.values_mut() {
            batch.clear();
        }
    }

    /// Iterate over all batches
    pub fn iter(&self) -> impl Iterator<Item = (Handle<Mesh>, Material, &[Mat4])> {
        self.batches.iter().map(|(key, transforms)| {
            (key.mesh, key.material, transforms.as_slice())
        })
    }

    /// Get the total number of instances across all batches
    pub fn instance_count(&self) -> usize {
        self.batches.values().map(|v| v.len()).sum()
    }

    /// Get the number of unique batches
    pub fn batch_count(&self) -> usize {
        self.batches.len()
    }
}

impl Default for RenderBatcher {
    fn default() -> Self {
        Self::new()
    }
}