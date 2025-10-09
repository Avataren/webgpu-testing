@group(4) @binding(0) var base_color_texture_binding: texture_2d<f32>;
@group(4) @binding(1) var metallic_roughness_texture_binding: texture_2d<f32>;
@group(4) @binding(2) var normal_texture_binding: texture_2d<f32>;
@group(4) @binding(3) var emissive_texture_binding: texture_2d<f32>;
@group(4) @binding(4) var occlusion_texture_binding: texture_2d<f32>;
@group(4) @binding(5) var tex_sampler_linear: sampler;
@group(4) @binding(6) var tex_sampler_nearest: sampler;

fn sample_base_color_texture(_index: u32, uv: vec2<f32>, use_nearest: bool) -> vec4<f32> {
    if (use_nearest) {
        return textureSample(base_color_texture_binding, tex_sampler_nearest, uv);
    }
    return textureSample(base_color_texture_binding, tex_sampler_linear, uv);
}

fn sample_metallic_roughness_texture(
    _index: u32,
    uv: vec2<f32>,
    use_nearest: bool,
) -> vec4<f32> {
    if (use_nearest) {
        return textureSample(metallic_roughness_texture_binding, tex_sampler_nearest, uv);
    }
    return textureSample(metallic_roughness_texture_binding, tex_sampler_linear, uv);
}

fn sample_normal_texture(_index: u32, uv: vec2<f32>, use_nearest: bool) -> vec3<f32> {
    if (use_nearest) {
        return textureSample(normal_texture_binding, tex_sampler_nearest, uv).xyz;
    }
    return textureSample(normal_texture_binding, tex_sampler_linear, uv).xyz;
}

fn sample_emissive_texture(_index: u32, uv: vec2<f32>, use_nearest: bool) -> vec3<f32> {
    if (use_nearest) {
        return textureSample(emissive_texture_binding, tex_sampler_nearest, uv).rgb;
    }
    return textureSample(emissive_texture_binding, tex_sampler_linear, uv).rgb;
}

fn sample_occlusion_texture(_index: u32, uv: vec2<f32>, use_nearest: bool) -> f32 {
    if (use_nearest) {
        return textureSample(occlusion_texture_binding, tex_sampler_nearest, uv).r;
    }
    return textureSample(occlusion_texture_binding, tex_sampler_linear, uv).r;
}
