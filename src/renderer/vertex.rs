use bytemuck::{Pod, Zeroable};
use std::mem;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}

impl Vertex {
    pub const ATTRS: [wgpu::VertexAttribute; 3] = wgpu::vertex_attr_array![
        0 => Float32x3,
        1 => Float32x3,
        2 => Float32x2
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
pub fn v(pos: [f32; 3], normal: [f32; 3], uv: [f32; 2]) -> Vertex {
    Vertex { pos, normal, uv }
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
}
