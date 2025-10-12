// src/shader/common.wgsl
// Main PBR shader - now using modular lighting system

// ============================================================================
// Camera and Global Bindings
// ============================================================================

struct Globals {
    view_proj: mat4x4<f32>,
    inverse_view_proj: mat4x4<f32>,
    camera_pos: vec3<f32>,
    _padding: f32,
};
@group(0) @binding(0) var<uniform> globals: Globals;

// ============================================================================
// Material System
// ============================================================================

struct Object {
    model: mat4x4<f32>,
    material_index: u32,
    _padding: array<u32, 3>,
    _padding2: array<u32, 4>,
};
@group(1) @binding(0) var<storage, read> objects: array<Object>;

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
@group(1) @binding(1) var<storage, read> materials: array<MaterialData>;

// Material flags
const FLAG_USE_BASE_COLOR_TEXTURE: u32 = 1u;
const FLAG_USE_METALLIC_ROUGHNESS_TEXTURE: u32 = 2u;
const FLAG_USE_NORMAL_TEXTURE: u32 = 4u;
const FLAG_USE_EMISSIVE_TEXTURE: u32 = 8u;
const FLAG_USE_OCCLUSION_TEXTURE: u32 = 16u;
const FLAG_ALPHA_BLEND: u32 = 32u;
const FLAG_UNLIT: u32 = 128u;
const FLAG_USE_NEAREST_SAMPLER: u32 = 256u;

// Note: Lighting, shadows, and environment are imported from modules below
// Group 2 bindings:
//   @binding(0) - lights (from lighting_common.wgsl)
//   @binding(1-7) - shadows (from shadows.wgsl)
//   @binding(8-10) - environment (from environment.wgsl)

// ============================================================================
// Texture Bindings (Bindless or Traditional)
// ============================================================================

// Include one of these based on your feature flag:
// #include "bindings_bindless.wgsl"
// or
// #include "bindings_traditional.wgsl"


//@group(3) @binding(0) var textures: binding_array<texture_2d<f32>, 256>;
@group(3) @binding(1) var tex_sampler_linear: sampler;
@group(3) @binding(2) var tex_sampler_nearest: sampler;

fn sample_base_color_texture(index: u32, uv: vec2<f32>, use_nearest: bool) -> vec4<f32> {
    if (use_nearest) {
        return textureSample(textures[index], tex_sampler_nearest, uv);
    }
    return textureSample(textures[index], tex_sampler_linear, uv);
}

fn sample_metallic_roughness_texture(index: u32, uv: vec2<f32>, use_nearest: bool) -> vec4<f32> {
    if (use_nearest) {
        return textureSample(textures[index], tex_sampler_nearest, uv);
    }
    return textureSample(textures[index], tex_sampler_linear, uv);
}

fn sample_normal_texture(index: u32, uv: vec2<f32>, use_nearest: bool) -> vec3<f32> {
    if (use_nearest) {
        return textureSample(textures[index], tex_sampler_nearest, uv).xyz;
    }
    return textureSample(textures[index], tex_sampler_linear, uv).xyz;
}

fn sample_emissive_texture(index: u32, uv: vec2<f32>, use_nearest: bool) -> vec3<f32> {
    if (use_nearest) {
        return textureSample(textures[index], tex_sampler_nearest, uv).rgb;
    }
    return textureSample(textures[index], tex_sampler_linear, uv).rgb;
}

fn sample_occlusion_texture(index: u32, uv: vec2<f32>, use_nearest: bool) -> f32 {
    if (use_nearest) {
        return textureSample(textures[index], tex_sampler_nearest, uv).r;
    }
    return textureSample(textures[index], tex_sampler_linear, uv).r;
}

// ============================================================================
// Import Lighting Modules
// ============================================================================

// Include in this order:
// 1. lighting_common.wgsl - Provides lights binding, PBR functions, light contribution
// 2. shadows.wgsl - Provides shadow bindings and sampling
// 3. environment.wgsl - Provides environment bindings and lighting
// 4. lighting_with_shadows.wgsl - Provides complete scene lighting function

// In your build system, you'd concatenate these files or use a proper include mechanism
// For demonstration, I'm showing the structure with comments

// ============================================================================
// Vertex Shader
// ============================================================================

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) tangent: vec4<f32>,  // xyz = tangent, w = handedness
    @builtin(instance_index) instance: u32,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) @interpolate(flat) instance_id: u32,
    @location(4) tangent: vec3<f32>,
    @location(5) bitangent: vec3<f32>,
    @location(6) @interpolate(flat) material_color: vec4<f32>,
    @location(7) @interpolate(flat) material_texture_indices0: vec4<u32>,
    @location(8) @interpolate(flat) material_texture_indices1: vec2<u32>,
    @location(9) @interpolate(flat) material_flags: u32,
    @location(10) @interpolate(flat) material_factors: vec3<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    let obj = objects[in.instance];
    let M = obj.model;
    let world_pos = M * vec4(in.pos, 1.0);
    let material = materials[obj.material_index];

    // Transform normal and tangent to world space
    let n = normalize((M * vec4(in.normal, 0.0)).xyz);
    let t = normalize((M * vec4(in.tangent.xyz, 0.0)).xyz);
    
    // Calculate bitangent using the handedness from the tangent w component
    let b = cross(n, t) * in.tangent.w;

    var out: VsOut;
    out.pos = globals.view_proj * world_pos;
    out.world_pos = world_pos.xyz;
    out.normal = n;
    out.uv = in.uv;
    out.instance_id = in.instance;
    out.tangent = t;
    out.bitangent = b;
    out.material_color = material.color;
    out.material_texture_indices0 = vec4<u32>(
        material.base_color_texture,
        material.metallic_roughness_texture,
        material.normal_texture,
        material.emissive_texture,
    );
    out.material_texture_indices1 = vec2<u32>(material.occlusion_texture, 0u);
    out.material_flags = material.material_flags;
    out.material_factors = vec3<f32>(
        material.metallic_factor,
        material.roughness_factor,
        material.emissive_strength,
    );
    return out;
}

// ============================================================================
// Fragment Shader
// ============================================================================

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // ALWAYS sample all textures (uniform control flow)
    let material_flags = in.material_flags;
    let use_nearest_sampler = (material_flags & FLAG_USE_NEAREST_SAMPLER) != 0u;
    let base_color_sample = sample_base_color_texture(
        in.material_texture_indices0.x, in.uv, use_nearest_sampler
    );
    let mr_sample = sample_metallic_roughness_texture(
        in.material_texture_indices0.y, in.uv, use_nearest_sampler
    );
    let normal_sample = sample_normal_texture(
        in.material_texture_indices0.z, in.uv, use_nearest_sampler
    );
    let emissive_sample = sample_emissive_texture(
        in.material_texture_indices0.w, in.uv, use_nearest_sampler
    );
    let occlusion_sample = sample_occlusion_texture(
        in.material_texture_indices1.x, in.uv, use_nearest_sampler
    );

    // Then conditionally USE the samples
    var base_color: vec4<f32>;
    if ((material_flags & FLAG_USE_BASE_COLOR_TEXTURE) != 0u) {
        base_color = base_color_sample * in.material_color;
    } else {
        base_color = in.material_color;
    }

    var metallic: f32;
    var roughness: f32;
    if ((material_flags & FLAG_USE_METALLIC_ROUGHNESS_TEXTURE) != 0u) {
        metallic = mr_sample.b * in.material_factors.x;
        roughness = mr_sample.g * in.material_factors.y;
    } else {
        metallic = in.material_factors.x;
        roughness = in.material_factors.y;
    }
    roughness = max(roughness, 0.01);

    var N: vec3<f32>;
    if ((material_flags & FLAG_USE_NORMAL_TEXTURE) != 0u) {
        let tangent_normal = normal_sample * 2.0 - 1.0;
        let T = normalize(in.tangent);
        let B = normalize(in.bitangent);
        let N_base = normalize(in.normal);
        let TBN = mat3x3<f32>(T, B, N_base);
        N = normalize(TBN * tangent_normal);
    } else {
        N = normalize(in.normal);
    }

    var occlusion = 1.0;
    if ((material_flags & FLAG_USE_OCCLUSION_TEXTURE) != 0u) {
        occlusion = occlusion_sample;
    }

    var emissive = vec3<f32>(0.0);
    if ((material_flags & FLAG_USE_EMISSIVE_TEXTURE) != 0u) {
        emissive = emissive_sample * in.material_factors.z;
    }

    // Calculate lighting using imported functions
    let V = normalize(globals.camera_pos - in.world_pos);
    
    // This function is from lighting_with_shadows.wgsl
    let Lo = calculate_scene_lighting(
        in.world_pos, N, V, base_color.rgb, metallic, roughness
    );
    
    // This function is from environment.wgsl
    let environment_light = calculate_environment_lighting(
        N, V, base_color.rgb, metallic, roughness, occlusion
    );
    
    // Conditionally use lighting based on material flags
    var color: vec3<f32>;
    if ((material_flags & FLAG_UNLIT) != 0u) {
        color = base_color.rgb + emissive;
    } else {
        color = environment_light + Lo + emissive;
    }
    
    // Tone mapping
    color = color / (color + vec3<f32>(1.0));
    
    return vec4<f32>(color, base_color.a);
}