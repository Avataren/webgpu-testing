// src/shader/fog_apply.wgsl
// The opaque scene's fog (src/renderer/fog.rs): after the opaque passes and before
// transparent surfaces, which fog themselves, one fullscreen pass over the scene's
// viewport finds each pixel's surface from the depth buffer and blends the fog between it
// and the camera over it: colour * transmittance + scattered light (the blend state does
// the multiply, and leaves the colour's alpha, which marks glass, alone). The background
// gets the fog along its view ray out to the fog's max distance.
//
// Needs camera.wgsl, fog_common.wgsl, fog.wgsl and the lights group's modules.

// The scene's depth (texture_depth_multisampled_2d with MSAA: sample 0).
@group(1) @binding(0) var fog_scene_depth: texture_depth_2d;

struct FogApplyOut {
    @builtin(position) position: vec4<f32>,
};

@vertex
fn vs_fog_apply(@builtin(vertex_index) index: u32) -> FogApplyOut {
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var out: FogApplyOut;
    out.position = vec4<f32>(corner * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    return out;
}

@fragment
fn fs_fog_apply(in: FogApplyOut) -> @location(0) vec4<f32> {
    let depth = textureLoad(fog_scene_depth, vec2<i32>(in.position.xy), 0);
    let uv = (in.position.xy - fog.viewport.xy) / max(fog.viewport.zw, vec2<f32>(1.0));
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    if (depth >= 1.0) {
        let on_ray = globals.inverse_view_proj * vec4<f32>(ndc, 0.5, 1.0);
        return fog_toward(normalize(on_ray.xyz / on_ray.w - globals.camera_pos));
    }
    let surface = globals.inverse_view_proj * vec4<f32>(ndc, depth, 1.0);
    return fog_at(surface.xyz / surface.w);
}
