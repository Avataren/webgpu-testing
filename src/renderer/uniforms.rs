// renderer/uniforms.rs
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct CameraUniform {
    pub view_proj: [[f32; 4]; 4],
    pub inverse_view_proj: [[f32; 4]; 4],
    pub camera_pos: [f32; 3],
    pub _padding: f32,
}

impl CameraUniform {
    pub fn new() -> Self {
        Self {
            view_proj: Mat4::IDENTITY.to_cols_array_2d(),
            inverse_view_proj: Mat4::IDENTITY.to_cols_array_2d(),
            camera_pos: [0.0, 0.0, 0.0],
            _padding: 0.0,
        }
    }

    pub fn from_matrix(view_proj: Mat4, camera_pos: Vec3) -> Self {
        let inverse = view_proj.inverse();
        Self::from_matrices(view_proj, inverse, camera_pos)
    }

    pub fn from_matrices(view_proj: Mat4, inverse_view_proj: Mat4, camera_pos: Vec3) -> Self {
        Self {
            view_proj: view_proj.to_cols_array_2d(),
            inverse_view_proj: inverse_view_proj.to_cols_array_2d(),
            camera_pos: camera_pos.to_array(),
            _padding: 0.0,
        }
    }
}

impl Default for CameraUniform {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, PartialEq, Debug)]
pub struct EnvironmentUniform {
    pub flags_intensity: [f32; 4],
    pub ambient_color: [f32; 4],
}

impl EnvironmentUniform {
    pub fn new() -> Self {
        Self {
            flags_intensity: [0.0, 1.0, 0.003, 0.0],
            ambient_color: [0.003, 0.003, 0.003, 1.0],
        }
    }
}

impl Default for EnvironmentUniform {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn camera_uniform_is_144_bytes() {
        // 2 * mat4x4<f32> = 128 bytes, vec3<f32> = 12 bytes, padding = 4 bytes = 144 bytes
        assert_eq!(std::mem::size_of::<CameraUniform>(), 144);
    }
}
