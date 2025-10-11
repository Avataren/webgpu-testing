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
    pub instance_source: InstanceSource,
    pub gpu_index: Option<u32>,
}

#[derive(Debug, Clone, Copy)]
pub struct InstanceData {
    pub transform: Transform, // Changed from Mat4
    pub material_index: u32,
    pub source: InstanceSource,
    pub gpu_index: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InstanceSource {
    Cpu,
    Gpu,
}

impl Default for InstanceSource {
    fn default() -> Self {
        Self::Cpu
    }
}

pub struct Batch<'a> {
    pub mesh: Handle<Mesh>,
    pub pass: RenderPass,
    pub depth_state: DepthState,
    pub instances: &'a [InstanceData],
    pub materials: &'a [Material],
}

/// Batching key - only splits by what ACTUALLY requires different draw calls
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct BatchKey {
    mesh: Handle<Mesh>,
    pass: RenderPass, // Only split if different pipeline needed
    depth_state: DepthState,
    source: InstanceSource,
}

/// Collects objects and batches by pipeline requirements
pub struct RenderBatcher {
    batches: HashMap<BatchKey, Vec<InstanceData>>,
    materials: Vec<Material>,
    material_lookup: HashMap<Material, u32>,
}

impl RenderBatcher {
    pub fn new() -> Self {
        Self {
            batches: HashMap::new(),
            materials: Vec::new(),
            material_lookup: HashMap::new(),
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
            source: obj.instance_source,
        };

        let material_index = *self.material_lookup.entry(obj.material).or_insert_with(|| {
            let index = self.materials.len() as u32;
            self.materials.push(obj.material);
            index
        });

        self.batches.entry(key).or_default().push(InstanceData {
            transform: obj.transform,
            material_index,
            source: obj.instance_source,
            gpu_index: obj.gpu_index,
        });
    }

    /// Clear all batches
    pub fn clear(&mut self) {
        for batch in self.batches.values_mut() {
            batch.clear();
        }
        self.materials.clear();
        self.material_lookup.clear();
    }

    pub fn iter(&self) -> impl Iterator<Item = Batch<'_>> {
        self.batches.iter().map(|(key, instances)| Batch {
            mesh: key.mesh,
            pass: key.pass,
            depth_state: key.depth_state,
            instances: instances.as_slice(),
            materials: self.materials.as_slice(),
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
                    materials: self.materials.as_slice(),
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

    pub fn materials(&self) -> &[Material] {
        &self.materials
    }
}

impl Default for RenderBatcher {
    fn default() -> Self {
        Self::new()
    }
}
