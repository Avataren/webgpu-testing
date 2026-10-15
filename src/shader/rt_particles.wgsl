// Expands a GPU particle system into ray tracing geometry: one copy of the particle mesh
// (its template, already in the ray tracing vertex pool) per particle slot, placed as
// particle_render.wgsl places it, written into the system's range of the pool for its
// bottom-level structure to be built from. One thread per output vertex.
//
// Dead slots get NaN positions, which makes their triangles inactive: rays never hit them.

const MAX_COLOR_KEYS : u32 = 4u;

// As in particle_render.wgsl (and `Particle` in src/gpu_particles/particle.rs).
struct Particle {
    position : vec3<f32>,
    lifetime : f32,
    velocity : vec3<f32>,
    max_lifetime : f32,
    rotation : vec4<f32>,
    scale : vec3<f32>,
    angular_velocity : f32,
    color : vec4<f32>,
    spawn_color : vec4<f32>,
    color_keys : array<vec4<f32>, MAX_COLOR_KEYS>,
    color_key_times : vec4<f32>,
    user_data : vec4<f32>,
}

// The engine's `Vertex`, as the pool stores it (`RtVertex` in rt_reflections.wgsl).
struct RtVertex {
    px : f32,
    py : f32,
    pz : f32,
    nx : f32,
    ny : f32,
    nz : f32,
    u : f32,
    v : f32,
    tangent : vec4<f32>,
    color : vec4<f32>,
}

// `ExpandParams` in src/renderer/raytracing/scene.rs.
struct ExpandParams {
    camera_forward : vec4<f32>,
    camera_up : vec4<f32>,
    template_base : u32,
    template_count : u32,
    output_base : u32,
    particle_count : u32,
    billboarded : u32,
    _padding0 : u32,
    _padding1 : u32,
    _padding2 : u32,
}

@group(0) @binding(0) var<storage, read> particles : array<Particle>;
@group(0) @binding(1) var<storage, read_write> vertices : array<RtVertex>;
@group(0) @binding(2) var<uniform> params : ExpandParams;

fn safe_normalize(v : vec3<f32>, fallback : vec3<f32>) -> vec3<f32> {
    let len_sq = dot(v, v);
    if len_sq <= 1e-6 {
        return fallback;
    }
    return v * inverseSqrt(len_sq);
}

// Normalized, or `fallback` when shorter than `min_length` (thresholds as in
// particle_render.wgsl, so particles land exactly where they are drawn).
fn normalize_or(v : vec3<f32>, min_length : f32, fallback : vec3<f32>) -> vec3<f32> {
    let len = length(v);
    if len < min_length {
        return fallback;
    }
    return v / len;
}

fn billboard_matrix(camera_forward : vec3<f32>, camera_up : vec3<f32>) -> mat3x3<f32> {
    let forward = -safe_normalize(camera_forward, vec3<f32>(0.0, 0.0, -1.0));
    let up_hint = safe_normalize(camera_up, vec3<f32>(0.0, 1.0, 0.0));
    let right = normalize_or(cross(up_hint, forward), 1e-4, vec3<f32>(1.0, 0.0, 0.0));
    let up = normalize_or(cross(forward, right), 1e-4, vec3<f32>(0.0, 1.0, 0.0));
    return mat3x3<f32>(right, up, forward);
}

fn axis_angle_to_matrix(axis : vec3<f32>, angle : f32) -> mat3x3<f32> {
    let c = cos(angle);
    let s = sin(angle);
    let t = 1.0 - c;
    let x = axis.x;
    let y = axis.y;
    let z = axis.z;
    return mat3x3<f32>(
        vec3<f32>(t * x * x + c, t * x * y - s * z, t * x * z + s * y),
        vec3<f32>(t * x * y + s * z, t * y * y + c, t * y * z - s * x),
        vec3<f32>(t * x * z - s * y, t * y * z + s * x, t * z * z + c),
    );
}

@compute @workgroup_size(64)
fn cs_expand_particles(@builtin(global_invocation_id) id : vec3<u32>) {
    let index = id.x;
    if index >= params.particle_count * params.template_count {
        return;
    }
    let particle = particles[index / params.template_count];
    var vertex = vertices[params.template_base + index % params.template_count];
    if particle.lifetime < 0.0 {
        vertex.px = bitcast<f32>(0x7fc00000u);
        vertices[params.output_base + index] = vertex;
        return;
    }

    var rotation : mat3x3<f32>;
    if params.billboarded != 0u {
        rotation = billboard_matrix(params.camera_forward.xyz, params.camera_up.xyz);
    } else {
        let axis = normalize_or(particle.rotation.xyz, 1e-5, vec3<f32>(0.0, 1.0, 0.0));
        rotation = axis_angle_to_matrix(axis, particle.rotation.w);
    }
    let position = rotation * (vec3<f32>(vertex.px, vertex.py, vertex.pz) * particle.scale)
        + particle.position;
    let normal = rotation * vec3<f32>(vertex.nx, vertex.ny, vertex.nz);
    vertex.px = position.x;
    vertex.py = position.y;
    vertex.pz = position.z;
    vertex.nx = normal.x;
    vertex.ny = normal.y;
    vertex.nz = normal.z;
    vertex.tangent = vec4<f32>(rotation * vertex.tangent.xyz, vertex.tangent.w);
    // The particle shader colours by the particle alone, not the mesh's vertex colour.
    vertex.color = particle.color;
    vertices[params.output_base + index] = vertex;
}
