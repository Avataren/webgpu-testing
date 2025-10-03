use bytemuck::{Pod, Zeroable};
use glam::Mat4;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct CameraUniform {
    pub view_proj: [[f32; 4]; 4],
}

impl CameraUniform {
    pub fn new() -> Self {
        Self {
            view_proj: Mat4::IDENTITY.to_cols_array_2d(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn camera_uniform_is_64_bytes() {
        // mat4x4<f32> = 16 * 4 bytes
        assert_eq!(std::mem::size_of::<CameraUniform>(), 64);
    }
}
