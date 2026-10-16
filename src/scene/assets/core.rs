use super::builder::SceneAssetBuilder;
use super::entity::{
    absolute_gltf_source, generate_material_asset_path, material_file_matches_pattern,
    normalize_material_path, relativize_material_texture_paths, sanitize_gltf_stem,
    SceneAssetEntity, SceneMaterialHandle,
};
use super::resources::resolve_material_handle;
use super::serialization::{
    ImportedGltfMeta, SerializedAnimationClip, SerializedLuaScriptSource, SerializedTransform,
};
use crate::asset::{Assets, Handle, MaterialAsset, MeshData};
use crate::io::{write_json, JsonLayout};
use crate::project::{normalize_absolute_path, relativize_path_to_project, CONTENT_DIR};
use crate::renderer::postprocess::{LegacyColorGrading, PostProcessEffects, PostProcessSettings};
use crate::scene::animation::{AnimationClip, AnimationState};
use crate::scene::components::{
    Billboard, CanCastShadow, Children, DirectionalLight, EditorEntityId, GltfMaterial, GltfNode,
    GltfPrimitive, GltfSource, MaterialComponent, MeshBounds, MeshComponent, Name, Parent,
    ParticleEmitterComponent, ParticleSystemComponent, PointLight, PostProcessVolume,
    PrimitiveMeshComponent, SpotLight, TransformComponent, Visible,
};
use crate::scene::gltf_material_registry::GltfMaterialRegistry;
use crate::scene::graph::SceneInstance;
use crate::scene::loader::SceneImportDevice;
use crate::scene::transform::Transform;
use hecs::World;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneAsset {
    pub name: String,
    pub root_transform: SerializedTransform,
    pub entities: Vec<SceneAssetEntity>,
    #[serde(default)]
    pub animations: Vec<SerializedAnimationClip>,
    #[serde(default)]
    pub animation_states: Vec<AnimationState>,
    #[serde(default)]
    pub mesh_data: Vec<MeshData>,
    #[serde(default)]
    pub active_camera: Option<usize>,
}

pub struct InstantiatedSceneAsset {
    pub world: World,
    pub animations: Vec<AnimationClip>,
    pub animation_states: Vec<AnimationState>,
}

impl SceneAsset {
    pub fn new(
        name: impl Into<String>,
        root_transform: SerializedTransform,
        entities: Vec<SceneAssetEntity>,
    ) -> Self {
        Self {
            name: name.into(),
            root_transform,
            entities,
            animations: Vec::new(),
            animation_states: Vec::new(),
            mesh_data: Vec::new(),
            active_camera: None,
        }
    }

    /// Move post-processing stored the old way (project-level effects and environment
    /// colour grading) into an infinite [`PostProcessVolume`] entity, unless the scene
    /// already has a volume. Returns whether an entity was added.
    ///
    /// [`PostProcessVolume`]: crate::scene::PostProcessVolume
    pub fn migrate_legacy_post_processing(
        &mut self,
        effects: Option<&PostProcessEffects>,
        project_grading: Option<LegacyColorGrading>,
    ) -> bool {
        let scene_grading = self
            .entities
            .iter_mut()
            .filter_map(|entity| entity.environment.as_mut())
            .fold(None, |found, environment| {
                found.or(environment.color_grading.take())
            });
        if self
            .entities
            .iter()
            .any(|entity| entity.post_process_volume.is_some())
        {
            return false;
        }
        let grading = scene_grading.or(project_grading);
        if effects.is_none() && grading.is_none() {
            return false;
        }

        let settings = PostProcessSettings::from_legacy(effects, grading.as_ref());
        self.entities.push(
            SceneAssetEntity::builder(SerializedTransform::from(Transform::IDENTITY))
                .with_name("Post Process Volume")
                .with_post_process_volume(PostProcessVolume::infinite(settings))
                .build(),
        );
        true
    }

    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string_pretty(self)
    }

    pub fn from_json(json: &str) -> serde_json::Result<Self> {
        serde_json::from_str(json)
    }

    pub fn builder(name: impl Into<String>) -> SceneAssetBuilder {
        SceneAssetBuilder::new(name)
    }

    /// Store script files inside `project_root` relative to it, like texture paths, so
    /// the project can be moved. Loading resolves them against the project again.
    pub fn relativize_script_paths(&mut self, project_root: &Path) {
        let canonical_project_root = std::fs::canonicalize(project_root)
            .map(normalize_absolute_path)
            .ok();
        for entity in &mut self.entities {
            if let Some(SerializedLuaScriptSource::File { path }) =
                entity.script.as_mut().map(|script| &mut script.source)
            {
                *path = relativize_path_to_project(
                    path.clone(),
                    project_root,
                    canonical_project_root.as_deref(),
                );
            }
        }
    }

    pub fn persist_material_assets(&mut self, project_root: &Path) -> Result<(), std::io::Error> {
        self.persist_material_assets_for_document(project_root, &self.name.clone())
    }

    pub(crate) fn persist_material_assets_for_document(
        &mut self,
        project_root: &Path,
        document_id: &str,
    ) -> Result<(), std::io::Error> {
        let materials_dir_rel = PathBuf::from(CONTENT_DIR).join("materials");
        let materials_dir_abs = project_root.join(&materials_dir_rel);
        fs::create_dir_all(&materials_dir_abs)?;

        let mut written: HashSet<PathBuf> = HashSet::new();
        let mut material_cache: HashMap<(PathBuf, usize), PathBuf> = HashMap::new();
        let canonical_project_root = std::fs::canonicalize(project_root)
            .map(normalize_absolute_path)
            .ok();
        let canonical_project_root_ref = canonical_project_root.as_deref();

        for entity in &mut self.entities {
            let Some(mut material_data) = entity.material_data.take() else {
                continue;
            };

            relativize_material_texture_paths(
                &mut material_data,
                project_root,
                canonical_project_root_ref,
            );

            let gltf_cache_key = entity.gltf_source.as_ref().and_then(|source| {
                entity.gltf_material.map(|material_index| {
                    (absolute_gltf_source(project_root, source), material_index)
                })
            });

            let sanitized_stem = entity.gltf_source.as_deref().map(sanitize_gltf_stem);

            let mut material_path = entity
                .material
                .as_ref()
                .map(|handle| normalize_material_path(project_root, handle.path()));

            if let Some(ref key) = gltf_cache_key {
                if let Some(cached) = material_cache.get(key) {
                    material_path = Some(cached.clone());
                }
            }

            let needs_regen = material_path.as_ref().is_none_or(|path| {
                if path.as_os_str().is_empty() {
                    return true;
                }

                if path.to_string_lossy().contains('#') {
                    return true;
                }

                if !path.to_string_lossy().ends_with(".mat.json") {
                    return true;
                }

                if let (Some(file_name), Some(stem), Some(material_index)) = (
                    path.file_name().and_then(|name| name.to_str()),
                    sanitized_stem.as_ref(),
                    entity.gltf_material,
                ) {
                    if !material_file_matches_pattern(file_name, stem, material_index) {
                        return true;
                    }
                }

                false
            });

            if needs_regen {
                let generated = generate_material_asset_path(
                    &materials_dir_rel,
                    &materials_dir_abs,
                    entity.gltf_source.as_deref(),
                    entity.gltf_material,
                    entity.editor_id,
                    document_id,
                    &written,
                );

                if let Some(ref key) = gltf_cache_key {
                    material_cache.insert(key.clone(), generated.clone());
                }

                material_path = Some(generated);
            }

            if let (Some(ref key), Some(ref path)) = (&gltf_cache_key, &material_path) {
                material_cache
                    .entry(key.clone())
                    .or_insert_with(|| path.clone());
            }

            let rel_path = material_path.expect("material path must be set");
            let abs_path = project_root.join(&rel_path);

            let handle = entity
                .material
                .get_or_insert_with(|| SceneMaterialHandle::new(rel_path.clone()));
            handle.set_path(rel_path.clone());

            if written.insert(rel_path.clone()) {
                write_json(&abs_path, &material_data, JsonLayout::Preserve)?;
            }

            entity.material_data = Some(material_data);
        }

        let registry = GltfMaterialRegistry::collect_from_asset(self, project_root);
        let mut meta_updates: HashMap<PathBuf, BTreeMap<String, ImportedGltfMeta>> = HashMap::new();

        for (key, binding) in registry.iter_bindings() {
            let Some(handle) = binding.handle.as_ref() else {
                continue;
            };

            let path = handle.path();
            if path.as_os_str().is_empty() {
                continue;
            }

            let relative_path = relativize_path_to_project(
                path.to_path_buf(),
                project_root,
                canonical_project_root_ref,
            );

            let Some(source_file) = key
                .source
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
            else {
                continue;
            };

            let Some(parent_dir) = key.source.parent() else {
                continue;
            };

            let entry = meta_updates
                .entry(parent_dir.to_path_buf())
                .or_default()
                .entry(source_file)
                .or_default();

            if let Some(material_index) = key.material_index {
                entry.record_material(material_index, relative_path.clone());
            }

            entry.record_material_key(key, relative_path);
        }

        for (meta_dir, sources) in meta_updates {
            fs::create_dir_all(&meta_dir)?;
            let meta_path = meta_dir.join("meta.json");

            let mut existing: BTreeMap<String, ImportedGltfMeta> = if meta_path.exists() {
                match fs::read_to_string(&meta_path) {
                    Ok(content) => match serde_json::from_str(&content) {
                        Ok(map) => map,
                        Err(err) => {
                            log::warn!(
                                "Failed to parse existing meta file {:?}: {}. Rebuilding.",
                                meta_path,
                                err
                            );
                            BTreeMap::new()
                        }
                    },
                    Err(err) => {
                        log::warn!(
                            "Failed to read existing meta file {:?}: {}. Rebuilding.",
                            meta_path,
                            err
                        );
                        BTreeMap::new()
                    }
                }
            } else {
                BTreeMap::new()
            };

            // The mapping is the import's default for new instances, shared by every scene
            // that uses the model. Scenes may point the same glTF material at different
            // files, so an entry that still names an existing file is kept; otherwise
            // whichever scene saved last would win, changing the file on every save.
            for (source_name, meta) in sources {
                let entry = existing.entry(source_name).or_default();
                let keep = |path: &PathBuf| project_root.join(path).is_file();
                for (index, path) in meta.materials {
                    if !entry.materials.get(&index).is_some_and(keep) {
                        entry.materials.insert(index, path);
                    }
                }
                for (key, path) in meta.materials_by_key {
                    if !entry.materials_by_key.get(&key).is_some_and(keep) {
                        entry.materials_by_key.insert(key, path);
                    }
                }
            }

            write_json(&meta_path, &existing, JsonLayout::Preserve)?;
        }

        Ok(())
    }

    pub(crate) fn instantiate(
        &self,
        mut renderer: Option<&mut dyn SceneImportDevice>,
        assets: &mut Assets,
    ) -> SceneInstance {
        let mut instance = SceneInstance::new();
        let mut builders = Vec::with_capacity(self.entities.len());
        let mut material_cache: HashMap<PathBuf, Handle<MaterialAsset>> = HashMap::new();

        for entity in &self.entities {
            let mut builder = hecs::EntityBuilder::new();

            if let Some(name) = &entity.name {
                builder.add(Name::new(name.clone()));
            }

            builder.add(TransformComponent(Transform::from(
                entity.transform.clone(),
            )));
            builder.add(Visible(entity.visible));

            if let Some(descriptor) = entity.primitive_mesh {
                let mut handle = assets.primitive_mesh_handle(descriptor);

                if handle.is_none() {
                    let ensured = super::resources::with_import_device(&mut renderer, |device| {
                        device.map(|device| assets.ensure_primitive_mesh(device, descriptor))
                    });
                    if let Some(primitive) = ensured {
                        handle = Some(primitive);
                    }
                }

                if handle.is_none() {
                    handle = entity.mesh_handle.map(Handle::new);
                }

                if let Some(handle) = handle {
                    builder.add(MeshComponent(handle));
                } else {
                    log::warn!(
                        "Primitive mesh {:?} missing shared handle and renderer; entity will spawn without mesh",
                        descriptor
                    );
                }

                builder.add(PrimitiveMeshComponent { descriptor });
            } else if let Some(mesh) = entity.mesh_handle {
                builder.add(MeshComponent(Handle::new(mesh)));
            }

            if let Some(bounds) = entity.mesh_bounds {
                builder.add(MeshBounds::from(bounds));
            }

            // Only renderable entities need a material; anything else would just pick
            // up the default one.
            let needs_material = entity.material.is_some()
                || entity.material_data.is_some()
                || entity.gltf_material.is_some()
                || entity.mesh_handle.is_some()
                || entity.primitive_mesh.is_some();
            let material_handle = needs_material
                .then(|| {
                    resolve_material_handle(
                        entity.material.as_ref(),
                        entity.material_data.as_ref(),
                        entity.gltf_source.as_deref(),
                        entity.gltf_material,
                        &mut renderer,
                        assets,
                        &mut material_cache,
                    )
                })
                .flatten();

            if let Some(handle) = material_handle {
                builder.add(MaterialComponent(handle));
            }

            if let Some(gltf_node) = entity.gltf_node {
                builder.add(GltfNode(gltf_node));
            }

            if let Some(gltf_mat) = entity.gltf_material {
                builder.add(GltfMaterial(gltf_mat));
            }

            if let Some(gltf_source) = &entity.gltf_source {
                builder.add(GltfSource(gltf_source.clone()));
            }

            if let Some(gltf_primitive) = entity.gltf_primitive {
                builder.add(GltfPrimitive(gltf_primitive));
            }

            if let Some(script) = &entity.script {
                builder.add(script.clone().into_component());
            }

            if let Some(light) = entity.directional_light {
                builder.add(DirectionalLight::from(light));
            }

            if let Some(light) = entity.point_light {
                builder.add(PointLight::from(light));
            }

            if let Some(light) = entity.spot_light {
                builder.add(SpotLight::from(light));
            }

            if let Some(casts_shadow) = entity.casts_shadow {
                builder.add(CanCastShadow(casts_shadow));
            }

            if let Some(billboard) = entity.billboard {
                builder.add(Billboard::from(billboard));
            }

            if let Some(camera) = entity.camera {
                builder.add(camera);
            }

            if let Some(particle_system) = &entity.particle_system {
                let mut system_component = ParticleSystemComponent::from(particle_system.clone());
                if let Some(behavior) = &entity.particle_behavior {
                    behavior.apply_to_component(&mut system_component);
                }
                builder.add(system_component);
            } else if let Some(behavior) = &entity.particle_behavior {
                builder.add(behavior.clone().into_component_with_spawn_rate(0.0));
            }

            if let Some(particle_emitter) = &entity.particle_emitter {
                builder.add(ParticleEmitterComponent::from(particle_emitter.clone()));
            }

            if let Some(editor_id) = entity.editor_id {
                builder.add(EditorEntityId(editor_id));
            }

            if let Some(environment) = &entity.environment {
                builder.add(environment.clone());
            }

            if let Some(volume) = &entity.post_process_volume {
                builder.add(volume.clone());
            }

            if let Some(volume) = &entity.light_probe_volume {
                builder.add(volume.clone());
            }

            if let Some(component) = &entity.rigid_body {
                builder.add(component.clone());
            }
            if let Some(component) = &entity.collider {
                builder.add(component.clone());
            }
            if let Some(component) = &entity.character_controller {
                builder.add(component.clone());
            }
            if let Some(component) = &entity.spring_camera {
                builder.add(component.clone());
            }

            if let Some(component) = &entity.animation_tree {
                builder.add(component.clone());
            }

            builders.push(builder);
        }

        // Snapshot entities go back to their original ids first, so fresh ids handed
        // out to the others cannot collide with them.
        let mut entity_map: Vec<Option<hecs::Entity>> = vec![None; self.entities.len()];
        for (index, (entity, builder)) in self.entities.iter().zip(&mut builders).enumerate() {
            if let Some(handle) = entity.source_entity {
                instance.world_mut().spawn_at(handle, builder.build());
                entity_map[index] = Some(handle);
            }
        }
        for (slot, builder) in entity_map.iter_mut().zip(&mut builders) {
            if slot.is_none() {
                *slot = Some(instance.world_mut().spawn(builder.build()));
            }
        }
        let entity_map: Vec<hecs::Entity> = entity_map.into_iter().flatten().collect();

        for (index, entity) in self.entities.iter().enumerate() {
            let entity_id = entity_map[index];

            if let Some(parent_index) = entity.parent {
                let parent = entity_map[parent_index];
                let _ = instance.world_mut().insert_one(entity_id, Parent(parent));
            }

            if !entity.children.is_empty() {
                let children_entities =
                    entity.children.iter().map(|idx| entity_map[*idx]).collect();
                let _ = instance
                    .world_mut()
                    .insert_one(entity_id, Children(children_entities));
            }
        }

        for (index, entity) in self.entities.iter().enumerate() {
            let entity_id = entity_map[index];
            if let Some(skin) = &entity.skin {
                match skin.to_component(&entity_map) {
                    Some(component) => {
                        let _ = instance.world_mut().insert_one(entity_id, component);
                    }
                    None => log::warn!(
                        "Entity {:?} has a skin with invalid joints; drawing it rigid",
                        entity.name
                    ),
                }
            }
            if let Some(animator) = &entity.animator {
                let _ = instance.world_mut().insert_one(entity_id, animator.clone());
            }
        }

        let active_camera = self
            .active_camera
            .and_then(|index| entity_map.get(index).copied());
        instance.set_active_camera(active_camera);
        instance.set_asset_entity_map(entity_map.clone());

        let animations: Vec<_> = self
            .animations
            .iter()
            .filter_map(|clip| clip.to_clip(&entity_map))
            .collect();
        let animation_states: Vec<_> = self
            .animation_states
            .iter()
            .filter(|state| state.clip_index < animations.len())
            .cloned()
            .collect();

        instance.set_animation_data(animations, animation_states);
        instance
    }

    pub fn instantiate_into_world(
        &self,
        renderer: Option<&mut dyn SceneImportDevice>,
        assets: &mut Assets,
    ) -> InstantiatedSceneAsset {
        let mut instance = self.instantiate(renderer, assets);
        let (animations, animation_states) = instance.take_animation_data();
        let world = instance.into_world();
        InstantiatedSceneAsset {
            world,
            animations,
            animation_states,
        }
    }

    pub(crate) fn apply_resource_mappings_from_maps(
        &mut self,
        mesh_map: &std::collections::HashMap<usize, usize>,
        texture_map: &std::collections::HashMap<u32, u32>,
    ) {
        if mesh_map.is_empty() && texture_map.is_empty() {
            return;
        }

        for entity in &mut self.entities {
            // Remap mesh handles using the map
            if entity.primitive_mesh.is_none() {
                if let Some(mesh_handle) = &mut entity.mesh_handle {
                    if let Some(&new_index) = mesh_map.get(mesh_handle) {
                        *mesh_handle = new_index;
                    } else if entity.gltf_source.is_none() {
                        log::warn!(
                            "Mesh handle {} not found in mesh_map during resource mapping",
                            mesh_handle
                        );
                    }
                }
            }

            // Remap texture indices in materials
            if let Some(material) = &mut entity.material_data {
                material.remap_textures(texture_map);
            }
        }
    }

    pub(crate) fn apply_resource_mappings(
        &mut self,
        mesh_offset: usize,
        texture_map: &std::collections::HashMap<u32, u32>,
    ) {
        if mesh_offset == 0 && texture_map.is_empty() {
            return;
        }

        for entity in &mut self.entities {
            if entity.primitive_mesh.is_none() {
                if let Some(mesh) = &mut entity.mesh_handle {
                    *mesh += mesh_offset;
                }
            }

            if let Some(material) = &mut entity.material_data {
                material.remap_textures(texture_map);
            }
        }
    }
}
