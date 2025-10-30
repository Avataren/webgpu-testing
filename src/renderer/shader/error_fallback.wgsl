// src/renderer/shader/error_fallback.wgsl
// Pink checkerboard fallback shader used when user-authored WGSL fails to compile.

struct Globals {
    view_proj: mat4x4<f32>,
    inverse_view_proj: mat4x4<f32>,
    camera_pos: vec3<f32>,
    _padding0: f32,
    camera_forward: vec3<f32>,
    _padding1: f32,
    camera_up: vec3<f32>,
    _padding2: f32,
};

@group(0) @binding(0) var<uniform> globals: Globals;

struct Object {
    model: mat4x4<f32>,
    material_index: u32,
    pick_id: array<u32, 2u>,
    _padding: u32,
    _padding2: array<u32, 4>,
};

@group(1) @binding(0) var<storage, read> objects: array<Object>;
@group(1) @binding(1) var<storage, read> materials: array<MaterialData>;

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
};

struct ShadedFragment {
    color: vec4<f32>,
    normal: vec4<f32>,
    world_pos: vec4<f32>,
};

struct FragmentOutNoPick {
    @location(0) color: vec4<f32>,
    @location(1) normal: vec4<f32>,
    @location(2) world_pos: vec4<f32>,
};

struct FragmentOut {
    @location(0) color: vec4<f32>,
    @location(1) normal: vec4<f32>,
    @location(2) world_pos: vec4<f32>,
    @location(3) pick: vec2<u32>,
};

struct FragmentOutPick {
    @location(0) color: vec4<f32>,
    @location(1) pick: vec2<u32>,
};

fn object_pick_output(instance_index: u32, alpha: f32, material_flags: u32) -> vec2<u32> {
    let obj = objects[instance_index];
    let pick_value = vec2<u32>(obj.pick_id[0u], obj.pick_id[1u]);

    if (material_flags & FLAG_ALPHA_BLEND) != 0u && alpha <= 0.0 {
        return vec2<u32>(0u, 0u);
    }

    return pick_value;
}

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

fn shade_fragment(in: VsOut) -> ShadedFragment {
    let color = checkerboard_color(in.uv, in.world_pos);
    return ShadedFragment(
        color,
        vec4<f32>(normalize(in.normal), 1.0),
        vec4<f32>(in.world_pos, 1.0),
    );
}

@vertex
fn vs_main(in: VsIn) -> VsOut {
    let obj = objects[in.instance];
    let model = obj.model;
    let world_pos = model * vec4<f32>(in.pos, 1.0);
    let material = materials[obj.material_index];

    let n = normalize((model * vec4<f32>(in.normal, 0.0)).xyz);
    let t = normalize((model * vec4<f32>(in.tangent.xyz, 0.0)).xyz);
    let b = cross(n, t) * in.tangent.w;

    var out: VsOut;
    out.pos = globals.view_proj * world_pos;
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
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return shade_fragment(in).color;
}

@fragment
fn fs_main_gbuffer(in: VsOut) -> FragmentOutNoPick {
    let shaded = shade_fragment(in);
    return FragmentOutNoPick(shaded.color, shaded.normal, shaded.world_pos);
}

@fragment
fn fs_main_pick(in: VsOut) -> FragmentOutPick {
    let shaded = shade_fragment(in);
    let pick_value = object_pick_output(in.instance_id, shaded.color.a, in.material_flags);
    return FragmentOutPick(shaded.color, pick_value);
}

@fragment
fn fs_main_gbuffer_pick(in: VsOut) -> FragmentOut {
    let shaded = shade_fragment(in);
    let pick_value = object_pick_output(in.instance_id, shaded.color.a, in.material_flags);
    return FragmentOut(shaded.color, shaded.normal, shaded.world_pos, pick_value);
}
