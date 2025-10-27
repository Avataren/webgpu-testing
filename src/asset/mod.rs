pub mod cache;
pub mod handle;
pub mod material;
pub mod mesh;

pub use cache::AssetCache;
pub use handle::Handle;
pub use material::{AssetTypeTag, MaterialAsset, MaterialParameterMetadata};
pub use mesh::{Mesh, MeshData};

use crate::renderer::material::Material;
use crate::renderer::primitives::PrimitiveMeshDescriptor;
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
            default_material,
        }
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
}

impl Default for Assets {
    fn default() -> Self {
        Self::new()
    }
}
