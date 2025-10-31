use std::fs;
use std::path::Path;

use super::{SceneImportDevice, SceneLoader};
use crate::asset::{MaterialTextureSlot, MeshData};
use crate::scene::gltf_package::{PackagedGltfDescriptor, PackagedScene};
use crate::scene::{SceneAsset, SceneAssetBundle, SceneAssetResources};

pub(super) fn load_packaged_descriptor(
    descriptor_path: &Path,
    renderer: &mut impl SceneImportDevice,
    scale: f32,
) -> Result<SceneAssetBundle, String> {
    let descriptor_text = fs::read_to_string(descriptor_path).map_err(|err| {
        format!(
            "Failed to read packaged glTF descriptor {:?}: {}",
            descriptor_path, err
        )
    })?;

    let descriptor: PackagedGltfDescriptor =
        serde_json::from_str(&descriptor_text).map_err(|err| {
            format!(
                "Failed to parse packaged glTF descriptor {:?}: {}",
                descriptor_path, err
            )
        })?;

    if let Some(scene) = descriptor.scene.clone() {
        let parent = descriptor_path.parent().ok_or_else(|| {
            format!(
                "Packaged glTF descriptor {:?} is missing a parent directory",
                descriptor_path
            )
        })?;

        let scene_path = parent.join(&scene.json);
        let scene_json = fs::read_to_string(&scene_path)
            .map_err(|err| format!("Failed to read packaged scene {:?}: {}", scene_path, err))?;

        let mut asset = SceneAsset::from_json(&scene_json)
            .map_err(|err| format!("Failed to parse packaged scene {:?}: {}", scene_path, err))?;

        if !scene.meshes.is_empty() {
            asset.mesh_data = read_packaged_meshes(&scene, parent)?;
        }

        apply_packaged_dependencies(&mut asset, descriptor_path, &descriptor);

        Ok(SceneAssetBundle::new(
            asset,
            SceneAssetResources::new(Vec::new(), Vec::new()),
        ))
    } else if let Some(source) = descriptor.source.as_ref() {
        let parent = descriptor_path.parent().ok_or_else(|| {
            format!(
                "Packaged glTF descriptor {:?} is missing a parent directory",
                descriptor_path
            )
        })?;
        let forwarded = parent.join(source);
        SceneLoader::load_gltf_asset(forwarded, renderer, scale)
    } else {
        Err(format!(
            "Packaged glTF descriptor {:?} does not include scene data",
            descriptor_path
        ))
    }
}

fn read_packaged_meshes(scene: &PackagedScene, base_dir: &Path) -> Result<Vec<MeshData>, String> {
    if scene.meshes.is_empty() {
        return Ok(Vec::new());
    }

    let mut entries = scene.meshes.clone();
    entries.sort_by_key(|mesh| mesh.index);

    let mut slots: Vec<Option<MeshData>> = Vec::new();
    for mesh in entries {
        let mesh_path = base_dir.join(&mesh.path);
        let mesh_json = fs::read_to_string(&mesh_path)
            .map_err(|err| format!("Failed to read packaged mesh {:?}: {}", mesh_path, err))?;
        let data: MeshData = serde_json::from_str(&mesh_json)
            .map_err(|err| format!("Failed to parse packaged mesh {:?}: {}", mesh_path, err))?;

        if slots.len() <= mesh.index {
            slots.resize(mesh.index + 1, None);
        }
        slots[mesh.index] = Some(data);
    }

    Ok(slots
        .into_iter()
        .map(|entry| {
            entry.unwrap_or_else(|| MeshData {
                vertices: Vec::new(),
                indices: Vec::new(),
            })
        })
        .collect())
}

fn apply_packaged_dependencies(
    asset: &mut SceneAsset,
    descriptor_path: &Path,
    descriptor: &PackagedGltfDescriptor,
) {
    if descriptor.scene.is_none() {
        return;
    }

    let descriptor_relative = crate::project::active_project_root()
        .and_then(|root| {
            descriptor_path
                .strip_prefix(&root)
                .ok()
                .map(Path::to_path_buf)
        })
        .unwrap_or_else(|| descriptor_path.to_path_buf());

    for entity in &mut asset.entities {
        if entity.gltf_source.is_some() {
            entity.gltf_source = Some(descriptor_relative.clone());
        }

        if let Some(material_handle) = entity.material.as_mut() {
            let material_path = material_handle.path().to_path_buf();
            if material_path.is_absolute() {
                if let Some(relative) = crate::project::active_project_root().and_then(|root| {
                    material_path
                        .strip_prefix(&root)
                        .ok()
                        .map(Path::to_path_buf)
                }) {
                    material_handle.set_path(relative);
                }
            }
        }

        if let Some(material) = entity.material_data.as_mut() {
            for slot in MaterialTextureSlot::all() {
                let slot_data = match slot {
                    MaterialTextureSlot::BaseColor => &mut material.base_color_texture,
                    MaterialTextureSlot::MetallicRoughness => {
                        &mut material.metallic_roughness_texture
                    }
                    MaterialTextureSlot::Normal => &mut material.normal_texture,
                    MaterialTextureSlot::Emissive => &mut material.emissive_texture,
                    MaterialTextureSlot::Occlusion => &mut material.occlusion_texture,
                };

                if let Some(existing) = slot_data.path.as_ref() {
                    if existing.is_absolute() {
                        if let Some(relative) =
                            crate::project::active_project_root().and_then(|root| {
                                existing.strip_prefix(&root).ok().map(Path::to_path_buf)
                            })
                        {
                            slot_data.path = Some(relative);
                        }
                    }
                }
            }
        }
    }
}
