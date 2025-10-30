pub mod cache;
pub mod handle;
pub mod material;
pub mod mesh;

pub use cache::AssetCache;
pub use handle::Handle;
pub use material::{
    AssetTypeTag, MaterialAsset, MaterialKind, MaterialParameterMetadata, MaterialTextureReference,
    MaterialTextureSlot, ShaderMaterialMetadata,
};
pub use mesh::{Mesh, MeshData};

use crate::renderer::material::Material;
use crate::renderer::primitives::PrimitiveMeshDescriptor;
use crate::renderer::texture::{
    DEFAULT_METALLIC_ROUGHNESS_TEXTURE_INDEX, DEFAULT_NORMAL_TEXTURE_INDEX,
    DEFAULT_WHITE_TEXTURE_INDEX,
};
use crate::renderer::Texture;
use crate::scene::loader::SceneImportDevice;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct Assets {
    pub meshes: AssetCache<Mesh>,
    pub textures: AssetCache<Texture>,
    pub materials: AssetCache<MaterialAsset>,
    primitive_meshes: HashMap<PrimitiveMeshDescriptor, Handle<Mesh>>,
    material_paths: HashMap<PathBuf, Handle<MaterialAsset>>,
    texture_paths: HashMap<PathBuf, Handle<Texture>>,
    default_material: Handle<MaterialAsset>,
}

impl Assets {
    pub fn new() -> Self {
        let meshes = AssetCache::new();
        let textures = AssetCache::new();
        let mut materials = AssetCache::new();
        let default_material = materials.insert(MaterialAsset::from_material(
            Material::pbr(),
            PathBuf::new(),
        ));

        Self {
            meshes,
            textures,
            materials,
            primitive_meshes: HashMap::new(),
            material_paths: HashMap::new(),
            texture_paths: HashMap::new(),
            default_material,
        }
    }

    /// Creates a new shader material asset using the default WGSL template and inserts it.
    pub fn create_shader_material(
        &mut self,
        material: Material,
        canonical_path: PathBuf,
    ) -> Handle<MaterialAsset> {
        self.insert_material_asset(MaterialAsset::shader(material, canonical_path))
    }

    pub fn ensure_primitive_mesh(
        &mut self,
        renderer: &mut dyn SceneImportDevice,
        descriptor: PrimitiveMeshDescriptor,
    ) -> Handle<Mesh> {
        if let Some(&handle) = self.primitive_meshes.get(&descriptor) {
            return handle;
        }

        let data = descriptor.mesh_data();
        let mesh = renderer.create_mesh(&data.vertices, &data.indices);
        let handle = self.meshes.insert(mesh);
        self.primitive_meshes.insert(descriptor, handle);
        handle
    }

    pub fn primitive_mesh_handle(
        &self,
        descriptor: PrimitiveMeshDescriptor,
    ) -> Option<Handle<Mesh>> {
        self.primitive_meshes.get(&descriptor).copied()
    }

    pub fn get_or_load_material<F, E>(
        &mut self,
        path: impl AsRef<Path>,
        mut loader: F,
    ) -> Result<Handle<MaterialAsset>, E>
    where
        F: FnMut(&Path) -> Result<MaterialAsset, E>,
    {
        let canonical_path = Self::canonicalize_path(path.as_ref());

        if let Some(handle) = self.material_paths.get(&canonical_path) {
            return Ok(*handle);
        }

        let mut asset = loader(&canonical_path)?;
        if asset.canonical_path() != canonical_path.as_path() {
            asset.set_canonical_path(canonical_path.clone());
        }

        Ok(self.insert_material_asset(asset))
    }

    pub fn insert_material_asset(&mut self, mut asset: MaterialAsset) -> Handle<MaterialAsset> {
        if asset.canonical_path().as_os_str().is_empty() {
            return self.materials.insert(asset);
        }

        let canonical_path = Self::canonicalize_path(asset.canonical_path());
        if let Some(handle) = self.material_paths.get(&canonical_path) {
            return *handle;
        }

        if asset.canonical_path() != canonical_path.as_path() {
            asset.set_canonical_path(canonical_path.clone());
        }

        let handle = self.materials.insert(asset);
        self.material_paths.insert(canonical_path, handle);
        handle
    }

    pub fn material_handle_for_path(
        &self,
        path: impl AsRef<Path>,
    ) -> Option<Handle<MaterialAsset>> {
        let canonical_path = Self::canonicalize_path(path.as_ref());
        self.material_paths.get(&canonical_path).copied()
    }

    pub fn material(&self, handle: Handle<MaterialAsset>) -> Option<&MaterialAsset> {
        self.materials.get(handle)
    }

    pub fn material_mut(&mut self, handle: Handle<MaterialAsset>) -> Option<&mut MaterialAsset> {
        self.materials.get_mut(handle)
    }

    pub fn default_material_handle(&self) -> Handle<MaterialAsset> {
        self.default_material
    }

    fn canonicalize_path(path: &Path) -> PathBuf {
        path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
    }

    pub fn register_texture_path(&mut self, handle: Handle<Texture>, path: PathBuf) {
        let canonical = Self::canonicalize_path(&path);
        self.texture_paths.insert(canonical, handle);
    }

    pub fn texture_handle_for_path(&self, path: impl AsRef<Path>) -> Option<Handle<Texture>> {
        let canonical_path = Self::canonicalize_path(path.as_ref());
        self.texture_paths.get(&canonical_path).copied()
    }

    pub fn get_or_load_texture_with<F>(
        &mut self,
        path: impl AsRef<Path>,
        mut loader: F,
    ) -> Result<Handle<Texture>, String>
    where
        F: FnMut(&Path) -> Result<Texture, String>,
    {
        let canonical_path = Self::canonicalize_path(path.as_ref());

        if let Some(handle) = self.texture_paths.get(&canonical_path) {
            return Ok(*handle);
        }

        let texture = loader(&canonical_path)?;
        let handle = self.textures.insert(texture);
        self.texture_paths.insert(canonical_path, handle);
        Ok(handle)
    }

    pub fn get_or_load_texture(
        &mut self,
        renderer: &mut dyn SceneImportDevice,
        path: impl AsRef<Path>,
        is_srgb: bool,
    ) -> Result<Handle<Texture>, String> {
        self.get_or_load_texture_with(path, |path| {
            Texture::from_path(renderer.device(), renderer.queue(), path, is_srgb)
        })
    }

    pub fn resolve_texture_index(
        &mut self,
        slot: MaterialTextureSlot,
        path: Option<&Path>,
        mut renderer: Option<&mut dyn SceneImportDevice>,
        is_srgb: bool,
    ) -> u32 {
        if let Some(path) = path {
            let canonical = Self::canonicalize_path(path);

            if let Some(handle) = self.texture_paths.get(&canonical) {
                return handle.index() as u32;
            }

            if let Some(renderer) = renderer.as_mut() {
                match self.get_or_load_texture(&mut **renderer, &canonical, is_srgb) {
                    Ok(handle) => return handle.index() as u32,
                    Err(err) => {
                        log::warn!(
                            "Failed to load texture {:?} for {:?} slot: {}",
                            canonical,
                            slot,
                            err
                        );
                    }
                }
            } else if !canonical.exists() {
                log::warn!(
                    "Texture {:?} for {:?} slot not found; using default",
                    canonical,
                    slot
                );
            } else {
                log::warn!(
                    "Renderer unavailable; cannot load texture {:?} for {:?} slot",
                    canonical,
                    slot
                );
            }
        }

        Self::default_texture_index(slot)
    }

    pub fn default_texture_index(slot: MaterialTextureSlot) -> u32 {
        match slot {
            MaterialTextureSlot::BaseColor => DEFAULT_WHITE_TEXTURE_INDEX,
            MaterialTextureSlot::MetallicRoughness => DEFAULT_METALLIC_ROUGHNESS_TEXTURE_INDEX,
            MaterialTextureSlot::Normal => DEFAULT_NORMAL_TEXTURE_INDEX,
            MaterialTextureSlot::Emissive => DEFAULT_WHITE_TEXTURE_INDEX,
            MaterialTextureSlot::Occlusion => DEFAULT_WHITE_TEXTURE_INDEX,
        }
    }
}

impl Default for Assets {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn resolve_texture_index_falls_back_to_default_when_missing() {
        let mut assets = Assets::new();
        let index = assets.resolve_texture_index(
            MaterialTextureSlot::BaseColor,
            Some(Path::new("missing_texture.png")),
            None,
            false,
        );

        assert_eq!(index, DEFAULT_WHITE_TEXTURE_INDEX);
    }
}
