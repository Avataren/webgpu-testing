// Path tracing: what the trace, denoise and composite passes share. No bindings: each pass
// binds `PtUniform` where its layout has it.
//
// The path tracer writes lighting demodulated (divided by what the surface's own colour
// does to it): diffuse light as it arrives, before the base colour multiplies it, and
// specular light divided by the specular albedo. Texture detail is then not noise for the
// denoiser to blur away; the composite multiplies it back in (pt_composite.wgsl).

// Mirrors `PtUniform` in src/renderer/postprocess/path_tracing.rs (432 bytes).
struct PtUniform {
    // Unjittered, this frame and last.
    view_proj : mat4x4<f32>,
    prev_view_proj : mat4x4<f32>,
    prev_view_proj_inv : mat4x4<f32>,
    view : mat4x4<f32>,
    prev_view : mat4x4<f32>,
    camera_position : vec4<f32>,
    prev_camera_position : vec4<f32>,
    // The scene region's origin in the scene targets (xy) and the path tracing targets'
    // size (zw), in pixels: the region's size, or a pixel per `scale.x` squared of it.
    region : vec4<f32>,
    // frame index, bounces, samples per pixel, PT_FLAG_*
    frame : vec4<u32>,
    // most frames diffuse light accumulates, most frames specular light accumulates,
    // directional lights' angular radius (radians), lamp radius (m)
    params : vec4<f32>,
    // world size of a target pixel: per meter of view depth (perspective) and constant
    // (orthographic); output pixel angle for texture levels; unused
    footprint : vec4<f32>,
    // scene pixels per target pixel along each axis (1, or 2 for half-resolution global
    // illumination), the scene region's size in pixels (yz), unused
    scale : vec4<f32>,
};

// PtUniform.frame.w
const PT_FLAG_HISTORY_VALID : u32 = 1u;
const PT_FLAG_DENOISE : u32 = 2u;
// Caustic photons carry the light through glass, so shadow rays stop at it.
const PT_FLAG_CAUSTICS : u32 = 4u;
// Global illumination: only the indirect diffuse light is traced, and added to the main
// pass's lighting instead of replacing it.
const PT_FLAG_GI : u32 = 8u;

// Hit distance written for rays that leave the scene.
const PT_MISS_DISTANCE : f32 = 1000.0;
// Accumulated frames below which the history is too short to trust on its own: it is
// blurred spatially (history fix) and its variance estimated from neighbours.
const PT_SHORT_HISTORY : f32 = 4.0;
// Frames the fast history (for anti-lag clamping) averages over.
const PT_FAST_FRAMES : f32 = 4.0;

fn pt_luminance(color : vec3<f32>) -> f32 {
    return dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// The main pass's octahedral normal encoding (encode_normal_oct in common.wgsl).
fn pt_encode_normal(n : vec3<f32>) -> vec2<f32> {
    let p = n.xy / max(abs(n.x) + abs(n.y) + abs(n.z), 1e-6);
    if n.z >= 0.0 {
        return p;
    }
    return (vec2<f32>(1.0) - abs(p.yx)) * select(vec2<f32>(-1.0), vec2<f32>(1.0), p >= vec2<f32>(0.0));
}

fn pt_decode_normal(e : vec2<f32>) -> vec3<f32> {
    var n = vec3<f32>(e, 1.0 - abs(e.x) - abs(e.y));
    let t = max(-n.z, 0.0);
    n.x = n.x + select(t, -t, n.x >= 0.0);
    n.y = n.y + select(t, -t, n.y >= 0.0);
    return normalize(n);
}

// How much of the light arriving over the hemisphere a GGX surface reflects specularly
// (the split-sum environment BRDF; Karis 2014's analytic fit). The trace divides specular
// light by it and the composite multiplies it back, so both must agree exactly.
fn pt_specular_albedo(f0 : vec3<f32>, n_dot_v : f32, roughness : f32) -> vec3<f32> {
    let c0 = vec4<f32>(-1.0, -0.0275, -0.572, 0.022);
    let c1 = vec4<f32>(1.0, 0.0425, 1.04, -0.04);
    let r = roughness * c0 + c1;
    let a004 = min(r.x * r.x, exp2(-9.28 * clamp(n_dot_v, 0.0, 1.0))) * r.x + r.y;
    let ab = vec2<f32>(-1.04, 1.04) * a004 + r.zw;
    return max(f0 * ab.x + ab.y, vec3<f32>(0.0));
}

// The scene pixel a target pixel stands for (the top left one of its block).
fn pt_scene_pixel(p : vec2<i32>) -> vec2<i32> {
    return p * i32(pt.scale.x) + vec2<i32>(pt.region.xy);
}

fn pt_view_depth(world : vec3<f32>) -> f32 {
    return -(pt.view * vec4<f32>(world, 1.0)).z;
}

fn pt_prev_view_depth(world : vec3<f32>) -> f32 {
    return -(pt.prev_view * vec4<f32>(world, 1.0)).z;
}

// World size of one scene pixel at view depth `depth`.
fn pt_footprint(depth : f32) -> f32 {
    return max(depth * pt.footprint.x + pt.footprint.y, 1e-5);
}

// Where `world` was on screen last frame, in region pixels; z is 0 behind the camera.
fn pt_prev_pixel(world : vec3<f32>) -> vec3<f32> {
    let clip = pt.prev_view_proj * vec4<f32>(world, 1.0);
    if clip.w <= 1e-5 {
        return vec3<f32>(-1.0, -1.0, 0.0);
    }
    let ndc = clip.xy / clip.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    return vec3<f32>(uv * pt.region.zw, 1.0);
}

// Last frame's view ray through region pixel position `pixel` (origin, direction).
fn pt_prev_ray(pixel : vec2<f32>) -> array<vec3<f32>, 2> {
    let uv = pixel / max(pt.region.zw, vec2<f32>(1.0));
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let near = pt.prev_view_proj_inv * vec4<f32>(ndc, 0.25, 1.0);
    let far = pt.prev_view_proj_inv * vec4<f32>(ndc, 0.75, 1.0);
    let a = near.xyz / near.w;
    let b = far.xyz / far.w;
    return array<vec3<f32>, 2>(a, normalize(b - a));
}
