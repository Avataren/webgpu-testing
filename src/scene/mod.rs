// scene/mod.rs

pub mod camera;
pub mod transform;
pub mod scene;
pub mod components;
pub mod builder;
pub mod loader;

// Re-export commonly used types
pub use camera::Camera;
pub use transform::Transform;
pub use scene::Scene;
pub use builder::EntityBuilder;
pub use loader::SceneLoader;

// Re-export all components
pub use components::{
    TransformComponent,
    MeshComponent,
    MaterialComponent,
    Visible,
    Name,
    Parent,
    Children,
    RotateAnimation,
    OrbitAnimation,
};