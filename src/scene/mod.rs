// scene/mod.rs

pub mod animation;
pub(crate) mod assets;
pub mod builder;
pub mod camera;
mod camera_control;
pub mod components;
mod environment_control;
pub(crate) mod gltf_material_registry;
pub mod gltf_package;
mod graph;
mod import_pipeline;
mod import_state;
mod importer;
pub(crate) mod internal;
pub mod loader;
pub mod picking;
mod render_bridge;
mod runtime_control;
mod runtime_state;
#[allow(clippy::module_inception)]
mod scene;
mod snapshot;
pub mod state;
pub mod transform;

// Re-export commonly used types
pub(crate) use self::{
    camera_control::SceneCamera, environment_control::SceneEnvironment,
    import_pipeline::SceneImportsManager, runtime_control::SceneRuntimeController,
};
pub use assets::{
    InstantiatedSceneAsset, SceneAsset, SceneAssetBuilder, SceneAssetBundle, SceneAssetEntity,
    SceneAssetEntityBuilder, SceneAssetResources, SceneAssetResourcesBuilder, SceneTreeAsset,
    SceneTreeAssetNode, SceneTreeAssetNodeBuilder, SerializedMaterial, SerializedMaterialKind,
    SerializedParticleBehavior, SerializedParticleEmitter, SerializedParticleSystem,
    SerializedRuneScript, SerializedRuneScriptSource, SerializedTransform,
};
pub use builder::EntityBuilder;
pub use camera::{Camera, CameraProjection};
pub use graph::SceneNodeId;
pub use loader::{SceneImportDevice, SceneLoader};
pub use picking::entity_for_pick_value;
pub use scene::Scene;
pub(crate) use snapshot::SceneSnapshot;
pub use snapshot::SceneStateSnapshot;
pub use state::{
    GizmoState, TransformGizmoAxis, TransformGizmoHandle, TransformGizmoMode, TransformGizmoSpace,
};
pub use transform::Transform;

// Re-export all components
pub use components::{
    BoidsBehaviorConfig, CameraComponent, CanCastShadow, Children, DirectionalLight,
    EditorEntityId, EnvironmentComponent, GltfMaterial, GltfNode, GltfPrimitive, GltfSource,
    MaterialComponent, MeshBounds, MeshComponent, Name, OptimizedBoidsBehaviorConfig,
    OrbitAnimation, Parent, ParticleBehaviorConfig, ParticleBehaviorPreset, ParticleColorGradient,
    ParticleColorKeyframe, ParticleEmissionShape, ParticleEmitterComponent, ParticleFloatRange,
    ParticleRenderBlendMode, ParticleSizeCurve, ParticleSizeKeyframe, ParticleSystemComponent,
    ParticleVec3Range, PhysicsBehaviorConfig, PointLight, PrimitiveMeshComponent, RotateAnimation,
    SelectedInEditor, SpotLight, StarfieldBehaviorConfig, TransformComponent, Visible,
    WorldTransform,
};
