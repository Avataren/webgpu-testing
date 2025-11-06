use hecs::Entity;
use std::path::PathBuf;

use wgpu_cube::asset::{Handle, MaterialAsset, MaterialKind, ShaderMaterialMetadata};
use wgpu_cube::renderer::Material;
use wgpu_cube::scene::components::Billboard;
use wgpu_cube::scene::{
    CameraComponent, DirectionalLight, EnvironmentComponent, ParticleBehaviorConfig,
    ParticleBehaviorPreset, ParticleEmitterComponent, ParticleSystemComponent, PointLight,
    SpotLight, Transform,
};
use wgpu_cube::scripting::RuneScriptComponent;

/// Actions that can be triggered from the inspector UI.
/// These actions are collected during UI rendering and processed by the application.
#[derive(Clone)]
pub enum InspectorAction {
    EditScript {
        entity: Entity,
        component: RuneScriptComponent,
    },
    EditShader {
        entity: Entity,
        handle: Handle<MaterialAsset>,
        metadata: ShaderMaterialMetadata,
    },
    UpdateTransform {
        entity: Entity,
        transform: Transform,
    },
    UpdateCamera {
        entity: Entity,
        component: CameraComponent,
    },
    UpdateMaterial {
        entity: Entity,
        handle: Handle<MaterialAsset>,
        material: Material,
    },
    CreateShaderMaterial {
        entity: Entity,
        source: Handle<MaterialAsset>,
    },
    SetMaterialKind {
        entity: Entity,
        handle: Handle<MaterialAsset>,
        kind: MaterialKind,
    },
    AssignShaderSource {
        entity: Entity,
        handle: Handle<MaterialAsset>,
        shader_path: PathBuf,
    },
    CreateShaderSource {
        entity: Entity,
        handle: Handle<MaterialAsset>,
        suggested_stem: String,
    },
    UpdatePointLight {
        entity: Entity,
        light: PointLight,
    },
    UpdateDirectionalLight {
        entity: Entity,
        light: DirectionalLight,
    },
    UpdateSpotLight {
        entity: Entity,
        light: SpotLight,
    },
    UpdateEnvironment {
        entity: Entity,
        component: EnvironmentComponent,
    },
    SetCanCastShadow {
        entity: Entity,
        casts_shadow: bool,
    },
    UpdateParticleSystem {
        entity: Entity,
        component: ParticleSystemComponent,
    },
    UpdateParticleEmitter {
        entity: Entity,
        component: ParticleEmitterComponent,
    },
    UpdateParticleBehavior {
        entity: Entity,
        behavior: ParticleBehaviorPreset,
        config: ParticleBehaviorConfig,
    },
    SetBillboard {
        entity: Entity,
        billboard: Option<Billboard>,
    },
    AddScript {
        entity: Entity,
    },
    ChangeScriptSource {
        entity: Entity,
        script_path: PathBuf,
    },
    AddCamera {
        entity: Entity,
    },
    AddMesh {
        entity: Entity,
    },
    AddPointLight {
        entity: Entity,
    },
    AddDirectionalLight {
        entity: Entity,
    },
    AddSpotLight {
        entity: Entity,
    },
    AddEnvironment {
        entity: Entity,
    },
    AddParticleSystem {
        entity: Entity,
    },
    RenameEntity {
        entity: Entity,
        new_name: String,
    },
}
