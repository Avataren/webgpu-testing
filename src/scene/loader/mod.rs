use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::assets::{ImportedGltfMeta, SceneMaterialHandle};
use crate::asset::Mesh;
use crate::renderer::{material::MaterialFlags, Renderer, Texture, Vertex};
use crate::scene::{Scene, SceneAssetBundle, SceneAssetResources};

mod animations;
mod materials;
mod nodes;
mod packaged;
mod textures;

pub use crate::scene::gltf_package::PackagedScene;

pub struct SceneLoader;

pub trait SceneImportDevice {
    fn device(&self) -> &wgpu::Device;
    fn queue(&self) -> &wgpu::Queue;
    fn create_mesh(&mut self, vertices: &[Vertex], indices: &[u32]) -> Mesh;
}

impl SceneImportDevice for Renderer {
    fn device(&self) -> &wgpu::Device {
        self.get_device()
    }

    fn queue(&self) -> &wgpu::Queue {
        self.get_queue()
    }

    fn create_mesh(&mut self, vertices: &[Vertex], indices: &[u32]) -> Mesh {
        Renderer::create_mesh(self, vertices, indices)
    }
}

pub(crate) struct SceneLoadContext<'a, D: SceneImportDevice> {
    pub(super) scene: &'a mut Scene,
    pub(super) renderer: &'a mut D,
    pub(super) scale: f32,
    pub(super) source_path: PathBuf,
    pub(super) base_dir: PathBuf,
    pub(super) material_meta: Option<&'a ImportedGltfMeta>,
}

impl<'a, D: SceneImportDevice> SceneLoadContext<'a, D> {
    fn new(
        path: &Path,
        scene: &'a mut Scene,
        renderer: &'a mut D,
        scale: f32,
        material_meta: Option<&'a ImportedGltfMeta>,
    ) -> Self {
        let base_dir = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));

        Self {
            scene,
            renderer,
            scale,
            source_path: path.to_path_buf(),
            base_dir,
            material_meta,
        }
    }

    pub(super) fn source_path(&self) -> &Path {
        &self.source_path
    }

    pub(super) fn base_dir(&self) -> &Path {
        &self.base_dir
    }
}

pub(super) type GltfImport = (
    gltf::Document,
    Vec<gltf::buffer::Data>,
    Vec<gltf::image::Data>,
);

impl SceneLoader {
    pub fn load_gltf(
        path: impl AsRef<Path>,
        scene: &mut Scene,
        renderer: &mut impl SceneImportDevice,
        scale: f32,
        material_meta: Option<&ImportedGltfMeta>,
    ) -> Result<(), String> {
        let path = path.as_ref();
        let mut ctx = SceneLoadContext::new(path, scene, renderer, scale, material_meta);

        log::info!("=== Loading glTF: {:?} ===", path);

        #[cfg(target_arch = "wasm32")]
        let (document, buffers, images) =
            Self::import_gltf_web(path).map_err(|e| format!("Failed to load glTF: {}", e))?;

        #[cfg(not(target_arch = "wasm32"))]
        let (document, buffers, images) =
            Self::import_gltf_native(path).map_err(|e| format!("Failed to load glTF: {}", e))?;

        log::info!(
            "Document info: {} meshes, {} materials, {} textures, {} scenes",
            document.meshes().len(),
            document.materials().len(),
            document.textures().len(),
            document.scenes().len()
        );

        let imported_textures = textures::load_textures(&mut ctx, &document, &images)?;

        let material_result = materials::load_materials(&mut ctx, &document, &imported_textures)?;

        let mesh_data = nodes::build_meshes(&mut ctx, &document, &buffers)?;

        let node_entities = nodes::instantiate_nodes(
            &mut ctx,
            &document,
            &mesh_data.mesh_primitives,
            &material_result.material_handles,
            material_result.default_material,
        )?;

        let animation_clips = animations::load_animation_clips(
            ctx.scale,
            ctx.source_path(),
            &document,
            &buffers,
            &node_entities,
        )?;

        for clip in animation_clips {
            let clip_index = ctx.scene.add_animation_clip(clip);
            let _ = ctx.scene.play_animation(clip_index, true);
        }

        log::info!("=== glTF loaded successfully ===");
        log::info!("Total entities in scene: {}", ctx.scene.world().len());

        let mesh_count = ctx
            .scene
            .world()
            .query::<&crate::scene::components::MeshComponent>()
            .iter()
            .count();
        let parent_count = ctx
            .scene
            .world()
            .query::<&crate::scene::components::Parent>()
            .iter()
            .count();
        let children_count = ctx
            .scene
            .world()
            .query::<&crate::scene::components::Children>()
            .iter()
            .count();

        log::info!("  Entities with meshes: {}", mesh_count);
        log::info!("  Entities with parent: {}", parent_count);
        log::info!("  Entities with children: {}", children_count);

        Ok(())
    }

    pub fn load_gltf_asset(
        path: impl AsRef<Path>,
        renderer: &mut impl SceneImportDevice,
        scale: f32,
    ) -> Result<SceneAssetBundle, String> {
        let source_path = path.as_ref().to_path_buf();
        let resolved_path = crate::project::resolve_project_path(&source_path);

        if resolved_path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("import"))
            .unwrap_or(false)
        {
            return packaged::load_packaged_descriptor(&resolved_path, renderer, scale);
        }

        let mut temp_scene = Scene::new();
        let imported_meta = Self::load_imported_meta(&resolved_path);
        Self::load_gltf(
            &resolved_path,
            &mut temp_scene,
            renderer,
            scale,
            imported_meta.as_ref(),
        )?;

        let default_name = resolved_path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Scene".to_string());

        let mut asset = temp_scene
            .export_main_asset(default_name)
            .ok_or_else(|| "Scene export produced no asset".to_string())?;

        for entity in &mut asset.entities {
            if entity.gltf_source.is_some() {
                entity.gltf_source = Some(source_path.clone());
            }
        }

        if let Some(meta) = imported_meta.as_ref() {
            for entity in &mut asset.entities {
                if entity.gltf_source.is_none() {
                    continue;
                }

                let resolved = meta
                    .lookup_material_path(
                        entity.gltf_node,
                        entity.gltf_primitive,
                        entity.gltf_material,
                    )
                    .cloned();

                if let Some(path) = resolved {
                    let handle = entity
                        .material
                        .get_or_insert_with(|| SceneMaterialHandle::new(path.clone()));
                    handle.set_path(path);
                }
            }
        }

        let mut used_texture_indices = BTreeSet::new();
        for entity in &asset.entities {
            if let Some(material) = &entity.material_data {
                let flags = MaterialFlags::from_bits(material.flags);
                if flags.contains(MaterialFlags::USE_BASE_COLOR_TEXTURE) {
                    if let Some(index) = material.base_color_texture.index {
                        used_texture_indices.insert(index);
                    }
                }
                if flags.contains(MaterialFlags::USE_METALLIC_ROUGHNESS_TEXTURE) {
                    if let Some(index) = material.metallic_roughness_texture.index {
                        used_texture_indices.insert(index);
                    }
                }
                if flags.contains(MaterialFlags::USE_NORMAL_TEXTURE) {
                    if let Some(index) = material.normal_texture.index {
                        used_texture_indices.insert(index);
                    }
                }
                if flags.contains(MaterialFlags::USE_EMISSIVE_TEXTURE) {
                    if let Some(index) = material.emissive_texture.index {
                        used_texture_indices.insert(index);
                    }
                }
                if flags.contains(MaterialFlags::USE_OCCLUSION_TEXTURE) {
                    if let Some(index) = material.occlusion_texture.index {
                        used_texture_indices.insert(index);
                    }
                }
            }
        }

        let mut texture_index_remap = HashMap::new();
        for (local_index, original_index) in used_texture_indices.iter().enumerate() {
            texture_index_remap.insert(*original_index, local_index as u32);
        }

        if !texture_index_remap.is_empty() {
            asset.apply_resource_mappings(0, &texture_index_remap);
        }

        let assets = std::mem::take(&mut temp_scene.assets);
        let mut mesh_resources: Vec<(usize, Mesh)> = asset
            .mesh_data
            .iter()
            .cloned()
            .enumerate()
            .map(|(local_index, data)| (local_index, Mesh::from_data(renderer.device(), data)))
            .collect();
        mesh_resources.sort_by_key(|(local_index, _)| *local_index);

        let mut textures: Vec<(u32, Texture)> = assets
            .textures
            .into_inner()
            .into_iter()
            .enumerate()
            .filter_map(|(index, texture)| {
                let index_u32 = index as u32;
                texture_index_remap
                    .get(&index_u32)
                    .copied()
                    .map(|local_index| (local_index, texture))
            })
            .collect();
        textures.sort_by_key(|(local_index, _)| *local_index);

        Ok(SceneAssetBundle::new(
            asset,
            SceneAssetResources::new(mesh_resources, textures),
        ))
    }

    fn load_imported_meta(path: &Path) -> Option<ImportedGltfMeta> {
        let parent = path.parent()?;
        let file_name = path.file_name()?.to_string_lossy().into_owned();
        let meta_path = parent.join("meta.json");

        let content = match fs::read_to_string(&meta_path) {
            Ok(content) => content,
            Err(err) => {
                if err.kind() != io::ErrorKind::NotFound {
                    log::warn!("Failed to read glTF meta file {:?}: {}", meta_path, err);
                }
                return None;
            }
        };

        match serde_json::from_str::<BTreeMap<String, ImportedGltfMeta>>(&content) {
            Ok(mut map) => map.remove(&file_name),
            Err(err) => {
                log::warn!("Failed to parse glTF meta file {:?}: {}", meta_path, err);
                None
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn import_gltf_native(path: &Path) -> Result<GltfImport, gltf::Error> {
        match gltf::import(path) {
            Ok(result) => Ok(result),
            Err(gltf::Error::Deserialize(original))
                if path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .map(|ext| ext.eq_ignore_ascii_case("gltf"))
                    .unwrap_or(false) =>
            {
                match animations::import_gltf_with_pointer_patch(path)? {
                    Some(result) => Ok(result),
                    None => Err(gltf::Error::Deserialize(original)),
                }
            }
            Err(err) => Err(err),
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn import_gltf_web(path: &Path) -> Result<GltfImport, String> {
        use gltf::Gltf;

        let bytes = crate::io::load_binary(path)?;
        let mut gltf = Gltf::from_slice(&bytes).map_err(|err| err.to_string())?;
        let document = gltf.document;
        let mut blob = gltf.blob;
        let base_dir = path.parent().map(|p| p.to_path_buf());

        let buffers = Self::import_buffers_web(&document, base_dir.as_deref(), &mut blob, path)?;
        let images = textures::import_images_web(&document, base_dir.as_deref(), &buffers)?;

        Ok((document, buffers, images))
    }

    #[cfg(target_arch = "wasm32")]
    fn import_buffers_web(
        document: &gltf::Document,
        base: Option<&Path>,
        blob: &mut Option<Vec<u8>>,
        original_path: &Path,
    ) -> Result<Vec<gltf::buffer::Data>, String> {
        let mut buffers = Vec::new();

        for buffer in document.buffers() {
            let mut data = match buffer.source() {
                gltf::buffer::Source::Uri(uri) => {
                    textures::load_external_resource(base, uri, Some(original_path))?
                }
                gltf::buffer::Source::Bin => blob
                    .take()
                    .ok_or_else(|| format!("Missing BIN chunk for buffer {}", buffer.index()))?,
            };

            while data.len() % 4 != 0 {
                data.push(0);
            }

            let expected = buffer.length() as usize;
            if data.len() < expected {
                return Err(format!(
                    "Buffer {} has {} bytes but expected {}",
                    buffer.index(),
                    data.len(),
                    expected
                ));
            }

            buffers.push(gltf::buffer::Data(data));
        }

        Ok(buffers)
    }
}

#[cfg(test)]
mod tests;
