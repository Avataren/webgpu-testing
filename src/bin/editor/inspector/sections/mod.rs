// Inspector section modules
// Each module handles rendering UI for a specific component type

pub mod camera;
pub mod mesh;
pub mod transform;

// Re-export the show functions for easy access
pub use camera::show_camera_section;
pub use mesh::show_mesh_section;
pub use transform::show_transform_section;
