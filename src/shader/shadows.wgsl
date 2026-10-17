// src/shader/shadows.wgsl
// Shared shadow sampling functions for all pipelines

const POINT_SHADOW_FACE_COUNT: u32 = 6u;

// ============================================================================
// Shadow Structures
// ============================================================================

// One cascade of a directional light's shadow maps (ShadowCascadeRaw in
// src/renderer/lights.rs).
struct ShadowCascade {
    // Light-space x and y to map coordinates: uv = xy * uv_transform.xy + uv_transform.zw.
    uv_transform: vec4<f32>,
    // x, y: light-space z to depth (z * x + y; x < 0, the light shines along -z); z: a
    // texel's width in meters.
    depth_transform: vec4<f32>,
};

// A directional light's cascaded shadow maps (DirectionalShadowRaw): layers params.y on
// of directional_shadow_maps, the nearest cascade first (src/renderer/internal/cascades.rs).
struct DirectionalShadow {
    // World to light space (a rotation).
    light_view: mat4x4<f32>,
    cascades: array<ShadowCascade, MAX_SHADOW_CASCADES>,
    // x: cascades (0: no shadow maps), y: the first one's layer, z: texels across a map,
    // w: tan of half the light's apparent size.
    params: vec4<f32>,
    // The camera's view depth of a point: dot(xyz, point) + w.
    view_depth: vec4<f32>,
    // x: view depth where shadows start to fade out, y: 1 / the fade's length (0: none),
    // z: the widest soft shadow edge in meters.
    fade: vec4<f32>,
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
    // x: directional lights, y: local shadow slots, z: the frame's noise index.
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

// Whether a shadow-map projection lands inside the map (and its depth range); outside
// it a surface counts as lit.
fn shadow_projection_valid(proj: vec3<f32>) -> bool {
    return proj.z >= 0.0 && proj.z <= 1.0
        && proj.x >= 0.0 && proj.x <= 1.0 && proj.y >= 0.0 && proj.y <= 1.0;
}

// ============================================================================
// Directional Light Cascades
// ============================================================================

// Soft shadows from the cascades (percentage-closer soft shadows): taps that look for
// casters around a point, and taps of the filter itself, which is as wide as the light's
// apparent size spread over the casters' distance above the point. Both lie on a Vogel
// disc turned per pixel and frame, which TAA averages.
const SHADOW_BLOCKER_TAPS: u32 = 12u;
const SHADOW_FILTER_TAPS: u32 = 16u;
// The filter's radius in texels: at least this (smooth edges whatever the light's size),
// at most the max (farther apart, the taps would miss thin casters).
const SHADOW_MIN_FILTER_TEXELS: f32 = 1.0;
const SHADOW_MAX_FILTER_TEXELS: f32 = 16.0;
// Toward the edge of its map (this share of the map's width), a cascade hands over to
// the next one.
const SHADOW_CASCADE_BLEND: f32 = 0.06;
// cos and sin of the golden angle, the turn from one Vogel disc tap to the next.
const SHADOW_GOLDEN_TURN: vec2<f32> = vec2<f32>(-0.73736888, 0.67549029);

// Interleaved gradient noise at framebuffer `pixel`, shifted every frame; surfaces with no
// pixel of their own (x < 0: transparent ones, particles) use `fallback`, a position fixed
// to the world.
fn shadow_noise(pixel: vec2<f32>, fallback: vec2<f32>) -> f32 {
    let base = select(fract(fallback / 4096.0) * 4096.0, pixel, pixel.x >= 0.0);
    let p = base + vec2<f32>(5.588238, 3.412) * f32(shadow_info.counts.z);
    return fract(52.9829189 * fract(dot(p, vec2<f32>(0.06711056, 0.00583715))));
}

// How far inside cascade `cascade`'s map light-space point `point` is, with `margin` left
// for the filter (in map coordinates; 0 or less: outside). A point past the map's far
// plane is outside too, though its map coordinates are in range: the casters between
// that plane and the point are not in the map (with a low sun a cascade's square reaches
// far across the ground behind its sphere).
fn cascade_edge(index: u32, cascade: u32, point: vec3<f32>, margin: f32) -> f32 {
    let info = shadow_info.directionals[index].cascades[cascade];
    if (point.z * info.depth_transform.x + info.depth_transform.y >= 1.0) {
        return -1.0;
    }
    let offset = abs(point.xy * info.uv_transform.xy + info.uv_transform.zw - 0.5);
    return 0.5 - margin - max(offset.x, offset.y);
}

// How much of directional light `index` reaches light-space point `point` (normal
// `normal` in light space) through cascade `cascade`: 1 lit, 0 shadowed. `turn` turns the
// filter's taps.
fn sample_cascade(
    index: u32,
    cascade: u32,
    point: vec3<f32>,
    normal: vec3<f32>,
    n_dot_l: f32,
    turn: f32,
) -> f32 {
    let info = shadow_info.directionals[index].cascades[cascade];
    let params = shadow_info.directionals[index].params;
    let texel = info.depth_transform.z;
    // Normal offset: look the shadow up a little above the surface, more where light
    // grazes it, so curved low-poly surfaces do not shadow themselves.
    let p = point + normal * (texel * (1.0 + 2.0 * (1.0 - n_dot_l)));
    let depth = p.z * info.depth_transform.x + info.depth_transform.y;
    if (depth <= 0.0) {
        return 1.0;
    }
    let receiver = min(depth, 1.0);
    let size = params.z;
    let layer = i32(params.y) + i32(cascade);
    let center = (p.xy * info.uv_transform.xy + info.uv_transform.zw) * size;
    // The receiving surface's plane as the light sees it: its depth change per texel of
    // tap offset (map y runs against light-space y), so taps off the centre compare with
    // the surface where they are and a wide filter does not shadow sloped ground.
    let slope = clamp(-normal.xy / max(normal.z, 0.1), vec2<f32>(-4.0), vec2<f32>(4.0));
    let depth_step = vec2<f32>(slope.x, -slope.y) * (texel * info.depth_transform.x);
    let search = clamp(
        shadow_info.directionals[index].fade.z / texel,
        SHADOW_MIN_FILTER_TEXELS,
        SHADOW_MAX_FILTER_TEXELS,
    );

    // Casters around the point: how many, and how far above it they are on average.
    let last = vec2<i32>(i32(size) - 1);
    var direction = vec2<f32>(cos(turn), sin(turn));
    var found = 0u;
    var gap = 0.0;
    for (var i = 0u; i < SHADOW_BLOCKER_TAPS; i = i + 1u) {
        let offset = direction * (sqrt((f32(i) + 0.5) / f32(SHADOW_BLOCKER_TAPS)) * search);
        direction = vec2<f32>(
            direction.x * SHADOW_GOLDEN_TURN.x - direction.y * SHADOW_GOLDEN_TURN.y,
            direction.x * SHADOW_GOLDEN_TURN.y + direction.y * SHADOW_GOLDEN_TURN.x,
        );
        let texel_at = clamp(vec2<i32>(floor(center + offset)), vec2<i32>(0), last);
        let caster = textureLoad(directional_shadow_maps, texel_at, layer, 0);
        // Where the surface would reach past the far plane, empty texels (1) stay lit.
        let surface = min(receiver + dot(offset, depth_step), 1.0);
        if (caster < surface) {
            found = found + 1u;
            gap = gap + (surface - caster);
        }
    }
    if (found == 0u) {
        return 1.0;
    }
    if (found == SHADOW_BLOCKER_TAPS) {
        return 0.0;
    }

    // The penumbra: the light's apparent size over the casters' distance above.
    let meters = gap / f32(found) / max(-info.depth_transform.x, 1e-12);
    let radius = clamp(meters * params.w / texel, SHADOW_MIN_FILTER_TEXELS, search);
    var lit = 0.0;
    direction = vec2<f32>(cos(turn), sin(turn));
    for (var i = 0u; i < SHADOW_FILTER_TAPS; i = i + 1u) {
        let offset = direction * (sqrt((f32(i) + 0.5) / f32(SHADOW_FILTER_TAPS)) * radius);
        direction = vec2<f32>(
            direction.x * SHADOW_GOLDEN_TURN.x - direction.y * SHADOW_GOLDEN_TURN.y,
            direction.x * SHADOW_GOLDEN_TURN.y + direction.y * SHADOW_GOLDEN_TURN.x,
        );
        lit += textureSampleCompareLevel(
            directional_shadow_maps,
            shadow_sampler,
            (center + offset) / size,
            layer,
            min(receiver + dot(offset, depth_step), 1.0),
        );
    }
    return lit / f32(SHADOW_FILTER_TAPS);
}

// ============================================================================
// Per-Light Shadow Sampling
// ============================================================================

// How much of directional light `index` reaches `world_pos` (normal `N`, toward the light
// `L`) past its cascades: 1 lit, 0 shadowed. The nearest cascade whose map holds the point
// serves it, blending into the next toward its map's edge; past the shadow distance the
// shadows fade out. `pixel` (framebuffer, x < 0 for none) turns the soft shadow filter.
// The maps have a single mip level, so the explicit-level samples need no uniform
// control flow: callers skip lights that cannot reach a fragment.
fn sample_directional_shadow(
    index: u32,
    world_pos: vec3<f32>,
    N: vec3<f32>,
    L: vec3<f32>,
    pixel: vec2<f32>,
) -> f32 {
    let params = shadow_info.directionals[index].params;
    let count = min(u32(params.x), MAX_SHADOW_CASCADES);
    if (count == 0u) {
        return 1.0;
    }
    let view_depth = shadow_info.directionals[index].view_depth;
    let fade = shadow_info.directionals[index].fade;
    let faded = clamp((dot(view_depth.xyz, world_pos) + view_depth.w - fade.x) * fade.y, 0.0, 1.0);
    if (faded >= 1.0) {
        return 1.0;
    }
    let light_view = shadow_info.directionals[index].light_view;
    let point = (light_view * vec4<f32>(world_pos, 1.0)).xyz;
    let normal = (light_view * vec4<f32>(N, 0.0)).xyz;
    let n_dot_l = clamp(dot(N, L), 0.0, 1.0);

    // Room for the widest filter and the normal offset.
    let margin = (SHADOW_MAX_FILTER_TEXELS + 4.0) / max(params.z, 1.0);
    var cascade = count;
    var edge = 0.0;
    for (var c = 0u; c < count; c = c + 1u) {
        edge = cascade_edge(index, c, point, margin);
        if (edge > 0.0) {
            cascade = c;
            break;
        }
    }
    if (cascade == count) {
        return 1.0;
    }
    let texel = shadow_info.directionals[index].cascades[cascade].depth_transform.z;
    let turn = TWO_PI * shadow_noise(pixel, point.xy / max(texel, 1e-6));
    var lit = sample_cascade(index, cascade, point, normal, n_dot_l, turn);
    if (edge < SHADOW_CASCADE_BLEND && cascade + 1u < count
        && cascade_edge(index, cascade + 1u, point, margin) > 0.0) {
        let next = sample_cascade(index, cascade + 1u, point, normal, n_dot_l, turn);
        lit = mix(next, lit, edge / SHADOW_CASCADE_BLEND);
    }
    return mix(lit, 1.0, faded);
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
