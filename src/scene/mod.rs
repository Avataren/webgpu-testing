// scene/mod.rs

pub mod animation;
pub mod asset;
pub mod builder;
pub mod camera;
pub mod components;
pub(crate) mod instance;
pub(crate) mod internal;
pub mod loader;
mod node;
mod scene_core;
pub mod transform;

// Re-export commonly used types
pub use asset::{
    SceneAsset, SceneAssetBundle, SceneAssetEntity, SceneAssetEntityBuilder, SceneAssetResources,
    SceneTreeAsset, SceneTreeAssetNode, SerializedMaterial, SerializedTransform,
};
pub use builder::EntityBuilder;
pub use camera::Camera;
pub use loader::SceneLoader;
pub use node::SceneNodeId;
pub use scene_core::Scene;
pub use transform::Transform;

// Re-export all components
pub use components::{
    Children, GltfMaterial, GltfNode, MaterialComponent, MeshComponent, Name, OrbitAnimation,
    Parent, RotateAnimation, TransformComponent, Visible,
};
