@group(1) @binding(30)
var bloom_down_texture : texture_2d<f32>;
@group(1) @binding(31)
var bloom_down_sampler : sampler;

@fragment
fn fs_bloom_downsample(in : VertexOutput) -> @location(0) vec4<f32> {
    let tex_size = vec2<f32>(textureDimensions(bloom_down_texture, 0));
    let texel = 1.0 / tex_size;
    var result = vec3<f32>(0.0);
    var total = 0.0;
    for (var x : i32 = -2; x <= 2; x = x + 1) {
        for (var y : i32 = -2; y <= 2; y = y + 1) {
            let offset = vec2<f32>(f32(x), f32(y));
            let weight = bloom_gaussian_weight(offset, 2.5);
            let sample_uv = clamp(in.uv + offset * texel, vec2<f32>(0.0), vec2<f32>(1.0));
            let color = textureSampleLevel(
                bloom_down_texture,
                bloom_down_sampler,
                sample_uv,
                0.0,
            ).rgb;
            result = result + color * weight;
            total = total + weight;
        }
    }
    return vec4<f32>(result / max(total, 1e-4), 1.0);
}
