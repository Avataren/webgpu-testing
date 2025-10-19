use super::vertex::{v, Vertex};
use glam::Vec3;
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

            // Two triangles per quad — reversed winding (swap the last two of each tri)
            indices.push(current);
            indices.push(current + 1);
            indices.push(next);

            indices.push(current + 1);
            indices.push(next + 1);
            indices.push(next);
        }
    }

    (vertices, indices)
}

pub fn cone_mesh(segments: u32) -> (Vec<Vertex>, Vec<u32>) {
    let segments = segments.max(3);
    let mut vertices = Vec::with_capacity((segments + 2) as usize);
    let mut indices = Vec::with_capacity((segments * 6) as usize);
    let tangent = [1.0, 0.0, 0.0, 1.0];

    // Apex at origin pointing toward -Z.
    vertices.push(v([0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.5, 1.0], tangent));

    for i in 0..=segments {
        let theta = 2.0 * PI * i as f32 / segments as f32;
        let x = theta.cos();
        let y = theta.sin();
        let pos = [x, y, -1.0];

        // Approximate normal by blending base direction with apex axis.
        let normal = Vec3::new(x, y, 1.0).normalize();
        let uv = [i as f32 / segments as f32, 0.0];

        vertices.push(v(pos, normal.to_array(), uv, tangent));
    }

    let base_center = vertices.len() as u32;
    vertices.push(v([0.0, 0.0, -1.0], [0.0, 0.0, -1.0], [0.5, 0.5], tangent));

    for i in 1..=segments {
        let next = if i == segments { 1 } else { i + 1 };
        // Side triangles (apex -> current -> next)
        indices.push(0);
        indices.push(i);
        indices.push(next);
    }

    for i in 1..=segments {
        let next = if i == segments { 1 } else { i + 1 };
        // Base cap (center -> next -> current)
        indices.push(base_center);
        indices.push(next);
        indices.push(i);
    }

    (vertices, indices)
}

pub fn cone_side_mesh(segments: u32) -> (Vec<Vertex>, Vec<u32>) {
    let segments = segments.max(3);
    let mut vertices = Vec::with_capacity((segments + 1) as usize);
    let mut indices = Vec::with_capacity((segments * 3) as usize);
    let tangent = [1.0, 0.0, 0.0, 1.0];

    // Apex at origin pointing toward -Z.
    vertices.push(v([0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.5, 1.0], tangent));

    for i in 0..=segments {
        let theta = 2.0 * PI * i as f32 / segments as f32;
        let x = theta.cos();
        let y = theta.sin();
        let pos = [x, y, -1.0];

        let normal = Vec3::new(x, y, 1.0).normalize();
        let uv = [i as f32 / segments as f32, 0.0];

        vertices.push(v(pos, normal.to_array(), uv, tangent));
    }

    for i in 1..=segments {
        let next = if i == segments { 1 } else { i + 1 };
        indices.push(0);
        indices.push(i);
        indices.push(next);
    }

    (vertices, indices)
}

pub fn cylinder_mesh(segments: u32) -> (Vec<Vertex>, Vec<u32>) {
    let segments = segments.max(3);
    let tangent = [1.0, 0.0, 0.0, 1.0];
    let mut vertices = Vec::new();
    let mut indices = Vec::new();

    // Generate side vertices (duplicated seam for UV continuity).
    for i in 0..=segments {
        let theta = 2.0 * PI * i as f32 / segments as f32;
        let x = theta.cos();
        let y = theta.sin();
        let normal = [x, y, 0.0];
        let u = i as f32 / segments as f32;

        vertices.push(v([x, y, -0.5], normal, [u, 1.0], tangent));
        vertices.push(v([x, y, 0.5], normal, [u, 0.0], tangent));
    }

    for i in 0..segments {
        let base = i * 2;
        let next = base + 2;

        indices.push(base);
        indices.push(next);
        indices.push(base + 1);

        indices.push(base + 1);
        indices.push(next);
        indices.push(next + 1);
    }

    let bottom_center_index = vertices.len() as u32;
    vertices.push(v([0.0, 0.0, -0.5], [0.0, 0.0, -1.0], [0.5, 0.5], tangent));

    let top_center_index = vertices.len() as u32;
    vertices.push(v([0.0, 0.0, 0.5], [0.0, 0.0, 1.0], [0.5, 0.5], tangent));

    // Bottom cap
    let base_ring_start = 0u32;
    for i in 0..segments {
        let current = base_ring_start + (i * 2);
        let next = base_ring_start + (((i + 1) % segments) * 2);

        indices.push(bottom_center_index);
        indices.push(current);
        indices.push(next);
    }

    // Top cap
    let top_ring_start = 1u32;
    for i in 0..segments {
        let current = top_ring_start + (i * 2);
        let next = top_ring_start + (((i + 1) % segments) * 2);

        indices.push(top_center_index);
        indices.push(next);
        indices.push(current);
    }

    (vertices, indices)
}

pub fn torus_mesh(
    segments: u32,
    ring_segments: u32,
    radius: f32,
    thickness: f32,
) -> (Vec<Vertex>, Vec<u32>) {
    let segments = segments.max(3);
    let ring_segments = ring_segments.max(3);
    let mut vertices = Vec::with_capacity((segments * ring_segments) as usize);
    let mut indices = Vec::with_capacity((segments * ring_segments * 6) as usize);

    for i in 0..segments {
        let u = i as f32 / segments as f32 * std::f32::consts::TAU;
        let cos_u = u.cos();
        let sin_u = u.sin();
        let center = Vec3::new(cos_u * radius, sin_u * radius, 0.0);
        let tangent = Vec3::new(-sin_u, cos_u, 0.0);
        let normal_base = Vec3::new(cos_u, sin_u, 0.0);

        for j in 0..ring_segments {
            let angle_v = j as f32 / ring_segments as f32 * std::f32::consts::TAU;
            let cos_v = angle_v.cos();
            let sin_v = angle_v.sin();

            let normal = (normal_base * cos_v) + (Vec3::Z * sin_v);
            let position = center + normal * thickness;
            let tangent_vec = tangent;
            let tangent4 = [tangent_vec.x, tangent_vec.y, tangent_vec.z, 1.0];
            let uv = [i as f32 / segments as f32, j as f32 / ring_segments as f32];

            vertices.push(v(
                position.to_array(),
                normal.normalize().to_array(),
                uv,
                tangent4,
            ));
        }
    }

    let ring_stride = ring_segments;
    for i in 0..segments {
        let next_i = (i + 1) % segments;
        for j in 0..ring_segments {
            let next_j = (j + 1) % ring_segments;
            let current = i * ring_stride + j;
            let next = i * ring_stride + next_j;
            let current_next = next_i * ring_stride + j;
            let next_next = next_i * ring_stride + next_j;

            indices.push(current);
            indices.push(next);
            indices.push(next_next);

            indices.push(current);
            indices.push(next_next);
            indices.push(current_next);
        }
    }

    (vertices, indices)
}

pub fn quad_mesh() -> (Vec<Vertex>, Vec<u32>) {
    let verts = vec![
        v(
            [-0.5, -0.5, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 1.0],
            [1.0, 0.0, 0.0, 1.0],
        ),
        v(
            [0.5, -0.5, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 1.0],
            [1.0, 0.0, 0.0, 1.0],
        ),
        v(
            [0.5, 0.5, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 0.0],
            [1.0, 0.0, 0.0, 1.0],
        ),
        v(
            [-0.5, 0.5, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0],
            [1.0, 0.0, 0.0, 1.0],
        ),
    ];

    let indices = vec![0, 1, 2, 0, 2, 3];

    (verts, indices)
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
