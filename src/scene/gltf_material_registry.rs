use crate::project::{normalize_absolute_path, resolve_project_path};
use crate::scene::assets::{SceneAsset, SceneMaterialHandle};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GltfMaterialKey {
    pub source: PathBuf,
    pub node: Option<usize>,
    pub primitive: Option<usize>,
    pub material_index: Option<usize>,
}

impl GltfMaterialKey {
    pub fn new(
        source: PathBuf,
        node: Option<usize>,
        primitive: Option<usize>,
        material_index: Option<usize>,
    ) -> Self {
        Self {
            source,
            node,
            primitive,
            material_index,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MaterialBinding {
    pub handle: Option<SceneMaterialHandle>,
}

#[derive(Debug, Default, Clone)]
pub struct GltfMaterialRegistry {
    bindings: HashMap<GltfMaterialKey, MaterialBinding>,
}

impl GltfMaterialRegistry {
    pub fn collect_from_asset(asset: &SceneAsset, project_root: &Path) -> Self {
        let mut registry = Self::default();

        for entity in &asset.entities {
            let Some(source) = entity.gltf_source.as_ref() else {
                continue;
            };

            let normalized_source = normalize_gltf_source(source, project_root);
            let binding = MaterialBinding {
                handle: entity.material.clone(),
            };

            if let (Some(node_index), Some(primitive_index)) =
                (entity.gltf_node, entity.gltf_primitive)
            {
                registry.bindings.insert(
                    GltfMaterialKey::new(
                        normalized_source.clone(),
                        Some(node_index),
                        Some(primitive_index),
                        None,
                    ),
                    binding.clone(),
                );

                if let Some(material_index) = entity.gltf_material {
                    registry.bindings.insert(
                        GltfMaterialKey::new(
                            normalized_source.clone(),
                            Some(node_index),
                            Some(primitive_index),
                            Some(material_index),
                        ),
                        binding.clone(),
                    );
                }
            }

            if let Some(material_index) = entity.gltf_material {
                registry.bindings.insert(
                    GltfMaterialKey::new(
                        normalized_source.clone(),
                        None,
                        None,
                        Some(material_index),
                    ),
                    binding.clone(),
                );
            }
        }

        registry
    }

    pub fn iter_bindings(&self) -> impl Iterator<Item = (&GltfMaterialKey, &MaterialBinding)> {
        self.bindings.iter()
    }
}

fn normalize_gltf_source(source: &Path, project_root: &Path) -> PathBuf {
    let resolved = resolve_project_path(source);
    let canonical = if resolved.is_absolute() {
        std::fs::canonicalize(&resolved).unwrap_or(resolved)
    } else {
        let combined = project_root.join(&resolved);
        std::fs::canonicalize(&combined).unwrap_or(combined)
    };

    normalize_absolute_path(canonical)
}
