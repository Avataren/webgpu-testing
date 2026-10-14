@group(1) @binding(20)
var scene_texture : texture_2d<f32>;
@group(1) @binding(21)
var scene_sampler : sampler;

// First bloom mip: downsample the HDR scene with the Karis average, then keep only what
// is above the threshold.
@fragment
fn fs_bloom_prefilter(in : VertexOutput) -> @location(0) vec4<f32> {
    let tex_size = vec2<f32>(textureDimensions(scene_texture, 0));
    let texel = bloom_texel(tex_size);
    let color = bloom_downsample13(scene_texture, scene_sampler, in.uv, texel, true);
    // Guard against NaN/inf from broken materials spreading over the whole glow.
    let safe = select(color, vec3<f32>(0.0), color != color);
    return vec4<f32>(bloom_apply_threshold(min(safe, vec3<f32>(65000.0))), 1.0);
}
