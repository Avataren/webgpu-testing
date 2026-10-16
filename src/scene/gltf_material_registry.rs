use crate::project::normalize_absolute_path;
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

/// The absolute path of a glTF source, relative ones taken from `project_root`: the
/// project the asset belongs to, which need not be the active one (opening a project
/// persists its scenes before the active root moves to it, and the global root then
/// sent their import metadata into the previous project).
fn normalize_gltf_source(source: &Path, project_root: &Path) -> PathBuf {
    let combined = if source.is_absolute() {
        source.to_path_buf()
    } else {
        project_root.join(source)
    };
    let canonical = std::fs::canonicalize(&combined).unwrap_or(combined);
    normalize_absolute_path(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources_resolve_against_the_assets_own_project() {
        let root = Path::new("/projects/character-test");
        assert_eq!(
            normalize_gltf_source(Path::new("content/gltf/Soldier/Soldier.glb.import"), root),
            PathBuf::from("/projects/character-test/content/gltf/Soldier/Soldier.glb.import")
        );
        let absolute = Path::new("/elsewhere/robot.glb");
        assert_eq!(normalize_gltf_source(absolute, root), absolute);
    }
}
