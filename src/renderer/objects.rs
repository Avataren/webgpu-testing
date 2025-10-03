use bytemuck::{Pod, Zeroable};
use glam::Mat4;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct ObjectData {
    pub model: [[f32; 4]; 4],      // 64 bytes
    pub color: [f32; 4],            // 16 bytes
    pub texture_index: u32,         // 4 bytes
    pub material_flags: u32,        // 4 bytes
    pub _padding: [u32; 2],         // 8 bytes (align to 16-byte boundary)
}

impl ObjectData {
    pub fn new(model: Mat4, color: [f32; 4], texture_index: u32, material_flags: u32) -> Self {
        Self {
            model: model.to_cols_array_2d(),
            color,
            texture_index,
            material_flags,
            _padding: [0, 0],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn object_data_is_96_bytes() {
        // 64 (mat4) + 16 (vec4) + 4 (u32) + 4 (u32) + 8 (padding) = 96
        assert_eq!(std::mem::size_of::<ObjectData>(), 96);
    }
}