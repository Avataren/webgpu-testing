@group(3) @binding(0) var textures: binding_array<texture_2d<f32>, 256>;
@group(3) @binding(1) var tex_sampler_linear: sampler;
@group(3) @binding(2) var tex_sampler_nearest: sampler;

// Texture slots past the array (more textures loaded than it holds) read slot 0, and
// the sample functions below swap in a neutral value for them.
fn texture_slot(index: u32) -> u32 {
    return select(0u, index, index < 256u);
}

// Material textures sample with material_mip_bias() (negative while upscaling, so
// textures keep the sharpness of the output resolution); ShaderBuilder adds a zero
// default where the shader does not define it.
fn texture_sample(index: u32, uv: vec2<f32>, missing: vec4<f32>) -> vec4<f32> {
    let value = textureSampleBias(textures[texture_slot(index)], tex_sampler_nearest, uv, material_mip_bias());
    return select(missing, value, index < 256u);
}

fn sample_base_color_texture(index: u32, uv: vec2<f32>) -> vec4<f32> {
    return texture_sample(index, uv, vec4<f32>(1.0));
}

fn sample_metallic_roughness_texture(
    index: u32,
    uv: vec2<f32>,
) -> vec4<f32> {
    return texture_sample(index, uv, vec4<f32>(1.0));
}

fn sample_normal_texture(index: u32, uv: vec2<f32>) -> vec3<f32> {
    return texture_sample(index, uv, vec4<f32>(0.5, 0.5, 1.0, 1.0)).xyz;
}

fn sample_emissive_texture(index: u32, uv: vec2<f32>) -> vec3<f32> {
    return texture_sample(index, uv, vec4<f32>(0.0)).rgb;
}

fn sample_occlusion_texture(index: u32, uv: vec2<f32>) -> f32 {
    return texture_sample(index, uv, vec4<f32>(1.0)).r;
}
