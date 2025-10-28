use crate::project::{normalize_absolute_path, resolve_project_path};
use crate::scene::assets::{SceneAsset, SceneMaterialHandle, SerializedMaterial};
use log::{error, warn};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::ErrorKind;
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
    pub data: SerializedMaterial,
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

            let Some(material_data) = entity.material_data.as_ref() else {
                continue;
            };

            let normalized_source = normalize_gltf_source(source, project_root);
            let binding = MaterialBinding {
                data: material_data.clone(),
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

    pub fn merge(
        &mut self,
        fresh: GltfMaterialRegistry,
        saved: &GltfMaterialRegistry,
        project_root: &Path,
    ) {
        for (key, mut binding) in fresh.bindings.into_iter() {
            let mut loaded_from_disk = false;

            if let Some(handle) = binding.handle.as_ref() {
                if let Some(overridden) = load_material_from_disk(handle, project_root) {
                    binding.data = overridden;
                    loaded_from_disk = true;
                }
            }

            if !loaded_from_disk {
                if let Some(saved_binding) = saved.lookup_binding(&key) {
                    let mut fallback = saved_binding.clone();

                    if let Some(handle) = fallback.handle.as_ref() {
                        if let Some(overridden) = load_material_from_disk(handle, project_root) {
                            fallback.data = overridden;
                        }
                    }

                    binding = fallback;
                }
            }

            self.bindings.insert(key, binding);
        }
    }

    pub fn apply_to_asset(&self, asset: &mut SceneAsset, project_root: &Path) {
        let mut written_paths: HashSet<PathBuf> = HashSet::new();

        for entity in &mut asset.entities {
            let Some(source) = entity.gltf_source.clone() else {
                continue;
            };

            let normalized_source = normalize_gltf_source(&source, project_root);
            let key = GltfMaterialKey::new(
                normalized_source,
                entity.gltf_node,
                entity.gltf_primitive,
                entity.gltf_material,
            );

            let Some(binding) = self.lookup_binding(&key) else {
                if let Some(material_index) = entity.gltf_material {
                    if entity.material_data.is_some() {
                        warn!(
                            "Missing reimported material for {:?} (material index {})",
                            source, material_index
                        );
                    }
                }
                continue;
            };

            entity.material_data = Some(binding.data.clone());

            if let Some(handle) = binding.handle.as_ref() {
                if !handle.path().as_os_str().is_empty() {
                    entity.material = Some(handle.clone());
                }
            }

            if let Some(handle) = entity.material.as_ref() {
                write_material_to_disk(handle, binding, project_root, &mut written_paths);
            }
        }
    }

    fn lookup_binding(&self, key: &GltfMaterialKey) -> Option<&MaterialBinding> {
        if let (Some(node), Some(primitive)) = (key.node, key.primitive) {
            let primitive_key =
                GltfMaterialKey::new(key.source.clone(), Some(node), Some(primitive), None);
            if let Some(binding) = self.bindings.get(&primitive_key) {
                return Some(binding);
            }
        }

        if let Some(material_index) = key.material_index {
            let material_key =
                GltfMaterialKey::new(key.source.clone(), None, None, Some(material_index));
            if let Some(binding) = self.bindings.get(&material_key) {
                return Some(binding);
            }
        }

        None
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

fn load_material_from_disk(
    handle: &SceneMaterialHandle,
    project_root: &Path,
) -> Option<SerializedMaterial> {
    let material_path = handle.path();
    if material_path.as_os_str().is_empty() {
        return None;
    }

    let absolute_path = if material_path.is_absolute() {
        material_path.to_path_buf()
    } else {
        project_root.join(material_path)
    };

    match fs::read(&absolute_path) {
        Ok(bytes) => match serde_json::from_slice(&bytes) {
            Ok(material) => Some(material),
            Err(err) => {
                warn!(
                    "Failed to deserialize material asset {:?}: {}",
                    absolute_path, err
                );
                None
            }
        },
        Err(err) => {
            if err.kind() != ErrorKind::NotFound {
                warn!("Failed to read material asset {:?}: {}", absolute_path, err);
            }
            None
        }
    }
}

fn write_material_to_disk(
    handle: &SceneMaterialHandle,
    binding: &MaterialBinding,
    project_root: &Path,
    written_paths: &mut HashSet<PathBuf>,
) {
    let material_path = handle.path();
    if material_path.as_os_str().is_empty() {
        return;
    }

    let absolute_path = if material_path.is_absolute() {
        material_path.to_path_buf()
    } else {
        project_root.join(material_path)
    };

    if !written_paths.insert(absolute_path.clone()) {
        return;
    }

    if let Some(parent) = absolute_path.parent() {
        if let Err(err) = fs::create_dir_all(parent) {
            error!("Failed to create material directory {:?}: {}", parent, err);
            return;
        }
    }

    match serde_json::to_vec_pretty(&binding.data) {
        Ok(json) => {
            let needs_write = match fs::read(&absolute_path) {
                Ok(existing) => existing != json,
                Err(err) => {
                    if err.kind() != ErrorKind::NotFound {
                        warn!(
                            "Failed to read existing material asset {:?}: {}",
                            absolute_path, err
                        );
                    }
                    true
                }
            };

            if needs_write {
                if let Err(err) = fs::write(&absolute_path, &json) {
                    error!(
                        "Failed to update material asset {:?}: {}",
                        absolute_path, err
                    );
                }
            }
        }
        Err(err) => {
            error!(
                "Failed to serialize material for {:?}: {}",
                material_path, err
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::assets::{
        SceneAsset, SceneAssetEntity, SceneMaterialHandle, SerializedTransform,
    };

    fn dummy_material(marker: u8) -> SerializedMaterial {
        SerializedMaterial {
            base_color: [marker, 0, 0, 255],
            flags: 0,
            base_color_texture: Default::default(),
            metallic_roughness_texture: Default::default(),
            normal_texture: Default::default(),
            emissive_texture: Default::default(),
            occlusion_texture: Default::default(),
            metallic_factor: 0,
            roughness_factor: 0,
            emissive_strength: 0,
        }
    }

    fn build_asset() -> SceneAsset {
        SceneAsset::builder("test")
            .add_entity(
                SceneAssetEntity::builder(SerializedTransform::identity())
                    .with_gltf_source(PathBuf::from("content/models/sample/scene.gltf"))
                    .with_gltf_node(4)
                    .with_gltf_primitive(2)
                    .with_gltf_material(7)
                    .with_material(
                        SceneMaterialHandle::new(PathBuf::from("materials/primitive.mat.json")),
                        Some(dummy_material(1)),
                    )
                    .build(),
            )
            .build()
    }

    #[test]
    fn lookup_prefers_node_primitive_keys() {
        let project_root = Path::new(".");
        let registry = GltfMaterialRegistry::collect_from_asset(&build_asset(), project_root);

        let source =
            normalize_gltf_source(Path::new("content/models/sample/scene.gltf"), project_root);
        let key = GltfMaterialKey::new(source, Some(4), Some(2), Some(7));
        let binding = registry.lookup_binding(&key).expect("binding");
        assert_eq!(binding.data.base_color[0], 1);
    }

    #[test]
    fn lookup_falls_back_to_material_index() {
        let project_root = Path::new(".");
        let asset = SceneAsset::builder("test")
            .add_entity(
                SceneAssetEntity::builder(SerializedTransform::identity())
                    .with_gltf_source(PathBuf::from("content/models/sample/scene.gltf"))
                    .with_gltf_material(3)
                    .with_material(
                        SceneMaterialHandle::new(PathBuf::from("materials/fallback.mat.json")),
                        Some(dummy_material(5)),
                    )
                    .build(),
            )
            .build();

        let registry = GltfMaterialRegistry::collect_from_asset(&asset, project_root);
        let source =
            normalize_gltf_source(Path::new("content/models/sample/scene.gltf"), project_root);
        let key = GltfMaterialKey::new(source, Some(1), Some(0), Some(3));
        let binding = registry.lookup_binding(&key).expect("fallback binding");
        assert_eq!(binding.data.base_color[0], 5);
    }
}
