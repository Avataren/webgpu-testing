struct ShadowGlobals {
    view_proj: mat4x4<f32>,
};
@group(0) @binding(0) var<uniform> shadow_globals: ShadowGlobals;

// `objects` and `materials` (group 1) come from objects.wgsl.

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) tangent: vec4<f32>,
    @builtin(instance_index) instance: u32,
};

fn shadow_position(in: VsIn) -> vec4<f32> {
    let obj = objects[in.instance];
    let world = obj.model * vec4<f32>(in.pos, 1.0);
    var clip = shadow_globals.view_proj * world;
    // A directional cascade (orthographic: w does not depend on the position) reaches
    // back past every caster it knows the extent of; casters nearer the light than that
    // are flattened onto its near plane instead of clipped away, so they still cast.
    let w_row = vec3<f32>(
        shadow_globals.view_proj[0].w,
        shadow_globals.view_proj[1].w,
        shadow_globals.view_proj[2].w,
    );
    if (all(w_row == vec3<f32>(0.0))) {
        clip.z = max(clip.z, 0.0);
    }
    return clip;
}

@vertex
fn vs_main(in: VsIn) -> @builtin(position) vec4<f32> {
    return shadow_position(in);
}
