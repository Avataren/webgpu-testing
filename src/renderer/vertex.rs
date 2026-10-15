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

    /// Last frame's positions as a second vertex stream (`@location(5)`), read from a
    /// buffer of whole [`Vertex`]es: a skinned instance's previous pose, or the mesh's
    /// own vertices for anything that does not deform. Motion vectors compare it with
    /// the current position.
    pub const PREVIOUS_POSITION_ATTRS: [wgpu::VertexAttribute; 1] =
        wgpu::vertex_attr_array![5 => Float32x3];

    pub fn previous_position_layout<'a>() -> wgpu::VertexBufferLayout<'a> {
        wgpu::VertexBufferLayout {
            array_stride: mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::PREVIOUS_POSITION_ATTRS,
        }
    }

    fn white() -> [f32; 4] {
        Self::WHITE
    }

    fn is_white(color: &[f32; 4]) -> bool {
        *color == Self::WHITE
    }
}

/// Joint indices and weights of one skinned vertex: the second vertex stream of a
/// skinned mesh, parallel to its [`Vertex`] array. 16 bytes.
///
/// Weights are unsigned normalized (`65535` = 1.0) and sum to exactly `65535`, so a
/// skinned vertex never shrinks or grows. Unused influences have weight 0 (their joint
/// index is then irrelevant, conventionally 0).
///
/// ```
/// use wgpu_cube::renderer::SkinVertex;
/// let skin = SkinVertex::from_influences([(3, 0.75), (7, 0.25)]);
/// assert_eq!(skin.joints[..2], [3, 7]);
/// assert_eq!(skin.weights.iter().map(|&w| w as u32).sum::<u32>(), 65535);
/// ```
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SkinVertex {
    pub joints: [u16; 4],
    pub weights: [u16; 4],
}

impl SkinVertex {
    /// Most influences per vertex.
    pub const MAX_INFLUENCES: usize = 4;

    /// Builds a vertex from any number of `(joint, weight)` influences: keeps the four
    /// strongest, normalizes them and quantizes so the weights sum to exactly 65535. A
    /// vertex without positive weights is bound fully to joint 0.
    pub fn from_influences(influences: impl IntoIterator<Item = (u16, f32)>) -> Self {
        let mut strongest: Vec<(u16, f32)> = influences
            .into_iter()
            .filter(|(_, weight)| weight.is_finite() && *weight > 0.0)
            .collect();
        strongest.sort_by(|a, b| b.1.total_cmp(&a.1));
        strongest.truncate(Self::MAX_INFLUENCES);

        let total: f32 = strongest.iter().map(|(_, weight)| weight).sum();
        if strongest.is_empty() || total <= 0.0 {
            return Self {
                joints: [0; 4],
                weights: [u16::MAX, 0, 0, 0],
            };
        }

        let mut skin = Self::default();
        let mut sum: u32 = 0;
        for (slot, (joint, weight)) in strongest.iter().enumerate() {
            let quantized = ((weight / total) * 65535.0).round() as u32;
            skin.joints[slot] = *joint;
            skin.weights[slot] = quantized.min(65535) as u16;
            sum += skin.weights[slot] as u32;
        }
        // Rounding can leave the sum a little off; the strongest influence absorbs it.
        let corrected = skin.weights[0] as i64 + 65535 - sum as i64;
        skin.weights[0] = corrected.clamp(0, 65535) as u16;
        skin
    }

    /// Weights as floats in 0..=1.
    pub fn weights_f32(&self) -> [f32; 4] {
        self.weights.map(|weight| weight as f32 / 65535.0)
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
    fn skin_vertex_is_16_bytes() {
        assert_eq!(std::mem::size_of::<SkinVertex>(), 16);
    }

    #[test]
    fn skin_weights_sum_exactly_and_keep_the_strongest_four() {
        let skin =
            SkinVertex::from_influences([(1, 0.1), (2, 0.3), (3, 0.05), (4, 0.25), (5, 0.3)]);
        let sum: u32 = skin.weights.iter().map(|&w| w as u32).sum();
        assert_eq!(sum, 65535);
        assert!(
            !skin.joints.contains(&3),
            "the weakest influence is dropped"
        );
        // Thirds never quantize exactly; the sum must still be exact.
        let thirds = SkinVertex::from_influences([(0, 1.0), (1, 1.0), (2, 1.0)]);
        assert_eq!(thirds.weights.iter().map(|&w| w as u32).sum::<u32>(), 65535);
        let unbound = SkinVertex::from_influences([(9, 0.0)]);
        assert_eq!(unbound.weights, [65535, 0, 0, 0]);
    }

    #[test]
    fn white_vertex_colour_is_left_out_of_json() {
        let vertex = v(
            [1.0, 2.0, 3.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0],
            [1.0, 0.0, 0.0, 1.0],
        );
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
