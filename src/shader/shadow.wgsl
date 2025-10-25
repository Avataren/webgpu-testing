struct ShadowGlobals {
    view_proj: mat4x4<f32>,
};
@group(0) @binding(0) var<uniform> shadow_globals: ShadowGlobals;

struct Object {
    model: mat4x4<f32>,
    material_index: u32,
    pick_id: array<u32, 2u>, // keep storage layout aligned with CPU ObjectData stride (96 bytes)
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

@vertex
fn vs_main(in: VsIn) -> @builtin(position) vec4<f32> {
    let obj = objects[in.instance];
    let world = obj.model * vec4<f32>(in.pos, 1.0);
    return shadow_globals.view_proj * world;
}
