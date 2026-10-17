// src/renderer/shader/error_fallback.wgsl
// Pink checkerboard fallback shader used when user-authored WGSL fails to compile.


// The camera and objects come from camera.wgsl and objects.wgsl.
// Last frame's model matrices, indexed like `objects` (motion vectors).
@group(1) @binding(2) var<storage, read> prev_models: array<mat4x4<f32>>;

// Last frame's model matrix for an instance; an all-zero entry means no history.
fn previous_model(instance: u32, current: mat4x4<f32>) -> mat4x4<f32> {
    let prev = prev_models[instance];
    if prev[3][3] == 0.0 {
        return current;
    }
    return prev;
}


// Octahedral normal encoding: a unit vector in two numbers, leaving room for velocity in
// the same G-buffer texel.
fn encode_normal_oct(n: vec3<f32>) -> vec2<f32> {
    let p = n.xy / max(abs(n.x) + abs(n.y) + abs(n.z), 1e-6);
    if n.z >= 0.0 {
        return p;
    }
    let signs = select(vec2<f32>(-1.0), vec2<f32>(1.0), p >= vec2<f32>(0.0));
    return (1.0 - abs(p.yx)) * signs;
}

// G-buffer normal texel: encoded normal in xy, motion vector in zw.
fn pack_normal_velocity(n: vec3<f32>, velocity: vec2<f32>) -> vec4<f32> {
    return vec4<f32>(encode_normal_oct(normalize(n)), velocity);
}

// Screen motion since last frame in UV units (+y down), jitter excluded.
fn motion_vector(curr_clip: vec4<f32>, prev_clip: vec4<f32>) -> vec2<f32> {
    if curr_clip.w <= 0.0 || prev_clip.w <= 0.0 {
        return vec2<f32>(0.0);
    }
    let delta = curr_clip.xy / curr_clip.w - prev_clip.xy / prev_clip.w;
    return delta * vec2<f32>(0.5, -0.5);
}


struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) tangent: vec4<f32>,
    @builtin(instance_index) instance: u32,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) @interpolate(flat) instance_id: u32,
    @location(4) tangent: vec3<f32>,
    @location(5) bitangent: vec3<f32>,
    @location(6) @interpolate(flat) material_color: vec4<f32>,
    @location(7) @interpolate(flat) material_texture_indices0: vec4<u32>,
    @location(8) @interpolate(flat) material_texture_indices1: vec2<u32>,
    @location(9) @interpolate(flat) material_flags: u32,
    @location(10) @interpolate(flat) material_factors: vec3<f32>,
    @location(11) curr_clip: vec4<f32>,
    @location(12) prev_clip: vec4<f32>,
};

struct ShadedFragment {
    color: vec4<f32>,
    normal: vec4<f32>,
    world_pos: vec4<f32>,
};

struct GBufferOut {
    @location(0) color: vec4<f32>,
    @location(1) normal: vec4<f32>,
    @location(2) world_pos: vec4<f32>,
};

fn checkerboard_color(uv: vec2<f32>, world_pos: vec3<f32>) -> vec4<f32> {
    let grid_uv = uv * 16.0 + world_pos.xz * 0.25;
    let cell = (i32(floor(grid_uv.x)) + i32(floor(grid_uv.y))) & 1;
    let primary = vec3<f32>(1.0, 0.2, 0.7);
    let secondary = vec3<f32>(0.2, 0.0, 0.3);

    if cell == 0 {
        return vec4<f32>(primary, 1.0);
    }

    return vec4<f32>(secondary, 1.0);
}

fn shade_fragment(in: VsOut, front_facing: bool) -> ShadedFragment {
    let color = checkerboard_color(in.uv, in.world_pos);
    return ShadedFragment(
        color,
        vec4<f32>(normalize(in.normal) * select(-1.0, 1.0, front_facing), 1.0),
        vec4<f32>(in.world_pos, 1.0),
    );
}

@vertex
fn vs_main(in: VsIn) -> VsOut {
    let obj = objects[in.instance];
    let model = obj.model;
    let world_pos = model * vec4<f32>(in.pos, 1.0);
    let material = materials[obj.material_index];

    let n = normalize(normal_matrix(model) * in.normal);
    let t = normalize((model * vec4<f32>(in.tangent.xyz, 0.0)).xyz);
    let b = cross(n, t) * in.tangent.w;

    var out: VsOut;
    out.pos = globals.view_proj * world_pos;
    out.curr_clip = globals.unjittered_view_proj * world_pos;
    out.prev_clip = globals.prev_view_proj
        * (previous_model(in.instance, model) * vec4<f32>(in.pos, 1.0));
    out.world_pos = world_pos.xyz;
    out.normal = n;
    out.uv = in.uv;
    out.instance_id = in.instance;
    out.tangent = t;
    out.bitangent = b;
    out.material_color = material.color;
    out.material_texture_indices0 = vec4<u32>(
        material.base_color_texture,
        material.metallic_roughness_texture,
        material.normal_texture,
        material.emissive_texture,
    );
    out.material_texture_indices1 = vec2<u32>(material.occlusion_texture, 0u);
    out.material_flags = material.material_flags;
    out.material_factors = vec3<f32>(
        material.metallic_factor,
        material.roughness_factor,
        material.emissive_strength,
    );
    return out;
}

@fragment
fn fs_main(in: VsOut, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    return shade_fragment(in, front_facing).color;
}

@fragment
fn fs_main_gbuffer(in: VsOut, @builtin(front_facing) front_facing: bool) -> GBufferOut {
    let shaded = shade_fragment(in, front_facing);
    return GBufferOut(
        shaded.color,
        pack_normal_velocity(shaded.normal.xyz, motion_vector(in.curr_clip, in.prev_clip)),
        shaded.world_pos,
    );
}
