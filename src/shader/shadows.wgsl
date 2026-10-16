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
// Ray-traced visibility per screen pixel, one bit per light (0-3 directional, 4-7 point,
// 8-11 spot), for the lights whose bit is set in lights.counts.w
// (src/renderer/raytracing/shadows.rs).
@group(2) @binding(11) var rt_shadow_mask: texture_2d<u32>;

// Ray-traced visibility of the light with mask bit `bit` at framebuffer `pixel` (0 or 1),
// or -1 when the frame did not trace it or `pixel` is not a real pixel (x < 0: surfaces the
// mask does not describe, such as transparent ones).
fn rt_shadow_bit(bit: u32, pixel: vec2<f32>) -> f32 {
    if pixel.x < 0.0 || ((lights.counts.w >> bit) & 1u) == 0u {
        return -1.0;
    }
    let dims = vec2<i32>(textureDimensions(rt_shadow_mask));
    let p = clamp(vec2<i32>(pixel), vec2<i32>(0), dims - vec2<i32>(1));
    return f32((textureLoad(rt_shadow_mask, p, 0).r >> bit) & 1u);
}

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

// The maps have a single mip level, so sampling level 0 explicitly gives the same result
// as an implicit-derivative sample while freeing the callers from uniform control flow:
// they skip the taps for lights that cannot reach a fragment instead of sampling anyway.
fn sample_shadow_pcf(
    texture: texture_depth_2d_array,
    smp: sampler_comparison,
    coords: vec2<f32>,
    layer: i32,
    depth: f32,
    texel_size: vec2<f32>,
) -> f32 {
    var result = 0.0;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let offset = vec2<f32>(f32(x), f32(y)) * texel_size;
            result += textureSampleCompareLevel(texture, smp, coords + offset, layer, depth);
        }
    }
    return result / 9.0;
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
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let offset = vec2<f32>(f32(x), f32(y)) * radius;
            acc += textureSampleCompareLevel(texture, smp, proj_xy + offset, layer, depth);
        }
    }
    return acc / 9.0;
}

// Whether a shadow-map projection lands inside the map (and its depth range); outside
// it a surface counts as lit.
fn shadow_projection_valid(proj: vec3<f32>) -> bool {
    return proj.z >= 0.0 && proj.z <= 1.0
        && proj.x >= 0.0 && proj.x <= 1.0 && proj.y >= 0.0 && proj.y <= 1.0;
}

// ============================================================================
// Per-Light Shadow Sampling
// ============================================================================

fn sample_directional_shadow(
    index: u32,
    world_pos: vec3<f32>,
    N: vec3<f32>,
    L: vec3<f32>,
) -> f32 {
    let info = shadow_info.directionals[index];
    if (info.params.x == 0.0) {
        return 1.0;
    }
    let texel = shadow_texel_size(directional_shadow_maps);
    // World size of one shadow-map texel (the light's projection is orthographic: its
    // first row is the x axis scaled by 2 / width).
    let x_axis = vec3<f32>(info.view_proj[0].x, info.view_proj[1].x, info.view_proj[2].x);
    let texel_world = 2.0 * texel.x / max(length(x_axis), 1e-6);
    // Normal offset: look the shadow up a little above the surface, more where light
    // grazes it, so curved low-poly surfaces do not shadow themselves (acne lines along
    // triangle edges and a hard band before the terminator).
    let n_dot_l = clamp(dot(N, L), 0.0, 1.0);
    let offset = texel_world * (1.0 + 2.0 * (1.0 - n_dot_l));
    let proj = project_shadow_with_normal_offset(info.view_proj, world_pos, N, offset);
    if (!shadow_projection_valid(proj)) {
        return 1.0;
    }
    return sample_shadow_pcf(
        directional_shadow_maps,
        directional_shadow_sampler,
        proj.xy,
        i32(index),
        proj.z,
        texel,
    );
}

fn sample_spot_shadow(index: u32, world_pos: vec3<f32>, N: vec3<f32>) -> f32 {
    let info = shadow_info.spots[index];
    // params.x: has_data (!=0)
    // params.y: far (for proper normalization)
    // params.z: receiver normal offset (0..0.004)
    // params.w: pcf_scale (1.5..3.0)
    if (info.params.x == 0.0) {
        return 1.0;
    }
    let receiver_offset = info.params.z;

    // Project into shadow map (with receiver offset)
    let proj = project_shadow_with_normal_offset(info.view_proj, world_pos, N, receiver_offset);
    if (!shadow_projection_valid(proj)) {
        return 1.0;
    }
    let texel = shadow_texel_size(spot_shadow_maps);
    return sample_shadow_pcf(
        spot_shadow_maps,
        spot_shadow_sampler,
        proj.xy,
        i32(index),
        proj.z,
        texel,
    );
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
    if (info.params.x == 0.0) {
        return 1.0;
    }
    let far_plane = max(info.params.y, 0.0001);
    let receiver_offset = info.params.z;
    let pcf_scale = select(2.0, info.params.w, info.params.w > 0.0);

    let proj = project_shadow_with_normal_offset(info.view_proj, world_pos, N, receiver_offset);
    if (!shadow_projection_valid(proj)) {
        return 1.0;
    }

    // Calculate view-space depth along spotlight forward axis
    let to_frag = world_pos - light_pos;
    let Lfwd = normalize(light_dir);
    let view_depth = max(dot(to_frag, Lfwd), 0.0);
    let view_depth_norm = clamp(view_depth / far_plane, 0.0, 1.0);

    let texel = shadow_texel_size(spot_shadow_maps);
    return sample_shadow_pcf_viewdepth(
        spot_shadow_maps,
        spot_shadow_sampler,
        proj.xy,
        i32(index),
        proj.z,
        texel,
        view_depth_norm,
        pcf_scale
    );
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
    return sample_point_shadow_offset(index, world_pos, vec3<f32>(0.0), light_pos, light_range);
}

// The same, looked up a little above the surface (normal `N`): about a texel's footprint
// at the fragment's distance, more where the light grazes, against acne on curved and
// low-poly surfaces (as sample_directional_shadow does).
fn sample_point_shadow_offset(
    index: u32,
    world_pos: vec3<f32>,
    N: vec3<f32>,
    light_pos: vec3<f32>,
    light_range: f32
) -> f32 {
    let info = shadow_info.points[index];
    let to_fragment = world_pos - light_pos;
    let distance = length(to_fragment);
    let in_range = light_range <= 0.0 || distance <= light_range;
    if (info.params.x == 0.0 || distance <= 0.0001 || !in_range) {
        return 1.0;
    }

    let dir = to_fragment / distance;
    let face = select_point_face(dir);
    let matrix = info.view_proj[face];
    // Each face spans 90 degrees: a texel is 2 * distance / size wide at the fragment.
    let texel_world = 2.0 * distance * shadow_texel_size(point_shadow_maps).x;
    let n_dot_l = clamp(dot(N, -dir), 0.0, 1.0);
    let offset = texel_world * (1.0 + 2.0 * (1.0 - n_dot_l));
    let proj = project_shadow_with_normal_offset(matrix, world_pos, N, offset);
    if (!shadow_projection_valid(proj)) {
        return 1.0;
    }
    let layer = i32(index * POINT_SHADOW_FACE_COUNT + face);
    let texel = shadow_texel_size(point_shadow_maps);
    return sample_shadow_pcf(
        point_shadow_maps,
        point_shadow_sampler,
        proj.xy,
        layer,
        proj.z,
        texel,
    );
}