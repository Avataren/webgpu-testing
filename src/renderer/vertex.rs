use bytemuck::{Pod, Zeroable};
use serde::{Deserialize, Serialize};
use std::mem;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, Serialize, Deserialize)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub tangent: [f32; 4], // xyz = tangent, w = handedness (+1 or -1)
    /// Linear RGBA that multiplies the material's base colour (glTF `COLOR_0`). White
    /// leaves the material as it is.
    #[serde(default = "Vertex::white", skip_serializing_if = "Vertex::is_white")]
    pub color: [f32; 4],
}

impl Vertex {
    pub const WHITE: [f32; 4] = [1.0; 4];

    pub const ATTRS: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
        0 => Float32x3,  // position
        1 => Float32x3,  // normal
        2 => Float32x2,  // uv
        3 => Float32x4,  // tangent
        4 => Float32x4   // color
    ];

    pub fn layout<'a>() -> wgpu::VertexBufferLayout<'a> {
        wgpu::VertexBufferLayout {
            array_stride: mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRS,
        }
    }

    fn white() -> [f32; 4] {
        Self::WHITE
    }

    fn is_white(color: &[f32; 4]) -> bool {
        *color == Self::WHITE
    }
}

#[inline]
pub fn v(pos: [f32; 3], normal: [f32; 3], uv: [f32; 2], tangent: [f32; 4]) -> Vertex {
    Vertex {
        pos,
        normal,
        uv,
        tangent,
        color: Vertex::WHITE,
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
    fn vertex_size_is_64_bytes() {
        // pos 3 + normal 3 + uv 2 + tangent 4 + color 4 = 16 floats = 64 bytes
        assert_eq!(std::mem::size_of::<Vertex>(), 64);
    }

    #[test]
    fn white_vertex_colour_is_left_out_of_json() {
        let vertex = v([1.0, 2.0, 3.0], [0.0, 1.0, 0.0], [0.0, 0.0], [1.0, 0.0, 0.0, 1.0]);
        let json = serde_json::to_string(&vertex).unwrap();
        assert!(!json.contains("color"), "{json}");
        let back: Vertex = serde_json::from_str(&json).unwrap();
        assert_eq!(back.color, Vertex::WHITE);

        let tinted = Vertex {
            color: [0.5, 0.25, 1.0, 1.0],
            ..vertex
        };
        let back: Vertex = serde_json::from_str(&serde_json::to_string(&tinted).unwrap()).unwrap();
        assert_eq!(back.color, [0.5, 0.25, 1.0, 1.0]);
    }
}
