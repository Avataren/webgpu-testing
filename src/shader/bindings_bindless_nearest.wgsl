@group(3) @binding(0) var textures: binding_array<texture_2d<f32>, 256>;
@group(3) @binding(1) var tex_sampler_linear: sampler;
@group(3) @binding(2) var tex_sampler_nearest: sampler;

// Material textures sample with material_mip_bias() (negative while upscaling, so
// textures keep the sharpness of the output resolution); ShaderBuilder adds a zero
// default where the shader does not define it.
fn sample_base_color_texture(index: u32, uv: vec2<f32>) -> vec4<f32> {
    return textureSampleBias(textures[index], tex_sampler_nearest, uv, material_mip_bias());
}

fn sample_metallic_roughness_texture(
    index: u32,
    uv: vec2<f32>,
) -> vec4<f32> {
    return textureSampleBias(textures[index], tex_sampler_nearest, uv, material_mip_bias());
}

fn sample_normal_texture(index: u32, uv: vec2<f32>) -> vec3<f32> {
    return textureSampleBias(textures[index], tex_sampler_nearest, uv, material_mip_bias()).xyz;
}

fn sample_emissive_texture(index: u32, uv: vec2<f32>) -> vec3<f32> {
    return textureSampleBias(textures[index], tex_sampler_nearest, uv, material_mip_bias()).rgb;
}

fn sample_occlusion_texture(index: u32, uv: vec2<f32>) -> f32 {
    return textureSampleBias(textures[index], tex_sampler_nearest, uv, material_mip_bias()).r;
}
