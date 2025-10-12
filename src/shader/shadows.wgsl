// src/shader/shadows.wgsl
// Shared shadow sampling functions for all pipelines

const POINT_SHADOW_FACE_COUNT: u32 = 6u;

// ============================================================================
// Shadow Structures
// ============================================================================

struct DirectionalShadow {
    view_proj: mat4x4<f32>,
    params: vec4<f32>,
    _padding: vec4<f32>,
};

struct PointShadow {
    view_proj: array<mat4x4<f32>, POINT_SHADOW_FACE_COUNT>,
    params: vec4<f32>,
};

struct SpotShadow {
    view_proj: mat4x4<f32>,
    params: vec4<f32>,
};

struct Shadows {
    counts: vec4<u32>,
    directionals: array<DirectionalShadow, 4>,  // MAX_DIRECTIONAL_LIGHTS
    points: array<PointShadow, 4>,              // MAX_POINT_LIGHTS
    spots: array<SpotShadow, 4>,                // MAX_SPOT_LIGHTS
};

// ============================================================================
// Shadow Bindings (group 2)
// ============================================================================

@group(2) @binding(1) var<uniform> shadow_info: Shadows;
@group(2) @binding(2) var directional_shadow_maps: texture_depth_2d_array;
@group(2) @binding(3) var directional_shadow_sampler: sampler_comparison;
@group(2) @binding(4) var spot_shadow_maps: texture_depth_2d_array;
@group(2) @binding(5) var spot_shadow_sampler: sampler_comparison;
@group(2) @binding(6) var point_shadow_maps: texture_depth_2d_array;
@group(2) @binding(7) var point_shadow_sampler: sampler_comparison;

// ============================================================================
// Shadow Projection
// ============================================================================

fn project_shadow(matrix: mat4x4<f32>, world_pos: vec3<f32>) -> vec3<f32> {
    let clip = matrix * vec4<f32>(world_pos, 1.0);
    if (clip.w <= 0.0) {
        return vec3<f32>(-1.0, -1.0, -1.0);
    }
    let ndc = clip.xyz / clip.w;
    // Map clip space to texture coordinates
    // X,Y: [-1,1] -> [0,1], flip Y for texture coords
    // Z: already in [0,1] for wgpu depth
    return vec3<f32>(
        ndc.x * 0.5 + 0.5,
        -ndc.y * 0.5 + 0.5,
        ndc.z,
    );
}

fn project_shadow_with_normal_offset(
    matrix: mat4x4<f32>,
    world_pos: vec3<f32>,
    N: vec3<f32>,
    receiver_offset: f32
) -> vec3<f32> {
    let p = world_pos + N * receiver_offset;
    let clip = matrix * vec4<f32>(p, 1.0);
    if (clip.w <= 0.0) { 
        return vec3<f32>(-1.0, -1.0, -1.0); 
    }
    let ndc = clip.xyz / clip.w;
    return vec3<f32>(
        ndc.x * 0.5 + 0.5, 
        -ndc.y * 0.5 + 0.5, 
        ndc.z
    );
}

// ============================================================================
// PCF Shadow Sampling
// ============================================================================

fn shadow_texel_size(texture: texture_depth_2d_array) -> vec2<f32> {
    let dims = textureDimensions(texture, 0u);
    return vec2<f32>(
        select(0.0, 1.0 / f32(dims.x), dims.x != 0u),
        select(0.0, 1.0 / f32(dims.y), dims.y != 0u),
    );
}

fn sample_shadow_pcf(
    texture: texture_depth_2d_array,
    smp: sampler_comparison,
    coords: vec2<f32>,
    layer: i32,
    depth: f32,
    texel_size: vec2<f32>,
) -> f32 {
    var result = 0.0;
    var samples = 0.0;

    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let offset = vec2<f32>(f32(x), f32(y)) * texel_size;
            result += textureSampleCompare(texture, smp, coords + offset, layer, depth);
            samples = samples + 1.0;
        }
    }

    if (samples > 0.0) {
        return result / samples;
    }
    return result;
}

// PCF with view-depth scaling for spot lights
fn sample_shadow_pcf_viewdepth(
    texture: texture_depth_2d_array,
    smp: sampler_comparison,
    proj_xy: vec2<f32>,
    layer: i32,
    depth: f32,
    base_texel: vec2<f32>,
    view_depth_norm: f32,
    pcf_scale: f32
) -> f32 {
    let radius = base_texel * max(view_depth_norm, 0.0) * pcf_scale;

    var acc = 0.0;
    var samples = 0.0;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let offset = vec2<f32>(f32(x), f32(y)) * radius;
            acc += textureSampleCompare(texture, smp, proj_xy + offset, layer, depth);
            samples = samples + 1.0;
        }
    }
    return acc / max(samples, 1.0);
}

// ============================================================================
// Per-Light Shadow Sampling
// ============================================================================

fn sample_directional_shadow(index: u32, world_pos: vec3<f32>) -> f32 {
    let info = shadow_info.directionals[index];
    let proj = project_shadow(info.view_proj, world_pos);
    let depth = clamp(proj.z, 0.0, 1.0);
    let texel = shadow_texel_size(directional_shadow_maps);
    
    // ALWAYS sample in uniform control flow
    let shadow_sample = sample_shadow_pcf(
        directional_shadow_maps,
        directional_shadow_sampler,
        proj.xy,
        i32(index),
        depth,
        texel,
    );
    
    // Then conditionally return based on validity checks
    let has_shadow_data = info.params.x != 0.0;
    let in_depth_range = proj.z >= 0.0 && proj.z <= 1.0;
    let in_bounds = proj.x >= 0.0 && proj.x <= 1.0 && proj.y >= 0.0 && proj.y <= 1.0;
    let valid = has_shadow_data && in_depth_range && in_bounds;
    
    return select(1.0, shadow_sample, valid);
}

fn sample_spot_shadow(index: u32, world_pos: vec3<f32>, N: vec3<f32>) -> f32 {
    let info = shadow_info.spots[index];
    
    // Import lights structure to get light position/direction
    // Note: This requires lights to be available where this function is used
    
    // params.x: has_data (!=0)
    // params.y: far (for proper normalization)
    // params.z: receiver normal offset (0..0.004)
    // params.w: pcf_scale (1.5..3.0)
    let far_plane = max(info.params.y, 0.0001);
    let receiver_offset = info.params.z;
    let pcf_scale = select(2.0, info.params.w, info.params.w > 0.0);

    // Project into shadow map (with receiver offset)
    let proj = project_shadow_with_normal_offset(info.view_proj, world_pos, N, receiver_offset);
    let depth = clamp(proj.z, 0.0, 1.0);

    // Note: view_depth calculation requires light position from lights structure
    // This will be calculated in the calling context

    // Always sample in uniform control flow
    let texel = shadow_texel_size(spot_shadow_maps);
    let shadow_sample = sample_shadow_pcf(
        spot_shadow_maps,
        spot_shadow_sampler,
        proj.xy,
        i32(index),
        depth,
        texel,
    );

    // Validity checks after sampling
    let has_shadow_data = info.params.x != 0.0;
    let in_depth_range = proj.z >= 0.0 && proj.z <= 1.0;
    let in_bounds = proj.x >= 0.0 && proj.x <= 1.0 && proj.y >= 0.0 && proj.y <= 1.0;
    let valid = has_shadow_data && in_depth_range && in_bounds;

    return select(1.0, shadow_sample, valid);
}

// Advanced spot shadow with depth-scaled PCF
fn sample_spot_shadow_advanced(
    index: u32, 
    world_pos: vec3<f32>, 
    N: vec3<f32>,
    light_pos: vec3<f32>,
    light_dir: vec3<f32>
) -> f32 {
    let info = shadow_info.spots[index];
    
    let far_plane = max(info.params.y, 0.0001);
    let receiver_offset = info.params.z;
    let pcf_scale = select(2.0, info.params.w, info.params.w > 0.0);

    let proj = project_shadow_with_normal_offset(info.view_proj, world_pos, N, receiver_offset);
    let depth = clamp(proj.z, 0.0, 1.0);

    // Calculate view-space depth along spotlight forward axis
    let to_frag = world_pos - light_pos;
    let Lfwd = normalize(light_dir);
    let view_depth = max(dot(to_frag, Lfwd), 0.0);
    let view_depth_norm = clamp(view_depth / far_plane, 0.0, 1.0);

    // Always sample in uniform control flow with depth-scaled PCF
    let texel = shadow_texel_size(spot_shadow_maps);
    let shadow_sample = sample_shadow_pcf_viewdepth(
        spot_shadow_maps,
        spot_shadow_sampler,
        proj.xy,
        i32(index),
        depth,
        texel,
        view_depth_norm,
        pcf_scale
    );

    let has_shadow_data = info.params.x != 0.0;
    let in_depth_range = proj.z >= 0.0 && proj.z <= 1.0;
    let in_bounds = proj.x >= 0.0 && proj.x <= 1.0 && proj.y >= 0.0 && proj.y <= 1.0;
    let valid = has_shadow_data && in_depth_range && in_bounds;

    return select(1.0, shadow_sample, valid);
}

fn select_point_face(direction: vec3<f32>) -> u32 {
    let abs_dir = abs(direction);
    if (abs_dir.x >= abs_dir.y && abs_dir.x >= abs_dir.z) {
        if (direction.x > 0.0) {
            return 0u;
        } else {
            return 1u;
        }
    } else if (abs_dir.y >= abs_dir.z) {
        if (direction.y > 0.0) {
            return 2u;
        } else {
            return 3u;
        }
    } else {
        if (direction.z > 0.0) {
            return 4u;
        } else {
            return 5u;
        }
    }
}

fn sample_point_shadow(
    index: u32, 
    world_pos: vec3<f32>,
    light_pos: vec3<f32>,
    light_range: f32
) -> f32 {
    let info = shadow_info.points[index];
    let to_fragment = world_pos - light_pos;
    let distance = length(to_fragment);
    
    let dir = normalize(to_fragment);
    let face = select_point_face(dir);
    let matrix = info.view_proj[face];
    let proj = project_shadow(matrix, world_pos);
    let layer = i32(index * POINT_SHADOW_FACE_COUNT + face);
    let depth = clamp(proj.z, 0.0, 1.0);
    let texel = shadow_texel_size(point_shadow_maps);
    
    // ALWAYS sample in uniform control flow
    let shadow_sample = sample_shadow_pcf(
        point_shadow_maps,
        point_shadow_sampler,
        proj.xy,
        layer,
        depth,
        texel,
    );
    
    // Then conditionally return based on validity checks
    let has_shadow_data = info.params.x != 0.0;
    let valid_distance = distance > 0.0001;
    let in_range = light_range <= 0.0 || distance <= light_range;
    let in_depth_range = proj.z >= 0.0 && proj.z <= 1.0;
    let in_bounds = proj.x >= 0.0 && proj.x <= 1.0 && proj.y >= 0.0 && proj.y <= 1.0;
    let valid = has_shadow_data && valid_distance && in_range && in_depth_range && in_bounds;
    
    return select(1.0, shadow_sample, valid);
}