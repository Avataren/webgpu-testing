// Linear blend skinning: deforms the vertices of skinned mesh instances with their joint
// palettes and writes them to each instance's slot in a pool buffer, which every render
// pass then draws like a rigid mesh. One dispatch covers every instance read from one
// source page and written to one output page: each instance has a record and a run of
// workgroups (one per 64 vertices), which find their record by binary search.
//
// Layouts (must match the Rust side, src/renderer/skinning.rs):
// - Vertex (64 bytes = 16 f32): pos.xyz, normal.xyz, uv.xy, tangent.xyzw, color.rgba
// - SkinVertex (16 bytes = 4 u32): joints as u16x4, weights as unorm16x4
// - Record (32 bytes): palette_offset, joint_count, source_offset, output_offset
//   (vertices), vertex_count, first_workgroup, padding
//
// The CPU fallback (`skin_vertices`) implements the same math; keep them in step.

struct Params {
    first_record: u32,
    record_count: u32,
    workgroup_count: u32,
    _padding: u32,
}

struct Record {
    palette_offset: u32,
    joint_count: u32,
    source_offset: u32,
    output_offset: u32,
    vertex_count: u32,
    first_workgroup: u32,
    _padding0: u32,
    _padding1: u32,
}

// group 0: the source page and the output page
@group(0) @binding(0) var<storage, read> source_vertices: array<f32>;
@group(0) @binding(1) var<storage, read> skin: array<u32>;
@group(0) @binding(2) var<storage, read_write> skinned_vertices: array<f32>;
// group 1: shared by all instances of the frame
@group(1) @binding(0) var<storage, read> palette: array<mat4x4<f32>>;
@group(1) @binding(1) var<storage, read> records: array<Record>;
@group(1) @binding(2) var<uniform> params: Params;

const VERTEX_FLOATS: u32 = 16u;
const WORKGROUP_SIZE: u32 = 64u;

fn joint_matrix(record: Record, joint: u32) -> mat4x4<f32> {
    return palette[record.palette_offset + min(joint, record.joint_count - 1u)];
}

fn safe_normalize(v: vec3<f32>, fallback: vec3<f32>) -> vec3<f32> {
    let len_sq = dot(v, v);
    if len_sq <= 1e-20 {
        return fallback;
    }
    return v * inverseSqrt(len_sq);
}

// The last record whose first workgroup is at or before `workgroup`.
fn find_record(workgroup: u32) -> Record {
    var lo = 0u;
    var hi = params.record_count - 1u;
    while lo < hi {
        let mid = (lo + hi + 1u) / 2u;
        if records[params.first_record + mid].first_workgroup <= workgroup {
            lo = mid;
        } else {
            hi = mid - 1u;
        }
    }
    return records[params.first_record + lo];
}

@compute @workgroup_size(64)
fn cs_main(
    @builtin(workgroup_id) workgroup_id: vec3<u32>,
    @builtin(num_workgroups) num_workgroups: vec3<u32>,
    @builtin(local_invocation_id) local_id: vec3<u32>,
) {
    let workgroup = workgroup_id.y * num_workgroups.x + workgroup_id.x;
    if workgroup >= params.workgroup_count {
        return;
    }
    let record = find_record(workgroup);
    let v = (workgroup - record.first_workgroup) * WORKGROUP_SIZE + local_id.x;
    if v >= record.vertex_count {
        return;
    }

    let s = (record.source_offset + v) * 4u;
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

    let m = joint_matrix(record, j01 & 0xffffu) * weights.x
        + joint_matrix(record, j01 >> 16u) * weights.y
        + joint_matrix(record, j23 & 0xffffu) * weights.z
        + joint_matrix(record, j23 >> 16u) * weights.w;
    let m3 = mat3x3<f32>(m[0].xyz, m[1].xyz, m[2].xyz);

    let o = (record.source_offset + v) * VERTEX_FLOATS;
    let position = vec3<f32>(source_vertices[o], source_vertices[o + 1u], source_vertices[o + 2u]);
    let normal = vec3<f32>(source_vertices[o + 3u], source_vertices[o + 4u], source_vertices[o + 5u]);
    let tangent = vec3<f32>(source_vertices[o + 8u], source_vertices[o + 9u], source_vertices[o + 10u]);

    let p = (m * vec4<f32>(position, 1.0)).xyz;
    // Exact for rotations and uniform scale; an approximation under non-uniform scale.
    let n = safe_normalize(m3 * normal, normal);
    let t = safe_normalize(m3 * tangent, tangent);

    let d = (record.output_offset + v) * VERTEX_FLOATS;
    skinned_vertices[d] = p.x;
    skinned_vertices[d + 1u] = p.y;
    skinned_vertices[d + 2u] = p.z;
    skinned_vertices[d + 3u] = n.x;
    skinned_vertices[d + 4u] = n.y;
    skinned_vertices[d + 5u] = n.z;
    skinned_vertices[d + 6u] = source_vertices[o + 6u];
    skinned_vertices[d + 7u] = source_vertices[o + 7u];
    skinned_vertices[d + 8u] = t.x;
    skinned_vertices[d + 9u] = t.y;
    skinned_vertices[d + 10u] = t.z;
    skinned_vertices[d + 11u] = source_vertices[o + 11u];
    skinned_vertices[d + 12u] = source_vertices[o + 12u];
    skinned_vertices[d + 13u] = source_vertices[o + 13u];
    skinned_vertices[d + 14u] = source_vertices[o + 14u];
    skinned_vertices[d + 15u] = source_vertices[o + 15u];
}
