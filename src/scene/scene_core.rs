use super::animation::{
    AnimationChannel, AnimationClip, AnimationInterpolation, AnimationOutput, AnimationSampler,
    AnimationState, AnimationTarget, MaterialProperty, TransformProperty,
};
use super::components::{
    Children, GltfMaterial, GltfNode, MaterialComponent, MeshComponent, Name, Parent,
    TransformComponent, Visible,
};
use super::internal::{animations, lights, rendering, transforms};
use crate::asset::{Assets, Handle, Mesh};
use crate::environment::Environment;
use crate::renderer::material::MaterialFlags;
use crate::renderer::{CustomRenderRequest, Material, RenderBatcher, Renderer, Texture};
use crate::scene::transform::Transform;
use crate::scene::Camera;
use crate::time::Instant;
use hecs::{Entity, World};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::convert::TryFrom;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SceneNodeId(u32);

impl SceneNodeId {
    fn index(self) -> usize {
        self.0 as usize
    }
}

struct SceneNode {
    name: String,
    parent: Option<SceneNodeId>,
    children: Vec<SceneNodeId>,
    local_transform: Transform,
    world_transform: Transform,
    instance: SceneInstance,
}

impl SceneNode {
    fn new(_id: SceneNodeId, name: impl Into<String>, instance: SceneInstance) -> Self {
        Self {
            name: name.into(),
            parent: None,
            children: Vec::new(),
            local_transform: Transform::IDENTITY,
            world_transform: Transform::IDENTITY,
            instance,
        }
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn set_name(&mut self, name: impl Into<String>) {
        self.name = name.into();
    }

    fn set_parent(&mut self, parent: Option<SceneNodeId>) {
        self.parent = parent;
    }

    fn add_child(&mut self, child: SceneNodeId) {
        self.children.push(child);
    }

    fn remove_child(&mut self, child: SceneNodeId) {
        if let Some(index) = self.children.iter().position(|&id| id == child) {
            self.children.swap_remove(index);
        }
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
    fn from_clip(clip: &AnimationClip, index_map: &HashMap<Entity, usize>) -> Option<Self> {
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

    fn to_clip(&self, entities: &[Entity]) -> Option<AnimationClip> {
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
    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string_pretty(self)
    }

    pub fn from_json(json: &str) -> serde_json::Result<Self> {
        serde_json::from_str(json)
    }

    fn instantiate(&self) -> SceneInstance {
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

        instance.animations = animations;
        instance.animation_states = animation_states;

        instance
    }

    fn apply_resource_offsets(&mut self, mesh_offset: usize, texture_offset: u32) {
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

    pub fn is_empty(&self) -> bool {
        self.meshes.is_empty() && self.textures.is_empty()
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

    pub fn register_resources(&mut self, scene: &mut Scene) -> bool {
        if self.resources_registered {
            return false;
        }

        let mesh_offset = scene.assets.meshes.len();
        for mesh in std::mem::take(&mut self.resources.meshes) {
            let _ = scene.assets.meshes.insert(mesh);
        }

        let texture_offset_usize = scene.assets.textures.len();
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
        for texture in std::mem::take(&mut self.resources.textures) {
            let _ = scene.assets.textures.insert(texture);
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

impl SerializedMaterial {
    fn apply_texture_offset(&mut self, offset: u32) {
        if offset == 0 {
            return;
        }

        let flags = MaterialFlags::from_bits(self.flags);

        if flags.contains(MaterialFlags::USE_BASE_COLOR_TEXTURE) {
            self.base_color_texture = self.base_color_texture.wrapping_add(offset);
        }

        if flags.contains(MaterialFlags::USE_METALLIC_ROUGHNESS_TEXTURE) {
            self.metallic_roughness_texture = self.metallic_roughness_texture.wrapping_add(offset);
        }

        if flags.contains(MaterialFlags::USE_NORMAL_TEXTURE) {
            self.normal_texture = self.normal_texture.wrapping_add(offset);
        }

        if flags.contains(MaterialFlags::USE_EMISSIVE_TEXTURE) {
            self.emissive_texture = self.emissive_texture.wrapping_add(offset);
        }

        if flags.contains(MaterialFlags::USE_OCCLUSION_TEXTURE) {
            self.occlusion_texture = self.occlusion_texture.wrapping_add(offset);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneTreeAsset {
    pub name: String,
    pub root: SceneTreeAssetNode,
}

impl SceneTreeAsset {
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

struct SceneInstance {
    world: World,
    animations: Vec<AnimationClip>,
    animation_states: Vec<AnimationState>,
}

impl SceneInstance {
    fn new() -> Self {
        Self {
            world: World::new(),
            animations: Vec::new(),
            animation_states: Vec::new(),
        }
    }

    fn update(&mut self, dt: f64, absolute_time: f64) {
        let world = &mut self.world;
        let animations = &self.animations;
        let animation_states = &mut self.animation_states;

        animations::advance_animations(world, animations, animation_states, dt);
        animations::update_rotate_animations(world, dt);
        animations::update_orbit_animations(world, absolute_time);
    }

    fn propagate_transforms(&mut self) {
        transforms::propagate_transforms(&mut self.world);
    }

    fn add_animation_clip(&mut self, clip: AnimationClip) -> usize {
        let index = self.animations.len();
        self.animations.push(clip);
        index
    }

    fn play_animation(&mut self, clip_index: usize, looping: bool) -> Option<usize> {
        if clip_index >= self.animations.len() {
            return None;
        }

        let mut state = AnimationState::new(clip_index);
        state.looping = looping;
        let index = self.animation_states.len();
        self.animation_states.push(state);
        Some(index)
    }

    fn animations(&self) -> &[AnimationClip] {
        &self.animations
    }

    fn animation_states(&self) -> &[AnimationState] {
        &self.animation_states
    }

    fn world(&self) -> &World {
        &self.world
    }

    fn world_mut(&mut self) -> &mut World {
        &mut self.world
    }

    fn into_world(self) -> World {
        self.world
    }
}

pub struct Scene {
    pub assets: Assets,
    environment: Environment,
    camera: Camera,
    time: f64,
    last_frame: Option<Instant>,
    nodes: Vec<Option<SceneNode>>,
    free_list: Vec<SceneNodeId>,
    root: SceneNodeId,
    main_scene: SceneNodeId,
}

impl Scene {
    pub fn new() -> Self {
        let mut nodes = Vec::new();
        let root_id = SceneNodeId(0);
        let root_node = SceneNode::new(root_id, "Root", SceneInstance::new());
        nodes.push(Some(root_node));

        Self {
            assets: Assets::default(),
            environment: Environment::default(),
            camera: Camera::default(),
            time: 0.0,
            last_frame: None,
            nodes,
            free_list: Vec::new(),
            root: root_id,
            main_scene: root_id,
        }
    }

    fn node(&self, id: SceneNodeId) -> &SceneNode {
        self.nodes[id.index()].as_ref().expect("Invalid scene node")
    }

    fn node_mut(&mut self, id: SceneNodeId) -> &mut SceneNode {
        self.nodes[id.index()].as_mut().expect("Invalid scene node")
    }

    fn is_valid_node(&self, id: SceneNodeId) -> bool {
        self.nodes
            .get(id.index())
            .and_then(|slot| slot.as_ref())
            .is_some()
    }

    fn nodes_iter(&self) -> impl Iterator<Item = &SceneNode> {
        self.nodes.iter().filter_map(|n| n.as_ref())
    }

    fn nodes_iter_mut(&mut self) -> impl Iterator<Item = &mut SceneNode> {
        self.nodes.iter_mut().filter_map(|n| n.as_mut())
    }

    fn attach_node(&mut self, child: SceneNodeId, parent: SceneNodeId) {
        {
            let child_node = self.node_mut(child);
            child_node.set_parent(Some(parent));
        }
        self.node_mut(parent).add_child(child);
    }

    fn detach_node(&mut self, node: SceneNodeId) -> Option<SceneNodeId> {
        let parent = { self.node(node).parent };
        if let Some(parent_id) = parent {
            {
                let parent_node = self.node_mut(parent_id);
                parent_node.remove_child(node);
            }
            self.node_mut(node).set_parent(None);
        }
        parent
    }

    pub fn root_id(&self) -> SceneNodeId {
        self.root
    }

    pub fn main_scene(&self) -> SceneNodeId {
        self.main_scene
    }

    pub fn set_main_scene(&mut self, node: SceneNodeId) {
        if self.is_valid_node(node) {
            self.main_scene = node;
        }
    }

    pub fn node_name(&self, node: SceneNodeId) -> &str {
        self.node(node).name()
    }

    pub fn set_node_name(&mut self, node: SceneNodeId, name: impl Into<String>) {
        self.node_mut(node).set_name(name);
    }

    pub fn node_parent(&self, node: SceneNodeId) -> Option<SceneNodeId> {
        self.node(node).parent
    }

    pub fn node_children(&self, node: SceneNodeId) -> &[SceneNodeId] {
        &self.node(node).children
    }

    pub fn iter_children(&self, node: SceneNodeId) -> impl Iterator<Item = SceneNodeId> + '_ {
        self.node(node).children.iter().copied()
    }

    pub fn node_local_transform(&self, node: SceneNodeId) -> &Transform {
        &self.node(node).local_transform
    }

    pub fn node_local_transform_mut(&mut self, node: SceneNodeId) -> &mut Transform {
        &mut self.node_mut(node).local_transform
    }

    pub fn node_world_transform(&self, node: SceneNodeId) -> &Transform {
        &self.node(node).world_transform
    }

    pub fn init_timer(&mut self) {
        self.last_frame = Some(Instant::now());
    }

    pub fn time(&self) -> f64 {
        self.time
    }

    pub fn last_frame(&self) -> Instant {
        self.last_frame
            .expect("Scene timer not initialized - call init_timer() first")
    }

    pub fn last_frame_instant(&self) -> Option<Instant> {
        self.last_frame
    }

    pub fn set_last_frame(&mut self, instant: Instant) {
        self.last_frame = Some(instant);
    }

    pub fn camera(&self) -> &Camera {
        &self.camera
    }

    pub fn camera_mut(&mut self) -> &mut Camera {
        &mut self.camera
    }

    pub fn set_camera(&mut self, camera: Camera) {
        self.camera = camera;
    }

    pub fn environment(&self) -> &Environment {
        &self.environment
    }

    pub fn environment_mut(&mut self) -> &mut Environment {
        &mut self.environment
    }

    pub fn set_environment(&mut self, environment: Environment) {
        self.environment = environment;
    }

    pub fn node_animations(&self, node: SceneNodeId) -> &[AnimationClip] {
        self.node(node).instance.animations()
    }

    pub fn node_animation_states(&self, node: SceneNodeId) -> &[AnimationState] {
        self.node(node).instance.animation_states()
    }

    pub fn add_node_animation_clip(&mut self, node: SceneNodeId, clip: AnimationClip) -> usize {
        self.node_mut(node).instance.add_animation_clip(clip)
    }

    pub fn play_node_animation(
        &mut self,
        node: SceneNodeId,
        clip_index: usize,
        looping: bool,
    ) -> Option<usize> {
        self.node_mut(node)
            .instance
            .play_animation(clip_index, looping)
    }

    fn main_instance(&self) -> &SceneInstance {
        &self.node(self.main_scene).instance
    }

    fn main_instance_mut(&mut self) -> &mut SceneInstance {
        &mut self.node_mut(self.main_scene).instance
    }

    pub fn main_world(&self) -> &World {
        self.main_instance().world()
    }

    pub fn main_world_mut(&mut self) -> &mut World {
        self.main_instance_mut().world_mut()
    }

    pub fn world(&self) -> &World {
        self.main_world()
    }

    pub fn world_mut(&mut self) -> &mut World {
        self.main_world_mut()
    }

    pub fn animations(&self) -> &[AnimationClip] {
        self.node_animations(self.main_scene)
    }

    pub fn animation_states(&self) -> &[AnimationState] {
        self.node_animation_states(self.main_scene)
    }

    pub fn add_animation_clip(&mut self, clip: AnimationClip) -> usize {
        self.add_node_animation_clip(self.main_scene, clip)
    }

    pub fn play_animation(&mut self, clip_index: usize, looping: bool) -> Option<usize> {
        self.play_node_animation(self.main_scene, clip_index, looping)
    }

    pub fn update(&mut self, dt: f64) {
        self.time += dt;

        let absolute_time = self.time;
        for node in self.nodes_iter_mut() {
            node.instance.update(dt, absolute_time);
        }

        self.update_world_transforms();
    }

    fn update_world_transforms(&mut self) {
        let root_transform = Transform::IDENTITY;
        self.update_world_transform_recursive(self.root, root_transform);
    }

    fn update_world_transform_recursive(&mut self, node_id: SceneNodeId, parent: Transform) {
        let (world_transform, children) = {
            let node = self.node_mut(node_id);
            node.world_transform = parent.mul_transform(&node.local_transform);
            (node.world_transform, node.children.clone())
        };

        for child in children {
            self.update_world_transform_recursive(child, world_transform);
        }
    }

    pub fn propagate_transforms(&mut self) {
        for node in self.nodes_iter_mut() {
            node.instance.propagate_transforms();
        }
    }

    pub fn render(
        &mut self,
        renderer: &mut Renderer,
        batcher: &mut RenderBatcher,
        custom_render: Option<CustomRenderRequest<'_>>,
    ) -> Result<crate::renderer::RenderFrame, wgpu::SurfaceError> {
        batcher.clear();
        let camera_vectors = rendering::CameraVectors::from_renderer(renderer);

        for node in self.nodes_iter_mut() {
            let world = &node.instance.world;
            let world_transform = node.world_transform;

            for mut object in rendering::build_render_objects(world, camera_vectors).into_iter() {
                object.transform = world_transform.mul_transform(&object.transform);
                batcher.add(object);
            }
        }

        let mut lights_data = crate::renderer::LightsData::default();

        for node in self.nodes_iter() {
            let node_lights =
                lights::collect_lights(&node.instance.world, camera_vectors, node.world_transform);
            lights_data.extend_from(&node_lights);
        }

        renderer.render(
            self,
            &self.assets,
            batcher,
            &lights_data,
            &self.environment,
            custom_render,
        )
    }

    pub fn add_default_lighting(&mut self) -> usize {
        lights::add_default_lighting(self.main_world_mut())
    }

    pub fn has_any_lights(&self) -> bool {
        lights::has_any_lights(self.main_world())
    }

    pub fn merge_as_child(&mut self, parent_entity: hecs::Entity, other: Scene) {
        if let Some(asset) = other.export_main_asset("MergedScene") {
            let instance = asset.instantiate();
            super::internal::composition::merge_world_as_child(
                self.main_world_mut(),
                parent_entity,
                instance.into_world(),
            );
        }
    }

    pub fn debug_print_transforms(&self) {
        super::internal::debug::debug_print_transforms(&self.main_instance().world);
    }

    pub fn create_node(
        &mut self,
        name: impl Into<String>,
        parent: Option<SceneNodeId>,
    ) -> SceneNodeId {
        let name = name.into();
        let parent_id = parent.unwrap_or(self.root);
        assert!(self.is_valid_node(parent_id), "Invalid parent node");
        let id = self.allocate_node(name, SceneInstance::new());
        self.attach_node(id, parent_id);
        self.update_world_transforms();
        id
    }

    pub fn instantiate_asset_named(
        &mut self,
        asset: &SceneAsset,
        name: impl Into<String>,
        parent: Option<SceneNodeId>,
    ) -> SceneNodeId {
        let name = name.into();
        let parent_id = parent.unwrap_or(self.root);
        assert!(self.is_valid_node(parent_id), "Invalid parent node");
        let id = self.allocate_node(name, asset.instantiate());
        {
            let node = self.node_mut(id);
            node.local_transform = Transform::from(asset.root_transform.clone());
        }
        self.attach_node(id, parent_id);
        self.update_world_transforms();
        id
    }

    pub fn instantiate_asset(
        &mut self,
        asset: &SceneAsset,
        parent: Option<SceneNodeId>,
    ) -> SceneNodeId {
        self.instantiate_asset_named(asset, asset.name.clone(), parent)
    }

    pub fn instantiate_tree_asset(
        &mut self,
        asset: &SceneTreeAsset,
        parent: Option<SceneNodeId>,
    ) -> SceneNodeId {
        let parent_id = parent.unwrap_or(self.root);
        assert!(self.is_valid_node(parent_id), "Invalid parent node");
        let node_id = self.instantiate_tree_node(&asset.root, parent_id);
        self.update_world_transforms();
        node_id
    }

    fn instantiate_tree_node(
        &mut self,
        node_asset: &SceneTreeAssetNode,
        parent: SceneNodeId,
    ) -> SceneNodeId {
        let node_id = if let Some(asset) = &node_asset.asset {
            self.instantiate_asset_named(asset, node_asset.name.clone(), Some(parent))
        } else {
            self.create_node(node_asset.name.clone(), Some(parent))
        };

        *self.node_local_transform_mut(node_id) = Transform::from(node_asset.transform.clone());

        for child in &node_asset.children {
            self.instantiate_tree_node(child, node_id);
        }

        node_id
    }

    pub fn export_main_asset(&self, name: impl Into<String>) -> Option<SceneAsset> {
        self.export_node_asset_internal(self.main_scene, Some(name.into()))
    }

    pub fn export_node_asset(&self, node: SceneNodeId) -> Option<SceneAsset> {
        self.export_node_asset_internal(node, None)
    }

    pub fn export_tree_asset(&self, name: impl Into<String>) -> SceneTreeAsset {
        SceneTreeAsset {
            name: name.into(),
            root: self.build_tree_asset_node(self.root),
        }
    }

    fn build_tree_asset_node(&self, node: SceneNodeId) -> SceneTreeAssetNode {
        let node_ref = self.node(node);
        SceneTreeAssetNode {
            name: node_ref.name().to_string(),
            transform: SerializedTransform::from(node_ref.local_transform),
            asset: self.export_node_asset_internal(node, None),
            children: node_ref
                .children
                .iter()
                .map(|&child| self.build_tree_asset_node(child))
                .collect(),
        }
    }

    pub fn remove_node(&mut self, node: SceneNodeId) -> Option<SceneTreeAssetNode> {
        if node == self.root || !self.is_valid_node(node) {
            return None;
        }

        if node == self.main_scene {
            self.main_scene = self.root;
        }

        self.detach_node(node);
        let removed = self.remove_node_recursive(node)?;
        self.update_world_transforms();
        Some(removed)
    }

    fn remove_node_recursive(&mut self, node: SceneNodeId) -> Option<SceneTreeAssetNode> {
        if !self.is_valid_node(node) {
            return None;
        }

        let asset = self.export_node_asset_internal(node, None);
        let (name, transform, children_ids) = {
            let node_ref = self.node(node);
            (
                node_ref.name().to_string(),
                SerializedTransform::from(node_ref.local_transform),
                node_ref.children.clone(),
            )
        };

        let mut children = Vec::with_capacity(children_ids.len());
        for child in children_ids {
            if let Some(child_asset) = self.remove_node_recursive(child) {
                children.push(child_asset);
            }
        }

        self.nodes[node.index()] = None;
        self.free_list.push(node);

        Some(SceneTreeAssetNode {
            name,
            transform,
            asset,
            children,
        })
    }

    fn export_node_asset_internal(
        &self,
        node: SceneNodeId,
        name_override: Option<String>,
    ) -> Option<SceneAsset> {
        if !self.is_valid_node(node) {
            return None;
        }

        let node_ref = self.node(node);
        let (entities, index_map) = Scene::serialize_world(&node_ref.instance.world);
        let animations: Vec<_> = node_ref
            .instance
            .animations()
            .iter()
            .filter_map(|clip| SerializedAnimationClip::from_clip(clip, &index_map))
            .collect();
        let animation_states = node_ref.instance.animation_states().to_vec();

        if entities.is_empty() && animations.is_empty() && animation_states.is_empty() {
            return None;
        }

        let name = name_override.unwrap_or_else(|| node_ref.name().to_string());

        Some(SceneAsset {
            name,
            root_transform: SerializedTransform::from(node_ref.local_transform),
            entities,
            animations,
            animation_states,
        })
    }

    fn serialize_world(world: &World) -> (Vec<SceneAssetEntity>, HashMap<Entity, usize>) {
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

    fn allocate_node(&mut self, name: String, instance: SceneInstance) -> SceneNodeId {
        if let Some(id) = self.free_list.pop() {
            self.nodes[id.index()] = Some(SceneNode::new(id, name, instance));
            id
        } else {
            let id = SceneNodeId(self.nodes.len() as u32);
            self.nodes.push(Some(SceneNode::new(id, name, instance)));
            id
        }
    }
}

impl Default for Scene {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedTransform {
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
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

    #[test]
    fn asset_json_roundtrip() {
        let mut scene = Scene::new();
        let world = scene.main_world_mut();
        let entity = world.spawn((
            Name::new("TestEntity"),
            TransformComponent(Transform::IDENTITY),
            Visible(true),
        ));
        let mut children = Vec::new();
        for i in 0..2 {
            let child = world.spawn((
                Name::new(format!("Child_{i}")),
                TransformComponent(Transform::from_translation(Vec3::new(i as f32, 0.0, 0.0))),
                Parent(entity),
            ));
            children.push(child);
        }
        let _ = world.insert_one(entity, Children(children));

        let asset = scene.export_main_asset("Test").unwrap();
        let json = asset.to_json().unwrap();
        let restored = SceneAsset::from_json(&json).unwrap();
        assert_eq!(restored.entities.len(), 3);
        assert_eq!(restored.name, "Test");
        assert!(restored.animations.is_empty());
        assert!(restored.animation_states.is_empty());

        let mut other_scene = Scene::new();
        let node = other_scene.instantiate_asset(&restored, None);
        assert_eq!(other_scene.node_children(other_scene.root_id()).len(), 1);
        assert_eq!(other_scene.node_parent(node), Some(other_scene.root_id()));
    }

    #[test]
    fn scene_node_creation_and_removal() {
        let mut base = Scene::new();
        let entity = base.main_world_mut().spawn((
            Name::new("Unit"),
            TransformComponent(Transform::IDENTITY),
            Visible(true),
        ));
        base.main_world_mut()
            .insert_one(entity, Children(Vec::new()))
            .ok();
        let unit_asset = base.export_main_asset("Unit").unwrap();

        let mut scene = Scene::new();
        let parent = scene.create_node("Parent", None);
        let child = scene.instantiate_asset_named(&unit_asset, "Child", Some(parent));

        assert_eq!(scene.node_name(parent), "Parent");
        assert_eq!(scene.node_name(child), "Child");
        assert_eq!(scene.node_parent(child), Some(parent));
        assert_eq!(scene.node_children(parent), &[child]);

        let removed = scene.remove_node(parent).expect("node removed");
        assert_eq!(removed.name, "Parent");
        assert!(removed.asset.is_none());
        assert_eq!(removed.children.len(), 1);
        assert_eq!(removed.children[0].name, "Child");
        assert!(scene.iter_children(scene.root_id()).next().is_none());

        let mut rebuilt = Scene::new();
        let tree = SceneTreeAsset {
            name: "Rebuilt".to_string(),
            root: removed,
        };
        let new_node = rebuilt.instantiate_tree_asset(&tree, None);
        assert_eq!(rebuilt.node_children(rebuilt.root_id()), &[new_node]);
        assert_eq!(rebuilt.node_children(new_node).len(), 1);
    }

    #[test]
    fn animation_serialization_roundtrip() {
        use glam::{Quat, Vec4};

        let mut scene = Scene::new();
        let entity = scene.main_world_mut().spawn((
            Name::new("Animated"),
            TransformComponent(Transform::IDENTITY),
            Visible(true),
        ));

        let sampler = AnimationSampler {
            times: vec![0.0, 1.0],
            output: AnimationOutput::Vec3(vec![Vec3::ZERO, Vec3::splat(1.0)]),
            interpolation: AnimationInterpolation::Linear,
        };

        let rotation_sampler = AnimationSampler {
            times: vec![0.0, 1.0],
            output: AnimationOutput::Quat(vec![Quat::IDENTITY, Quat::from_rotation_y(1.0)]),
            interpolation: AnimationInterpolation::Linear,
        };

        let color_sampler = AnimationSampler {
            times: vec![0.0, 1.0],
            output: AnimationOutput::Vec4(vec![Vec4::ZERO, Vec4::splat(1.0)]),
            interpolation: AnimationInterpolation::Linear,
        };

        let mut clip = AnimationClip::new("Move");
        clip.add_channel(AnimationChannel {
            sampler,
            target: AnimationTarget::Transform {
                entity,
                property: TransformProperty::Translation,
            },
        });
        clip.add_channel(AnimationChannel {
            sampler: rotation_sampler,
            target: AnimationTarget::Transform {
                entity,
                property: TransformProperty::Rotation,
            },
        });
        clip.add_channel(AnimationChannel {
            sampler: color_sampler,
            target: AnimationTarget::Material {
                material_index: 0,
                property: MaterialProperty::BaseColorFactor,
            },
        });

        let clip_index = scene.add_animation_clip(clip);
        assert_eq!(clip_index, 0);
        scene.play_animation(clip_index, true);

        let asset = scene.export_main_asset("Anim").unwrap();
        assert_eq!(asset.animations.len(), 1);
        assert_eq!(asset.animation_states.len(), 1);

        let json = asset.to_json().unwrap();
        let restored = SceneAsset::from_json(&json).unwrap();
        assert_eq!(restored.animations.len(), 1);
        assert_eq!(restored.animation_states.len(), 1);

        let mut other = Scene::new();
        let node = other.instantiate_asset(&restored, None);
        assert_eq!(other.node_animations(node).len(), 1);
        assert_eq!(other.node_animation_states(node).len(), 1);
    }

    #[test]
    fn scene_tree_asset_roundtrip() {
        let mut base_a = Scene::new();
        let _ = base_a.main_world_mut().spawn((
            Name::new("A"),
            TransformComponent(Transform::IDENTITY),
            Visible(true),
        ));
        let asset_a = base_a.export_main_asset("AssetA").unwrap();

        let mut base_b = Scene::new();
        let _ = base_b.main_world_mut().spawn((
            Name::new("B"),
            TransformComponent(Transform::IDENTITY),
            Visible(true),
        ));
        let asset_b = base_b.export_main_asset("AssetB").unwrap();

        let mut scene = Scene::new();
        let node_a = scene.instantiate_asset(&asset_a, None);
        let _node_b = scene.instantiate_asset_named(&asset_b, "NodeB", Some(node_a));

        let tree = scene.export_tree_asset("SceneGraph");
        let json = tree.to_json().unwrap();
        let restored = SceneTreeAsset::from_json(&json).unwrap();
        assert_eq!(restored.name, "SceneGraph");
        assert_eq!(restored.root.children.len(), 1);

        let mut other = Scene::new();
        let instantiated_root = other.instantiate_tree_asset(&restored, None);
        assert_eq!(other.node_children(other.root_id()), &[instantiated_root]);
        assert_eq!(other.node_children(instantiated_root).len(), 1);
    }
}
