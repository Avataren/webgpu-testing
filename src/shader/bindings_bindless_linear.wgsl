@group(3) @binding(0) var textures: binding_array<texture_2d<f32>, 256>;
@group(3) @binding(1) var tex_sampler_linear: sampler;
@group(3) @binding(2) var tex_sampler_nearest: sampler;

fn sample_base_color_texture(index: u32, uv: vec2<f32>) -> vec4<f32> {
    return textureSample(textures[index], tex_sampler_linear, uv);
}

fn sample_metallic_roughness_texture(
    index: u32,
    uv: vec2<f32>,
) -> vec4<f32> {
    return textureSample(textures[index], tex_sampler_linear, uv);
}

fn sample_normal_texture(index: u32, uv: vec2<f32>) -> vec3<f32> {
    return textureSample(textures[index], tex_sampler_linear, uv).xyz;
}

fn sample_emissive_texture(index: u32, uv: vec2<f32>) -> vec3<f32> {
    return textureSample(textures[index], tex_sampler_linear, uv).rgb;
}

fn sample_occlusion_texture(index: u32, uv: vec2<f32>) -> f32 {
    return textureSample(textures[index], tex_sampler_linear, uv).r;
}
