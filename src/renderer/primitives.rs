use super::vertex::{v, Vertex};
use std::f32::consts::PI;

pub fn sphere_mesh(segments: u32, rings: u32) -> (Vec<Vertex>, Vec<u32>) {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();

    // Generate vertices
    for ring in 0..=rings {
        let phi = PI * ring as f32 / rings as f32;
        let y = phi.cos();
        let ring_radius = phi.sin();

        for segment in 0..=segments {
            let theta = 2.0 * PI * segment as f32 / segments as f32;
            let x = ring_radius * theta.cos();
            let z = ring_radius * theta.sin();

            let pos = [x, y, z];
            let normal = [x, y, z]; // For unit sphere, position = normal

            // UV coordinates
            let u = segment as f32 / segments as f32;
            let tex_v = ring as f32 / rings as f32;

            // Tangent points in the direction of increasing theta (around the sphere)
            let tangent = [-theta.sin(), 0.0, theta.cos(), 1.0];

            vertices.push(v(pos, normal, [u, tex_v], tangent));
        }
    }

    // Generate indices
    for ring in 0..rings {
        for segment in 0..segments {
            let current = ring * (segments + 1) + segment;
            let next = current + segments + 1;

            // Two triangles per quad
            indices.push(current);
            indices.push(next);
            indices.push(current + 1);

            indices.push(current + 1);
            indices.push(next);
            indices.push(next + 1);
        }
    }

    (vertices, indices)
}

pub fn cube_mesh() -> (Vec<Vertex>, Vec<u32>) {
    let p = |x, y, z| [x, y, z];

    // For each face, tangent points along U direction, bitangent along V direction
    // Handedness is typically +1.0

    let verts = vec![
        // Right face (+X) - tangent points in +Z, normal in +X
        v(
            p(0.5, -0.5, -0.5),
            [1.0, 0.0, 0.0],
            [0.0, 1.0],
            [0.0, 0.0, 1.0, 1.0],
        ),
        v(
            p(0.5, 0.5, -0.5),
            [1.0, 0.0, 0.0],
            [0.0, 0.0],
            [0.0, 0.0, 1.0, 1.0],
        ),
        v(
            p(0.5, 0.5, 0.5),
            [1.0, 0.0, 0.0],
            [1.0, 0.0],
            [0.0, 0.0, 1.0, 1.0],
        ),
        v(
            p(0.5, -0.5, 0.5),
            [1.0, 0.0, 0.0],
            [1.0, 1.0],
            [0.0, 0.0, 1.0, 1.0],
        ),
        // Left face (-X) - tangent points in -Z, normal in -X
        v(
            p(-0.5, -0.5, 0.5),
            [-1.0, 0.0, 0.0],
            [0.0, 1.0],
            [0.0, 0.0, -1.0, 1.0],
        ),
        v(
            p(-0.5, 0.5, 0.5),
            [-1.0, 0.0, 0.0],
            [0.0, 0.0],
            [0.0, 0.0, -1.0, 1.0],
        ),
        v(
            p(-0.5, 0.5, -0.5),
            [-1.0, 0.0, 0.0],
            [1.0, 0.0],
            [0.0, 0.0, -1.0, 1.0],
        ),
        v(
            p(-0.5, -0.5, -0.5),
            [-1.0, 0.0, 0.0],
            [1.0, 1.0],
            [0.0, 0.0, -1.0, 1.0],
        ),
        // Top face (+Y) - tangent points in +X, normal in +Y
        v(
            p(-0.5, 0.5, -0.5),
            [0.0, 1.0, 0.0],
            [0.0, 1.0],
            [1.0, 0.0, 0.0, 1.0],
        ),
        v(
            p(-0.5, 0.5, 0.5),
            [0.0, 1.0, 0.0],
            [0.0, 0.0],
            [1.0, 0.0, 0.0, 1.0],
        ),
        v(
            p(0.5, 0.5, 0.5),
            [0.0, 1.0, 0.0],
            [1.0, 0.0],
            [1.0, 0.0, 0.0, 1.0],
        ),
        v(
            p(0.5, 0.5, -0.5),
            [0.0, 1.0, 0.0],
            [1.0, 1.0],
            [1.0, 0.0, 0.0, 1.0],
        ),
        // Bottom face (-Y) - tangent points in +X, normal in -Y
        v(
            p(-0.5, -0.5, 0.5),
            [0.0, -1.0, 0.0],
            [0.0, 1.0],
            [1.0, 0.0, 0.0, 1.0],
        ),
        v(
            p(-0.5, -0.5, -0.5),
            [0.0, -1.0, 0.0],
            [0.0, 0.0],
            [1.0, 0.0, 0.0, 1.0],
        ),
        v(
            p(0.5, -0.5, -0.5),
            [0.0, -1.0, 0.0],
            [1.0, 0.0],
            [1.0, 0.0, 0.0, 1.0],
        ),
        v(
            p(0.5, -0.5, 0.5),
            [0.0, -1.0, 0.0],
            [1.0, 1.0],
            [1.0, 0.0, 0.0, 1.0],
        ),
        // Front face (+Z) - tangent points in +X, normal in +Z
        v(
            p(0.5, -0.5, 0.5),
            [0.0, 0.0, 1.0],
            [0.0, 1.0],
            [1.0, 0.0, 0.0, 1.0],
        ),
        v(
            p(0.5, 0.5, 0.5),
            [0.0, 0.0, 1.0],
            [0.0, 0.0],
            [1.0, 0.0, 0.0, 1.0],
        ),
        v(
            p(-0.5, 0.5, 0.5),
            [0.0, 0.0, 1.0],
            [1.0, 0.0],
            [1.0, 0.0, 0.0, 1.0],
        ),
        v(
            p(-0.5, -0.5, 0.5),
            [0.0, 0.0, 1.0],
            [1.0, 1.0],
            [1.0, 0.0, 0.0, 1.0],
        ),
        // Back face (-Z) - tangent points in -X, normal in -Z
        v(
            p(-0.5, -0.5, -0.5),
            [0.0, 0.0, -1.0],
            [0.0, 1.0],
            [-1.0, 0.0, 0.0, 1.0],
        ),
        v(
            p(-0.5, 0.5, -0.5),
            [0.0, 0.0, -1.0],
            [0.0, 0.0],
            [-1.0, 0.0, 0.0, 1.0],
        ),
        v(
            p(0.5, 0.5, -0.5),
            [0.0, 0.0, -1.0],
            [1.0, 0.0],
            [-1.0, 0.0, 0.0, 1.0],
        ),
        v(
            p(0.5, -0.5, -0.5),
            [0.0, 0.0, -1.0],
            [1.0, 1.0],
            [-1.0, 0.0, 0.0, 1.0],
        ),
    ];

    let idx = (0..6)
        .flat_map(|f| {
            let o = f * 4;
            [o, o + 1, o + 2, o, o + 2, o + 3]
        })
        .map(|i| i as u32)
        .collect::<Vec<_>>();

    (verts, idx)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cube_counts_look_right() {
        let (v, i) = cube_mesh();
        assert_eq!(v.len(), 24);
        assert_eq!(i.len(), 36);
    }
}
