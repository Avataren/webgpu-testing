// Linear blend skinning: deforms one mesh instance's vertices with its joint palette
// and writes them to the instance's own vertex buffer, which every render pass then
// draws like a rigid mesh. One invocation per vertex.
//
// Layouts (must match the Rust side, src/renderer/skinning.rs):
// - Vertex (64 bytes = 16 f32): pos.xyz, normal.xyz, uv.xy, tangent.xyzw, color.rgba
// - SkinVertex (16 bytes = 4 u32): joints as u16x4, weights as unorm16x4
//
// The CPU fallback (`skin_vertices`) implements the same math; keep them in step.

struct Params {
    vertex_count: u32,
    palette_offset: u32,
    joint_count: u32,
    _padding: u32,
}

// group 0: per instance
@group(0) @binding(0) var<storage, read> source_vertices: array<f32>;
@group(0) @binding(1) var<storage, read> skin: array<u32>;
@group(0) @binding(2) var<storage, read_write> skinned_vertices: array<f32>;
// group 1: shared by all instances of the frame
@group(1) @binding(0) var<storage, read> palette: array<mat4x4<f32>>;
@group(1) @binding(1) var<uniform> params: Params;

const VERTEX_FLOATS: u32 = 16u;

fn joint_matrix(joint: u32) -> mat4x4<f32> {
    return palette[params.palette_offset + min(joint, params.joint_count - 1u)];
}

fn safe_normalize(v: vec3<f32>, fallback: vec3<f32>) -> vec3<f32> {
    let len_sq = dot(v, v);
    if len_sq <= 1e-20 {
        return fallback;
    }
    return v * inverseSqrt(len_sq);
}

@compute @workgroup_size(64)
fn cs_main(@builtin(global_invocation_id) id: vec3<u32>) {
    let v = id.x;
    if v >= params.vertex_count {
        return;
    }

    let s = v * 4u;
    let j01 = skin[s];
    let j23 = skin[s + 1u];
    let w01 = skin[s + 2u];
    let w23 = skin[s + 3u];
    let weights = vec4<f32>(
        f32(w01 & 0xffffu),
        f32(w01 >> 16u),
        f32(w23 & 0xffffu),
        f32(w23 >> 16u),
    ) / 65535.0;

    let m = joint_matrix(j01 & 0xffffu) * weights.x
        + joint_matrix(j01 >> 16u) * weights.y
        + joint_matrix(j23 & 0xffffu) * weights.z
        + joint_matrix(j23 >> 16u) * weights.w;
    let m3 = mat3x3<f32>(m[0].xyz, m[1].xyz, m[2].xyz);

    let o = v * VERTEX_FLOATS;
    let position = vec3<f32>(source_vertices[o], source_vertices[o + 1u], source_vertices[o + 2u]);
    let normal = vec3<f32>(source_vertices[o + 3u], source_vertices[o + 4u], source_vertices[o + 5u]);
    let tangent = vec3<f32>(source_vertices[o + 8u], source_vertices[o + 9u], source_vertices[o + 10u]);

    let p = (m * vec4<f32>(position, 1.0)).xyz;
    // Exact for rotations and uniform scale; an approximation under non-uniform scale.
    let n = safe_normalize(m3 * normal, normal);
    let t = safe_normalize(m3 * tangent, tangent);

    skinned_vertices[o] = p.x;
    skinned_vertices[o + 1u] = p.y;
    skinned_vertices[o + 2u] = p.z;
    skinned_vertices[o + 3u] = n.x;
    skinned_vertices[o + 4u] = n.y;
    skinned_vertices[o + 5u] = n.z;
    skinned_vertices[o + 6u] = source_vertices[o + 6u];
    skinned_vertices[o + 7u] = source_vertices[o + 7u];
    skinned_vertices[o + 8u] = t.x;
    skinned_vertices[o + 9u] = t.y;
    skinned_vertices[o + 10u] = t.z;
    skinned_vertices[o + 11u] = source_vertices[o + 11u];
    skinned_vertices[o + 12u] = source_vertices[o + 12u];
    skinned_vertices[o + 13u] = source_vertices[o + 13u];
    skinned_vertices[o + 14u] = source_vertices[o + 14u];
    skinned_vertices[o + 15u] = source_vertices[o + 15u];
}
