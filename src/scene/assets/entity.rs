use super::builder::SceneAssetEntityBuilder;
use super::prefabs::ScenePrefabRef;
use crate::scene::components::{
    Billboard, CanCastShadow, Children, DirectionalLight, EditorEntityId, EnvironmentComponent,
    GltfMaterial, GltfNode, GltfPrimitive, GltfSource, MaterialComponent, MeshBounds,
    MeshComponent, Name, Parent, ParticleEmitterComponent, ParticleSystemComponent, PointLight,
    PrimitiveMeshComponent, SpotLight, TransformComponent, Visible, CameraComponent,
};
use crate::renderer::primitives::PrimitiveMeshDescriptor;
use super::serialization::{
    path_serde, SerializedBillboard, SerializedDirectionalLight, SerializedMaterial,
    SerializedMeshBounds, SerializedParticleBehavior, SerializedParticleEmitter,
    SerializedParticleSystem, SerializedPointLight, SerializedLuaScript, SerializedSpotLight,
    SerializedTransform,
};
use crate::scene::transform::Transform;
use crate::asset::{Assets, MaterialAsset, MaterialTextureSlot};
use crate::renderer::Material;
use crate::project::relativize_path_to_project;
use crate::scripting::LuaScriptComponent;
use hecs::{Entity, World};
use rand::{distributions::Alphanumeric, thread_rng, Rng};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

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
    pub script: Option<SerializedLuaScript>,
    #[serde(default)]
    pub directional_light: Option<SerializedDirectionalLight>,
    #[serde(default)]
    pub point_light: Option<SerializedPointLight>,
    #[serde(default)]
    pub spot_light: Option<SerializedSpotLight>,
    #[serde(default)]
    pub casts_shadow: Option<bool>,
    #[serde(default)]
    pub billboard: Option<SerializedBillboard>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_ref: Option<ScenePrefabRef>,
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
    script: Option<SerializedLuaScript>,
    #[serde(default)]
    directional_light: Option<SerializedDirectionalLight>,
    #[serde(default)]
    point_light: Option<SerializedPointLight>,
    #[serde(default)]
    spot_light: Option<SerializedSpotLight>,
    #[serde(default)]
    casts_shadow: Option<bool>,
    #[serde(default)]
    billboard: Option<SerializedBillboard>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    scene_ref: Option<ScenePrefabRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum SceneMaterialField {
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
            billboard: data.billboard,
            editor_id: data.editor_id,
            particle_system: data.particle_system,
            particle_emitter: data.particle_emitter,
            particle_behavior: data.particle_behavior,
            environment: data.environment,
            camera: data.camera,
            scene_ref: data.scene_ref,
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
            billboard: entity.billboard,
            editor_id: entity.editor_id,
            particle_system: entity.particle_system,
            particle_emitter: entity.particle_emitter,
            particle_behavior: entity.particle_behavior,
            environment: entity.environment,
            camera: entity.camera,
            scene_ref: entity.scene_ref,
        }
    }
}

pub(crate) fn load_material_asset_from_file(path: &Path) -> Result<MaterialAsset, String> {
    let data = fs::read_to_string(path)
        .map_err(|err| format!("failed to read material asset {:?}: {}", path, err))?;
    let serialized: SerializedMaterial = serde_json::from_str(&data)
        .map_err(|err| format!("failed to parse material asset {:?}: {}", path, err))?;
    let mut asset =
        MaterialAsset::from_material(Material::from(serialized.clone()), path.to_path_buf());
    serialized.apply_metadata_to_asset(&mut asset);
    Ok(asset)
}

pub(crate) fn normalize_material_path(project_root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        match path.strip_prefix(project_root) {
            Ok(stripped) => stripped.to_path_buf(),
            Err(_) => path.to_path_buf(),
        }
    } else {
        path.to_path_buf()
    }
}

pub(crate) fn relativize_material_texture_paths(
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

pub(crate) fn absolute_gltf_source(project_root: &Path, source: &Path) -> PathBuf {
    if source.is_absolute() {
        source.to_path_buf()
    } else {
        project_root.join(source)
    }
}

pub(crate) fn sanitize_gltf_stem(source: &Path) -> String {
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

pub(crate) fn material_file_matches_pattern(
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

pub(crate) fn generate_material_asset_path(
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



impl SceneAssetEntity {
    pub fn builder(transform: SerializedTransform) -> SceneAssetEntityBuilder {
        SceneAssetEntityBuilder::new(transform)
    }

    pub(crate) fn from_world_entity(
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
            .get::<&LuaScriptComponent>(entity)
            .ok()
            .map(|component| SerializedLuaScript::from(&*component));

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

        let billboard = world
            .get::<&Billboard>(entity)
            .ok()
            .map(|component| SerializedBillboard::from(*component));

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
            billboard,
            editor_id,
            particle_system,
            particle_emitter,
            particle_behavior,
            environment,
            camera,
            scene_ref: None,
        }
    }
}

