use crate::scene::animation::AnimationState;
use crate::scene::components::{CameraComponent, EnvironmentComponent};
use super::core::SceneAsset;
use super::entity::{SceneAssetEntity, SceneMaterialHandle};
use super::prefabs::ScenePrefabRef;
use crate::renderer::primitives::PrimitiveMeshDescriptor;
use super::serialization::{
    SerializedAnimationClip, SerializedBillboard, SerializedDirectionalLight, SerializedMaterial,
    SerializedMeshBounds, SerializedParticleBehavior, SerializedParticleEmitter,
    SerializedParticleSystem, SerializedPointLight, SerializedRuneScript, SerializedSpotLight,
    SerializedTransform,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
    billboard: Option<SerializedBillboard>,
    editor_id: Option<u128>,
    particle_system: Option<SerializedParticleSystem>,
    particle_emitter: Option<SerializedParticleEmitter>,
    particle_behavior: Option<SerializedParticleBehavior>,
    environment: Option<EnvironmentComponent>,
    camera: Option<CameraComponent>,
    scene_ref: Option<ScenePrefabRef>,
}

impl SceneAssetEntityBuilder {
    pub(crate) fn new(transform: SerializedTransform) -> Self {
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
            billboard: None,
            editor_id: None,
            particle_system: None,
            particle_emitter: None,
            particle_behavior: None,
            environment: None,
            camera: None,
            scene_ref: None,
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

    pub fn with_billboard(mut self, billboard: SerializedBillboard) -> Self {
        self.billboard = Some(billboard);
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

    pub fn with_scene_ref(mut self, scene_ref: ScenePrefabRef) -> Self {
        self.scene_ref = Some(scene_ref);
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
            billboard: self.billboard,
            editor_id: self.editor_id,
            particle_system: self.particle_system,
            particle_emitter: self.particle_emitter,
            particle_behavior: self.particle_behavior,
            environment: self.environment,
            camera: self.camera,
            scene_ref: self.scene_ref,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]

pub struct SceneAssetBuilder {
    name: String,
    root_transform: SerializedTransform,
    entities: Vec<SceneAssetEntity>,
    animations: Vec<SerializedAnimationClip>,
    animation_states: Vec<AnimationState>,
    active_camera: Option<usize>,
}

impl SceneAssetBuilder {
    pub(crate) fn new(name: impl Into<String>) -> Self {
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

