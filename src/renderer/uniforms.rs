// renderer/uniforms.rs
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};

#[repr(C, align(16))]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct CameraUniform {
    pub view_proj: [[f32; 4]; 4],
    pub inverse_view_proj: [[f32; 4]; 4],
    pub camera_pos: [f32; 3],
    /// LOD bias for material textures: log2 of the render scale while upscaling (so
    /// textures stay as sharp as at output resolution), else 0.
    pub mip_bias: f32,
    pub camera_forward: [f32; 3],
    pub _padding1: f32,
    pub camera_up: [f32; 3],
    pub _padding2: f32,
    /// `view_proj` without the TAA sub-pixel jitter: motion vectors compare this...
    pub unjittered_view_proj: [[f32; 4]; 4],
    /// ...with last frame's unjittered view-projection.
    pub prev_view_proj: [[f32; 4]; 4],
}

impl CameraUniform {
    pub fn new() -> Self {
        Self {
            view_proj: Mat4::IDENTITY.to_cols_array_2d(),
            inverse_view_proj: Mat4::IDENTITY.to_cols_array_2d(),
            camera_pos: [0.0, 0.0, 0.0],
            mip_bias: 0.0,
            camera_forward: [0.0, 0.0, -1.0],
            _padding1: 0.0,
            camera_up: [0.0, 1.0, 0.0],
            _padding2: 0.0,
            unjittered_view_proj: Mat4::IDENTITY.to_cols_array_2d(),
            prev_view_proj: Mat4::IDENTITY.to_cols_array_2d(),
        }
    }

    /// `view_proj` is what geometry is drawn with (jittered while TAA is on);
    /// `unjittered_view_proj` and `prev_view_proj` feed motion vectors.
    #[allow(clippy::too_many_arguments)]
    pub fn from_matrices(
        view_proj: Mat4,
        inverse_view_proj: Mat4,
        camera_pos: Vec3,
        camera_forward: Vec3,
        camera_up: Vec3,
        unjittered_view_proj: Mat4,
        prev_view_proj: Mat4,
    ) -> Self {
        Self {
            unjittered_view_proj: unjittered_view_proj.to_cols_array_2d(),
            prev_view_proj: prev_view_proj.to_cols_array_2d(),
            view_proj: view_proj.to_cols_array_2d(),
            inverse_view_proj: inverse_view_proj.to_cols_array_2d(),
            camera_pos: camera_pos.to_array(),
            mip_bias: 0.0,
            camera_forward: camera_forward.to_array(),
            _padding1: 0.0,
            camera_up: camera_up.to_array(),
            _padding2: 0.0,
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
    /// x: 1 while ray-traced global illumination brings opaque surfaces their indirect
    /// diffuse light (the main pass leaves the environment's diffuse light out); y: 1
    /// while ray-traced reflections also bring them the sky's reflection.
    pub traced: [f32; 4],
}

impl EnvironmentUniform {
    pub fn new() -> Self {
        Self {
            flags_intensity: [0.0, 1.0, 0.003, 0.0],
            ambient_color: [0.003, 0.003, 0.003, 0.003],
            traced: [0.0; 4],
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
    fn camera_uniform_is_expected_size() {
        // 2 * mat4x4<f32> = 128 bytes,
        // 3 vec3<f32> with padding = 3 * 16 bytes = 48 bytes,
        // unjittered and previous view-projection = 128 bytes
        // Total = 304 bytes
        assert_eq!(std::mem::size_of::<CameraUniform>(), 304);
        assert_eq!(std::mem::align_of::<CameraUniform>(), 16);
    }
}
