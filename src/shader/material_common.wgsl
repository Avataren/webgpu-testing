// src/shader/material_common.wgsl
//
// Shared material data structure and flag definitions used across renderer
// and particle shaders. Keeping this module separate ensures CPU-side
// `MaterialData` stays in sync with WGSL layouts.

struct MaterialData {
    color: vec4<f32>,
    base_color_texture: u32,
    metallic_roughness_texture: u32,
    normal_texture: u32,
    emissive_texture: u32,
    occlusion_texture: u32,
    material_flags: u32,
    metallic_factor: f32,
    roughness_factor: f32,
    emissive_strength: f32,
    // Share of light passing through (refracting glass), and its index of refraction.
    transmission: f32,
    ior: f32,
    // 20 / Abbe number (glTF KHR_materials_dispersion): how much ior varies with wavelength.
    dispersion: f32,
};

const FLAG_USE_BASE_COLOR_TEXTURE: u32 = 1u;
const FLAG_USE_METALLIC_ROUGHNESS_TEXTURE: u32 = 2u;
const FLAG_USE_NORMAL_TEXTURE: u32 = 4u;
const FLAG_USE_EMISSIVE_TEXTURE: u32 = 8u;
const FLAG_USE_OCCLUSION_TEXTURE: u32 = 16u;
const FLAG_ALPHA_BLEND: u32 = 32u;
const FLAG_DOUBLE_SIDED: u32 = 64u;
const FLAG_UNLIT: u32 = 128u;
const FLAG_USE_NEAREST_FILTERING: u32 = 256u;
const FLAG_BILLBOARDED: u32 = 512u;

// Alpha below which the surface is cut away (glTF MASK), from the flags' top byte; 0 for
// no cutoff.
fn material_alpha_cutoff(flags: u32) -> f32 {
    return f32(flags >> 24u) / 255.0;
}

// Turns object-space normals into world space for `model`: the cofactor matrix of its
// upper 3x3, which is the inverse transpose up to a positive scale (normalise the
// result). Transforming normals by the model matrix itself tilts them off the surface
// under non-uniform scale. The determinant's sign keeps mirrored objects facing out.
fn normal_matrix(model: mat4x4<f32>) -> mat3x3<f32> {
    let x = model[0].xyz;
    let y = model[1].xyz;
    let z = model[2].xyz;
    let cofactor = mat3x3<f32>(cross(y, z), cross(z, x), cross(x, y));
    return cofactor * select(1.0, -1.0, dot(x, cross(y, z)) < 0.0);
}
