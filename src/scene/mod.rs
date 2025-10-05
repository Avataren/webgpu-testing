// scene/mod.rs

pub mod builder;
pub mod camera;
pub mod components;
pub mod loader;
pub mod scene;
pub mod transform;

// Re-export commonly used types
pub use builder::EntityBuilder;
pub use camera::Camera;
pub use loader::SceneLoader;
pub use scene::Scene;
pub use transform::Transform;

// Re-export all components
pub use components::{
    Children, MaterialComponent, MeshComponent, Name, OrbitAnimation, Parent, RotateAnimation,
    TransformComponent, Visible,
};
