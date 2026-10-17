// src/shader/fog_common.wgsl
// Height fog (src/fog.rs) and the froxel volume the renderer gathers it in near the
// camera (src/renderer/fog.rs): the parameters and the math every pass that fogs shares.
// The including shader declares `fog` (FogParams) at its own binding: fog.wgsl in the
// lights group, fog_integrate.wgsl in its own.
//
// The volume covers the camera's view out to fog.volume.x meters of view depth, in
// slices that grow with depth (slice w in 0-1 starts at depth far * w^2, so near slices
// are thin). Each froxel holds the light the fog scatters toward the camera there and its
// extinction (fog_volume.wgsl), and the integration pass turns each column into the fog
// between the camera and each slice's far end (fog_integrate.wgsl).

struct FogParams {
    // x: 1 while the scene has fog, y: 1 while the volume holds this frame's volumetric
    // fog (else the fog is analytic all the way), z: the frame's noise index, w: the share
    // of last frame's scattering the volume keeps (0: none)
    flags: vec4<f32>,
    // x: extinction at and below the base height (1/m), y: the base height (m), z: 1 / the
    // height over which it falls to 37% above it (0: the same at every height), w: the
    // distance past which fog adds nothing more (m)
    density: vec4<f32>,
    // rgb: albedo, w: anisotropy (Henyey-Greenstein g)
    albedo: vec4<f32>,
    // rgb: emitted radiance, w: how much ambient light the fog scatters
    emission: vec4<f32>,
    // x: the view depth the volume reaches (m), y: its slices, z: how much of the lights'
    // light the fog scatters, w: unused
    volume: vec4<f32>,
    // The volume's froxels along x, y and depth.
    dims: vec4<u32>,
    // The scene camera without TAA jitter, and last frame's (the volume's history).
    view_proj: mat4x4<f32>,
    inverse_view_proj: mat4x4<f32>,
    prev_view_proj: mat4x4<f32>,
    camera_position: vec4<f32>,
    camera_forward: vec4<f32>,
    prev_camera_position: vec4<f32>,
    prev_camera_forward: vec4<f32>,
    // The scene's viewport in framebuffer pixels: x, y, width, height.
    viewport: vec4<f32>,
};

const FOG_INV_FOUR_PI: f32 = 0.0795774715;

fn fog_enabled() -> bool {
    return fog.flags.x > 0.5;
}

fn fog_volumetric() -> bool {
    return fog.flags.y > 0.5;
}

// The fog's extinction at world point `p` (1/m).
fn fog_density_at(p: vec3<f32>) -> f32 {
    let above = p.y - fog.density.y;
    if (above <= 0.0) {
        return fog.density.x;
    }
    return fog.density.x * exp(-above * fog.density.z);
}

// The fog's optical depth along origin + dir * t for t from t0 to t1 (dir a unit vector):
// full density below the base height, falling off exponentially above it, integrated in
// closed form.
fn fog_optical_depth(origin: vec3<f32>, dir: vec3<f32>, t0: f32, t1: f32) -> f32 {
    let density = fog.density.x;
    if (t1 <= t0 || density <= 0.0) {
        return 0.0;
    }
    let k = fog.density.z;
    if (k <= 0.0) {
        return density * (t1 - t0);
    }
    let base = fog.density.y;
    // Heights above the base at both ends.
    let y0 = origin.y + dir.y * t0 - base;
    let y1 = origin.y + dir.y * t1 - base;
    if (y0 <= 0.0 && y1 <= 0.0) {
        return density * (t1 - t0);
    }
    // The part below the base height, and the span [a, b] above it.
    var below = 0.0;
    var a = t0;
    var b = t1;
    if (y0 < 0.0 || y1 < 0.0) {
        let cross = t0 + (t1 - t0) * (y0 / (y0 - y1));
        if (y0 < 0.0) {
            below = cross - t0;
            a = cross;
        } else {
            below = t1 - cross;
            b = cross;
        }
    }
    let ya = max(origin.y + dir.y * a - base, 0.0);
    let yb = max(origin.y + dir.y * b - base, 0.0);
    let span = b - a;
    var above: f32;
    if (abs(k * (yb - ya)) < 1e-4) {
        // Nearly level: the density hardly changes along the span.
        above = exp(-k * 0.5 * (ya + yb)) * span;
    } else {
        above = (exp(-k * ya) - exp(-k * yb)) * span / (k * (yb - ya));
    }
    return density * (below + above);
}

// The share of light scattered from direction `to_light` (unit, toward the light) toward
// the eye along `view_dir` (unit, from the eye), per steradian: Henyey-Greenstein.
fn fog_phase(to_light: vec3<f32>, view_dir: vec3<f32>) -> f32 {
    let g = fog.albedo.w;
    let g2 = g * g;
    let denom = max(1.0 + g2 - 2.0 * g * dot(to_light, view_dir), 1e-4);
    return (1.0 - g2) * FOG_INV_FOUR_PI / (denom * sqrt(denom));
}

// Where slice coordinate `w` (0-1) lies in view depth (m).
fn fog_slice_depth(w: f32) -> f32 {
    return fog.volume.x * w * w;
}

// The slice coordinate of view depth `depth`.
fn fog_depth_slice(depth: f32) -> f32 {
    return sqrt(clamp(depth / fog.volume.x, 0.0, 1.0));
}

// The view depth of world point `p`.
fn fog_view_depth(p: vec3<f32>) -> f32 {
    return dot(p - fog.camera_position.xyz, fog.camera_forward.xyz);
}

// The world point at screen uv `uv` (the scene's viewport, +y down) and view depth
// `depth`, on the unjittered camera's view ray (perspective or orthographic).
fn fog_view_point(uv: vec2<f32>, depth: f32) -> vec3<f32> {
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let near = fog.inverse_view_proj * vec4<f32>(ndc, 0.0, 1.0);
    let mid = fog.inverse_view_proj * vec4<f32>(ndc, 0.5, 1.0);
    let a = near.xyz / near.w;
    let b = mid.xyz / mid.w;
    let da = fog_view_depth(a);
    let db = fog_view_depth(b);
    return a + (b - a) * ((depth - da) / max(db - da, 1e-6));
}

// The unit direction of the unjittered camera's view ray at screen uv `uv` (the forward
// axis for an orthographic camera). Taken from two points far apart along the ray rather
// than from a point near the camera: one a hair's breadth away can round to the camera's
// own position, and its direction then to NaN.
fn fog_view_ray(uv: vec2<f32>) -> vec3<f32> {
    return normalize(fog_view_point(uv, 2.0) - fog_view_point(uv, 1.0));
}

// Where world point `p` lies in the volume, as texture coordinates of the integrated
// volume: uv on screen, and the slice whose far end is `depth` meters deep (each slice of
// the integrated volume holds the fog up to its far end). Kept on texel centres at the
// edges, so filtering never wraps.
fn fog_integrated_uvw(p: vec3<f32>, depth: f32) -> vec3<f32> {
    let clip = fog.view_proj * vec4<f32>(p, 1.0);
    let ndc = clip.xy / max(clip.w, 1e-6);
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    let half_texel = vec3<f32>(0.5) / vec3<f32>(fog.dims.xyz);
    let w = fog_depth_slice(depth) - half_texel.z;
    return clamp(vec3<f32>(uv, w), half_texel, vec3<f32>(1.0) - half_texel);
}

// The share of light from world point `p` that gets through the fog to the camera, given
// `volume`, the integrated volume's transmittance up to `p` (or its far end, past it):
// past the volume (and all the way without one) the height fog's closed form.
fn fog_transmittance_to(p: vec3<f32>, volume: f32) -> f32 {
    if (!fog_enabled()) {
        return 1.0;
    }
    let to_point = p - fog.camera_position.xyz;
    let distance = length(to_point);
    let depth = dot(to_point, fog.camera_forward.xyz);
    var through = 1.0;
    var start = 0.0;
    if (fog_volumetric()) {
        through = volume;
        if (depth <= fog.volume.x) {
            return through;
        }
        start = fog.volume.x * distance / depth;
    }
    let end = min(distance, fog.density.w);
    if (end <= start) {
        return through;
    }
    let dir = to_point / max(distance, 1e-6);
    return through * exp(-fog_optical_depth(fog.camera_position.xyz, dir, start, end));
}
