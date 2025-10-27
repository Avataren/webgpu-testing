// scene/mod.rs

pub mod animation;
mod assets;
pub mod builder;
pub mod camera;
pub mod components;
mod graph;
mod importer;
pub(crate) mod internal;
pub mod loader;
pub mod picking;
mod render_bridge;
mod scene_core;
pub mod state;
pub mod transform;

// Re-export commonly used types
pub use assets::{
    SceneAsset, SceneAssetBuilder, SceneAssetBundle, SceneAssetEntity, SceneAssetEntityBuilder,
    SceneAssetResources, SceneAssetResourcesBuilder, SceneTreeAsset, SceneTreeAssetNode,
    SceneTreeAssetNodeBuilder, SerializedMaterial, SerializedParticleBehavior,
    SerializedParticleEmitter, SerializedParticleSystem, SerializedRuneScript,
    SerializedRuneScriptSource, SerializedTransform,
};
pub use builder::EntityBuilder;
pub use camera::{Camera, CameraProjection};
pub use graph::SceneNodeId;
pub use loader::{SceneImportDevice, SceneLoader};
pub use picking::entity_for_pick_value;
pub(crate) use scene_core::SceneSnapshot;
pub use scene_core::{Scene, SceneStateSnapshot};
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
