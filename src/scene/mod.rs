// scene/mod.rs

pub mod animation;
mod assets;
pub mod builder;
pub mod camera;
pub mod components;
mod graph;
pub(crate) mod internal;
pub mod loader;
mod scene_core;
pub mod transform;

// Re-export commonly used types
pub use assets::{
    SceneAsset, SceneAssetBuilder, SceneAssetBundle, SceneAssetEntity, SceneAssetEntityBuilder,
    SceneAssetResources, SceneAssetResourcesBuilder, SceneTreeAsset, SceneTreeAssetNode,
    SceneTreeAssetNodeBuilder, SerializedRuneScript, SerializedRuneScriptSource,
    SerializedTransform,
};
pub use builder::EntityBuilder;
pub use camera::Camera;
pub use graph::SceneNodeId;
pub use loader::SceneLoader;
pub(crate) use scene_core::SceneSnapshot;
pub use scene_core::{
    Scene, SceneStateSnapshot, TransformGizmoAxis, TransformGizmoHandle, TransformGizmoMode,
    TransformGizmoSpace,
};
pub use transform::Transform;

// Re-export all components
pub use components::{
    CanCastShadow, Children, DirectionalLight, EditorEntityId, GltfMaterial, GltfNode,
    MaterialComponent, MeshBounds, MeshComponent, Name, OrbitAnimation, Parent, PointLight,
    RotateAnimation, SelectedInEditor, SpotLight, TransformComponent, Visible, WorldTransform,
};
