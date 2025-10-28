use super::animation::{
    AnimationChannel, AnimationClip, AnimationInterpolation, AnimationOutput, AnimationSampler,
    AnimationState, AnimationTarget, MaterialProperty, TransformProperty,
};
use super::components::{
    CameraComponent, CanCastShadow, Children, DirectionalLight, EditorEntityId,
    EnvironmentComponent, GltfMaterial, GltfNode, GltfPrimitive, GltfSource, MaterialComponent,
    MeshBounds, MeshComponent, Name, Parent, ParticleBehaviorConfig, ParticleBehaviorPreset,
    ParticleColorGradient, ParticleEmissionShape, ParticleEmitterComponent, ParticleFloatRange,
    ParticleRenderBlendMode, ParticleSizeCurve, ParticleSystemComponent, ParticleVec3Range,
    PointLight, PrimitiveMeshComponent, SpotLight, TransformComponent, Visible,
};
use super::graph::SceneInstance;
use super::loader::SceneImportDevice;
use crate::asset::{
    Assets, Handle, MaterialAsset, MaterialTextureReference, MaterialTextureSlot, Mesh, MeshData,
};
use crate::project::{
    active_project_root, normalize_absolute_path, relativize_path_to_project, resolve_project_path,
    CONTENT_DIR,
};
use crate::renderer::material::MaterialFlags;
use crate::renderer::primitives::PrimitiveMeshDescriptor;
use crate::renderer::texture::{
    DEFAULT_METALLIC_ROUGHNESS_TEXTURE_INDEX, DEFAULT_NORMAL_TEXTURE_INDEX,
    DEFAULT_WHITE_TEXTURE_INDEX,
};
use crate::renderer::{Material, Texture};
use crate::scene::transform::Transform;
use crate::scripting::{RuneScriptComponent, RuneScriptSource};
use hecs::{Entity, World};
use rand::{distributions::Alphanumeric, thread_rng, Rng};
use serde::de::{self, MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::convert::TryFrom;
use std::fs;
use std::path::{Path, PathBuf};

mod path_serde {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::path::{Path, PathBuf};

    pub fn serialize<S>(path: &Path, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(path.to_string_lossy().as_ref())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<PathBuf, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(PathBuf::from(value))
    }
}

mod material_map_serde {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    pub fn serialize<S>(value: &BTreeMap<usize, PathBuf>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mapped: BTreeMap<String, String> = value
            .iter()
            .map(|(index, path)| (index.to_string(), path.to_string_lossy().into_owned()))
            .collect();
        serde::Serialize::serialize(&mapped, serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<BTreeMap<usize, PathBuf>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mapped = BTreeMap::<String, String>::deserialize(deserializer)?;
        let mut result = BTreeMap::new();
        for (key, value) in mapped {
            let index = key.parse::<usize>().map_err(serde::de::Error::custom)?;
            result.insert(index, PathBuf::from(value));
        }
        Ok(result)
    }
}

/// Metadata recorded for a glTF import to preserve material assignments across reloads.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImportedGltfMeta {
    /// Map of glTF material indices to relative material asset paths.
    #[serde(default, with = "material_map_serde")]
    pub materials: BTreeMap<usize, PathBuf>,
}

impl ImportedGltfMeta {
    pub fn record_material(&mut self, index: usize, path: PathBuf) {
        self.materials.insert(index, path);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SerializedAnimationOutput {
    Vec3(Vec<[f32; 3]>),
    Quat(Vec<[f32; 4]>),
    Vec4(Vec<[f32; 4]>),
}

impl SerializedAnimationOutput {
    fn from_output(output: &AnimationOutput) -> Self {
        match output {
            AnimationOutput::Vec3(values) => {
                let converted = values.iter().map(|v| v.to_array()).collect();
                SerializedAnimationOutput::Vec3(converted)
            }
            AnimationOutput::Quat(values) => {
                let converted = values.iter().map(|v| v.to_array()).collect();
                SerializedAnimationOutput::Quat(converted)
            }
            AnimationOutput::Vec4(values) => {
                let converted = values.iter().map(|v| v.to_array()).collect();
                SerializedAnimationOutput::Vec4(converted)
            }
        }
    }

    fn into_output(self) -> AnimationOutput {
        match self {
            SerializedAnimationOutput::Vec3(values) => {
                AnimationOutput::Vec3(values.iter().copied().map(glam::Vec3::from_array).collect())
            }
            SerializedAnimationOutput::Quat(values) => {
                AnimationOutput::Quat(values.iter().copied().map(glam::Quat::from_array).collect())
            }
            SerializedAnimationOutput::Vec4(values) => {
                AnimationOutput::Vec4(values.iter().copied().map(glam::Vec4::from_array).collect())
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedAnimationSampler {
    pub times: Vec<f32>,
    pub output: SerializedAnimationOutput,
    pub interpolation: AnimationInterpolation,
}

impl SerializedAnimationSampler {
    fn from_sampler(sampler: &AnimationSampler) -> Self {
        Self {
            times: sampler.times.clone(),
            output: SerializedAnimationOutput::from_output(&sampler.output),
            interpolation: sampler.interpolation,
        }
    }

    fn into_sampler(self) -> AnimationSampler {
        AnimationSampler {
            times: self.times,
            output: self.output.into_output(),
            interpolation: self.interpolation,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SerializedAnimationTarget {
    Transform {
        entity_index: usize,
        property: TransformProperty,
    },
    Material {
        material_index: usize,
        property: MaterialProperty,
    },
}

impl SerializedAnimationTarget {
    fn from_target(target: &AnimationTarget, index_map: &HashMap<Entity, usize>) -> Option<Self> {
        match target {
            AnimationTarget::Transform { entity, property } => {
                let entity_index = *index_map.get(entity)?;
                Some(SerializedAnimationTarget::Transform {
                    entity_index,
                    property: *property,
                })
            }
            AnimationTarget::Material {
                material_index,
                property,
            } => Some(SerializedAnimationTarget::Material {
                material_index: *material_index,
                property: *property,
            }),
        }
    }

    fn into_target(self, entities: &[Entity]) -> Option<AnimationTarget> {
        match self {
            SerializedAnimationTarget::Transform {
                entity_index,
                property,
            } => {
                let entity = *entities.get(entity_index)?;
                Some(AnimationTarget::Transform { entity, property })
            }
            SerializedAnimationTarget::Material {
                material_index,
                property,
            } => Some(AnimationTarget::Material {
                material_index,
                property,
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedAnimationChannel {
    pub sampler: SerializedAnimationSampler,
    pub target: SerializedAnimationTarget,
}

impl SerializedAnimationChannel {
    fn from_channel(
        channel: &AnimationChannel,
        index_map: &HashMap<Entity, usize>,
    ) -> Option<Self> {
        let target = SerializedAnimationTarget::from_target(&channel.target, index_map)?;
        Some(Self {
            sampler: SerializedAnimationSampler::from_sampler(&channel.sampler),
            target,
        })
    }

    fn into_channel(self, entities: &[Entity]) -> Option<AnimationChannel> {
        let target = self.target.into_target(entities)?;
        Some(AnimationChannel {
            sampler: self.sampler.into_sampler(),
            target,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedAnimationClip {
    pub name: String,
    pub duration: f32,
    pub channels: Vec<SerializedAnimationChannel>,
}

impl SerializedAnimationClip {
    pub(crate) fn from_clip(
        clip: &AnimationClip,
        index_map: &HashMap<Entity, usize>,
    ) -> Option<Self> {
        let mut channels = Vec::with_capacity(clip.channels.len());
        for channel in &clip.channels {
            if let Some(serialized) = SerializedAnimationChannel::from_channel(channel, index_map) {
                channels.push(serialized);
            }
        }

        Some(Self {
            name: clip.name.clone(),
            duration: clip.duration,
            channels,
        })
    }

    pub(crate) fn to_clip(&self, entities: &[Entity]) -> Option<AnimationClip> {
        let mut clip = AnimationClip::new(self.name.clone());
        for channel in &self.channels {
            if let Some(deserialized) = channel.clone().into_channel(entities) {
                clip.add_channel(deserialized);
            }
        }
        clip.duration = self.duration;
        Some(clip)
    }
}

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

    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string_pretty(self)
    }

    pub fn from_json(json: &str) -> serde_json::Result<Self> {
        serde_json::from_str(json)
    }

    pub fn builder(name: impl Into<String>) -> SceneAssetBuilder {
        SceneAssetBuilder::new(name)
    }

    pub fn persist_material_assets(&mut self, project_root: &Path) -> Result<(), std::io::Error> {
        let materials_dir_rel = PathBuf::from(CONTENT_DIR).join("materials");
        let materials_dir_abs = project_root.join(&materials_dir_rel);
        fs::create_dir_all(&materials_dir_abs)?;

        let mut written: HashSet<PathBuf> = HashSet::new();
        let mut material_cache: HashMap<(PathBuf, usize), PathBuf> = HashMap::new();
        let mut meta_updates: HashMap<PathBuf, BTreeMap<String, ImportedGltfMeta>> = HashMap::new();
        let canonical_project_root = std::fs::canonicalize(project_root)
            .map(|root| normalize_absolute_path(root))
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

            let needs_regen = material_path.as_ref().map_or(true, |path| {
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

            let first_write = written.insert(rel_path.clone());
            if first_write || !abs_path.exists() {
                let json = serde_json::to_string_pretty(&material_data)?;
                fs::write(&abs_path, json)?;
            }

            if let (Some(source), Some(material_index)) =
                (entity.gltf_source.as_ref(), entity.gltf_material)
            {
                if let Some(file_name) = source
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                {
                    if let Some(parent) = project_root.join(source).parent() {
                        let entry = meta_updates
                            .entry(parent.to_path_buf())
                            .or_insert_with(BTreeMap::new)
                            .entry(file_name)
                            .or_insert_with(ImportedGltfMeta::default);
                        entry.record_material(material_index, rel_path.clone());
                    }
                }
            }

            entity.material_data = Some(material_data);
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

            for (source_name, meta) in sources {
                existing.insert(source_name, meta);
            }

            let json = serde_json::to_string_pretty(&existing)
                .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err))?;
            fs::write(meta_path, json)?;
        }

        Ok(())
    }

    pub(crate) fn instantiate(
        &self,
        mut renderer: Option<&mut dyn SceneImportDevice>,
        assets: &mut Assets,
    ) -> SceneInstance {
        let mut instance = SceneInstance::new();
        let mut entity_map = Vec::with_capacity(self.entities.len());
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
                    let ensured = with_import_device(&mut renderer, |device| {
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

            let material_handle = resolve_material_handle(
                entity.material.as_ref(),
                entity.material_data.as_ref(),
                entity.gltf_source.as_deref(),
                entity.gltf_material,
                &mut renderer,
                assets,
                &mut material_cache,
            );

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

            let entity_id = instance.world_mut().spawn(builder.build());
            entity_map.push(entity_id);
        }

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

        let active_camera = self
            .active_camera
            .and_then(|index| entity_map.get(index).copied());
        instance.set_active_camera(active_camera);

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

#[derive(Debug, Default)]
pub struct SceneAssetResources {
    meshes: Vec<(usize, Mesh)>,
    textures: Vec<(u32, Texture)>,
}

fn determine_canonical_material_path(
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

fn canonicalize_material_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn with_import_device<R>(
    renderer: &mut Option<&mut dyn SceneImportDevice>,
    f: impl FnOnce(Option<&mut dyn SceneImportDevice>) -> R,
) -> R {
    match renderer.as_mut() {
        Some(device) => f(Some(&mut **device)),
        None => f(None),
    }
}

fn resolve_material_handle(
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
            let (material, references) = with_import_device(renderer, |device| {
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
            let handle = assets.insert_material_asset(asset);
            cache.insert(key, handle);
            return Some(handle);
        } else {
            let mut asset =
                MaterialAsset::from_material(Material::from(serialized.clone()), PathBuf::new());
            let (material, references) = with_import_device(renderer, |device| {
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

fn update_material_asset_from_references(
    assets: &mut Assets,
    handle: Handle<MaterialAsset>,
    material: Material,
    references: Vec<(MaterialTextureSlot, Option<MaterialTextureReference>)>,
) {
    if let Some(asset) = assets.material_mut(handle) {
        *asset.material_mut() = material;
        for (slot, reference) in references {
            if let Some(reference) = reference {
                asset.set_texture_reference(slot, reference);
            } else {
                asset.clear_texture_reference(slot);
            }
        }
    }
}

fn apply_serialized_material_to_handle(
    assets: &mut Assets,
    handle: Handle<MaterialAsset>,
    serialized: &SerializedMaterial,
    renderer: Option<&mut dyn SceneImportDevice>,
) {
    let (material, references) = serialized.resolve_material(assets, renderer);
    update_material_asset_from_references(assets, handle, material, references);
}

fn ensure_asset_textures_from_metadata(
    assets: &mut Assets,
    handle: Handle<MaterialAsset>,
    renderer: Option<&mut dyn SceneImportDevice>,
) {
    let serialized = if let Some(asset) = assets.material(handle) {
        SerializedMaterial::from_material_asset(asset)
    } else {
        return;
    };

    let (material, references) = serialized.resolve_material(assets, renderer);
    update_material_asset_from_references(assets, handle, material, references);
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

fn next_mesh_index(meshes: &[(usize, Mesh)]) -> usize {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(from = "SceneAssetEntityData", into = "SceneAssetEntityData")]
pub struct SceneAssetEntity {
    pub name: Option<String>,
    pub transform: SerializedTransform,
    pub visible: bool,
    pub mesh_handle: Option<usize>,
    #[serde(default)]
    pub primitive_mesh: Option<PrimitiveMeshDescriptor>,
    #[serde(default)]
    pub mesh_bounds: Option<SerializedMeshBounds>,
    #[serde(default)]
    pub material: Option<SceneMaterialHandle>,
    #[serde(skip)]
    pub material_data: Option<SerializedMaterial>,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub gltf_node: Option<usize>,
    pub gltf_material: Option<usize>,
    #[serde(default)]
    pub gltf_source: Option<PathBuf>,
    #[serde(default)]
    pub gltf_primitive: Option<usize>,
    #[serde(default)]
    pub script: Option<SerializedRuneScript>,
    #[serde(default)]
    pub directional_light: Option<SerializedDirectionalLight>,
    #[serde(default)]
    pub point_light: Option<SerializedPointLight>,
    #[serde(default)]
    pub spot_light: Option<SerializedSpotLight>,
    #[serde(default)]
    pub casts_shadow: Option<bool>,
    #[serde(default)]
    pub editor_id: Option<u128>,
    #[serde(default)]
    pub particle_system: Option<SerializedParticleSystem>,
    #[serde(default)]
    pub particle_emitter: Option<SerializedParticleEmitter>,
    #[serde(default)]
    pub particle_behavior: Option<SerializedParticleBehavior>,
    #[serde(default)]
    pub environment: Option<EnvironmentComponent>,
    #[serde(default)]
    pub camera: Option<CameraComponent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SceneMaterialHandle {
    #[serde(with = "path_serde")]
    path: PathBuf,
}

impl SceneMaterialHandle {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn set_path(&mut self, path: PathBuf) {
        self.path = path;
    }
}

impl Default for SceneMaterialHandle {
    fn default() -> Self {
        Self {
            path: PathBuf::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SceneAssetEntityData {
    name: Option<String>,
    transform: SerializedTransform,
    visible: bool,
    mesh_handle: Option<usize>,
    #[serde(default)]
    primitive_mesh: Option<PrimitiveMeshDescriptor>,
    #[serde(default)]
    mesh_bounds: Option<SerializedMeshBounds>,
    #[serde(default)]
    material: Option<SceneMaterialField>,
    parent: Option<usize>,
    children: Vec<usize>,
    gltf_node: Option<usize>,
    gltf_material: Option<usize>,
    #[serde(default)]
    gltf_source: Option<PathBuf>,
    #[serde(default)]
    gltf_primitive: Option<usize>,
    #[serde(default)]
    script: Option<SerializedRuneScript>,
    #[serde(default)]
    directional_light: Option<SerializedDirectionalLight>,
    #[serde(default)]
    point_light: Option<SerializedPointLight>,
    #[serde(default)]
    spot_light: Option<SerializedSpotLight>,
    #[serde(default)]
    casts_shadow: Option<bool>,
    #[serde(default)]
    editor_id: Option<u128>,
    #[serde(default)]
    particle_system: Option<SerializedParticleSystem>,
    #[serde(default)]
    particle_emitter: Option<SerializedParticleEmitter>,
    #[serde(default)]
    particle_behavior: Option<SerializedParticleBehavior>,
    #[serde(default)]
    environment: Option<EnvironmentComponent>,
    #[serde(default)]
    camera: Option<CameraComponent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum SceneMaterialField {
    Handle(SceneMaterialHandle),
    Legacy(Box<SerializedMaterial>),
}

impl From<SceneAssetEntityData> for SceneAssetEntity {
    fn from(data: SceneAssetEntityData) -> Self {
        let (material, material_data) = match data.material {
            Some(SceneMaterialField::Handle(handle)) => (Some(handle), None),
            Some(SceneMaterialField::Legacy(legacy)) => (None, Some(*legacy)),
            None => (None, None),
        };

        SceneAssetEntity {
            name: data.name,
            transform: data.transform,
            visible: data.visible,
            mesh_handle: data.mesh_handle,
            primitive_mesh: data.primitive_mesh,
            mesh_bounds: data.mesh_bounds,
            material,
            material_data,
            parent: data.parent,
            children: data.children,
            gltf_node: data.gltf_node,
            gltf_material: data.gltf_material,
            gltf_source: data.gltf_source,
            gltf_primitive: data.gltf_primitive,
            script: data.script,
            directional_light: data.directional_light,
            point_light: data.point_light,
            spot_light: data.spot_light,
            casts_shadow: data.casts_shadow,
            editor_id: data.editor_id,
            particle_system: data.particle_system,
            particle_emitter: data.particle_emitter,
            particle_behavior: data.particle_behavior,
            environment: data.environment,
            camera: data.camera,
        }
    }
}

impl From<SceneAssetEntity> for SceneAssetEntityData {
    fn from(entity: SceneAssetEntity) -> Self {
        let material = entity
            .material
            .clone()
            .map(SceneMaterialField::Handle)
            .or_else(|| {
                entity
                    .material_data
                    .clone()
                    .map(|data| SceneMaterialField::Legacy(Box::new(data)))
            });

        SceneAssetEntityData {
            name: entity.name,
            transform: entity.transform,
            visible: entity.visible,
            mesh_handle: entity.mesh_handle,
            primitive_mesh: entity.primitive_mesh,
            mesh_bounds: entity.mesh_bounds,
            material,
            parent: entity.parent,
            children: entity.children,
            gltf_node: entity.gltf_node,
            gltf_material: entity.gltf_material,
            gltf_source: entity.gltf_source,
            gltf_primitive: entity.gltf_primitive,
            script: entity.script,
            directional_light: entity.directional_light,
            point_light: entity.point_light,
            spot_light: entity.spot_light,
            casts_shadow: entity.casts_shadow,
            editor_id: entity.editor_id,
            particle_system: entity.particle_system,
            particle_emitter: entity.particle_emitter,
            particle_behavior: entity.particle_behavior,
            environment: entity.environment,
            camera: entity.camera,
        }
    }
}

fn load_material_asset_from_file(path: &Path) -> Result<MaterialAsset, String> {
    let data = fs::read_to_string(path)
        .map_err(|err| format!("failed to read material asset {:?}: {}", path, err))?;
    let serialized: SerializedMaterial = serde_json::from_str(&data)
        .map_err(|err| format!("failed to parse material asset {:?}: {}", path, err))?;
    let mut asset =
        MaterialAsset::from_material(Material::from(serialized.clone()), path.to_path_buf());
    serialized.apply_metadata_to_asset(&mut asset);
    Ok(asset)
}

fn normalize_material_path(project_root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        match path.strip_prefix(project_root) {
            Ok(stripped) => stripped.to_path_buf(),
            Err(_) => path.to_path_buf(),
        }
    } else {
        path.to_path_buf()
    }
}

fn relativize_material_texture_paths(
    material: &mut SerializedMaterial,
    project_root: &Path,
    canonical_project_root: Option<&Path>,
) {
    for slot in MaterialTextureSlot::all() {
        let slot_data = material.texture_slot_mut(slot);
        if let Some(path) = slot_data.path.as_mut() {
            let relative =
                relativize_path_to_project(path.clone(), project_root, canonical_project_root);
            *path = relative;
        }
    }
}

fn absolute_gltf_source(project_root: &Path, source: &Path) -> PathBuf {
    if source.is_absolute() {
        source.to_path_buf()
    } else {
        project_root.join(source)
    }
}

fn sanitize_gltf_stem(source: &Path) -> String {
    let stem = source
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("material");

    let mut sanitized = String::with_capacity(stem.len());
    let mut last_was_separator = false;

    for ch in stem.chars() {
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_alphanumeric() {
            sanitized.push(lower);
            last_was_separator = false;
        } else if !last_was_separator {
            sanitized.push('_');
            last_was_separator = true;
        }
    }

    let trimmed = sanitized.trim_matches('_');
    if trimmed.is_empty() {
        "material".to_string()
    } else {
        trimmed.to_string()
    }
}

fn material_file_matches_pattern(
    file_name: &str,
    sanitized_stem: &str,
    material_index: usize,
) -> bool {
    if !file_name.ends_with(".mat.json") {
        return false;
    }

    let expected_prefix = format!("{}_{:03}", sanitized_stem, material_index);
    match file_name.strip_suffix(".mat.json") {
        Some(prefix) if prefix == expected_prefix => true,
        Some(prefix) => prefix
            .strip_prefix(&(expected_prefix + "_"))
            .map(|suffix| !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit()))
            .unwrap_or(false),
        None => false,
    }
}

fn generate_material_asset_path(
    materials_dir_rel: &Path,
    materials_dir_abs: &Path,
    gltf_source: Option<&Path>,
    material_index: Option<usize>,
    used: &HashSet<PathBuf>,
) -> PathBuf {
    if let (Some(source), Some(index)) = (gltf_source, material_index) {
        let sanitized = sanitize_gltf_stem(source);
        let base = format!("{}_{:03}", sanitized, index);
        let mut suffix: u32 = 0;

        loop {
            let file_name = if suffix == 0 {
                format!("{}.mat.json", base)
            } else {
                format!("{}_{}.mat.json", base, suffix)
            };
            let rel_path = materials_dir_rel.join(&file_name);

            if used.contains(&rel_path) {
                suffix += 1;
                continue;
            }

            if materials_dir_abs.join(&file_name).exists() {
                suffix += 1;
                continue;
            }

            return rel_path;
        }
    }

    let mut rng = thread_rng();

    loop {
        let suffix: String = (&mut rng)
            .sample_iter(&Alphanumeric)
            .take(12)
            .map(char::from)
            .collect();
        let file_name = format!("material_{}.mat.json", suffix);
        let rel_path = materials_dir_rel.join(&file_name);

        if used.contains(&rel_path) {
            continue;
        }

        if materials_dir_abs.join(&file_name).exists() {
            continue;
        }

        return rel_path;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SerializedRuneScriptSource {
    Inline { name: String, source: String },
    File { path: PathBuf },
}

impl From<&RuneScriptSource> for SerializedRuneScriptSource {
    fn from(source: &RuneScriptSource) -> Self {
        match source {
            RuneScriptSource::Inline { name, source } => Self::Inline {
                name: name.to_string(),
                source: source.to_string(),
            },
            RuneScriptSource::File { path } => Self::File { path: path.clone() },
        }
    }
}

impl From<SerializedRuneScriptSource> for RuneScriptSource {
    fn from(serialized: SerializedRuneScriptSource) -> Self {
        match serialized {
            SerializedRuneScriptSource::Inline { name, source } => {
                RuneScriptSource::inline(name, source)
            }
            SerializedRuneScriptSource::File { path } => RuneScriptSource::file(path),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedRuneScript {
    pub source: SerializedRuneScriptSource,
    pub created_called: bool,
}

impl From<&RuneScriptComponent> for SerializedRuneScript {
    fn from(component: &RuneScriptComponent) -> Self {
        Self {
            source: SerializedRuneScriptSource::from(component.source()),
            created_called: component.created_called(),
        }
    }
}

impl SerializedRuneScript {
    fn into_component(self) -> RuneScriptComponent {
        let mut component = RuneScriptComponent::new(self.source.into());
        component.set_created_called(self.created_called);
        component
    }
}

impl SceneAssetEntity {
    pub fn builder(transform: SerializedTransform) -> SceneAssetEntityBuilder {
        SceneAssetEntityBuilder::new(transform)
    }

    fn from_world_entity(
        entity: Entity,
        world: &World,
        assets: &Assets,
        index_map: &HashMap<Entity, usize>,
    ) -> Self {
        let name = world.get::<&Name>(entity).ok().map(|n| n.0.clone());

        let transform = world
            .get::<&TransformComponent>(entity)
            .map(|t| SerializedTransform::from(t.0))
            .unwrap_or_else(|_| SerializedTransform::from(Transform::IDENTITY));

        let visible = world.get::<&Visible>(entity).map(|v| v.0).unwrap_or(true);
        let mesh_handle = world
            .get::<&MeshComponent>(entity)
            .ok()
            .map(|m| m.0.index());
        let primitive_mesh = world
            .get::<&PrimitiveMeshComponent>(entity)
            .ok()
            .map(|component| component.descriptor);
        let mesh_bounds = world
            .get::<&MeshBounds>(entity)
            .ok()
            .map(|bounds| SerializedMeshBounds::from(*bounds));
        let (material, material_data) = world
            .get::<&MaterialComponent>(entity)
            .ok()
            .and_then(|component| assets.material(component.0))
            .map(|asset| {
                let serialized = SerializedMaterial::from_material_asset(asset);
                let handle = if asset.canonical_path().as_os_str().is_empty() {
                    None
                } else {
                    Some(SceneMaterialHandle::new(
                        asset.canonical_path().to_path_buf(),
                    ))
                };
                (handle, Some(serialized))
            })
            .unwrap_or((None, None));

        let particle_emitter = world
            .get::<&ParticleEmitterComponent>(entity)
            .ok()
            .map(|component| SerializedParticleEmitter::from(&*component));

        let (particle_system, particle_behavior) =
            match world.get::<&ParticleSystemComponent>(entity) {
                Ok(component) => (
                    Some(SerializedParticleSystem::from(&*component)),
                    Some(SerializedParticleBehavior::from(&*component)),
                ),
                Err(_) => (None, None),
            };

        let parent = world
            .get::<&Parent>(entity)
            .ok()
            .and_then(|p| index_map.get(&p.0).copied());

        let children = world
            .get::<&Children>(entity)
            .ok()
            .map(|children| {
                children
                    .0
                    .iter()
                    .filter_map(|child| index_map.get(child).copied())
                    .collect()
            })
            .unwrap_or_default();

        let gltf_node = world.get::<&GltfNode>(entity).ok().map(|node| node.0);
        let gltf_material = world.get::<&GltfMaterial>(entity).ok().map(|mat| mat.0);
        let gltf_source = world
            .get::<&GltfSource>(entity)
            .ok()
            .map(|source| source.0.clone());
        let gltf_primitive = world
            .get::<&GltfPrimitive>(entity)
            .ok()
            .map(|primitive| primitive.0);
        let script = world
            .get::<&RuneScriptComponent>(entity)
            .ok()
            .map(|component| SerializedRuneScript::from(&*component));

        let directional_light = world
            .get::<&DirectionalLight>(entity)
            .ok()
            .map(|light| SerializedDirectionalLight::from(*light));

        let point_light = world
            .get::<&PointLight>(entity)
            .ok()
            .map(|light| SerializedPointLight::from(*light));

        let spot_light = world
            .get::<&SpotLight>(entity)
            .ok()
            .map(|light| SerializedSpotLight::from(*light));

        let casts_shadow = world.get::<&CanCastShadow>(entity).ok().map(|flag| flag.0);
        let editor_id = world.get::<&EditorEntityId>(entity).ok().map(|id| id.0);
        let environment = world
            .get::<&EnvironmentComponent>(entity)
            .ok()
            .map(|component| (*component).clone());
        let camera = world
            .get::<&CameraComponent>(entity)
            .ok()
            .map(|component| *component);

        Self {
            name,
            transform,
            visible,
            mesh_handle,
            primitive_mesh,
            mesh_bounds,
            material,
            material_data,
            parent,
            children,
            gltf_node,
            gltf_material,
            gltf_source,
            gltf_primitive,
            script,
            directional_light,
            point_light,
            spot_light,
            casts_shadow,
            editor_id,
            particle_system,
            particle_emitter,
            particle_behavior,
            environment,
            camera,
        }
    }
}

pub struct SceneAssetEntityBuilder {
    name: Option<String>,
    transform: SerializedTransform,
    visible: bool,
    mesh_handle: Option<usize>,
    primitive_mesh: Option<PrimitiveMeshDescriptor>,
    mesh_bounds: Option<SerializedMeshBounds>,
    material: Option<SceneMaterialHandle>,
    material_data: Option<SerializedMaterial>,
    parent: Option<usize>,
    children: Vec<usize>,
    gltf_node: Option<usize>,
    gltf_material: Option<usize>,
    gltf_source: Option<PathBuf>,
    gltf_primitive: Option<usize>,
    script: Option<SerializedRuneScript>,
    directional_light: Option<SerializedDirectionalLight>,
    point_light: Option<SerializedPointLight>,
    spot_light: Option<SerializedSpotLight>,
    casts_shadow: Option<bool>,
    editor_id: Option<u128>,
    particle_system: Option<SerializedParticleSystem>,
    particle_emitter: Option<SerializedParticleEmitter>,
    particle_behavior: Option<SerializedParticleBehavior>,
    environment: Option<EnvironmentComponent>,
    camera: Option<CameraComponent>,
}

impl SceneAssetEntityBuilder {
    fn new(transform: SerializedTransform) -> Self {
        Self {
            name: None,
            transform,
            visible: true,
            mesh_handle: None,
            primitive_mesh: None,
            mesh_bounds: None,
            material: None,
            material_data: None,
            parent: None,
            children: Vec::new(),
            gltf_node: None,
            gltf_material: None,
            gltf_source: None,
            gltf_primitive: None,
            script: None,
            directional_light: None,
            point_light: None,
            spot_light: None,
            casts_shadow: None,
            editor_id: None,
            particle_system: None,
            particle_emitter: None,
            particle_behavior: None,
            environment: None,
            camera: None,
        }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn with_visibility(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }

    pub fn with_mesh_handle(mut self, handle: usize) -> Self {
        self.mesh_handle = Some(handle);
        self
    }

    pub fn with_primitive_mesh(mut self, descriptor: PrimitiveMeshDescriptor) -> Self {
        self.primitive_mesh = Some(descriptor);
        self
    }

    pub fn with_mesh_bounds(mut self, bounds: SerializedMeshBounds) -> Self {
        self.mesh_bounds = Some(bounds);
        self
    }

    pub fn with_material(
        mut self,
        handle: SceneMaterialHandle,
        data: Option<SerializedMaterial>,
    ) -> Self {
        self.material = Some(handle);
        self.material_data = data;
        self
    }

    pub fn with_parent(mut self, parent: usize) -> Self {
        self.parent = Some(parent);
        self
    }

    pub fn with_children<I>(mut self, children: I) -> Self
    where
        I: IntoIterator<Item = usize>,
    {
        self.children = children.into_iter().collect();
        self
    }

    pub fn with_gltf_node(mut self, node: usize) -> Self {
        self.gltf_node = Some(node);
        self
    }

    pub fn with_gltf_material(mut self, material: usize) -> Self {
        self.gltf_material = Some(material);
        self
    }

    pub fn with_gltf_source(mut self, source: PathBuf) -> Self {
        self.gltf_source = Some(source);
        self
    }

    pub fn with_gltf_primitive(mut self, primitive: usize) -> Self {
        self.gltf_primitive = Some(primitive);
        self
    }

    pub fn with_script(mut self, script: SerializedRuneScript) -> Self {
        self.script = Some(script);
        self
    }

    pub fn with_directional_light(mut self, light: SerializedDirectionalLight) -> Self {
        self.directional_light = Some(light);
        self
    }

    pub fn with_point_light(mut self, light: SerializedPointLight) -> Self {
        self.point_light = Some(light);
        self
    }

    pub fn with_spot_light(mut self, light: SerializedSpotLight) -> Self {
        self.spot_light = Some(light);
        self
    }

    pub fn with_shadow_flag(mut self, casts_shadow: bool) -> Self {
        self.casts_shadow = Some(casts_shadow);
        self
    }

    pub fn with_editor_id(mut self, editor_id: u128) -> Self {
        self.editor_id = Some(editor_id);
        self
    }

    pub fn with_particle_system(mut self, system: SerializedParticleSystem) -> Self {
        self.particle_system = Some(system);
        self
    }

    pub fn with_particle_emitter<T>(mut self, emitter: T) -> Self
    where
        T: Into<SerializedParticleEmitter>,
    {
        self.particle_emitter = Some(emitter.into());
        self
    }

    pub fn with_particle_behavior(mut self, behavior: SerializedParticleBehavior) -> Self {
        self.particle_behavior = Some(behavior);
        self
    }

    pub fn with_environment(mut self, component: EnvironmentComponent) -> Self {
        self.environment = Some(component);
        self
    }

    pub fn with_camera(mut self, component: CameraComponent) -> Self {
        self.camera = Some(component);
        self
    }

    pub fn build(self) -> SceneAssetEntity {
        SceneAssetEntity {
            name: self.name,
            transform: self.transform,
            visible: self.visible,
            mesh_handle: self.mesh_handle,
            primitive_mesh: self.primitive_mesh,
            mesh_bounds: self.mesh_bounds,
            material: self.material,
            material_data: self.material_data,
            parent: self.parent,
            children: self.children,
            gltf_node: self.gltf_node,
            gltf_material: self.gltf_material,
            gltf_source: self.gltf_source,
            gltf_primitive: self.gltf_primitive,
            script: self.script,
            directional_light: self.directional_light,
            point_light: self.point_light,
            spot_light: self.spot_light,
            casts_shadow: self.casts_shadow,
            editor_id: self.editor_id,
            particle_system: self.particle_system,
            particle_emitter: self.particle_emitter,
            particle_behavior: self.particle_behavior,
            environment: self.environment,
            camera: self.camera,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedParticleSystem {
    pub spawn_rate: f32,
    #[serde(default)]
    pub behavior: Option<ParticleBehaviorPreset>,
    #[serde(default)]
    pub behavior_config: Option<ParticleBehaviorConfig>,
    #[serde(default)]
    pub render_mode: ParticleRenderBlendMode,
}

impl From<&ParticleSystemComponent> for SerializedParticleSystem {
    fn from(component: &ParticleSystemComponent) -> Self {
        Self {
            spawn_rate: component.spawn_rate,
            behavior: Some(component.behavior),
            behavior_config: Some(component.behavior_config.clone()),
            render_mode: component.render_mode,
        }
    }
}

impl From<ParticleSystemComponent> for SerializedParticleSystem {
    fn from(component: ParticleSystemComponent) -> Self {
        Self::from(&component)
    }
}

impl From<SerializedParticleSystem> for ParticleSystemComponent {
    fn from(serialized: SerializedParticleSystem) -> Self {
        let preset = serialized.behavior.unwrap_or_default();
        let mut component = ParticleSystemComponent::new(serialized.spawn_rate, preset);
        if let Some(config) = serialized.behavior_config {
            component.behavior_config = config.ensure_variant(preset);
        }
        component.render_mode = serialized.render_mode;
        component
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedParticleBehavior {
    pub preset: ParticleBehaviorPreset,
    #[serde(default)]
    pub config: ParticleBehaviorConfig,
}

impl SerializedParticleBehavior {
    fn config_for_preset(&self) -> ParticleBehaviorConfig {
        self.config.clone().ensure_variant(self.preset)
    }

    pub fn apply_to_component(&self, component: &mut ParticleSystemComponent) {
        component.set_behavior(self.preset);
        component.behavior_config = self.config_for_preset();
    }

    pub fn into_component_with_spawn_rate(self, spawn_rate: f32) -> ParticleSystemComponent {
        let mut component = ParticleSystemComponent::new(spawn_rate, self.preset);
        component.behavior_config = self.config.ensure_variant(self.preset);
        component
    }
}

impl From<&ParticleSystemComponent> for SerializedParticleBehavior {
    fn from(component: &ParticleSystemComponent) -> Self {
        Self {
            preset: component.behavior,
            config: component.behavior_config.clone(),
        }
    }
}

impl From<ParticleSystemComponent> for SerializedParticleBehavior {
    fn from(component: ParticleSystemComponent) -> Self {
        Self::from(&component)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedParticleEmitter {
    pub spawn_rate: f32,
    #[serde(default)]
    pub burst_count: Option<u32>,
    #[serde(default)]
    pub position: [f32; 3],
    #[serde(default)]
    pub emission_shape: ParticleEmissionShape,
    #[serde(default)]
    pub initial_velocity_range: ParticleVec3Range,
    #[serde(default = "SerializedParticleEmitter::default_scale_range")]
    pub initial_scale_range: ParticleVec3Range,
    #[serde(default = "SerializedParticleEmitter::default_lifetime_range")]
    pub lifetime_range: ParticleFloatRange,
    #[serde(default)]
    pub color_gradient: ParticleColorGradient,
    #[serde(default)]
    pub size_curve: ParticleSizeCurve,
    #[serde(default)]
    pub radial_velocity: ParticleFloatRange,
    #[serde(default)]
    pub auto_respawn: bool,
}

impl SerializedParticleEmitter {
    const fn default_scale_range() -> ParticleVec3Range {
        ParticleVec3Range::splat(1.0)
    }

    const fn default_lifetime_range() -> ParticleFloatRange {
        ParticleFloatRange::new(5.0, 5.0)
    }
}

impl From<&ParticleEmitterComponent> for SerializedParticleEmitter {
    fn from(component: &ParticleEmitterComponent) -> Self {
        Self {
            spawn_rate: component.spawn_rate,
            burst_count: component.burst_count,
            position: component.position,
            emission_shape: component.emission_shape.clone(),
            initial_velocity_range: component.initial_velocity_range,
            initial_scale_range: component.initial_scale_range,
            lifetime_range: component.lifetime_range,
            color_gradient: component.color_gradient.clone(),
            size_curve: component.size_curve.clone(),
            radial_velocity: component.radial_velocity,
            auto_respawn: component.auto_respawn,
        }
    }
}

impl From<ParticleEmitterComponent> for SerializedParticleEmitter {
    fn from(component: ParticleEmitterComponent) -> Self {
        Self::from(&component)
    }
}

impl From<SerializedParticleEmitter> for ParticleEmitterComponent {
    fn from(serialized: SerializedParticleEmitter) -> Self {
        ParticleEmitterComponent {
            spawn_rate: serialized.spawn_rate,
            burst_count: serialized.burst_count,
            position: serialized.position,
            emission_shape: serialized.emission_shape,
            initial_velocity_range: serialized.initial_velocity_range,
            initial_scale_range: serialized.initial_scale_range,
            lifetime_range: serialized.lifetime_range,
            color_gradient: serialized.color_gradient,
            size_curve: serialized.size_curve,
            radial_velocity: serialized.radial_velocity,
            auto_respawn: serialized.auto_respawn,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SerializedMeshBounds {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl From<MeshBounds> for SerializedMeshBounds {
    fn from(bounds: MeshBounds) -> Self {
        Self {
            min: bounds.min.to_array(),
            max: bounds.max.to_array(),
        }
    }
}

impl From<SerializedMeshBounds> for MeshBounds {
    fn from(serialized: SerializedMeshBounds) -> Self {
        MeshBounds::new(
            glam::Vec3::from_array(serialized.min),
            glam::Vec3::from_array(serialized.max),
        )
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SerializedDirectionalLight {
    pub color: [f32; 3],
    pub intensity: f32,
    pub shadow_size: f32,
}

impl From<DirectionalLight> for SerializedDirectionalLight {
    fn from(light: DirectionalLight) -> Self {
        Self {
            color: light.color.to_array(),
            intensity: light.intensity,
            shadow_size: light.shadow_size,
        }
    }
}

impl From<SerializedDirectionalLight> for DirectionalLight {
    fn from(serialized: SerializedDirectionalLight) -> Self {
        let mut light = DirectionalLight::new(
            glam::Vec3::from_array(serialized.color),
            serialized.intensity,
        );
        light.shadow_size = serialized.shadow_size;
        light
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SerializedPointLight {
    pub color: [f32; 3],
    pub intensity: f32,
    pub range: f32,
}

impl From<PointLight> for SerializedPointLight {
    fn from(light: PointLight) -> Self {
        Self {
            color: light.color.to_array(),
            intensity: light.intensity,
            range: light.range,
        }
    }
}

impl From<SerializedPointLight> for PointLight {
    fn from(serialized: SerializedPointLight) -> Self {
        PointLight {
            color: glam::Vec3::from_array(serialized.color),
            intensity: serialized.intensity,
            range: serialized.range,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SerializedSpotLight {
    pub color: [f32; 3],
    pub intensity: f32,
    pub inner_angle: f32,
    pub outer_angle: f32,
    pub range: f32,
}

impl From<SpotLight> for SerializedSpotLight {
    fn from(light: SpotLight) -> Self {
        Self {
            color: light.color.to_array(),
            intensity: light.intensity,
            inner_angle: light.inner_angle,
            outer_angle: light.outer_angle,
            range: light.range,
        }
    }
}

impl From<SerializedSpotLight> for SpotLight {
    fn from(serialized: SerializedSpotLight) -> Self {
        SpotLight {
            color: glam::Vec3::from_array(serialized.color),
            intensity: serialized.intensity,
            inner_angle: serialized.inner_angle,
            outer_angle: serialized.outer_angle,
            range: serialized.range,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SerializedTextureSlot {
    pub path: Option<PathBuf>,
    pub name: Option<String>,
    pub index: Option<u32>,
}

impl SerializedTextureSlot {
    fn from_index(index: u32) -> Self {
        Self {
            path: None,
            name: None,
            index: Some(index),
        }
    }

    fn remap(&mut self, mapping: &std::collections::HashMap<u32, u32>) {
        if let Some(index) = self.index {
            if let Some(&mapped) = mapping.get(&index) {
                self.index = Some(mapped);
            }
        }
    }

    fn is_empty(&self) -> bool {
        self.path.is_none() && self.name.is_none() && self.index.is_none()
    }
}

impl Serialize for SerializedTextureSlot {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        if self.path.is_none() && self.name.is_none() {
            if let Some(index) = self.index {
                serializer.serialize_u32(index)
            } else {
                serializer.serialize_none()
            }
        } else {
            let mut entries = 0;
            if self.path.is_some() {
                entries += 1;
            }
            if self.name.is_some() {
                entries += 1;
            }
            if self.path.is_none() && self.index.is_some() {
                entries += 1;
            }

            let mut map = serializer.serialize_map(Some(entries))?;
            if let Some(path) = &self.path {
                map.serialize_entry("path", &path.to_string_lossy().to_string())?;
            }
            if let Some(name) = &self.name {
                map.serialize_entry("name", name)?;
            }
            if self.path.is_none() {
                if let Some(index) = self.index {
                    map.serialize_entry("index", &index)?;
                }
            }
            map.end()
        }
    }
}

impl<'de> Deserialize<'de> for SerializedTextureSlot {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct SlotVisitor;

        impl<'de> Visitor<'de> for SlotVisitor {
            type Value = SerializedTextureSlot;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a texture index or metadata map")
            }

            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(SerializedTextureSlot::from_index(value as u32))
            }

            fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                if value < 0 {
                    return Err(E::custom("negative texture index"));
                }
                Ok(SerializedTextureSlot::from_index(value as u32))
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(SerializedTextureSlot {
                    path: Some(PathBuf::from(value)),
                    name: None,
                    index: None,
                })
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                self.visit_str(&value)
            }

            fn visit_none<E>(self) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(SerializedTextureSlot::default())
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(SerializedTextureSlot::default())
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut path: Option<PathBuf> = None;
                let mut name: Option<String> = None;
                let mut index: Option<u32> = None;

                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "path" => {
                            let value: String = map.next_value()?;
                            path = Some(PathBuf::from(value));
                        }
                        "name" => {
                            name = Some(map.next_value()?);
                        }
                        "index" => {
                            index = Some(map.next_value()?);
                        }
                        _ => {
                            let _: serde_json::Value = map.next_value()?;
                        }
                    }
                }

                Ok(SerializedTextureSlot { path, name, index })
            }
        }

        deserializer.deserialize_any(SlotVisitor)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedMaterial {
    pub base_color: [u8; 4],
    pub flags: u32,
    #[serde(default, skip_serializing_if = "SerializedTextureSlot::is_empty")]
    pub base_color_texture: SerializedTextureSlot,
    #[serde(default, skip_serializing_if = "SerializedTextureSlot::is_empty")]
    pub metallic_roughness_texture: SerializedTextureSlot,
    #[serde(default, skip_serializing_if = "SerializedTextureSlot::is_empty")]
    pub normal_texture: SerializedTextureSlot,
    #[serde(default, skip_serializing_if = "SerializedTextureSlot::is_empty")]
    pub emissive_texture: SerializedTextureSlot,
    #[serde(default, skip_serializing_if = "SerializedTextureSlot::is_empty")]
    pub occlusion_texture: SerializedTextureSlot,
    pub metallic_factor: u8,
    pub roughness_factor: u8,
    pub emissive_strength: u8,
}

impl From<Material> for SerializedMaterial {
    fn from(material: Material) -> Self {
        Self {
            base_color: material.base_color,
            flags: material.flags.bits(),
            base_color_texture: SerializedTextureSlot::from_index(material.base_color_texture),
            metallic_roughness_texture: SerializedTextureSlot::from_index(
                material.metallic_roughness_texture,
            ),
            normal_texture: SerializedTextureSlot::from_index(material.normal_texture),
            emissive_texture: SerializedTextureSlot::from_index(material.emissive_texture),
            occlusion_texture: SerializedTextureSlot::from_index(material.occlusion_texture),
            metallic_factor: material.metallic_factor,
            roughness_factor: material.roughness_factor,
            emissive_strength: material.emissive_strength,
        }
    }
}

impl SerializedMaterial {
    fn remap_textures(&mut self, mapping: &std::collections::HashMap<u32, u32>) {
        self.base_color_texture.remap(mapping);
        self.metallic_roughness_texture.remap(mapping);
        self.normal_texture.remap(mapping);
        self.emissive_texture.remap(mapping);
        self.occlusion_texture.remap(mapping);
    }

    fn texture_slot(&self, slot: MaterialTextureSlot) -> &SerializedTextureSlot {
        match slot {
            MaterialTextureSlot::BaseColor => &self.base_color_texture,
            MaterialTextureSlot::MetallicRoughness => &self.metallic_roughness_texture,
            MaterialTextureSlot::Normal => &self.normal_texture,
            MaterialTextureSlot::Emissive => &self.emissive_texture,
            MaterialTextureSlot::Occlusion => &self.occlusion_texture,
        }
    }

    fn texture_slot_mut(&mut self, slot: MaterialTextureSlot) -> &mut SerializedTextureSlot {
        match slot {
            MaterialTextureSlot::BaseColor => &mut self.base_color_texture,
            MaterialTextureSlot::MetallicRoughness => &mut self.metallic_roughness_texture,
            MaterialTextureSlot::Normal => &mut self.normal_texture,
            MaterialTextureSlot::Emissive => &mut self.emissive_texture,
            MaterialTextureSlot::Occlusion => &mut self.occlusion_texture,
        }
    }

    pub fn from_material_asset(asset: &MaterialAsset) -> Self {
        let mut serialized = SerializedMaterial::from(*asset.material());
        let project_root = active_project_root();
        let canonical_project_root = project_root
            .as_ref()
            .and_then(|root| std::fs::canonicalize(root).ok())
            .map(normalize_absolute_path);

        for slot in MaterialTextureSlot::all() {
            if let Some(reference) = asset.texture_reference(slot) {
                let slot_mut = serialized.texture_slot_mut(slot);
                if let Some(path) = reference.canonical_path() {
                    let mut stored_path = path.to_path_buf();
                    if let Some(root) = project_root.as_deref() {
                        stored_path = relativize_path_to_project(
                            stored_path,
                            root,
                            canonical_project_root.as_deref(),
                        );
                    }
                    slot_mut.path = Some(stored_path);
                }
                if let Some(name) = reference.display_name() {
                    slot_mut.name = Some(name.to_string());
                }
            }
        }

        serialized
    }

    pub fn resolve_material(
        &self,
        assets: &mut Assets,
        mut renderer: Option<&mut dyn SceneImportDevice>,
    ) -> (
        Material,
        Vec<(MaterialTextureSlot, Option<MaterialTextureReference>)>,
    ) {
        let mut material = Material::from(self.clone());
        let mut references = Vec::new();

        for slot in MaterialTextureSlot::all() {
            let slot_data = self.texture_slot(slot);
            let mut resolved_path = slot_data.path.as_ref().map(resolve_project_path);

            let texture_index = if let Some(resolved) = resolved_path.as_deref() {
                with_import_device(&mut renderer, |device| {
                    assets.resolve_texture_index(slot, Some(resolved), device, false)
                })
            } else if let Some(index) = slot_data.index {
                index
            } else {
                Assets::default_texture_index(slot)
            };

            match slot {
                MaterialTextureSlot::BaseColor => material.base_color_texture = texture_index,
                MaterialTextureSlot::MetallicRoughness => {
                    material.metallic_roughness_texture = texture_index
                }
                MaterialTextureSlot::Normal => material.normal_texture = texture_index,
                MaterialTextureSlot::Emissive => material.emissive_texture = texture_index,
                MaterialTextureSlot::Occlusion => material.occlusion_texture = texture_index,
            }

            let reference = if let Some(path_buf) = resolved_path.take() {
                let canonical = path_buf.canonicalize().unwrap_or_else(|_| path_buf.clone());
                Some(MaterialTextureReference::new(
                    Some(canonical),
                    slot_data.name.clone(),
                ))
            } else if slot_data.name.is_some() {
                let mut reference = MaterialTextureReference::default();
                reference.set_display_name(slot_data.name.clone());
                Some(reference)
            } else {
                None
            };

            references.push((slot, reference));
        }

        (material, references)
    }

    pub fn apply_metadata_to_asset(&self, asset: &mut MaterialAsset) {
        for slot in MaterialTextureSlot::all() {
            let slot_data = self.texture_slot(slot);
            if let Some(path) = slot_data.path.as_ref() {
                let resolved = resolve_project_path(path);
                let canonical = resolved.canonicalize().unwrap_or_else(|_| resolved.clone());
                asset.set_texture_reference(
                    slot,
                    MaterialTextureReference::new(Some(canonical), slot_data.name.clone()),
                );
            } else if slot_data.name.is_some() {
                let mut reference = MaterialTextureReference::default();
                reference.set_display_name(slot_data.name.clone());
                asset.set_texture_reference(slot, reference);
            } else {
                asset.clear_texture_reference(slot);
            }
        }
    }
}

impl From<SerializedMaterial> for Material {
    fn from(serialized: SerializedMaterial) -> Self {
        Material {
            base_color: serialized.base_color,
            flags: MaterialFlags::from_bits(serialized.flags),
            base_color_texture: serialized
                .base_color_texture
                .index
                .unwrap_or(DEFAULT_WHITE_TEXTURE_INDEX),
            metallic_roughness_texture: serialized
                .metallic_roughness_texture
                .index
                .unwrap_or(DEFAULT_METALLIC_ROUGHNESS_TEXTURE_INDEX),
            normal_texture: serialized
                .normal_texture
                .index
                .unwrap_or(DEFAULT_NORMAL_TEXTURE_INDEX),
            emissive_texture: serialized
                .emissive_texture
                .index
                .unwrap_or(DEFAULT_WHITE_TEXTURE_INDEX),
            occlusion_texture: serialized
                .occlusion_texture
                .index
                .unwrap_or(DEFAULT_WHITE_TEXTURE_INDEX),
            metallic_factor: serialized.metallic_factor,
            roughness_factor: serialized.roughness_factor,
            emissive_strength: serialized.emissive_strength,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneTreeAsset {
    pub name: String,
    pub root: SceneTreeAssetNode,
}

impl SceneTreeAsset {
    pub fn new(name: impl Into<String>, root: SceneTreeAssetNode) -> Self {
        Self {
            name: name.into(),
            root,
        }
    }

    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string_pretty(self)
    }

    pub fn from_json(json: &str) -> serde_json::Result<Self> {
        serde_json::from_str(json)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneTreeAssetNode {
    pub name: String,
    pub transform: SerializedTransform,
    pub asset: Option<SceneAsset>,
    #[serde(default)]
    pub children: Vec<SceneTreeAssetNode>,
}

impl SceneTreeAssetNode {
    pub fn builder(name: impl Into<String>) -> SceneTreeAssetNodeBuilder {
        SceneTreeAssetNodeBuilder::new(name)
    }
}

pub struct SceneTreeAssetNodeBuilder {
    name: String,
    transform: SerializedTransform,
    asset: Option<SceneAsset>,
    children: Vec<SceneTreeAssetNode>,
}

impl SceneTreeAssetNodeBuilder {
    fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            transform: SerializedTransform::identity(),
            asset: None,
            children: Vec::new(),
        }
    }

    pub fn with_transform(mut self, transform: SerializedTransform) -> Self {
        self.transform = transform;
        self
    }

    pub fn with_asset(mut self, asset: SceneAsset) -> Self {
        self.asset = Some(asset);
        self
    }

    pub fn with_children<I>(mut self, children: I) -> Self
    where
        I: IntoIterator<Item = SceneTreeAssetNode>,
    {
        self.children = children.into_iter().collect();
        self
    }

    pub fn push_child(&mut self, child: SceneTreeAssetNode) {
        self.children.push(child);
    }

    pub fn add_child(mut self, child: SceneTreeAssetNode) -> Self {
        self.children.push(child);
        self
    }

    pub fn build(self) -> SceneTreeAssetNode {
        SceneTreeAssetNode {
            name: self.name,
            transform: self.transform,
            asset: self.asset,
            children: self.children,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedTransform {
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

impl SerializedTransform {
    pub fn identity() -> Self {
        Transform::IDENTITY.into()
    }
}

impl From<Transform> for SerializedTransform {
    fn from(transform: Transform) -> Self {
        Self {
            translation: [
                transform.translation.x,
                transform.translation.y,
                transform.translation.z,
            ],
            rotation: [
                transform.rotation.x,
                transform.rotation.y,
                transform.rotation.z,
                transform.rotation.w,
            ],
            scale: [transform.scale.x, transform.scale.y, transform.scale.z],
        }
    }
}

impl From<SerializedTransform> for Transform {
    fn from(serialized: SerializedTransform) -> Self {
        Self {
            translation: glam::Vec3::from_array(serialized.translation),
            rotation: glam::Quat::from_array(serialized.rotation),
            scale: glam::Vec3::from_array(serialized.scale),
        }
    }
}

pub(crate) fn serialize_world(
    world: &World,
    assets: &Assets,
) -> (Vec<SceneAssetEntity>, HashMap<Entity, usize>) {
    let mut entities: Vec<Entity> = Vec::new();
    for (entity, _) in world.query::<()>().iter() {
        entities.push(entity);
    }

    let index_map: HashMap<Entity, usize> = entities
        .iter()
        .enumerate()
        .map(|(idx, entity)| (*entity, idx))
        .collect();

    let serialized = entities
        .iter()
        .map(|entity| SceneAssetEntity::from_world_entity(*entity, world, assets, &index_map))
        .collect();

    (serialized, index_map)
}

pub(crate) fn build_tree_asset_node(
    node_name: &str,
    local_transform: Transform,
    asset: Option<SceneAsset>,
    children: Vec<SceneTreeAssetNode>,
) -> SceneTreeAssetNode {
    SceneTreeAssetNode {
        name: node_name.to_string(),
        transform: SerializedTransform::from(local_transform),
        asset,
        children,
    }
}

pub struct SceneAssetBuilder {
    name: String,
    root_transform: SerializedTransform,
    entities: Vec<SceneAssetEntity>,
    animations: Vec<SerializedAnimationClip>,
    animation_states: Vec<AnimationState>,
    active_camera: Option<usize>,
}

impl SceneAssetBuilder {
    fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            root_transform: SerializedTransform::identity(),
            entities: Vec::new(),
            animations: Vec::new(),
            animation_states: Vec::new(),
            active_camera: None,
        }
    }

    pub fn with_root_transform(mut self, transform: SerializedTransform) -> Self {
        self.root_transform = transform;
        self
    }

    pub fn add_entity(mut self, entity: SceneAssetEntity) -> Self {
        self.entities.push(entity);
        self
    }

    pub fn add_animation(mut self, clip: SerializedAnimationClip) -> Self {
        self.animations.push(clip);
        self
    }

    pub fn add_animation_state(mut self, state: AnimationState) -> Self {
        self.animation_states.push(state);
        self
    }

    pub fn with_active_camera(mut self, index: Option<usize>) -> Self {
        self.active_camera = index;
        self
    }

    pub fn build(self) -> SceneAsset {
        SceneAsset {
            name: self.name,
            root_transform: self.root_transform,
            entities: self.entities,
            animations: self.animations,
            animation_states: self.animation_states,
            mesh_data: Vec::new(),
            active_camera: self.active_camera,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::tempdir;

    fn make_material(flags: MaterialFlags, base_texture: u32) -> SerializedMaterial {
        SerializedMaterial {
            base_color: [255, 255, 255, 255],
            flags: flags.bits(),
            base_color_texture: SerializedTextureSlot::from_index(base_texture),
            metallic_roughness_texture: SerializedTextureSlot::from_index(0),
            normal_texture: SerializedTextureSlot::from_index(0),
            emissive_texture: SerializedTextureSlot::from_index(0),
            occlusion_texture: SerializedTextureSlot::from_index(0),
            metallic_factor: 0,
            roughness_factor: 0,
            emissive_strength: 0,
        }
    }

    #[test]
    fn serialized_transform_roundtrip() {
        let transform = Transform::from_trs(
            Vec3::new(1.0, 2.0, 3.0),
            glam::Quat::from_rotation_y(1.2),
            Vec3::new(0.5, 0.75, 1.25),
        );

        let serialized = SerializedTransform::from(transform);
        let restored: Transform = serialized.into();

        assert!(restored
            .translation
            .abs_diff_eq(transform.translation, 1e-5));
        assert!(restored.rotation.abs_diff_eq(transform.rotation, 1e-5));
        assert!(restored.scale.abs_diff_eq(transform.scale, 1e-5));
    }

    #[test]
    fn texture_indices_can_be_rebased_multiple_times() {
        let mut asset = SceneAsset {
            name: "Test".into(),
            root_transform: SerializedTransform::identity(),
            entities: vec![SceneAssetEntity {
                name: Some("Entity".into()),
                transform: SerializedTransform::identity(),
                visible: true,
                mesh_handle: None,
                primitive_mesh: None,
                mesh_bounds: None,
                material: None,
                material_data: Some(make_material(MaterialFlags::USE_BASE_COLOR_TEXTURE, 10)),
                parent: None,
                children: Vec::new(),
                gltf_node: None,
                gltf_material: None,
                gltf_source: None,
                gltf_primitive: None,
                script: None,
                directional_light: None,
                point_light: None,
                spot_light: None,
                casts_shadow: None,
                editor_id: None,
                particle_system: None,
                particle_emitter: None,
                particle_behavior: None,
                environment: None,
                camera: None,
            }],
            animations: Vec::new(),
            animation_states: Vec::new(),
            mesh_data: Vec::new(),
            active_camera: None,
        };

        let mut to_local = std::collections::HashMap::new();
        to_local.insert(10, 0);
        asset.apply_resource_mappings(0, &to_local);

        let material = asset.entities[0]
            .material_data
            .as_ref()
            .expect("material present");
        assert_eq!(material.base_color_texture.index, Some(0));

        let mut to_global = std::collections::HashMap::new();
        to_global.insert(0, 42);
        asset.apply_resource_mappings(0, &to_global);

        let material = asset.entities[0]
            .material_data
            .as_ref()
            .expect("material present");
        assert_eq!(material.base_color_texture.index, Some(42));
    }

    #[test]
    fn persist_material_assets_writes_meta_mapping() {
        let project_dir = tempdir().unwrap();
        let project_root = project_dir.path();
        let gltf_dir = project_root.join("content/models/sample");
        std::fs::create_dir_all(&gltf_dir).unwrap();

        let mut asset = SceneAsset {
            name: "Test".into(),
            root_transform: SerializedTransform::identity(),
            entities: vec![SceneAssetEntity {
                name: Some("Entity".into()),
                transform: SerializedTransform::identity(),
                visible: true,
                mesh_handle: None,
                primitive_mesh: None,
                mesh_bounds: None,
                material: None,
                material_data: Some(make_material(MaterialFlags::NONE, 0)),
                parent: None,
                children: Vec::new(),
                gltf_node: None,
                gltf_material: Some(0),
                gltf_source: Some(PathBuf::from("content/models/sample/scene.gltf")),
                gltf_primitive: None,
                script: None,
                directional_light: None,
                point_light: None,
                spot_light: None,
                casts_shadow: None,
                editor_id: None,
                particle_system: None,
                particle_emitter: None,
                particle_behavior: None,
                environment: None,
                camera: None,
            }],
            animations: Vec::new(),
            animation_states: Vec::new(),
            mesh_data: Vec::new(),
            active_camera: None,
        };

        asset.persist_material_assets(project_root).unwrap();

        let entity = &asset.entities[0];
        let material_handle = entity.material.as_ref().expect("material handle assigned");
        let meta_path = gltf_dir.join("meta.json");
        assert!(meta_path.exists(), "meta file should be created");

        let material_file = material_handle
            .path()
            .file_name()
            .and_then(|name| name.to_str())
            .expect("material file name available");
        assert_eq!(material_file, "scene_000.mat.json");

        let contents = std::fs::read_to_string(meta_path).unwrap();
        let parsed: BTreeMap<String, ImportedGltfMeta> = serde_json::from_str(&contents).unwrap();
        let meta = parsed
            .get("scene.gltf")
            .expect("meta entry for glTF present");
        assert_eq!(
            meta.materials.get(&0),
            Some(&material_handle.path().to_path_buf()),
            "meta should point at generated material path"
        );
    }

    #[test]
    fn persist_material_assets_serializes_relative_texture_paths() {
        let project_dir = tempdir().unwrap();
        let project_root = project_dir.path();

        let texture_dir = project_root.join(CONTENT_DIR).join("textures");
        fs::create_dir_all(&texture_dir).unwrap();
        let texture_path = texture_dir.join("albedo.png");
        fs::write(&texture_path, b"dummy").unwrap();

        let mut material = make_material(MaterialFlags::USE_BASE_COLOR_TEXTURE, 0);
        material.base_color_texture.path = Some(texture_path.clone());
        material
            .base_color_texture
            .name
            .get_or_insert_with(|| "albedo.png".to_string());

        let mut asset = SceneAsset {
            name: "Test".into(),
            root_transform: SerializedTransform::identity(),
            entities: vec![SceneAssetEntity {
                name: Some("Entity".into()),
                transform: SerializedTransform::identity(),
                visible: true,
                mesh_handle: None,
                primitive_mesh: None,
                mesh_bounds: None,
                material: None,
                material_data: Some(material),
                parent: None,
                children: Vec::new(),
                gltf_node: None,
                gltf_material: Some(0),
                gltf_source: Some(PathBuf::from("content/models/sample/scene.gltf")),
                gltf_primitive: None,
                script: None,
                directional_light: None,
                point_light: None,
                spot_light: None,
                casts_shadow: None,
                editor_id: None,
                particle_system: None,
                particle_emitter: None,
                particle_behavior: None,
                environment: None,
                camera: None,
            }],
            animations: Vec::new(),
            animation_states: Vec::new(),
            mesh_data: Vec::new(),
            active_camera: None,
        };

        asset.persist_material_assets(project_root).unwrap();

        let entity = &asset.entities[0];
        let expected_rel = PathBuf::from(CONTENT_DIR)
            .join("textures")
            .join("albedo.png");

        let stored_material = entity
            .material_data
            .as_ref()
            .expect("material data should be restored");
        assert_eq!(
            stored_material
                .base_color_texture
                .path
                .as_ref()
                .expect("path should be present"),
            &expected_rel
        );
        assert!(
            stored_material
                .base_color_texture
                .path
                .as_ref()
                .unwrap()
                .is_relative(),
            "stored material data should be relative"
        );

        let material_handle = entity.material.as_ref().expect("material handle assigned");
        let abs_material_path = project_root.join(material_handle.path());
        let contents = fs::read_to_string(abs_material_path).unwrap();
        let parsed: SerializedMaterial = serde_json::from_str(&contents).unwrap();
        assert_eq!(
            parsed
                .base_color_texture
                .path
                .as_ref()
                .expect("serialized material should contain texture path"),
            &expected_rel
        );
        assert!(
            parsed
                .base_color_texture
                .path
                .as_ref()
                .unwrap()
                .is_relative(),
            "material file should contain relative texture path"
        );
    }

    #[test]
    fn persist_material_assets_reuses_cached_paths_for_duplicate_materials() {
        let project_dir = tempdir().unwrap();
        let project_root = project_dir.path();
        let gltf_dir = project_root.join("content/models/sample");
        std::fs::create_dir_all(&gltf_dir).unwrap();

        let gltf_source = PathBuf::from("content/models/sample/My Scene 01.gltf");

        let mut asset = SceneAsset {
            name: "Test".into(),
            root_transform: SerializedTransform::identity(),
            entities: vec![
                SceneAssetEntity {
                    name: Some("Entity A".into()),
                    transform: SerializedTransform::identity(),
                    visible: true,
                    mesh_handle: None,
                    primitive_mesh: None,
                    mesh_bounds: None,
                    material: None,
                    material_data: Some(make_material(MaterialFlags::NONE, 0)),
                    parent: None,
                    children: Vec::new(),
                    gltf_node: None,
                    gltf_material: Some(0),
                    gltf_source: Some(gltf_source.clone()),
                    gltf_primitive: None,
                    script: None,
                    directional_light: None,
                    point_light: None,
                    spot_light: None,
                    casts_shadow: None,
                    editor_id: None,
                    particle_system: None,
                    particle_emitter: None,
                    particle_behavior: None,
                    environment: None,
                    camera: None,
                },
                SceneAssetEntity {
                    name: Some("Entity B".into()),
                    transform: SerializedTransform::identity(),
                    visible: true,
                    mesh_handle: None,
                    primitive_mesh: None,
                    mesh_bounds: None,
                    material: None,
                    material_data: Some(make_material(MaterialFlags::NONE, 0)),
                    parent: None,
                    children: Vec::new(),
                    gltf_node: None,
                    gltf_material: Some(0),
                    gltf_source: Some(gltf_source.clone()),
                    gltf_primitive: None,
                    script: None,
                    directional_light: None,
                    point_light: None,
                    spot_light: None,
                    casts_shadow: None,
                    editor_id: None,
                    particle_system: None,
                    particle_emitter: None,
                    particle_behavior: None,
                    environment: None,
                    camera: None,
                },
            ],
            animations: Vec::new(),
            animation_states: Vec::new(),
            mesh_data: Vec::new(),
            active_camera: None,
        };

        asset.persist_material_assets(project_root).unwrap();

        let first = asset.entities[0]
            .material
            .as_ref()
            .expect("first material assigned")
            .path()
            .to_path_buf();
        let second = asset.entities[1]
            .material
            .as_ref()
            .expect("second material assigned")
            .path()
            .to_path_buf();

        assert_eq!(first, second, "materials should reuse cached path");
        assert_eq!(
            first
                .file_name()
                .and_then(|name| name.to_str())
                .expect("file name present"),
            "my_scene_01_000.mat.json"
        );
    }

    #[test]
    fn persist_material_assets_adds_suffix_when_colliding() {
        let project_dir = tempdir().unwrap();
        let project_root = project_dir.path();
        let gltf_dir_a = project_root.join("content/models/a");
        let gltf_dir_b = project_root.join("content/models/b");
        std::fs::create_dir_all(&gltf_dir_a).unwrap();
        std::fs::create_dir_all(&gltf_dir_b).unwrap();

        let mut asset = SceneAsset {
            name: "Test".into(),
            root_transform: SerializedTransform::identity(),
            entities: vec![
                SceneAssetEntity {
                    name: Some("Entity A".into()),
                    transform: SerializedTransform::identity(),
                    visible: true,
                    mesh_handle: None,
                    primitive_mesh: None,
                    mesh_bounds: None,
                    material: None,
                    material_data: Some(make_material(MaterialFlags::NONE, 0)),
                    parent: None,
                    children: Vec::new(),
                    gltf_node: None,
                    gltf_material: Some(0),
                    gltf_source: Some(PathBuf::from("content/models/a/Scene.gltf")),
                    gltf_primitive: None,
                    script: None,
                    directional_light: None,
                    point_light: None,
                    spot_light: None,
                    casts_shadow: None,
                    editor_id: None,
                    particle_system: None,
                    particle_emitter: None,
                    particle_behavior: None,
                    environment: None,
                    camera: None,
                },
                SceneAssetEntity {
                    name: Some("Entity B".into()),
                    transform: SerializedTransform::identity(),
                    visible: true,
                    mesh_handle: None,
                    primitive_mesh: None,
                    mesh_bounds: None,
                    material: None,
                    material_data: Some(make_material(MaterialFlags::NONE, 0)),
                    parent: None,
                    children: Vec::new(),
                    gltf_node: None,
                    gltf_material: Some(0),
                    gltf_source: Some(PathBuf::from("content/models/b/Scene.gltf")),
                    gltf_primitive: None,
                    script: None,
                    directional_light: None,
                    point_light: None,
                    spot_light: None,
                    casts_shadow: None,
                    editor_id: None,
                    particle_system: None,
                    particle_emitter: None,
                    particle_behavior: None,
                    environment: None,
                    camera: None,
                },
            ],
            animations: Vec::new(),
            animation_states: Vec::new(),
            mesh_data: Vec::new(),
            active_camera: None,
        };

        asset.persist_material_assets(project_root).unwrap();

        let first_name = asset.entities[0]
            .material
            .as_ref()
            .expect("first material assigned")
            .path()
            .file_name()
            .and_then(|name| name.to_str())
            .expect("file name present");
        let second_name = asset.entities[1]
            .material
            .as_ref()
            .expect("second material assigned")
            .path()
            .file_name()
            .and_then(|name| name.to_str())
            .expect("file name present");

        assert_eq!(first_name, "scene_000.mat.json");
        assert_eq!(second_name, "scene_000_1.mat.json");
    }
}
