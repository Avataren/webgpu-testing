// src/shader/light_probe_keyframes.wgsl
//
// Blends a light probe volume's two keyframes around the time of day into its tile rows
// of the irradiance atlas (src/renderer/light_probe_keyframes.rs). The probes' open or
// inside flags (alpha) stay as the atlas has them.

struct KeyframeBlend {
    // x, y: the volume's first texel in the atlas; z, w: the two keyframes.
    region: vec4<u32>,
    // x: the second keyframe's share.
    weight: vec4<f32>,
};

@group(0) @binding(0) var<uniform> blend : KeyframeBlend;
@group(0) @binding(1) var keyframes : texture_2d_array<f32>;
@group(0) @binding(2) var atlas : texture_2d<f32>;

@fragment
fn fs_keyframe_blend(in : VertexOutput) -> @location(0) vec4<f32> {
    let texel = vec2<u32>(in.position.xy);
    let local = texel - blend.region.xy;
    let first = textureLoad(keyframes, local, blend.region.z, 0).rgb;
    let second = textureLoad(keyframes, local, blend.region.w, 0).rgb;
    let open = textureLoad(atlas, texel, 0).a;
    return vec4<f32>(mix(first, second, blend.weight.x), open);
}
