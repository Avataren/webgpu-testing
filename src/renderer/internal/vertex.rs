use bytemuck::{Pod, Zeroable};
use std::mem;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub tangent: [f32; 4], // xyz = tangent, w = handedness (+1 or -1)
}

impl Vertex {
    pub const ATTRS: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
        0 => Float32x3,  // position
        1 => Float32x3,  // normal
        2 => Float32x2,  // uv
        3 => Float32x4   // tangent
    ];

    pub fn layout<'a>() -> wgpu::VertexBufferLayout<'a> {
        wgpu::VertexBufferLayout {
            array_stride: mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRS,
        }
    }
}

#[inline]
pub fn v(pos: [f32; 3], normal: [f32; 3], uv: [f32; 2], tangent: [f32; 4]) -> Vertex {
    Vertex {
        pos,
        normal,
        uv,
        tangent,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vertex_stride_matches_struct_size() {
        assert_eq!(
            Vertex::layout().array_stride,
            std::mem::size_of::<Vertex>() as wgpu::BufferAddress
        );
    }

    #[test]
    fn vertex_size_is_48_bytes() {
        // 3 floats (pos) + 3 floats (normal) + 2 floats (uv) + 4 floats (tangent) = 12 floats = 48 bytes
        assert_eq!(std::mem::size_of::<Vertex>(), 48);
    }
}
