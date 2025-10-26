use super::animation::{
    AnimationChannel, AnimationClip, AnimationInterpolation, AnimationOutput, AnimationSampler,
    AnimationState, AnimationTarget, MaterialProperty, TransformProperty,
};
use super::components::{
    CameraComponent, CanCastShadow, Children, DirectionalLight, EditorEntityId,
    EnvironmentComponent, GltfMaterial, GltfNode, GltfPrimitive, GltfSource, MaterialComponent,
    MeshBounds, MeshComponent, Name, Parent, ParticleBehaviorConfig, ParticleBehaviorPreset,
    ParticleEmitterComponent, ParticleSystemComponent, PointLight, SpotLight, TransformComponent,
    Visible,
};
use super::graph::SceneInstance;
use super::loader::SceneImportDevice;
use crate::asset::{Assets, Handle, Mesh, MeshData};
use crate::renderer::material::MaterialFlags;
use crate::renderer::{Material, Texture};
use crate::scene::transform::Transform;
use crate::scripting::{RuneScriptComponent, RuneScriptSource};
use hecs::{Entity, World};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::convert::TryFrom;
use std::path::PathBuf;

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

    pub(crate) fn instantiate(&self) -> SceneInstance {
        let mut instance = SceneInstance::new();
        let mut entity_map = Vec::with_capacity(self.entities.len());

        for entity in &self.entities {
            let mut builder = hecs::EntityBuilder::new();

            if let Some(name) = &entity.name {
                builder.add(Name::new(name.clone()));
            }

            builder.add(TransformComponent(Transform::from(
                entity.transform.clone(),
            )));
            builder.add(Visible(entity.visible));

            if let Some(mesh) = entity.mesh_handle {
                builder.add(MeshComponent(Handle::new(mesh)));
            }

            if let Some(bounds) = entity.mesh_bounds {
                builder.add(MeshBounds::from(bounds));
            }

            if let Some(material) = &entity.material {
                builder.add(MaterialComponent(material.clone().into()));
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
                builder.add(ParticleSystemComponent::from(particle_system.clone()));
            }

            if let Some(particle_emitter) = &entity.particle_emitter {
                builder.add(particle_emitter.clone());
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

            // Remap texture indices in materials
            if let Some(material) = &mut entity.material {
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
            if let Some(mesh) = &mut entity.mesh_handle {
                *mesh += mesh_offset;
            }

            if let Some(material) = &mut entity.material {
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
        renderer: &R,
        assets: &mut Assets,
    ) -> ResourceRegistration {
        if self.resources_registered {
            return ResourceRegistration::default();
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
            if let Some(material) = &entity.material {
                log::info!(
                    "Entity {}: base_color_texture={}, metallic_roughness_texture={}, normal_texture={}",
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
            if let Some(material) = &entity.material {
                log::info!("Entity {}: base_color_texture={}, metallic_roughness_texture={}, normal_texture={}", 
            i, material.base_color_texture, material.metallic_roughness_texture, material.normal_texture);
            }
        }

        log::info!("=== MESH-MATERIAL ASSOCIATIONS ===");
        for (i, entity) in self.asset.entities.iter().enumerate() {
            if let Some(mesh) = entity.mesh_handle {
                if let Some(mat) = &entity.material {
                    log::info!(
                        "Entity {}: mesh={}, base_texture={}, metallic_texture={}, normal_texture={}, gltf_mat_idx={:?}",
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
pub struct SceneAssetEntity {
    pub name: Option<String>,
    pub transform: SerializedTransform,
    pub visible: bool,
    pub mesh_handle: Option<usize>,
    #[serde(default)]
    pub mesh_bounds: Option<SerializedMeshBounds>,
    pub material: Option<SerializedMaterial>,
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
    pub particle_emitter: Option<ParticleEmitterComponent>,
    #[serde(default)]
    pub environment: Option<EnvironmentComponent>,
    #[serde(default)]
    pub camera: Option<CameraComponent>,
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
        let mesh_bounds = world
            .get::<&MeshBounds>(entity)
            .ok()
            .map(|bounds| SerializedMeshBounds::from(*bounds));
        let material = world
            .get::<&MaterialComponent>(entity)
            .ok()
            .map(|m| SerializedMaterial::from(m.0));

        let particle_system = world
            .get::<&ParticleSystemComponent>(entity)
            .ok()
            .map(|component| SerializedParticleSystem::from((*component).clone()));
        let particle_emitter = world
            .get::<&ParticleEmitterComponent>(entity)
            .ok()
            .map(|component| (*component).clone());

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
            mesh_bounds,
            material,
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
    mesh_bounds: Option<SerializedMeshBounds>,
    material: Option<SerializedMaterial>,
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
    particle_emitter: Option<ParticleEmitterComponent>,
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
            mesh_bounds: None,
            material: None,
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

    pub fn with_mesh_bounds(mut self, bounds: SerializedMeshBounds) -> Self {
        self.mesh_bounds = Some(bounds);
        self
    }

    pub fn with_material(mut self, material: SerializedMaterial) -> Self {
        self.material = Some(material);
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

    pub fn with_particle_emitter(mut self, emitter: ParticleEmitterComponent) -> Self {
        self.particle_emitter = Some(emitter);
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
            mesh_bounds: self.mesh_bounds,
            material: self.material,
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
            environment: self.environment,
            camera: self.camera,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedParticleSystem {
    pub spawn_rate: f32,
    #[serde(default)]
    pub behavior: ParticleBehaviorPreset,
    #[serde(default)]
    pub behavior_config: ParticleBehaviorConfig,
}

impl From<ParticleSystemComponent> for SerializedParticleSystem {
    fn from(component: ParticleSystemComponent) -> Self {
        Self {
            spawn_rate: component.spawn_rate,
            behavior: component.behavior,
            behavior_config: component.behavior_config.clone(),
        }
    }
}

impl From<SerializedParticleSystem> for ParticleSystemComponent {
    fn from(serialized: SerializedParticleSystem) -> Self {
        let mut component =
            ParticleSystemComponent::new(serialized.spawn_rate, serialized.behavior);
        component.behavior_config = serialized
            .behavior_config
            .ensure_variant(serialized.behavior);
        component
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedMaterial {
    pub base_color: [u8; 4],
    pub flags: u32,
    pub base_color_texture: u32,
    pub metallic_roughness_texture: u32,
    pub normal_texture: u32,
    pub emissive_texture: u32,
    pub occlusion_texture: u32,
    pub metallic_factor: u8,
    pub roughness_factor: u8,
    pub emissive_strength: u8,
}

impl From<Material> for SerializedMaterial {
    fn from(material: Material) -> Self {
        Self {
            base_color: material.base_color,
            flags: material.flags.bits(),
            base_color_texture: material.base_color_texture,
            metallic_roughness_texture: material.metallic_roughness_texture,
            normal_texture: material.normal_texture,
            emissive_texture: material.emissive_texture,
            occlusion_texture: material.occlusion_texture,
            metallic_factor: material.metallic_factor,
            roughness_factor: material.roughness_factor,
            emissive_strength: material.emissive_strength,
        }
    }
}

impl SerializedMaterial {
    fn remap_textures(&mut self, mapping: &std::collections::HashMap<u32, u32>) {
        if let Some(&mapped) = mapping.get(&self.base_color_texture) {
            self.base_color_texture = mapped;
        }
        if let Some(&mapped) = mapping.get(&self.metallic_roughness_texture) {
            self.metallic_roughness_texture = mapped;
        }
        if let Some(&mapped) = mapping.get(&self.normal_texture) {
            self.normal_texture = mapped;
        }
        if let Some(&mapped) = mapping.get(&self.emissive_texture) {
            self.emissive_texture = mapped;
        }
        if let Some(&mapped) = mapping.get(&self.occlusion_texture) {
            self.occlusion_texture = mapped;
        }
    }
}

impl From<SerializedMaterial> for Material {
    fn from(serialized: SerializedMaterial) -> Self {
        Material {
            base_color: serialized.base_color,
            flags: MaterialFlags::from_bits(serialized.flags),
            base_color_texture: serialized.base_color_texture,
            metallic_roughness_texture: serialized.metallic_roughness_texture,
            normal_texture: serialized.normal_texture,
            emissive_texture: serialized.emissive_texture,
            occlusion_texture: serialized.occlusion_texture,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneTreeAssetHandle {
    pub root: SceneTreeAssetNode,
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

pub(crate) fn serialize_world(world: &World) -> (Vec<SceneAssetEntity>, HashMap<Entity, usize>) {
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
        .map(|entity| SceneAssetEntity::from_world_entity(*entity, world, &index_map))
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

    fn make_material(flags: MaterialFlags, base_texture: u32) -> SerializedMaterial {
        SerializedMaterial {
            base_color: [255, 255, 255, 255],
            flags: flags.bits(),
            base_color_texture: base_texture,
            metallic_roughness_texture: 0,
            normal_texture: 0,
            emissive_texture: 0,
            occlusion_texture: 0,
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
                mesh_bounds: None,
                material: Some(make_material(MaterialFlags::USE_BASE_COLOR_TEXTURE, 10)),
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
            .material
            .as_ref()
            .expect("material present");
        assert_eq!(material.base_color_texture, 0);

        let mut to_global = std::collections::HashMap::new();
        to_global.insert(0, 42);
        asset.apply_resource_mappings(0, &to_global);

        let material = asset.entities[0]
            .material
            .as_ref()
            .expect("material present");
        assert_eq!(material.base_color_texture, 42);
    }
}
