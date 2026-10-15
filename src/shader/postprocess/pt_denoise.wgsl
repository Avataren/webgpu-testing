// The path tracer's denoiser: a spatiotemporal filter after NVIDIA's ReLAX (Zhdan,
// "ReBLUR/ReLAX", NRD; itself a descendant of SVGF, Schied et al. 2017), run on the
// demodulated diffuse and specular light separately. Every pass works on the scene
// region; the path tracing targets are its size (pixel (0, 0) is the region's corner), or
// half of it for half-resolution global illumination, whose target pixels each stand for
// the top-left scene pixel of their 2x2 block (pt_scene_pixel).
//
// 1. Temporal accumulation (cs_pt_temporal): each pixel's history is reprojected with
//    the G-buffer's motion vectors, bilinearly, from the history taps that show the same
//    surface (normal and depth tests, so disocclusions start over). Specular light on
//    smooth surfaces is reprojected where its reflection was instead (virtual motion: the
//    point the reflection ray hit, seen through the surface), so mirrors do not smear.
//    The new sample is blended in with weight 1 / frames, up to a cap (shorter for
//    smooth specular). Isolated samples far brighter than their neighbours and history
//    (fireflies) are clamped first. Luminance moments give the variance, and a fast
//    history (a few frames) tracks changing light.
// 2. History fix and anti-lag (cs_pt_history_fix): pixels with a short history are
//    blurred wider from their neighbours, weighted by how long theirs is; long histories
//    are clamped into the fast history's neighbourhood range, so moving lights and
//    opened doors do not leave trails. This becomes next frame's history.
// 3. Spatial filter (cs_pt_atrous, 5 iterations): an edge-avoiding a-trous wavelet
//    (B3 spline, steps 1 to 16 pixels) weighted by normals, plane-aware depth, roughness
//    and luminance against the local variance (which shrinks as the history converges).
//    Each pixel's filter radius is limited by its rays' hit distance: diffuse light by
//    how far away its occluders are (contact shadows stay sharp), specular light by the
//    lobe's footprint at the reflection's distance (mirrors are not blurred at all).

@group(0) @binding(0)
var<uniform> pt : PtUniform;

// Variance needs luminance squared; this keeps it inside a half float.
const PT_MOMENT_LUMINANCE_MAX : f32 = 200.0;
// A sample may be at most this much brighter than its brightest neighbour or history.
const PT_FIREFLY_RATIO : f32 = 8.0;

// Geometry targets (geometry, prev geometry): octahedral normal (xy), view depth (z),
// roughness (w; 0 where nothing is path traced).
fn pt_geometry_valid(geometry : vec4<f32>) -> bool {
    return geometry.w > 0.0;
}

fn pt_in_region(p : vec2<i32>) -> bool {
    return all(p >= vec2<i32>(0)) && all(p < vec2<i32>(pt.region.zw));
}

// The direction from a surface to the camera.
fn pt_view_direction(world : vec3<f32>) -> vec3<f32> {
    if pt.camera_position.w > 0.5 {
        return normalize(pt.camera_position.xyz - world);
    }
    return normalize(vec3<f32>(pt.view[0].z, pt.view[1].z, pt.view[2].z));
}

// How much view depth may change per pixel across a surface at this angle, in meters.
fn pt_depth_slope(depth : f32, n : vec3<f32>, v : vec3<f32>) -> f32 {
    let c = clamp(abs(dot(n, v)), 0.1, 1.0);
    return pt_footprint(depth) * sqrt(1.0 - c * c) / c;
}

// ---------------------------------------------------------------------------------------
// 1. Temporal accumulation
// ---------------------------------------------------------------------------------------

@group(1) @binding(0)
var ta_diffuse : texture_2d<f32>;
@group(1) @binding(1)
var ta_specular : texture_2d<f32>;
@group(1) @binding(2)
var ta_material : texture_2d<f32>;
@group(1) @binding(3)
var ta_gbuffer_normal : texture_2d<f32>;
@group(1) @binding(4)
var ta_gbuffer_position : texture_2d<f32>;
@group(1) @binding(5)
var ta_prev_diffuse : texture_2d<f32>;
@group(1) @binding(6)
var ta_prev_specular : texture_2d<f32>;
@group(1) @binding(7)
var ta_prev_moments : texture_2d<f32>;
@group(1) @binding(8)
var ta_prev_geometry : texture_2d<f32>;
@group(1) @binding(9)
var ta_prev_hit : texture_2d<f32>;
@group(1) @binding(10)
var ta_out_diffuse : texture_storage_2d<rgba16float, write>;
@group(1) @binding(11)
var ta_out_specular : texture_storage_2d<rgba16float, write>;
@group(1) @binding(12)
var ta_out_moments : texture_storage_2d<rgba16float, write>;
@group(1) @binding(13)
var ta_out_geometry : texture_storage_2d<rgba16float, write>;
@group(1) @binding(14)
var ta_out_hit : texture_storage_2d<rgba16float, write>;

// Reprojected history: diffuse and specular light (rgb) with their frame counts (a),
// moments (diffuse m2, specular m2, diffuse fast, specular fast) and hit distances.
struct PtHistory {
    diffuse : vec4<f32>,
    specular : vec4<f32>,
    moments : vec4<f32>,
    hit : vec4<f32>,
    weight : f32,
};

fn pt_history_add(sum : ptr<function, PtHistory>, q : vec2<i32>, w : f32) {
    (*sum).diffuse += textureLoad(ta_prev_diffuse, q, 0) * w;
    (*sum).specular += textureLoad(ta_prev_specular, q, 0) * w;
    (*sum).moments += textureLoad(ta_prev_moments, q, 0) * w;
    (*sum).hit += textureLoad(ta_prev_hit, q, 0) * w;
    (*sum).weight += w;
}

fn pt_history_normalize(sum : PtHistory) -> PtHistory {
    var out = sum;
    if sum.weight > 1e-3 {
        let inv = 1.0 / sum.weight;
        out.diffuse *= inv;
        out.specular *= inv;
        out.moments *= inv;
        out.hit *= inv;
    }
    return out;
}

// Last frame's history at region position `prev` (pixel centres at +0.5), from the
// bilinear taps that show this surface: same orientation, and a view depth within
// tolerance of `expected` (where this point was, as seen from last frame's camera) or of
// `depth` (where it is now: objects moving toward the camera).
fn pt_reproject_surface(
    prev : vec2<f32>,
    n : vec3<f32>,
    expected : f32,
    depth : f32,
    tolerance : f32,
) -> PtHistory {
    var sum : PtHistory;
    let base = floor(prev - vec2<f32>(0.5));
    let f = prev - vec2<f32>(0.5) - base;
    for (var i = 0; i < 4; i = i + 1) {
        let offset = vec2<i32>(i & 1, i >> 1u);
        let q = vec2<i32>(base) + offset;
        if !pt_in_region(q) {
            continue;
        }
        let geometry = textureLoad(ta_prev_geometry, q, 0);
        if !pt_geometry_valid(geometry) || dot(n, pt_decode_normal(geometry.xy)) < 0.9 {
            continue;
        }
        if abs(geometry.z - expected) > tolerance && abs(geometry.z - depth) > tolerance {
            continue;
        }
        let fw = select(vec2<f32>(1.0) - f, f, offset == vec2<i32>(1));
        let w = fw.x * fw.y;
        if w > 0.0 {
            pt_history_add(&sum, q, w);
        }
    }
    return pt_history_normalize(sum);
}

// The same for a reflection's virtual position: the taps must lie on this surface's plane
// (a static mirror), checked by intersecting last frame's view ray through each with it.
fn pt_reproject_virtual(prev : vec2<f32>, n : vec3<f32>, world : vec3<f32>, depth : f32) -> PtHistory {
    var sum : PtHistory;
    let base = floor(prev - vec2<f32>(0.5));
    let f = prev - vec2<f32>(0.5) - base;
    for (var i = 0; i < 4; i = i + 1) {
        let offset = vec2<i32>(i & 1, i >> 1u);
        let q = vec2<i32>(base) + offset;
        if !pt_in_region(q) {
            continue;
        }
        let geometry = textureLoad(ta_prev_geometry, q, 0);
        if !pt_geometry_valid(geometry) || dot(n, pt_decode_normal(geometry.xy)) < 0.95 {
            continue;
        }
        let ray = pt_prev_ray(vec2<f32>(q) + vec2<f32>(0.5));
        let facing = dot(n, ray[1]);
        if abs(facing) < 1e-3 {
            continue;
        }
        let on_plane = ray[0] + ray[1] * (dot(n, world - ray[0]) / facing);
        let expected = pt_prev_view_depth(on_plane);
        if abs(geometry.z - expected) > 0.01 * depth + 0.01 {
            continue;
        }
        let fw = select(vec2<f32>(1.0) - f, f, offset == vec2<i32>(1));
        let w = fw.x * fw.y;
        if w > 0.0 {
            pt_history_add(&sum, q, w);
        }
    }
    return pt_history_normalize(sum);
}

// `sample` with its luminance limited to `limit` (colour kept).
fn pt_limit_luminance(sample : vec3<f32>, limit : f32) -> vec3<f32> {
    let l = pt_luminance(sample);
    if l > limit && l > 0.0 {
        return sample * (limit / l);
    }
    return sample;
}

@compute @workgroup_size(8, 8, 1)
fn cs_pt_temporal(@builtin(global_invocation_id) id : vec3<u32>) {
    let p = vec2<i32>(id.xy);
    if !pt_in_region(p) {
        return;
    }
    let g = pt_scene_pixel(p);
    let material = textureLoad(ta_material, p, 0);
    let surface = textureLoad(ta_gbuffer_position, g, 0);
    if material.a <= 0.0 || surface.w < 0.5 {
        textureStore(ta_out_diffuse, p, vec4<f32>(0.0));
        textureStore(ta_out_specular, p, vec4<f32>(0.0));
        textureStore(ta_out_moments, p, vec4<f32>(0.0));
        textureStore(ta_out_geometry, p, vec4<f32>(0.0));
        textureStore(ta_out_hit, p, vec4<f32>(0.0));
        return;
    }
    let normal_motion = textureLoad(ta_gbuffer_normal, g, 0);
    let n = pt_decode_normal(normal_motion.xy);
    let world = surface.xyz;
    let depth = max(pt_view_depth(world), 1e-3);
    let roughness = material.a;
    textureStore(ta_out_geometry, p, vec4<f32>(normal_motion.xy, depth, roughness));

    var diffuse = textureLoad(ta_diffuse, p, 0);
    var specular = textureLoad(ta_specular, p, 0);

    // Reproject.
    let v = pt_view_direction(world);
    let history_valid = (pt.frame.w & PT_FLAG_HISTORY_VALID) != 0u;
    var surface_history : PtHistory;
    var virtual_history : PtHistory;
    var virtual_amount = 0.0;
    let prev = vec2<f32>(p) + vec2<f32>(0.5) - normal_motion.zw * pt.region.zw;
    if history_valid {
        let expected = pt_prev_view_depth(world);
        let tolerance = 0.02 * depth + 2.0 * pt_depth_slope(depth, n, v);
        surface_history = pt_reproject_surface(prev, n, expected, depth, tolerance);

        // Smooth specular: where the reflected point was, for a static surface (its
        // motion is the camera's alone) seen in perspective.
        let smooth_amount = 1.0 - smoothstep(0.15, 0.5, roughness);
        let static_prev = pt_prev_pixel(world);
        if smooth_amount > 0.0 && pt.camera_position.w > 0.5 && static_prev.z > 0.5
            && distance(static_prev.xy, prev) < 1.0 {
            // As an offset from where the motion vector puts the surface: both then read
            // the history at the same sub-pixel phase (the G-buffer's jitter is in neither).
            let reflected = world - v * min(specular.a, PT_MISS_DISTANCE);
            let virtual_prev = pt_prev_pixel(reflected);
            if virtual_prev.z > 0.5 {
                let virtual_position = prev + virtual_prev.xy - static_prev.xy;
                virtual_history = pt_reproject_virtual(virtual_position, n, world, depth);
                if virtual_history.weight > 0.5 {
                    virtual_amount = smooth_amount;
                }
            }
        }
    }

    // Firefly suppression: against the brightest neighbour and the history.
    var neighbour_diffuse = 0.0;
    var neighbour_specular = 0.0;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let q = p + vec2<i32>(x, y);
            if (x == 0 && y == 0) || !pt_in_region(q) || textureLoad(ta_material, q, 0).a <= 0.0 {
                continue;
            }
            neighbour_diffuse = max(neighbour_diffuse, pt_luminance(textureLoad(ta_diffuse, q, 0).rgb));
            neighbour_specular = max(neighbour_specular, pt_luminance(textureLoad(ta_specular, q, 0).rgb));
        }
    }
    let history_diffuse_luminance = select(0.0, pt_luminance(surface_history.diffuse.rgb), surface_history.weight > 1e-3);
    var history_specular_luminance = select(0.0, pt_luminance(surface_history.specular.rgb), surface_history.weight > 1e-3);
    if virtual_amount > 0.0 {
        history_specular_luminance = max(history_specular_luminance, pt_luminance(virtual_history.specular.rgb));
    }
    diffuse = vec4<f32>(
        pt_limit_luminance(diffuse.rgb, PT_FIREFLY_RATIO * max(neighbour_diffuse, history_diffuse_luminance) + 1e-3),
        diffuse.a,
    );
    specular = vec4<f32>(
        pt_limit_luminance(specular.rgb, PT_FIREFLY_RATIO * max(neighbour_specular, history_specular_luminance) + 1e-3),
        specular.a,
    );

    // Diffuse.
    var diffuse_frames = 1.0;
    var history = surface_history;
    if history.weight > 1e-3 {
        diffuse_frames = min(history.diffuse.a + 1.0, pt.params.x);
    }
    let diffuse_alpha = 1.0 / diffuse_frames;
    let diffuse_luminance = min(pt_luminance(diffuse.rgb), PT_MOMENT_LUMINANCE_MAX);
    let out_diffuse = mix(history.diffuse.rgb, diffuse.rgb, diffuse_alpha);
    let diffuse_m2 = mix(history.moments.x, diffuse_luminance * diffuse_luminance, diffuse_alpha);
    let diffuse_fast = mix(history.moments.z, diffuse_luminance, max(diffuse_alpha, 1.0 / PT_FAST_FRAMES));
    let diffuse_hit = mix(history.hit.x, diffuse.a, max(diffuse_alpha, 0.125));

    // Specular: the surface's history, or the reflection's on smooth static surfaces.
    var specular_history = surface_history;
    if virtual_amount > 0.0 {
        if surface_history.weight > 1e-3 {
            specular_history.specular = mix(surface_history.specular, virtual_history.specular, virtual_amount);
            specular_history.moments = mix(surface_history.moments, virtual_history.moments, virtual_amount);
            specular_history.hit = mix(surface_history.hit, virtual_history.hit, virtual_amount);
        } else {
            specular_history = virtual_history;
        }
    }
    // Smooth reflections seen through surface motion move against it when the camera
    // moves: accumulate them only briefly then. Rough ones, well reprojected smooth ones,
    // and any seen without parallax (a still or turning camera: the reflection depends on
    // where the eye is, not where it looks) accumulate fully.
    let parallax = select(
        length(normalize(world - pt.camera_position.xyz) - normalize(world - pt.prev_camera_position.xyz)),
        0.0,
        pt.camera_position.w < 0.5,
    );
    let still = 1.0 - smoothstep(0.0, max(roughness * roughness, 0.002) * 2.0, parallax);
    let specular_cap = mix(
        min(4.0, pt.params.y),
        pt.params.y,
        clamp(smoothstep(0.1, 0.45, roughness) + virtual_amount + still, 0.0, 1.0),
    );
    var specular_frames = 1.0;
    if specular_history.weight > 1e-3 {
        specular_frames = min(specular_history.specular.a + 1.0, specular_cap);
    }
    let specular_alpha = 1.0 / specular_frames;
    let specular_luminance = min(pt_luminance(specular.rgb), PT_MOMENT_LUMINANCE_MAX);
    let out_specular = mix(specular_history.specular.rgb, specular.rgb, specular_alpha);
    let specular_m2 = mix(specular_history.moments.y, specular_luminance * specular_luminance, specular_alpha);
    let specular_fast = mix(specular_history.moments.w, specular_luminance, max(specular_alpha, 1.0 / PT_FAST_FRAMES));
    let specular_hit = mix(specular_history.hit.y, specular.a, max(specular_alpha, 0.125));

    textureStore(ta_out_diffuse, p, vec4<f32>(out_diffuse, diffuse_frames));
    textureStore(ta_out_specular, p, vec4<f32>(out_specular, specular_frames));
    textureStore(ta_out_moments, p, vec4<f32>(diffuse_m2, specular_m2, diffuse_fast, specular_fast));
    textureStore(ta_out_hit, p, vec4<f32>(diffuse_hit, specular_hit, 0.0, 0.0));
}

// ---------------------------------------------------------------------------------------
// 2. History fix and anti-lag
// ---------------------------------------------------------------------------------------

@group(1) @binding(20)
var hf_diffuse : texture_2d<f32>;
@group(1) @binding(21)
var hf_specular : texture_2d<f32>;
@group(1) @binding(22)
var hf_moments : texture_2d<f32>;
@group(1) @binding(23)
var hf_geometry : texture_2d<f32>;
@group(1) @binding(24)
var hf_out_diffuse : texture_storage_2d<rgba16float, write>;
@group(1) @binding(25)
var hf_out_specular : texture_storage_2d<rgba16float, write>;

// How far the slow history may stray from the fast history's neighbourhood mean, in
// standard deviations.
const PT_ANTILAG_SIGMA : f32 = 2.0;

// `slow` with its luminance clamped into the fast history's range: the neighbourhood's
// mean +- sigma, widened to the pixel's own fast value (a thin bright detail is far from
// its neighbourhood's mean, but its own fast history agrees with it; changed light moves
// that too, within a few frames).
fn pt_antilag(slow : vec3<f32>, mean : f32, sigma : f32, own : f32) -> vec3<f32> {
    let l = pt_luminance(slow);
    let low = min(max(mean - PT_ANTILAG_SIGMA * sigma, 0.0), own);
    let high = max(mean + PT_ANTILAG_SIGMA * sigma, own) + 1e-4;
    let clamped = clamp(l, low, high);
    if l > 1e-6 {
        return slow * (clamped / l);
    }
    return vec3<f32>(clamped);
}

@compute @workgroup_size(8, 8, 1)
fn cs_pt_history_fix(@builtin(global_invocation_id) id : vec3<u32>) {
    let p = vec2<i32>(id.xy);
    if !pt_in_region(p) {
        return;
    }
    let geometry = textureLoad(hf_geometry, p, 0);
    var diffuse = textureLoad(hf_diffuse, p, 0);
    var specular = textureLoad(hf_specular, p, 0);
    if !pt_geometry_valid(geometry) {
        textureStore(hf_out_diffuse, p, vec4<f32>(0.0));
        textureStore(hf_out_specular, p, vec4<f32>(0.0));
        return;
    }
    let n = pt_decode_normal(geometry.xy);
    let depth = geometry.z;
    let roughness = geometry.w;

    // Anti-lag: the fast history's 5x5 neighbourhood.
    if diffuse.a > PT_FAST_FRAMES || specular.a > PT_FAST_FRAMES {
        var sum = vec2<f32>(0.0);
        var sum_sq = vec2<f32>(0.0);
        var count = 0.0;
        for (var y = -2; y <= 2; y = y + 1) {
            for (var x = -2; x <= 2; x = x + 1) {
                let q = p + vec2<i32>(x, y);
                if !pt_in_region(q) || !pt_geometry_valid(textureLoad(hf_geometry, q, 0)) {
                    continue;
                }
                let fast = textureLoad(hf_moments, q, 0).zw;
                sum += fast;
                sum_sq += fast * fast;
                count += 1.0;
            }
        }
        let mean = sum / count;
        let sigma = sqrt(max(sum_sq / count - mean * mean, vec2<f32>(0.0)));
        let own = textureLoad(hf_moments, p, 0).zw;
        if diffuse.a > PT_FAST_FRAMES {
            diffuse = vec4<f32>(pt_antilag(diffuse.rgb, mean.x, sigma.x, own.x), diffuse.a);
        }
        if specular.a > PT_FAST_FRAMES {
            specular = vec4<f32>(pt_antilag(specular.rgb, mean.y, sigma.y, own.y), specular.a);
        }
    }

    // History fix: blur a short history from neighbours, wider the shorter it is, giving
    // more weight to neighbours with longer histories.
    let fix_diffuse = diffuse.a < PT_SHORT_HISTORY;
    let fix_specular = specular.a < PT_SHORT_HISTORY && roughness > 0.1;
    if fix_diffuse || fix_specular {
        let shortest = min(select(PT_SHORT_HISTORY, diffuse.a, fix_diffuse), select(PT_SHORT_HISTORY, specular.a, fix_specular));
        let stride = i32(PT_SHORT_HISTORY - shortest) + 1;
        var diffuse_sum = vec4<f32>(diffuse.rgb, 1.0) * diffuse.a;
        var specular_sum = vec4<f32>(specular.rgb, 1.0) * specular.a;
        let slope = pt_footprint(depth) * 4.0;
        for (var y = -2; y <= 2; y = y + 1) {
            for (var x = -2; x <= 2; x = x + 1) {
                if x == 0 && y == 0 {
                    continue;
                }
                let q = p + vec2<i32>(x, y) * stride;
                if !pt_in_region(q) {
                    continue;
                }
                let qg = textureLoad(hf_geometry, q, 0);
                if !pt_geometry_valid(qg) {
                    continue;
                }
                let distance_px = length(vec2<f32>(f32(x), f32(y))) * f32(stride);
                let w_normal = pow(max(dot(n, pt_decode_normal(qg.xy)), 0.0), 8.0);
                let w_depth = exp(-abs(qg.z - depth) / (0.02 * depth + slope * distance_px));
                let w = w_normal * w_depth;
                if w <= 1e-4 {
                    continue;
                }
                if fix_diffuse {
                    let qd = textureLoad(hf_diffuse, q, 0);
                    diffuse_sum += vec4<f32>(qd.rgb, 1.0) * (w * max(qd.a, 0.5));
                }
                if fix_specular {
                    let qs = textureLoad(hf_specular, q, 0);
                    let w_roughness = exp(-abs(qg.w - roughness) * 10.0);
                    specular_sum += vec4<f32>(qs.rgb, 1.0) * (w * w_roughness * max(qs.a, 0.5));
                }
            }
        }
        if fix_diffuse && diffuse_sum.w > 0.0 {
            diffuse = vec4<f32>(diffuse_sum.rgb / diffuse_sum.w, diffuse.a);
        }
        if fix_specular && specular_sum.w > 0.0 {
            specular = vec4<f32>(specular_sum.rgb / specular_sum.w, specular.a);
        }
    }
    textureStore(hf_out_diffuse, p, diffuse);
    textureStore(hf_out_specular, p, specular);
}

// ---------------------------------------------------------------------------------------
// 3. Spatial filter
// ---------------------------------------------------------------------------------------

// The iteration's step, in pixels (1, 2, 4, ...), and its index.
struct PtAtrousStep {
    step : i32,
    iteration : u32,
    _padding : vec2<u32>,
};

@group(1) @binding(40)
var at_diffuse : texture_2d<f32>;
@group(1) @binding(41)
var at_specular : texture_2d<f32>;
@group(1) @binding(42)
var at_moments : texture_2d<f32>;
@group(1) @binding(43)
var at_geometry : texture_2d<f32>;
@group(1) @binding(44)
var at_hit : texture_2d<f32>;
@group(1) @binding(45)
var at_gbuffer_position : texture_2d<f32>;
@group(1) @binding(46)
var<uniform> at_step : PtAtrousStep;
@group(1) @binding(47)
var at_out_diffuse : texture_storage_2d<rgba16float, write>;
@group(1) @binding(48)
var at_out_specular : texture_storage_2d<rgba16float, write>;

// Luminance edge-stopping strength, in standard deviations (SVGF's sigma_l).
const PT_LUMINANCE_SIGMA : f32 = 4.0;
// Smallest filter radius any pixel keeps, in pixels.
const PT_MIN_RADIUS : f32 = 1.0;

// Iteration 0 reads the history (alpha: frames), later ones the previous iteration
// (alpha: variance). A pixel's luminance variance there.
fn pt_input_variance(value : vec4<f32>, m2 : f32) -> f32 {
    if at_step.iteration == 0u {
        let l = min(pt_luminance(value.rgb), PT_MOMENT_LUMINANCE_MAX);
        return max(m2 - l * l, 0.0);
    }
    return value.a;
}

@compute @workgroup_size(8, 8, 1)
fn cs_pt_atrous(@builtin(global_invocation_id) id : vec3<u32>) {
    let p = vec2<i32>(id.xy);
    if !pt_in_region(p) {
        return;
    }
    let geometry = textureLoad(at_geometry, p, 0);
    let center_diffuse = textureLoad(at_diffuse, p, 0);
    let center_specular = textureLoad(at_specular, p, 0);
    if !pt_geometry_valid(geometry) {
        textureStore(at_out_diffuse, p, vec4<f32>(0.0));
        textureStore(at_out_specular, p, vec4<f32>(0.0));
        return;
    }
    let n = pt_decode_normal(geometry.xy);
    let depth = geometry.z;
    let roughness = geometry.w;
    let world = textureLoad(at_gbuffer_position, pt_scene_pixel(p), 0).xyz;
    let v = pt_view_direction(world);
    let slope = pt_depth_slope(depth, n, v);
    let footprint = pt_footprint(depth);
    let first = at_step.iteration == 0u;

    // Variance at the centre: from the moments (or, with a short history, the 3x3
    // neighbourhood) in the first iteration; prefiltered 3x3 later.
    var diffuse_variance = 0.0;
    var specular_variance = 0.0;
    if first {
        let moments = textureLoad(at_moments, p, 0);
        diffuse_variance = pt_input_variance(center_diffuse, moments.x);
        specular_variance = pt_input_variance(center_specular, moments.y);
        if center_diffuse.a < PT_SHORT_HISTORY || center_specular.a < PT_SHORT_HISTORY {
            var sum = vec2<f32>(0.0);
            var sum_sq = vec2<f32>(0.0);
            var count = 0.0;
            for (var y = -1; y <= 1; y = y + 1) {
                for (var x = -1; x <= 1; x = x + 1) {
                    let q = p + vec2<i32>(x, y);
                    if !pt_in_region(q) || !pt_geometry_valid(textureLoad(at_geometry, q, 0)) {
                        continue;
                    }
                    let l = min(vec2<f32>(
                        pt_luminance(textureLoad(at_diffuse, q, 0).rgb),
                        pt_luminance(textureLoad(at_specular, q, 0).rgb),
                    ), vec2<f32>(PT_MOMENT_LUMINANCE_MAX));
                    sum += l;
                    sum_sq += l * l;
                    count += 1.0;
                }
            }
            let mean = sum / count;
            let spatial = max(sum_sq / count - mean * mean, vec2<f32>(0.0));
            if center_diffuse.a < PT_SHORT_HISTORY {
                diffuse_variance = max(diffuse_variance, spatial.x * (PT_SHORT_HISTORY / max(center_diffuse.a, 1.0)));
            }
            if center_specular.a < PT_SHORT_HISTORY {
                specular_variance = max(specular_variance, spatial.y * (PT_SHORT_HISTORY / max(center_specular.a, 1.0)));
            }
        }
    } else {
        var sum = vec2<f32>(0.0);
        var weight = 0.0;
        for (var y = -1; y <= 1; y = y + 1) {
            for (var x = -1; x <= 1; x = x + 1) {
                let q = p + vec2<i32>(x, y);
                if !pt_in_region(q) || !pt_geometry_valid(textureLoad(at_geometry, q, 0)) {
                    continue;
                }
                let w = select(0.5, 1.0, x == 0) * select(0.5, 1.0, y == 0);
                sum += vec2<f32>(textureLoad(at_diffuse, q, 0).a, textureLoad(at_specular, q, 0).a) * w;
                weight += w;
            }
        }
        let filtered = sum / max(weight, 1e-6);
        diffuse_variance = filtered.x;
        specular_variance = filtered.y;
    }

    // Each signal's radius, from its rays' hit distance: diffuse light changes over
    // about the distance to what occludes it; a specular lobe spreads over its width at
    // the reflection's distance (GGX alpha is roughly the lobe's half-angle tangent).
    let hit = textureLoad(at_hit, p, 0);
    let alpha = roughness * roughness;
    let diffuse_radius = max(hit.x / footprint, PT_MIN_RADIUS);
    let specular_radius = max(hit.y * alpha * 1.5 / footprint, select(0.0, PT_MIN_RADIUS, roughness > 0.2));
    let diffuse_luminance = pt_luminance(center_diffuse.rgb);
    let specular_luminance = pt_luminance(center_specular.rgb);
    let diffuse_sigma = PT_LUMINANCE_SIGMA * sqrt(diffuse_variance) + 1e-4;
    let specular_sigma = PT_LUMINANCE_SIGMA * sqrt(specular_variance) + 1e-4;
    let specular_normal_power = mix(512.0, 32.0, clamp(roughness * 1.5, 0.0, 1.0));

    var kernel = array<f32, 3>(0.375, 0.25, 0.0625);
    var diffuse_sum = center_diffuse.rgb * 0.140625;
    var specular_sum = center_specular.rgb * 0.140625;
    var diffuse_weight = 0.140625;
    var specular_weight = 0.140625;
    var diffuse_variance_sum = diffuse_variance * 0.140625 * 0.140625;
    var specular_variance_sum = specular_variance * 0.140625 * 0.140625;
    let step = at_step.step;
    for (var y = -2; y <= 2; y = y + 1) {
        for (var x = -2; x <= 2; x = x + 1) {
            if x == 0 && y == 0 {
                continue;
            }
            let q = p + vec2<i32>(x, y) * step;
            if !pt_in_region(q) {
                continue;
            }
            let qg = textureLoad(at_geometry, q, 0);
            if !pt_geometry_valid(qg) {
                continue;
            }
            let h = kernel[abs(x)] * kernel[abs(y)];
            let distance_px = length(vec2<f32>(f32(x), f32(y))) * f32(step);
            let qn = pt_decode_normal(qg.xy);
            let cos_normal = max(dot(n, qn), 0.0);
            let w_depth = exp(-abs(qg.z - depth) / (0.01 * depth + 2.0 * slope * distance_px));
            let geometry_weight = h * w_depth;
            if geometry_weight <= 1e-5 {
                continue;
            }
            let qd = textureLoad(at_diffuse, q, 0);
            let qs = textureLoad(at_specular, q, 0);
            var qm = vec4<f32>(0.0);
            if first {
                qm = textureLoad(at_moments, q, 0);
            }

            let radius_d = distance_px / diffuse_radius;
            let wd = geometry_weight
                * pow(cos_normal, 64.0)
                * exp(-abs(pt_luminance(qd.rgb) - diffuse_luminance) / diffuse_sigma)
                * exp(-radius_d * radius_d);
            if wd > 1e-6 {
                diffuse_sum += qd.rgb * wd;
                diffuse_weight += wd;
                diffuse_variance_sum += pt_input_variance(qd, qm.x) * wd * wd;
            }

            let radius_s = distance_px / max(specular_radius, 1e-3);
            let ws = geometry_weight
                * pow(cos_normal, specular_normal_power)
                * exp(-abs(qg.w - roughness) * 20.0)
                * exp(-abs(pt_luminance(qs.rgb) - specular_luminance) / specular_sigma)
                * exp(-radius_s * radius_s);
            if ws > 1e-6 {
                specular_sum += qs.rgb * ws;
                specular_weight += ws;
                specular_variance_sum += pt_input_variance(qs, qm.y) * ws * ws;
            }
        }
    }
    textureStore(at_out_diffuse, p, vec4<f32>(
        diffuse_sum / diffuse_weight,
        diffuse_variance_sum / (diffuse_weight * diffuse_weight),
    ));
    textureStore(at_out_specular, p, vec4<f32>(
        specular_sum / specular_weight,
        specular_variance_sum / (specular_weight * specular_weight),
    ));
}
