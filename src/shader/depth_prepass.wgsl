// The camera and objects come from camera.wgsl and objects.wgsl.

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) tangent: vec4<f32>,
    @builtin(instance_index) instance: u32,
};

fn prepass_position(in: VsIn) -> vec4<f32> {
    let object = objects[in.instance];
    let world_pos = object.model * vec4(in.pos, 1.0);
    return globals.view_proj * world_pos;
}

@vertex
fn vs_main(in: VsIn) -> @builtin(position) vec4<f32> {
    return prepass_position(in);
}
