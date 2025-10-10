struct Globals {
    view_proj: mat4x4<f32>,
    inverse_view_proj: mat4x4<f32>,
    camera_pos: vec3<f32>,
    _padding: f32,
};
@group(0) @binding(0) var<uniform> globals: Globals;

struct Object {
    model: mat4x4<f32>,
    material_index: u32,
    _padding: vec3<u32>,
};
@group(1) @binding(0) var<storage, read> objects: array<Object>;

struct MaterialData {
    color: vec4<f32>,
    base_color_texture: u32,
    metallic_roughness_texture: u32,
    normal_texture: u32,
    emissive_texture: u32,
    occlusion_texture: u32,
    material_flags: u32,
    metallic_factor: f32,
    roughness_factor: f32,
    emissive_strength: f32,
    _padding: u32,
    _padding2: vec2<u32>,
};
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
