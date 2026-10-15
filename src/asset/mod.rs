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
use crate::scene::skeletal::ClipLibrary;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone)]
pub struct Assets {
    pub meshes: AssetCache<Mesh>,
    pub textures: AssetCache<Texture>,
    pub materials: AssetCache<MaterialAsset>,
    primitive_meshes: HashMap<PrimitiveMeshDescriptor, Handle<Mesh>>,
    /// Meshes built at runtime (e.g. voxel models) by content key, so equal geometry
    /// shares one mesh.
    generated_meshes: HashMap<u64, Handle<Mesh>>,
    material_paths: HashMap<PathBuf, Handle<MaterialAsset>>,
    texture_paths: HashMap<PathBuf, Handle<Texture>>,
    /// Whether each texture loaded by path was sampled as sRGB, for reloading it.
    texture_srgb: HashMap<PathBuf, bool>,
    default_material: Handle<MaterialAsset>,
    /// Skeletal clip libraries by path, loaded on first use. Failures are cached too, so
    /// a missing file is reported once rather than read every frame.
    clip_libraries: HashMap<PathBuf, Result<Arc<ClipLibrary>, String>>,
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
            generated_meshes: HashMap::new(),
            material_paths: HashMap::new(),
            texture_paths: HashMap::new(),
            texture_srgb: HashMap::new(),
            default_material,
            clip_libraries: HashMap::new(),
        }
    }

    /// The clip library (animations) of a glTF/GLB file, loaded on first use.
    pub fn clip_library(&mut self, path: &Path) -> Result<Arc<ClipLibrary>, String> {
        self.clip_libraries
            .entry(path.to_path_buf())
            .or_insert_with(|| {
                let result = ClipLibrary::load(path).map(Arc::new);
                match &result {
                    Ok(library) => log::info!(
                        "Loaded {} animation clip(s) from {}",
                        library.clips.len(),
                        path.display()
                    ),
                    Err(err) => log::warn!("Animation library: {err}"),
                }
                result
            })
            .clone()
    }

    /// Forget a cached clip library (after the file changed); the next use reloads it.
    pub fn invalidate_clip_library(&mut self, path: &Path) {
        self.clip_libraries.remove(path);
    }

    /// Insert an already loaded library under `path` (tests, generated clips).
    pub fn insert_clip_library(&mut self, path: impl Into<PathBuf>, library: ClipLibrary) {
        self.clip_libraries
            .insert(path.into(), Ok(Arc::new(library)));
    }

    /// Creates a new shader material asset using the default WGSL template and inserts it.
    ///
    /// The generated material includes documentation about the available `// @include_*`
    /// markers so artists can enable lighting, shadows, or environment helpers directly in
    /// the WGSL source before authoring their shader code.
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

    /// The mesh for runtime-built geometry, uploading it the first time its key is seen.
    pub fn ensure_generated_mesh(
        &mut self,
        renderer: &mut dyn SceneImportDevice,
        mesh: &crate::procgen::GeneratedMesh,
    ) -> Handle<Mesh> {
        if let Some(&handle) = self.generated_meshes.get(&mesh.key) {
            return handle;
        }
        let handle = self
            .meshes
            .insert(renderer.create_mesh(&mesh.data.vertices, &mesh.data.indices));
        self.generated_meshes.insert(mesh.key, handle);
        handle
    }

    /// Release generated meshes whose handle is not in `in_use`. Returns how many were
    /// released.
    ///
    /// Scripts that rebuild geometry (destructible or streamed voxels) would otherwise
    /// keep every version they ever made.
    pub fn release_unused_generated_meshes(
        &mut self,
        in_use: &std::collections::HashSet<usize>,
    ) -> usize {
        let meshes = &mut self.meshes;
        let before = self.generated_meshes.len();
        self.generated_meshes.retain(|_, handle| {
            let keep = in_use.contains(&handle.index());
            if !keep {
                meshes.release(*handle);
            }
            keep
        });
        before - self.generated_meshes.len()
    }

    pub fn generated_mesh_count(&self) -> usize {
        self.generated_meshes.len()
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

    /// Release inline PBR materials (not loaded from a file, not shaders, not the
    /// default) whose handle is not in `in_use`. Returns how many were released.
    ///
    /// Every primitive gets its own inline material, so without this a game that spawns
    /// and despawns entities grows the material list without bound.
    pub fn release_unused_materials(&mut self, in_use: &std::collections::HashSet<usize>) -> usize {
        let unused: Vec<Handle<MaterialAsset>> = self
            .materials
            .iter()
            .filter(|(handle, asset)| {
                *handle != self.default_material
                    && !in_use.contains(&handle.index())
                    && asset.canonical_path().as_os_str().is_empty()
                    && matches!(asset.kind(), MaterialKind::Pbr)
            })
            .map(|(handle, _)| handle)
            .collect();
        for &handle in &unused {
            self.materials.release(handle);
        }
        unused.len()
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
        let handle = self.get_or_load_texture_with(&path, |path| {
            Texture::from_path(renderer.device(), renderer.queue(), path, is_srgb)
        })?;
        self.texture_srgb
            .entry(Self::canonicalize_path(path.as_ref()))
            .or_insert(is_srgb);
        Ok(handle)
    }

    /// Re-read textures from disk into their existing handles, so every material using
    /// them shows the new image. Reloads every texture loaded from a file, or only those
    /// under `filter` (a file or a directory). Returns each texture's path and the
    /// outcome. The renderer's texture bind group must be rebuilt afterwards.
    pub fn reload_textures(
        &mut self,
        renderer: &mut dyn SceneImportDevice,
        filter: Option<&Path>,
    ) -> Vec<(PathBuf, Result<(), String>)> {
        let filter = filter.map(Self::canonicalize_path);
        let mut paths: Vec<(PathBuf, Handle<Texture>)> = self
            .texture_paths
            .iter()
            .filter(|(path, _)| {
                filter
                    .as_ref()
                    .is_none_or(|filter| path.starts_with(filter))
            })
            .map(|(path, handle)| (path.clone(), *handle))
            .collect();
        paths.sort_by(|a, b| a.0.cmp(&b.0));
        paths
            .into_iter()
            .map(|(path, handle)| {
                let outcome = match self.texture_srgb.get(&path) {
                    None => Err("loaded as part of a model; re-import the model instead".into()),
                    Some(&is_srgb) => {
                        Texture::from_path(renderer.device(), renderer.queue(), &path, is_srgb)
                            .and_then(|texture| {
                                let slot = self
                                    .textures
                                    .get_mut(handle)
                                    .ok_or("texture handle is gone")?;
                                *slot = texture;
                                Ok(())
                            })
                    }
                };
                (path, outcome)
            })
            .collect()
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
