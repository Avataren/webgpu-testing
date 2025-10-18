@group(1) @binding(10)
var color_source : texture_2d<f32>;
@group(1) @binding(11)
var color_sampler : sampler;

fn apply_color_controls(color: vec3<f32>) -> vec3<f32> {
    let exposure = max(post_uniform.color_adjust.x, 0.0);
    let saturation = max(post_uniform.color_adjust.y, 0.0);
    let contrast = max(post_uniform.color_adjust.z, 0.0);

    var adjusted = color * exposure;
    let luma = dot(adjusted, vec3<f32>(0.2126, 0.7152, 0.0722));
    adjusted = mix(vec3<f32>(luma), adjusted, saturation);
    let pivot = vec3<f32>(0.5, 0.5, 0.5);
    adjusted = (adjusted - pivot) * contrast + pivot;
    return adjusted;
}

@fragment
fn fs_color_adjust(in : VertexOutput) -> @location(0) vec4<f32> {
    let uv = clamp(in.uv, vec2<f32>(0.0), vec2<f32>(1.0));
    let base = textureSample(color_source, color_sampler, uv);
    let adjusted = apply_color_controls(base.rgb);
    return vec4<f32>(adjusted, base.a);
}
