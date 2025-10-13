use super::animation::{
    AnimationChannel, AnimationClip, AnimationInterpolation, AnimationOutput, AnimationSampler,
    AnimationState, AnimationTarget, MaterialProperty, TransformProperty,
};
use super::components::{
    Children, GltfMaterial, GltfNode, MaterialComponent, MeshComponent, Name, Parent,
    TransformComponent, Visible,
};
use super::graph::SceneInstance;
use crate::asset::{Assets, Handle, Mesh};
use crate::renderer::material::MaterialFlags;
use crate::renderer::{Material, Texture};
use crate::scene::transform::Transform;
use hecs::{Entity, World};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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

            if let Some(material) = &entity.material {
                builder.add(MaterialComponent(material.clone().into()));
            }

            if let Some(gltf_node) = entity.gltf_node {
                builder.add(GltfNode(gltf_node));
            }

            if let Some(gltf_mat) = entity.gltf_material {
                builder.add(GltfMaterial(gltf_mat));
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

    pub(crate) fn apply_resource_offsets(&mut self, mesh_offset: usize, texture_offset: u32) {
        if mesh_offset == 0 && texture_offset == 0 {
            return;
        }

        for entity in &mut self.entities {
            if let Some(mesh) = &mut entity.mesh_handle {
                *mesh += mesh_offset;
            }

            if let Some(material) = &mut entity.material {
                material.apply_texture_offset(texture_offset);
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct SceneAssetResources {
    meshes: Vec<Mesh>,
    textures: Vec<Texture>,
}

impl SceneAssetResources {
    pub fn new(meshes: Vec<Mesh>, textures: Vec<Texture>) -> Self {
        Self { meshes, textures }
    }

    pub fn builder() -> SceneAssetResourcesBuilder {
        SceneAssetResourcesBuilder::default()
    }

    pub fn is_empty(&self) -> bool {
        self.meshes.is_empty() && self.textures.is_empty()
    }

    fn take_meshes(&mut self) -> Vec<Mesh> {
        std::mem::take(&mut self.meshes)
    }

    fn take_textures(&mut self) -> Vec<Texture> {
        std::mem::take(&mut self.textures)
    }
}

#[derive(Debug, Default)]
pub struct SceneAssetResourcesBuilder {
    meshes: Vec<Mesh>,
    textures: Vec<Texture>,
}

impl SceneAssetResourcesBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_mesh(mut self, mesh: Mesh) -> Self {
        self.meshes.push(mesh);
        self
    }

    pub fn with_texture(mut self, texture: Texture) -> Self {
        self.textures.push(texture);
        self
    }

    pub fn add_mesh(&mut self, mesh: Mesh) {
        self.meshes.push(mesh);
    }

    pub fn add_texture(&mut self, texture: Texture) {
        self.textures.push(texture);
    }

    pub fn build(self) -> SceneAssetResources {
        SceneAssetResources::new(self.meshes, self.textures)
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

    pub fn register_resources(&mut self, assets: &mut Assets) -> bool {
        if self.resources_registered {
            return false;
        }

        let mesh_offset = assets.meshes.len();
        for mesh in self.resources.take_meshes() {
            let _ = assets.meshes.insert(mesh);
        }

        let texture_offset_usize = assets.textures.len();
        let texture_offset = match u32::try_from(texture_offset_usize) {
            Ok(offset) => offset,
            Err(_) => {
                log::warn!(
                    "Texture count {} exceeds u32::MAX; new textures may reference invalid indices",
                    texture_offset_usize
                );
                u32::MAX
            }
        };

        let mut textures_added = false;
        for texture in self.resources.take_textures() {
            let _ = assets.textures.insert(texture);
            textures_added = true;
        }

        self.asset
            .apply_resource_offsets(mesh_offset, texture_offset);

        self.resources_registered = true;
        textures_added
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneAssetEntity {
    pub name: Option<String>,
    pub transform: SerializedTransform,
    pub visible: bool,
    pub mesh_handle: Option<usize>,
    pub material: Option<SerializedMaterial>,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub gltf_node: Option<usize>,
    pub gltf_material: Option<usize>,
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
        let material = world
            .get::<&MaterialComponent>(entity)
            .ok()
            .map(|m| SerializedMaterial::from(m.0));

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

        Self {
            name,
            transform,
            visible,
            mesh_handle,
            material,
            parent,
            children,
            gltf_node,
            gltf_material,
        }
    }
}

pub struct SceneAssetEntityBuilder {
    name: Option<String>,
    transform: SerializedTransform,
    visible: bool,
    mesh_handle: Option<usize>,
    material: Option<SerializedMaterial>,
    parent: Option<usize>,
    children: Vec<usize>,
    gltf_node: Option<usize>,
    gltf_material: Option<usize>,
}

impl SceneAssetEntityBuilder {
    fn new(transform: SerializedTransform) -> Self {
        Self {
            name: None,
            transform,
            visible: true,
            mesh_handle: None,
            material: None,
            parent: None,
            children: Vec::new(),
            gltf_node: None,
            gltf_material: None,
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

    pub fn build(self) -> SceneAssetEntity {
        SceneAssetEntity {
            name: self.name,
            transform: self.transform,
            visible: self.visible,
            mesh_handle: self.mesh_handle,
            material: self.material,
            parent: self.parent,
            children: self.children,
            gltf_node: self.gltf_node,
            gltf_material: self.gltf_material,
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
    fn apply_texture_offset(&mut self, offset: u32) {
        self.base_color_texture = self.base_color_texture.wrapping_add(offset);
        self.metallic_roughness_texture = self.metallic_roughness_texture.wrapping_add(offset);
        self.normal_texture = self.normal_texture.wrapping_add(offset);
        self.emissive_texture = self.emissive_texture.wrapping_add(offset);
        self.occlusion_texture = self.occlusion_texture.wrapping_add(offset);
    }
}

impl From<SerializedMaterial> for Material {
    fn from(serialized: SerializedMaterial) -> Self {
        let mut material = Material::default();
        material.base_color = serialized.base_color;
        material.flags = MaterialFlags::from_bits(serialized.flags);
        material.base_color_texture = serialized.base_color_texture;
        material.metallic_roughness_texture = serialized.metallic_roughness_texture;
        material.normal_texture = serialized.normal_texture;
        material.emissive_texture = serialized.emissive_texture;
        material.occlusion_texture = serialized.occlusion_texture;
        material.metallic_factor = serialized.metallic_factor;
        material.roughness_factor = serialized.roughness_factor;
        material.emissive_strength = serialized.emissive_strength;
        material
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

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

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
}

pub struct SceneAssetBuilder {
    name: String,
    root_transform: SerializedTransform,
    entities: Vec<SceneAssetEntity>,
    animations: Vec<SerializedAnimationClip>,
    animation_states: Vec<AnimationState>,
}

impl SceneAssetBuilder {
    fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            root_transform: SerializedTransform::identity(),
            entities: Vec::new(),
            animations: Vec::new(),
            animation_states: Vec::new(),
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

    pub fn build(self) -> SceneAsset {
        SceneAsset {
            name: self.name,
            root_transform: self.root_transform,
            entities: self.entities,
            animations: self.animations,
            animation_states: self.animation_states,
        }
    }
}
