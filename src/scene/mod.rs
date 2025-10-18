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
    SceneTreeAssetNodeBuilder, SerializedTransform,
};
pub use builder::EntityBuilder;
pub use camera::Camera;
pub use graph::SceneNodeId;
pub use loader::SceneLoader;
pub use scene_core::Scene;
pub(crate) use scene_core::SceneSnapshot;
pub use transform::Transform;

// Re-export all components
pub use components::{
    Children, GltfMaterial, GltfNode, MaterialComponent, MeshBounds, MeshComponent, Name,
    OrbitAnimation, Parent, RotateAnimation, SelectedInEditor, TransformComponent, Visible,
    WorldTransform,
};
