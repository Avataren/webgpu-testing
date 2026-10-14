@group(1) @binding(30)
var bloom_down_texture : texture_2d<f32>;
@group(1) @binding(31)
var bloom_down_sampler : sampler;

@fragment
fn fs_bloom_downsample(in : VertexOutput) -> @location(0) vec4<f32> {
    let tex_size = vec2<f32>(textureDimensions(bloom_down_texture, 0));
    let texel = bloom_texel(tex_size);
    let color = bloom_downsample13(bloom_down_texture, bloom_down_sampler, in.uv, texel, false);
    return vec4<f32>(color, 1.0);
}
