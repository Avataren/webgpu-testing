@group(3) @binding(0) var base_color_texture_binding: texture_2d<f32>;
@group(3) @binding(1) var metallic_roughness_texture_binding: texture_2d<f32>;
@group(3) @binding(2) var normal_texture_binding: texture_2d<f32>;
@group(3) @binding(3) var emissive_texture_binding: texture_2d<f32>;
@group(3) @binding(4) var occlusion_texture_binding: texture_2d<f32>;
@group(3) @binding(5) var tex_sampler_linear: sampler;
@group(3) @binding(6) var tex_sampler_nearest: sampler;

// Material textures sample with material_mip_bias() (negative while upscaling, so
// textures keep the sharpness of the output resolution); ShaderBuilder adds a zero
// default where the shader does not define it.
fn sample_base_color_texture(_index: u32, uv: vec2<f32>) -> vec4<f32> {
    return textureSampleBias(base_color_texture_binding, tex_sampler_nearest, uv, material_mip_bias());
}

fn sample_metallic_roughness_texture(
    _index: u32,
    uv: vec2<f32>,
) -> vec4<f32> {
    return textureSampleBias(metallic_roughness_texture_binding, tex_sampler_nearest, uv, material_mip_bias());
}

fn sample_normal_texture(_index: u32, uv: vec2<f32>) -> vec3<f32> {
    return textureSampleBias(normal_texture_binding, tex_sampler_nearest, uv, material_mip_bias()).xyz;
}

fn sample_emissive_texture(_index: u32, uv: vec2<f32>) -> vec3<f32> {
    return textureSampleBias(emissive_texture_binding, tex_sampler_nearest, uv, material_mip_bias()).rgb;
}

fn sample_occlusion_texture(_index: u32, uv: vec2<f32>) -> f32 {
    return textureSampleBias(occlusion_texture_binding, tex_sampler_nearest, uv, material_mip_bias()).r;
}
