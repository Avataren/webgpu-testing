// renderer/uniforms.rs
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct CameraUniform {
    pub view_proj: [[f32; 4]; 4],
    pub camera_pos: [f32; 3],
    pub _padding: f32,
}

impl CameraUniform {
    pub fn new() -> Self {
        Self {
            view_proj: Mat4::IDENTITY.to_cols_array_2d(),
            camera_pos: [0.0, 0.0, 0.0],
            _padding: 0.0,
        }
    }

    pub fn from_matrix(view_proj: Mat4, camera_pos: Vec3) -> Self {
        Self {
            view_proj: view_proj.to_cols_array_2d(),
            camera_pos: camera_pos.to_array(),
            _padding: 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn camera_uniform_is_80_bytes() {
        // mat4x4<f32> = 64 bytes, vec3<f32> = 12 bytes, padding = 4 bytes = 80 bytes
        assert_eq!(std::mem::size_of::<CameraUniform>(), 80);
    }
}
