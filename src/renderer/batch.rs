// renderer/batch.rs (Smart version)
use super::material::Material;
use crate::renderer::internal::MaterialPipelineKey;
use crate::{
    asset::{Assets, Handle, MaterialAsset, MaterialKind, Mesh},
    renderer::PickId,
    scene::components::DepthState,
    scene::transform::Transform,
};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RenderPass {
    Opaque,      // Normal opaque geometry
    Transparent, // Alpha blended (needs sorting)
    Overlay,     // Draw last, typically with depth disabled
    Gizmo,       // Editor gizmo wireframes (rendered as lines)
    GizmoSolid,  // Editor gizmo icons / solid overlays
}

impl RenderPass {
    /// Returns true when instances in this pass should be sorted from back to
    /// front relative to the camera.  Transparent and overlay elements need
    /// back-to-front ordering so blending behaves as expected.
    pub fn requires_back_to_front_sort(self) -> bool {
        matches!(
            self,
            Self::Transparent | Self::Overlay | Self::Gizmo | Self::GizmoSolid
        )
    }

    /// Returns true when the pass intrinsically requires alpha blending.
    pub fn uses_alpha_blending(self) -> bool {
        matches!(
            self,
            Self::Transparent | Self::Overlay | Self::Gizmo | Self::GizmoSolid
        )
    }

    /// Sample count for the color attachment used by this pass.  Overlay
    /// passes are resolved directly into the swap chain, so MSAA is not used.
    pub fn color_sample_count(self, msaa_samples: u32) -> u32 {
        if matches!(self, Self::Overlay | Self::Gizmo | Self::GizmoSolid) {
            1
        } else {
            msaa_samples
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CullMode {
    Back,
    Front,
    None,
}

impl Default for CullMode {
    fn default() -> Self {
        Self::Back
    }
}

/// A single renderable object instance
pub struct RenderObject {
    pub mesh: Handle<Mesh>,
    pub material: Handle<MaterialAsset>,
    pub resolved_material: Option<Material>,
    pub transform: Transform, // Changed from Mat4
    pub depth_state: DepthState,
    pub force_overlay: bool,
    pub render_pass: Option<RenderPass>,
    pub instance_source: InstanceSource,
    pub gpu_index: Option<u32>,
    pub cull_mode: CullMode,
    pub pick_id: PickId,
}

#[derive(Debug, Clone, Copy)]
pub struct InstanceData {
    pub transform: Transform, // Changed from Mat4
    pub material_index: u32,
    pub source: InstanceSource,
    pub gpu_index: Option<u32>,
    pub pick_id: PickId,
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
    pub use_nearest_filtering: bool,
    pub cull_mode: CullMode,
}

/// Batching key - only splits by what ACTUALLY requires different draw calls
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct BatchKey {
    mesh: Handle<Mesh>,
    pass: RenderPass, // Only split if different pipeline needed
    depth_state: DepthState,
    source: InstanceSource,
    use_nearest_filtering: bool,
    cull_mode: CullMode,
    material_pipeline_key: MaterialPipelineKey,
}

/// Collects objects and batches by pipeline requirements
pub struct RenderBatcher {
    batches: HashMap<BatchKey, Vec<InstanceData>>,
    materials: Vec<Handle<MaterialAsset>>,
    resolved_materials: Vec<Material>,
    material_pipeline_keys: Vec<MaterialPipelineKey>,
    material_lookup: HashMap<MaterialCacheKey, u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum MaterialCacheKey {
    Pbr(Material),
    Shader(Handle<MaterialAsset>),
}

#[derive(Debug, thiserror::Error)]
pub enum BatchError {
    #[error("material handle {handle:?} is not present in the asset cache")]
    MissingMaterial { handle: Handle<MaterialAsset> },
}

impl RenderBatcher {
    pub fn new() -> Self {
        Self {
            batches: HashMap::new(),
            materials: Vec::new(),
            resolved_materials: Vec::new(),
            material_pipeline_keys: Vec::new(),
            material_lookup: HashMap::new(),
        }
    }

    /// Add an object to be rendered
    pub fn add(&mut self, obj: RenderObject, assets: &Assets) -> Result<(), BatchError> {
        let material_asset = assets.material(obj.material);

        let material = if let Some(resolved) = obj.resolved_material {
            resolved
        } else {
            material_asset
                .map(|asset| *asset.material())
                .ok_or(BatchError::MissingMaterial {
                    handle: obj.material,
                })?
        };

        let material_kind = material_asset
            .map(|asset| asset.kind().clone())
            .unwrap_or(MaterialKind::Pbr);

        let material_pipeline_key = match &material_kind {
            MaterialKind::Pbr => MaterialPipelineKey::Pbr,
            MaterialKind::Shader(_) => MaterialPipelineKey::Shader(obj.material),
        };

        // Determine which pass this object belongs to
        let pass = obj.render_pass.unwrap_or_else(|| {
            if obj.force_overlay {
                RenderPass::Overlay
            } else if material.requires_separate_pass() {
                RenderPass::Transparent
            } else {
                RenderPass::Opaque
            }
        });

        let key = BatchKey {
            mesh: obj.mesh,
            pass,
            depth_state: obj.depth_state,
            source: obj.instance_source,
            use_nearest_filtering: material.uses_nearest_filtering(),
            cull_mode: obj.cull_mode,
            material_pipeline_key,
        };

        let lookup_key = match material_pipeline_key {
            MaterialPipelineKey::Pbr => MaterialCacheKey::Pbr(material),
            MaterialPipelineKey::Shader(handle) => MaterialCacheKey::Shader(handle),
        };

        let material_index = *self.material_lookup.entry(lookup_key).or_insert_with(|| {
            let index = self.materials.len() as u32;
            self.materials.push(obj.material);
            self.resolved_materials.push(material);
            self.material_pipeline_keys.push(material_pipeline_key);
            index
        });

        self.batches.entry(key).or_default().push(InstanceData {
            transform: obj.transform,
            material_index,
            source: obj.instance_source,
            gpu_index: obj.gpu_index,
            pick_id: obj.pick_id,
        });

        Ok(())
    }

    /// Clear all batches
    pub fn clear(&mut self) {
        for batch in self.batches.values_mut() {
            batch.clear();
        }
        self.materials.clear();
        self.resolved_materials.clear();
        self.material_pipeline_keys.clear();
        self.material_lookup.clear();
    }

    pub fn iter(&self) -> impl Iterator<Item = Batch<'_>> {
        self.batches.iter().map(|(key, instances)| Batch {
            mesh: key.mesh,
            pass: key.pass,
            depth_state: key.depth_state,
            instances: instances.as_slice(),
            use_nearest_filtering: key.use_nearest_filtering,
            cull_mode: key.cull_mode,
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
                    use_nearest_filtering: key.use_nearest_filtering,
                    cull_mode: key.cull_mode,
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
        &self.resolved_materials
    }

    pub fn material_handles(&self) -> &[Handle<MaterialAsset>] {
        &self.materials
    }

    pub(crate) fn material_pipeline_keys(&self) -> &[MaterialPipelineKey] {
        &self.material_pipeline_keys
    }
}

impl Default for RenderBatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset::Assets;
    use std::path::PathBuf;

    #[test]
    fn shader_materials_use_distinct_pipeline_keys() {
        let mut batcher = RenderBatcher::new();
        let mut assets = Assets::new();
        let mesh = Handle::new(7);

        let pbr_handle = assets.default_material_handle();
        let shader_handle =
            assets.create_shader_material(Material::pbr(), PathBuf::from("shader_material.wgsl"));

        let make_object = |material| RenderObject {
            mesh,
            material,
            resolved_material: None,
            transform: Transform::IDENTITY,
            depth_state: DepthState::default(),
            force_overlay: false,
            render_pass: None,
            instance_source: InstanceSource::Cpu,
            gpu_index: None,
            cull_mode: CullMode::Back,
            pick_id: 0,
        };

        batcher
            .add(make_object(pbr_handle), &assets)
            .expect("pbr material valid");
        batcher
            .add(make_object(shader_handle), &assets)
            .expect("shader material valid");

        let keys = batcher.material_pipeline_keys();
        assert!(keys.contains(&MaterialPipelineKey::Pbr));
        assert!(keys.contains(&MaterialPipelineKey::Shader(shader_handle)));

        let mut batch_keys: Vec<MaterialPipelineKey> = batcher
            .iter()
            .filter_map(|batch| {
                batch
                    .instances
                    .first()
                    .and_then(|inst| keys.get(inst.material_index as usize))
                    .copied()
            })
            .collect();
        batch_keys.sort_by_key(|key| match key {
            MaterialPipelineKey::Pbr => 0,
            MaterialPipelineKey::Shader(_) => 1,
        });

        assert_eq!(batch_keys.len(), 2);
        assert_eq!(batch_keys[0], MaterialPipelineKey::Pbr);
        assert_eq!(batch_keys[1], MaterialPipelineKey::Shader(shader_handle));
    }
}
