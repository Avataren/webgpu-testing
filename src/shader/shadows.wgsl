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

// A point or spot light's shadow maps: tiles of the shadow atlas (LocalShadowRaw in
// src/renderer/lights.rs).
struct LocalShadow {
    // Per face: a point light's look along +X, -X, +Y, -Y, +Z, -Z; a spot light has one.
    view_proj: array<mat4x4<f32>, POINT_SHADOW_FACE_COUNT>,
    // Each face's tile in the atlas, in texture coordinates: x, y, width, height.
    rects: array<vec4<f32>, POINT_SHADOW_FACE_COUNT>,
    // x: faces, y: texels across a tile, z: tan(fov / 2) of a face.
    params: vec4<f32>,
};

struct Shadows {
    counts: vec4<u32>,
    directionals: array<DirectionalShadow, MAX_DIRECTIONAL_LIGHTS>,
    locals: array<LocalShadow, MAX_SHADOWED_LOCAL_LIGHTS>,
};

// ============================================================================
// Shadow Bindings (group 2)
// ============================================================================

@group(2) @binding(1) var<uniform> shadow_info: Shadows;
@group(2) @binding(2) var directional_shadow_maps: texture_depth_2d_array;
@group(2) @binding(3) var shadow_sampler: sampler_comparison;
// Every point and spot light's shadow maps, in tiles (src/renderer/internal/shadows.rs).
@group(2) @binding(4) var local_shadow_atlas: texture_depth_2d;
// Ray-traced visibility per screen pixel, one bit per light (0-3 directional, 4-31 the
// point and spot lights given a bit, LocalLight.params.z), for the lights whose bit is
// set in lights.counts.w (src/renderer/raytracing/shadows.rs).
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
        shadow_sampler,
        proj.xy,
        i32(index),
        proj.z,
        texel,
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

// How much of a point or spot light reaches `world_pos` (normal `N`) past its shadow
// map: 1 lit, 0 shadowed, 1 for a light without one. The lookup is a little above the
// surface (about a texel's footprint at the fragment's distance, more where the light
// grazes), against acne on curved and low-poly surfaces, and the 3x3 filter's taps stay
// inside the face's tile (each face is rendered with a border for that).
fn sample_local_shadow(light: LocalLight, world_pos: vec3<f32>, N: vec3<f32>) -> f32 {
    let slot = i32(light.params.y);
    if (slot < 0) {
        return 1.0;
    }
    let info = shadow_info.locals[slot];
    let to_fragment = world_pos - light.position_range.xyz;
    let distance = length(to_fragment);
    if (distance <= 0.0001 || distance > light.position_range.w) {
        return 1.0;
    }
    let dir = to_fragment / distance;
    var face = 0u;
    var depth = dot(to_fragment, normalize(light.direction.xyz));
    if (info.params.x > 1.5) {
        face = select_point_face(dir);
        depth = distance;
    }
    let texels = max(info.params.y, 1.0);
    let texel_world = 2.0 * max(depth, 0.0) * info.params.z / texels;
    let n_dot_l = clamp(dot(N, -dir), 0.0, 1.0);
    let offset = texel_world * (1.0 + 2.0 * (1.0 - n_dot_l));
    let proj = project_shadow_with_normal_offset(info.view_proj[face], world_pos, N, offset);
    if (!shadow_projection_valid(proj)) {
        return 1.0;
    }
    let rect = info.rects[face];
    let texel = rect.zw / texels;
    let lo = rect.xy + texel * 0.5;
    let hi = rect.xy + rect.zw - texel * 0.5;
    let center = rect.xy + proj.xy * rect.zw;
    var result = 0.0;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let uv = clamp(center + vec2<f32>(f32(x), f32(y)) * texel, lo, hi);
            result += textureSampleCompareLevel(local_shadow_atlas, shadow_sampler, uv, proj.z);
        }
    }
    return result / 9.0;
}
