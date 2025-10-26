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
    pick_id: array<u32, 2u>, // match CPU ObjectData pick identifier packing (96 byte stride)
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
    let object = objects[in.instance];
    let world_pos = object.model * vec4(in.pos, 1.0);
    return globals.view_proj * world_pos;
}
