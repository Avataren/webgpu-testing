// scene/mod.rs

pub mod animation;
mod assets;
pub mod builder;
pub mod camera;
pub mod components;
mod graph;
pub(crate) mod internal;
pub mod loader;
pub mod picking;
mod scene_core;
pub mod transform;

// Re-export commonly used types
pub use assets::{
    SceneAsset, SceneAssetBuilder, SceneAssetBundle, SceneAssetEntity, SceneAssetEntityBuilder,
    SceneAssetResources, SceneAssetResourcesBuilder, SceneTreeAsset, SceneTreeAssetNode,
    SceneTreeAssetNodeBuilder, SerializedMaterial, SerializedParticleSystem, SerializedRuneScript,
    SerializedRuneScriptSource, SerializedTransform,
};
pub use builder::EntityBuilder;
pub use camera::{Camera, CameraProjection};
pub use graph::SceneNodeId;
pub use loader::{SceneImportDevice, SceneLoader};
pub use picking::entity_for_pick_value;
pub(crate) use scene_core::SceneSnapshot;
pub use scene_core::{
    Scene, SceneStateSnapshot, TransformGizmoAxis, TransformGizmoHandle, TransformGizmoMode,
    TransformGizmoSpace,
};
pub use transform::Transform;

// Re-export all components
pub use components::{
    CameraComponent, CanCastShadow, Children, DirectionalLight, EditorEntityId,
    EnvironmentComponent, GltfMaterial, GltfNode, GltfPrimitive, GltfSource, MaterialComponent,
    MeshBounds, MeshComponent, Name, OrbitAnimation, Parent, ParticleBehaviorPreset,
    ParticleSystemComponent, PointLight, RotateAnimation, SelectedInEditor, SpotLight,
    TransformComponent, Visible, WorldTransform,
};
