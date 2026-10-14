@group(1) @binding(40)
var bloom_upsample_texture : texture_2d<f32>;
@group(1) @binding(42)
var bloom_upsample_sampler : sampler;

// 3x3 tent upsample of the next smaller mip. The pipeline blends the result over the
// current mip with the blend constant set to the scatter amount, so each level keeps
// (1 - scatter) of its own detail and takes scatter from the wider levels.
@fragment
fn fs_bloom_upsample(in : VertexOutput) -> @location(0) vec4<f32> {
    let tex_size = vec2<f32>(textureDimensions(bloom_upsample_texture, 0));
    let texel = bloom_texel(tex_size);
    let t = bloom_upsample_texture;
    let s = bloom_upsample_sampler;
    var color = bloom_tap(t, s, in.uv) * 4.0;
    color += (bloom_tap(t, s, in.uv + texel * vec2<f32>(-1.0, 0.0))
        + bloom_tap(t, s, in.uv + texel * vec2<f32>(1.0, 0.0))
        + bloom_tap(t, s, in.uv + texel * vec2<f32>(0.0, -1.0))
        + bloom_tap(t, s, in.uv + texel * vec2<f32>(0.0, 1.0))) * 2.0;
    color += bloom_tap(t, s, in.uv + texel * vec2<f32>(-1.0, -1.0))
        + bloom_tap(t, s, in.uv + texel * vec2<f32>(1.0, -1.0))
        + bloom_tap(t, s, in.uv + texel * vec2<f32>(-1.0, 1.0))
        + bloom_tap(t, s, in.uv + texel * vec2<f32>(1.0, 1.0));
    return vec4<f32>(color / 16.0, 1.0);
}
