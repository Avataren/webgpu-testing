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
    _padding: u32,
    _padding2: vec2<u32>,
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
