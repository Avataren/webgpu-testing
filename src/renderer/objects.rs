use bytemuck::{Pod, Zeroable};
use glam::Mat4;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct ObjectData {
    pub model: [[f32; 4]; 4],
}

impl From<Mat4> for ObjectData {
    fn from(m: Mat4) -> Self {
        Self {
            model: m.to_cols_array_2d(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn object_data_is_64_bytes() {
        assert_eq!(std::mem::size_of::<ObjectData>(), 64);
    }
}
