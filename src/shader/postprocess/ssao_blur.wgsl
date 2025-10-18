@group(1) @binding(60)
var ssao_blur_texture : texture_2d<f32>;
@group(1) @binding(61)
var ssao_blur_sampler : sampler;

fn ssao_texel_size() -> vec2<f32> {
    let dims = vec2<f32>(textureDimensions(ssao_blur_texture, 0));
    let safe = max(dims, vec2<f32>(1.0, 1.0));
    return vec2<f32>(1.0, 1.0) / safe;
}

fn ssao_blur_value(uv : vec2<f32>, direction : vec2<f32>) -> f32 {
    let texel = ssao_texel_size();
    let sigma = 2.0;
    var accum = 0.0;
    var weight_sum = 0.0;

    for (var i : i32 = -2; i <= 2; i = i + 1) {
        let offset = f32(i);
        let weight = exp(-(offset * offset) / (2.0 * sigma * sigma));
        let sample_uv = clamp(
            uv + direction * vec2<f32>(texel.x * offset, texel.y * offset),
            vec2<f32>(0.0),
            vec2<f32>(1.0),
        );
        let sample_value = textureSampleLevel(
            ssao_blur_texture,
            ssao_blur_sampler,
            sample_uv,
            0.0,
        ).r;
        accum = accum + sample_value * weight;
        weight_sum = weight_sum + weight;
    }

    return accum / max(weight_sum, 1e-4);
}

@fragment
fn fs_ssao_blur_horizontal(in : VertexOutput) -> @location(0) vec4<f32> {
    let ao = ssao_blur_value(in.uv, vec2<f32>(1.0, 0.0));
    return vec4<f32>(ao, ao, ao, 1.0);
}

@fragment
fn fs_ssao_blur_vertical(in : VertexOutput) -> @location(0) vec4<f32> {
    let ao = ssao_blur_value(in.uv, vec2<f32>(0.0, 1.0));
    return vec4<f32>(ao, ao, ao, 1.0);
}
