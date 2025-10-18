@group(1) @binding(40)
var bloom_upsample_texture : texture_2d<f32>;
@group(1) @binding(41)
var bloom_upsample_base : texture_2d<f32>;
@group(1) @binding(42)
var bloom_upsample_sampler : sampler;

@fragment
fn fs_bloom_upsample(in : VertexOutput) -> @location(0) vec4<f32> {
    let tex_size = vec2<f32>(textureDimensions(bloom_upsample_texture, 0));
    let texel = 1.0 / tex_size;
    var filtered = vec3<f32>(0.0);
    var total = 0.0;
    for (var x : i32 = -1; x <= 1; x = x + 1) {
        for (var y : i32 = -1; y <= 1; y = y + 1) {
            let offset = vec2<f32>(f32(x), f32(y));
            let weight = bloom_gaussian_weight(offset, 1.5);
            let sample_uv = clamp(in.uv + offset * texel, vec2<f32>(0.0), vec2<f32>(1.0));
            let color = textureSampleLevel(
                bloom_upsample_texture,
                bloom_upsample_sampler,
                sample_uv,
                0.0,
            ).rgb;
            filtered = filtered + color * weight;
            total = total + weight;
        }
    }
    filtered = filtered / max(total, 1e-4);
    let base = textureSampleLevel(
        bloom_upsample_base,
        bloom_upsample_sampler,
        clamp(in.uv, vec2<f32>(0.0), vec2<f32>(1.0)),
        0.0,
    ).rgb;
    let scatter = clamp(post_uniform.bloom_params.z, 0.0, 1.0);
    return vec4<f32>(base + filtered * scatter, 1.0);
}
