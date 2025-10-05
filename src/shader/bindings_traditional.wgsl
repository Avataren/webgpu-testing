@group(3) @binding(0) var base_color_texture_binding: texture_2d<f32>;
@group(3) @binding(1) var metallic_roughness_texture_binding: texture_2d<f32>;
@group(3) @binding(2) var normal_texture_binding: texture_2d<f32>;
@group(3) @binding(3) var emissive_texture_binding: texture_2d<f32>;
@group(3) @binding(4) var occlusion_texture_binding: texture_2d<f32>;
@group(3) @binding(5) var tex_sampler: sampler;

fn sample_base_color_texture(_index: u32, uv: vec2<f32>) -> vec4<f32> {
    return textureSample(base_color_texture_binding, tex_sampler, uv);
}

fn sample_metallic_roughness_texture(_index: u32, uv: vec2<f32>) -> vec4<f32> {
    return textureSample(metallic_roughness_texture_binding, tex_sampler, uv);
}

fn sample_normal_texture(_index: u32, uv: vec2<f32>) -> vec3<f32> {
    return textureSample(normal_texture_binding, tex_sampler, uv).xyz;
}

fn sample_emissive_texture(_index: u32, uv: vec2<f32>) -> vec3<f32> {
    return textureSample(emissive_texture_binding, tex_sampler, uv).rgb;
}

fn sample_occlusion_texture(_index: u32, uv: vec2<f32>) -> f32 {
    return textureSample(occlusion_texture_binding, tex_sampler, uv).r;
}
