struct Globals {
    view_proj: mat4x4<f32>,
};
@group(0) @binding(0) var<uniform> globals: Globals;

struct Object {
    model: mat4x4<f32>,
};
@group(1) @binding(0) var<storage, read> objects: array<Object>;

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @builtin(instance_index) instance: u32,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    let M = objects[in.instance].model;
    let world_pos = M * vec4(in.pos, 1.0);

    // For correctness with non-uniform scales you'd use inverse-transpose
    let n = normalize((M * vec4(in.normal, 0.0)).xyz);

    var out: VsOut;
    out.pos = globals.view_proj * world_pos;
    out.normal = n;
    out.uv = in.uv;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // simple lambert-ish: light from +Z
    let L = normalize(vec3<f32>(0.5, 0.3, 1.0));
    let ndotl = max(dot(normalize(in.normal), L), 0.2); // min ambient
    let base = vec3<f32>(0.8, 0.7, 0.9);
    return vec4(base * ndotl, 1.0);
}
