@group(1) @binding(20)
var scene_texture : texture_2d<f32>;
@group(1) @binding(21)
var scene_sampler : sampler;

@fragment
fn fs_bloom_prefilter(in : VertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(scene_texture, scene_sampler, in.uv).rgb;
    let brightness = max(max(color.r, color.g), color.b);
    let threshold = max(post_uniform.bloom_params.x, 0.0);
    let knee = max(post_uniform.bloom_params.y, 1e-4);
    let soft = brightness - threshold + knee;
    let clamped = clamp(soft, 0.0, 2.0 * knee);
    let soft_curve = clamped * clamped / (4.0 * knee + 1e-5);
    let contribution = max(soft_curve, brightness - threshold);
    let weight = contribution / max(brightness, 1e-4);
    return vec4<f32>(color * weight, 1.0);
}
