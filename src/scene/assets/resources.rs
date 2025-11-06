use super::core::SceneAsset;
use super::entity::{SceneMaterialHandle, load_material_asset_from_file};
use crate::scene::loader::SceneImportDevice;
use super::serialization::SerializedMaterial;
use crate::asset::{Assets, Handle, MaterialAsset, MaterialKind, MaterialTextureReference, MaterialTextureSlot, Mesh};
use crate::project::resolve_project_path;
use crate::renderer::{Material, Texture};
use std::collections::HashMap;
use std::convert::TryFrom;
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub struct SceneAssetResources {
    meshes: Vec<(usize, Mesh)>,
    textures: Vec<(u32, Texture)>,
}

pub(crate) fn determine_canonical_material_path(
    material_ref: Option<&SceneMaterialHandle>,
    gltf_source: Option<&Path>,
    gltf_material: Option<usize>,
) -> Option<PathBuf> {
    if let Some(material_ref) = material_ref {
        let path = material_ref.path();
        if path.as_os_str().is_empty() {
            None
        } else {
            Some(resolve_project_path(path))
        }
    } else if let (Some(source), Some(material_index)) = (gltf_source, gltf_material) {
        let resolved_source = resolve_project_path(source);
        Some(PathBuf::from(format!(
            "{}#material{}",
            resolved_source.display(),
            material_index
        )))
    } else {
        None
    }
}

pub(crate) fn canonicalize_material_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

pub(crate) fn with_import_device<R>(
    renderer: &mut Option<&mut dyn SceneImportDevice>,
    f: impl FnOnce(Option<&mut dyn SceneImportDevice>) -> R,
) -> R {
    match renderer.as_mut() {
        Some(device) => f(Some(&mut **device)),
        None => f(None),
    }
}

pub(crate) fn resolve_material_handle(
    material_ref: Option<&SceneMaterialHandle>,
    material_data: Option<&SerializedMaterial>,
    gltf_source: Option<&Path>,
    gltf_material: Option<usize>,
    renderer: &mut Option<&mut dyn SceneImportDevice>,
    assets: &mut Assets,
    cache: &mut HashMap<PathBuf, Handle<MaterialAsset>>,
) -> Option<Handle<MaterialAsset>> {
    let canonical_path =
        determine_canonical_material_path(material_ref, gltf_source, gltf_material);

    if let Some(path) = canonical_path.as_ref() {
        let key = canonicalize_material_path(path);

        if let Some(&handle) = cache.get(&key) {
            if let Some(serialized) = material_data {
                with_import_device(renderer, |device| {
                    apply_serialized_material_to_handle(assets, handle, serialized, device)
                });
            } else {
                with_import_device(renderer, |device| {
                    ensure_asset_textures_from_metadata(assets, handle, device)
                });
            }
            return Some(handle);
        }

        if let Some(handle) = assets.material_handle_for_path(path) {
            if let Some(serialized) = material_data {
                with_import_device(renderer, |device| {
                    apply_serialized_material_to_handle(assets, handle, serialized, device)
                });
            } else {
                with_import_device(renderer, |device| {
                    ensure_asset_textures_from_metadata(assets, handle, device)
                });
            }
            cache.insert(key, handle);
            return Some(handle);
        }
    }

    if let Some(path) = canonical_path.as_ref() {
        if path.exists() {
            match assets.get_or_load_material(path, load_material_asset_from_file) {
                Ok(handle) => {
                    if let Some(serialized) = material_data {
                        with_import_device(renderer, |device| {
                            apply_serialized_material_to_handle(assets, handle, serialized, device)
                        });
                    } else {
                        with_import_device(renderer, |device| {
                            ensure_asset_textures_from_metadata(assets, handle, device)
                        });
                    }

                    let key = canonicalize_material_path(path);
                    cache.insert(key, handle);
                    return Some(handle);
                }
                Err(err) => {
                    log::error!("Failed to load material asset {:?}: {}", path, err);
                }
            }
        }
    }

    if let Some(serialized) = material_data {
        if let Some(path) = canonical_path.clone() {
            let key = canonicalize_material_path(&path);
            let mut asset = MaterialAsset::from_material(Material::from(serialized.clone()), path);
            let (material, kind, references) = with_import_device(renderer, |device| {
                serialized.resolve_material(assets, device)
            });
            for (slot, reference) in references {
                if let Some(reference) = reference {
                    asset.set_texture_reference(slot, reference);
                } else {
                    asset.clear_texture_reference(slot);
                }
            }
            *asset.material_mut() = material;
            asset.set_kind(kind);
            let handle = assets.insert_material_asset(asset);
            cache.insert(key, handle);
            return Some(handle);
        } else {
            let mut asset =
                MaterialAsset::from_material(Material::from(serialized.clone()), PathBuf::new());
            let (material, kind, references) = with_import_device(renderer, |device| {
                serialized.resolve_material(assets, device)
            });
            for (slot, reference) in references {
                if let Some(reference) = reference {
                    asset.set_texture_reference(slot, reference);
                } else {
                    asset.clear_texture_reference(slot);
                }
            }
            *asset.material_mut() = material;
            asset.set_kind(kind);
            let handle = assets.insert_material_asset(asset);
            return Some(handle);
        }
    }

    if let Some(path) = canonical_path {
        log::warn!(
            "Material asset {:?} missing and no embedded data available",
            path
        );
    }

    let handle = assets.default_material_handle();
    with_import_device(renderer, |device| {
        ensure_asset_textures_from_metadata(assets, handle, device)
    });
    Some(handle)
}

pub(crate) fn update_material_asset_from_references(
    assets: &mut Assets,
    handle: Handle<MaterialAsset>,
    material: Material,
    kind: MaterialKind,
    references: Vec<(MaterialTextureSlot, Option<MaterialTextureReference>)>,
) {
    if let Some(asset) = assets.material_mut(handle) {
        *asset.material_mut() = material;
        asset.set_kind(kind);
        for (slot, reference) in references {
            if let Some(reference) = reference {
                asset.set_texture_reference(slot, reference);
            } else {
                asset.clear_texture_reference(slot);
            }
        }
    }
}

pub(crate) fn apply_serialized_material_to_handle(
    assets: &mut Assets,
    handle: Handle<MaterialAsset>,
    serialized: &SerializedMaterial,
    renderer: Option<&mut dyn SceneImportDevice>,
) {
    let (material, kind, references) = serialized.resolve_material(assets, renderer);
    update_material_asset_from_references(assets, handle, material, kind, references);
}

pub(crate) fn ensure_asset_textures_from_metadata(
    assets: &mut Assets,
    handle: Handle<MaterialAsset>,
    renderer: Option<&mut dyn SceneImportDevice>,
) {
    let serialized = if let Some(asset) = assets.material(handle) {
        SerializedMaterial::from_material_asset(asset)
    } else {
        return;
    };

    let (material, kind, references) = serialized.resolve_material(assets, renderer);
    update_material_asset_from_references(assets, handle, material, kind, references);
}

impl SceneAssetResources {
    pub fn new(meshes: Vec<(usize, Mesh)>, textures: Vec<(u32, Texture)>) -> Self {
        Self { meshes, textures }
    }

    pub fn builder() -> SceneAssetResourcesBuilder {
        SceneAssetResourcesBuilder::default()
    }

    pub fn is_empty(&self) -> bool {
        self.meshes.is_empty() && self.textures.is_empty()
    }

    pub fn add_mesh(&mut self, mesh: Mesh) {
        let next_index = next_mesh_index(&self.meshes);
        self.meshes.push((next_index, mesh));
    }

    pub fn add_mesh_with_index(&mut self, index: usize, mesh: Mesh) {
        self.meshes.push((index, mesh));
    }

    fn take_meshes(&mut self) -> Vec<(usize, Mesh)> {
        std::mem::take(&mut self.meshes)
    }

    fn take_textures(&mut self) -> Vec<(u32, Texture)> {
        std::mem::take(&mut self.textures)
    }
}

pub(crate) fn next_mesh_index(meshes: &[(usize, Mesh)]) -> usize {
    meshes
        .iter()
        .map(|(index, _)| *index)
        .max()
        .map_or(0, |index| index + 1)
}

#[derive(Debug, Default)]
pub struct SceneAssetResourcesBuilder {
    meshes: Vec<(usize, Mesh)>,
    textures: Vec<(u32, Texture)>,
}

impl SceneAssetResourcesBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_mesh(mut self, mesh: Mesh) -> Self {
        let index = next_mesh_index(&self.meshes);
        self.meshes.push((index, mesh));
        self
    }

    pub fn with_mesh_at(mut self, index: usize, mesh: Mesh) -> Self {
        self.meshes.push((index, mesh));
        self
    }

    pub fn with_texture(mut self, texture: Texture) -> Self {
        let index = self.textures.len();
        self.textures.push((index as u32, texture));
        self
    }

    pub fn add_mesh(&mut self, mesh: Mesh) {
        let index = next_mesh_index(&self.meshes);
        self.meshes.push((index, mesh));
    }

    pub fn add_mesh_with_index(&mut self, index: usize, mesh: Mesh) {
        self.meshes.push((index, mesh));
    }

    pub fn add_texture(&mut self, texture: Texture) {
        let index = self.textures.len();
        self.textures.push((index as u32, texture));
    }

    pub fn build(self) -> SceneAssetResources {
        SceneAssetResources::new(self.meshes, self.textures)
    }
}

#[derive(Debug, Default)]
pub struct ResourceRegistration {
    pub mesh_map: HashMap<usize, usize>,
    pub texture_map: HashMap<u32, u32>,
    pub textures_changed: bool,
}

impl ResourceRegistration {
    pub fn textures_changed(&self) -> bool {
        self.textures_changed
    }
}

#[derive(Debug)]
pub struct SceneAssetBundle {
    pub asset: SceneAsset,
    resources: SceneAssetResources,
    resources_registered: bool,
}

impl SceneAssetBundle {
    pub fn new(asset: SceneAsset, resources: SceneAssetResources) -> Self {
        Self {
            asset,
            resources,
            resources_registered: false,
        }
    }

    pub fn is_registered(&self) -> bool {
        self.resources_registered
    }

    pub fn register_resources<R: SceneImportDevice>(
        &mut self,
        renderer: &mut R,
        assets: &mut Assets,
    ) -> ResourceRegistration {
        if self.resources_registered {
            return ResourceRegistration::default();
        }

        for entity in &mut self.asset.entities {
            if let Some(descriptor) = entity.primitive_mesh {
                let handle = assets.ensure_primitive_mesh(renderer, descriptor);
                entity.mesh_handle = Some(handle.index());
            }
        }

        if self.resources.meshes.is_empty() && !self.asset.mesh_data.is_empty() {
            for (local_index, data) in self.asset.mesh_data.iter().cloned().enumerate() {
                let mesh = Mesh::from_data(renderer.device(), data);
                self.resources.add_mesh_with_index(local_index, mesh);
            }
        }

        let mut mesh_map = std::collections::HashMap::new();
        for (local_index, mesh) in self.resources.take_meshes().into_iter() {
            let handle = assets.meshes.insert(mesh);
            mesh_map.insert(local_index, handle.index());
        }

        let mut textures_added = false;
        let mut texture_map = std::collections::HashMap::new();
        for (original_index, texture) in self.resources.take_textures() {
            let handle = assets.textures.insert(texture);
            if let Ok(new_index) = u32::try_from(handle.index()) {
                texture_map.insert(original_index, new_index);
            } else {
                log::warn!(
                    "Texture index {} exceeds u32::MAX; skipping texture remap",
                    handle.index()
                );
            }
            textures_added = true;
        }

        // self.asset
        //     .apply_resource_mappings_from_maps(&mesh_map, &texture_map);

        // DEBUG: Log the mappings
        log::info!("=== GLTF IMPORT DEBUG ===");
        log::info!("Mesh map: {:?}", mesh_map);
        log::info!("Texture map: {:?}", texture_map);

        // DEBUG: Log what's in the asset before remapping
        for (i, entity) in self.asset.entities.iter().enumerate() {
            if let Some(mesh_handle) = entity.mesh_handle {
                log::info!("Entity {}: mesh_handle={}", i, mesh_handle);
            }
            if let Some(material) = &entity.material_data {
                log::info!(
                    "Entity {}: base_color_texture={:?}, metallic_roughness_texture={:?}, normal_texture={:?}",
                    i,
                    material.base_color_texture,
                    material.metallic_roughness_texture,
                    material.normal_texture,
                );
            }
        }

        // Apply both mappings to remap local indices to global indices
        self.asset
            .apply_resource_mappings_from_maps(&mesh_map, &texture_map);

        // DEBUG: Log what's in the asset after remapping
        log::info!("=== AFTER REMAPPING ===");
        for (i, entity) in self.asset.entities.iter().enumerate() {
            if let Some(mesh_handle) = entity.mesh_handle {
                log::info!("Entity {}: mesh_handle={}", i, mesh_handle);
            }
            if let Some(material) = &entity.material_data {
                log::info!(
                    "Entity {}: base_color_texture={:?}, metallic_roughness_texture={:?}, normal_texture={:?}",
                    i,
                    material.base_color_texture,
                    material.metallic_roughness_texture,
                    material.normal_texture,
                );
            }
        }

        log::info!("=== MESH-MATERIAL ASSOCIATIONS ===");
        for (i, entity) in self.asset.entities.iter().enumerate() {
            if let Some(mesh) = entity.mesh_handle {
                if let Some(mat) = &entity.material_data {
                    log::info!(
                        "Entity {}: mesh={}, base_texture={:?}, metallic_texture={:?}, normal_texture={:?}, gltf_mat_idx={:?}",
                        i,
                        mesh,
                        mat.base_color_texture,
                        mat.metallic_roughness_texture,
                        mat.normal_texture,
                        entity.gltf_material,
                    );
                } else {
                    log::info!("Entity {}: mesh={}, NO MATERIAL", i, mesh);
                }
            }
        }

        self.resources_registered = true;
        ResourceRegistration {
            mesh_map,
            texture_map,
            textures_changed: textures_added,
        }
    }
}
