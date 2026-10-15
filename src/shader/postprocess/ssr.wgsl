// Screen-space reflections. A half-resolution (or full, with ssr.full_resolution) pass
// reflects the view ray off every smooth
// surface and marches it through the G-buffer positions until it passes behind something,
// a temporal pass averages it over frames, two separable blur passes, as wide as each
// surface is rough, turn the result into a
// glossy sheen, and a full-resolution pass adds it to the HDR scene (before TAA), weighted
// by the surface's smoothness and the viewing angle.
//
// Stable over time: each pixel's ray jitter is fixed (the blur averages the pattern out),
// so a still scene reflects the same way every frame, and reflections are blurred in
// compressed brightness, so one ray that just catches a small, very bright thing (a neon
// rail, a glowing eye) does not flare into a blob that comes and goes. TAA cannot hide
// either kind of flicker: after the blur it is as smooth as a real change.

@group(1) @binding(140)
var ssr_scene : texture_2d<f32>;
@group(1) @binding(141)
var ssr_normal : texture_2d<f32>;
@group(1) @binding(142)
var ssr_position : texture_2d<f32>;
@group(1) @binding(143)
var ssr_sampler : sampler;

@group(1) @binding(150)
var ssr_blur_source : texture_2d<f32>;
@group(1) @binding(151)
var ssr_blur_sampler : sampler;
@group(1) @binding(152)
var ssr_blur_position : texture_2d<f32>;

@group(1) @binding(160)
var ssr_reflection : texture_2d<f32>;
@group(1) @binding(161)
var ssr_apply_sampler : sampler;
@group(1) @binding(162)
var ssr_apply_normal : texture_2d<f32>;
@group(1) @binding(163)
var ssr_apply_position : texture_2d<f32>;

const SSR_STEPS : i32 = 48;
const SSR_REFINE_STEPS : i32 = 5;
const SSR_FAR_AWAY : f32 = 1.0e6;

// How far the ray point lies behind what the screen shows at its pixel, in meters
// (negative in front of it), and that pixel's scene uv.
struct SsrSample {
    behind : f32,
    scene_uv : vec2<f32>,
    pixel : vec2<i32>,
    on_screen : bool,
}

fn ssr_sample(world : vec3<f32>) -> SsrSample {
    var out : SsrSample;
    let projected = ssr_viewport_uv(world);
    out.on_screen = projected.z > 0.5 && ssr_on_viewport(projected.xy);
    out.scene_uv = viewport_to_scene_uv(clamp(projected.xy, vec2<f32>(0.0), vec2<f32>(1.0)));
    let dims = vec2<i32>(textureDimensions(ssr_position));
    let pixel = clamp(vec2<i32>(out.scene_uv * vec2<f32>(dims)), vec2<i32>(0), dims - vec2<i32>(1));
    out.pixel = pixel;
    let surface = textureLoad(ssr_position, pixel, 0);
    var scene_depth = SSR_FAR_AWAY;
    if surface.w >= 0.5 {
        scene_depth = ssr_view_depth(surface.xyz);
    }
    out.behind = ssr_view_depth(world) - scene_depth;
    return out;
}

@fragment
fn fs_ssr_trace(in : VertexOutput) -> @location(0) vec4<f32> {
    let dims = vec2<i32>(textureDimensions(ssr_position));
    let pixel = ssr_source_pixel(in.position.xy, dims);
    let position = textureLoad(ssr_position, pixel, 0);
    let smoothness = ssr_smoothness(position);
    if position.w < 0.5 || smoothness < SSR_MIN_SMOOTHNESS {
        return vec4<f32>(0.0);
    }
    let n = decode_normal_oct(textureLoad(ssr_normal, pixel, 0).xy);
    let origin = position.xyz;
    let v = normalize(origin - post_uniform.camera_position.xyz);
    let r = reflect(v, n);
    // Rays heading straight back at the camera only find the hidden back sides of things.
    // Rays that merely lose view depth are fine: off a floor seen from above, every
    // reflected ray rises toward the camera plane yet hits the fronts of what stands on it.
    // Hits on back sides are rejected below by the hit surface's normal.
    let toward_camera = world_to_view_normal(r).z;
    let facing_fade = 1.0 - smoothstep(0.8, 0.97, toward_camera);
    if facing_fade <= 0.0 {
        return vec4<f32>(0.0);
    }

    let max_distance = post_uniform.ssr.y;
    let thickness = post_uniform.ssr.z;
    let jitter = ssr_jitter(in.position.xy);
    // Start a little off the surface so it does not hit itself.
    let start = origin + n * (0.01 + 0.002 * ssr_view_depth(origin));

    var previous_t = 0.0;
    var hit_t = -1.0;
    for (var i = 0; i < SSR_STEPS; i = i + 1) {
        // Quadratic spacing: fine steps near the surface, where contact matters most.
        let f = (f32(i) + jitter) / f32(SSR_STEPS);
        let t = max_distance * f * f;
        let s = ssr_sample(start + r * t);
        if !s.on_screen {
            break;
        }
        // A step may cross a thin object; allow for the distance it covers.
        let step_length = t - previous_t;
        if s.behind > 0.0 && s.behind < thickness + step_length {
            hit_t = t;
            break;
        }
        previous_t = t;
    }
    if hit_t < 0.0 {
        return vec4<f32>(0.0);
    }

    // Binary search between the last step in front and the first one behind.
    var lo = previous_t;
    var hi = hit_t;
    for (var k = 0; k < SSR_REFINE_STEPS; k = k + 1) {
        let mid = 0.5 * (lo + hi);
        if ssr_sample(start + r * mid).behind > 0.0 {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    let hit = ssr_sample(start + r * hi);
    if hit.behind > thickness + (hit_t - previous_t) {
        return vec4<f32>(0.0);
    }
    // The screen only shows the sides of things that face the camera; a ray that meets a
    // surface from behind passed through it, so what is on screen there is not what it hit.
    let hit_normal = decode_normal_oct(textureLoad(ssr_normal, hit.pixel, 0).xy);
    let front_fade = clamp(-dot(hit_normal, r) * 4.0, 0.0, 1.0);

    var color = textureSampleLevel(ssr_scene, ssr_sampler, hit.scene_uv, 0.0).rgb;
    color = select(color, vec3<f32>(0.0), color != color);
    color = min(color, vec3<f32>(SSR_MAX_RADIANCE));

    // Fade out toward the edges of the view and the end of the ray instead of cutting off.
    let viewport_uv = scene_to_viewport_uv(hit.scene_uv);
    let edge = min(min(viewport_uv.x, 1.0 - viewport_uv.x), min(viewport_uv.y, 1.0 - viewport_uv.y));
    let edge_fade = clamp(edge / 0.08, 0.0, 1.0);
    let distance_fade = 1.0 - smoothstep(0.6, 1.0, hi / max_distance);
    let weight = facing_fade * front_fade * edge_fade * distance_fade;
    return vec4<f32>(ssr_compress(color) * weight, weight);
}

@group(1) @binding(155)
var ssr_temporal_current : texture_2d<f32>;
@group(1) @binding(156)
var ssr_temporal_history : texture_2d<f32>;
@group(1) @binding(157)
var ssr_temporal_motion : texture_2d<f32>;
@group(1) @binding(158)
var ssr_temporal_sampler : sampler;

// Share of each new trace in the accumulated reflection (about the last eight frames).
const SSR_TEMPORAL_BLEND : f32 = 0.125;

// Temporal accumulation of the trace, before the blur. TAA jitters the
// G-buffer by a fraction of a pixel every frame, and on curved or detailed mirrors that
// sends a pixel's reflected ray to very different things from one frame to the next (a
// neon rail, then the dark wall beside it). TAA's tight neighbourhood clip cannot tell
// that from motion and lets it through as shimmer. Averaging here first hands TAA a steady
// reflection. The history follows the surface's motion and is kept inside the range the
// current trace shows around the pixel, so moving reflections do not smear.
@fragment
fn fs_ssr_temporal(in : VertexOutput) -> @location(0) vec4<f32> {
    let dims = vec2<i32>(textureDimensions(ssr_temporal_current));
    let pixel = vec2<i32>(in.position.xy);
    let current = textureLoad(ssr_temporal_current, pixel, 0);
    var lo = current;
    var hi = current;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let p = clamp(pixel + vec2<i32>(x, y), vec2<i32>(0), dims - vec2<i32>(1));
            let c = textureLoad(ssr_temporal_current, p, 0);
            lo = min(lo, c);
            hi = max(hi, c);
        }
    }

    // The full-resolution pixel the trace started from (see fs_ssr_trace).
    let full_dims = vec2<i32>(textureDimensions(ssr_temporal_motion));
    let source = ssr_source_pixel(in.position.xy, full_dims);
    let motion = textureLoad(ssr_temporal_motion, source, 0).zw;
    let previous_uv = in.uv - motion * post_uniform.viewport_scale;
    let lo_uv = post_uniform.viewport_offset;
    let hi_uv = lo_uv + post_uniform.viewport_scale;
    if any(previous_uv < lo_uv) || any(previous_uv > hi_uv) {
        return current;
    }
    let history = textureSampleLevel(ssr_temporal_history, ssr_temporal_sampler, previous_uv, 0.0);
    return mix(clamp(history, lo, hi), current, SSR_TEMPORAL_BLEND);
}

// 9-tap Gaussian, as wide as the surface is rough: a smooth floor keeps a crisp mirror
// image, a rough one a glossy sheen. The blur setting scales it (at 1 a fully rough surface
// spreads 16 screen pixels between taps). `pixel` is the reflection texel's position.
fn ssr_blur(uv : vec2<f32>, pixel : vec2<f32>, direction : vec2<f32>) -> vec4<f32> {
    let dims = vec2<i32>(textureDimensions(ssr_blur_position));
    let surface = textureLoad(ssr_blur_position, ssr_source_pixel(pixel, dims), 0);
    let roughness = 1.0 - ssr_smoothness(surface);
    let texel = 1.0 / vec2<f32>(textureDimensions(ssr_blur_source));
    let spread = post_uniform.ssr.w * 16.0 * roughness / ssr_texel_scale();
    let tap_step = direction * texel * spread;
    var weights = array<f32, 5>(0.2270270, 0.1945946, 0.1216216, 0.0540541, 0.0162162);
    var sum = textureSampleLevel(ssr_blur_source, ssr_blur_sampler, uv, 0.0) * weights[0];
    for (var i = 1; i < 5; i = i + 1) {
        let offset = tap_step * f32(i);
        sum = sum + textureSampleLevel(ssr_blur_source, ssr_blur_sampler, uv + offset, 0.0) * weights[i];
        sum = sum + textureSampleLevel(ssr_blur_source, ssr_blur_sampler, uv - offset, 0.0) * weights[i];
    }
    return sum;
}

@fragment
fn fs_ssr_blur_horizontal(in : VertexOutput) -> @location(0) vec4<f32> {
    return ssr_blur(in.uv, in.position.xy, vec2<f32>(1.0, 0.0));
}

@fragment
fn fs_ssr_blur_vertical(in : VertexOutput) -> @location(0) vec4<f32> {
    return ssr_blur(in.uv, in.position.xy, vec2<f32>(0.0, 1.0));
}

// Added to the scene (taking the ambient a metal no longer needs off): smoother surfaces
// and grazing views reflect more (Schlick's Fresnel from the surface's specular colour, so
// metals reflect fully and in their own colour; dielectrics get a floor so a surface seen
// head-on still shows a sheen).
@fragment
fn fs_ssr_apply(in : VertexOutput) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(in.position.xy);
    let position = textureLoad(ssr_apply_position, pixel, 0);
    let smoothness = ssr_smoothness(position);
    if position.w < 0.5 || smoothness < SSR_MIN_SMOOTHNESS {
        return vec4<f32>(0.0);
    }
    let n = decode_normal_oct(textureLoad(ssr_apply_normal, pixel, 0).xy);
    let v = normalize(post_uniform.camera_position.xyz - position.xyz);
    let n_dot_v = clamp(dot(n, v), 0.0, 1.0);
    let f0 = ssr_specular_color(position);
    let fresnel = f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - n_dot_v, 5.0);
    let metalness = clamp((max(f0.r, max(f0.g, f0.b)) - 0.04) / 0.5, 0.0, 1.0);
    let sheen = 0.2 * (1.0 - metalness);
    let weight = post_uniform.ssr.x * smoothness * smoothness * (sheen + (1.0 - sheen) * fresnel);
    let reflection = textureSampleLevel(ssr_reflection, ssr_apply_sampler, viewport_to_scene_uv(in.uv), 0.0);
    // The main pass lit a metal with the flat ambient light too, its stand-in for the
    // surroundings the metal reflects (ambient times base colour, which is F0). Where a
    // reflection was found (alpha: the share of rays that hit) it shows those surroundings
    // instead, so that ambient comes off; otherwise mirrors look hazy. Dielectrics keep
    // theirs: on them it stands for diffuse light, not a reflection.
    let found = clamp(reflection.a, 0.0, 1.0) * min(post_uniform.ssr.x, 1.0) * smoothness * smoothness;
    let replaced = post_uniform.reflection.yzw * f0 * metalness * found;
    return vec4<f32>(ssr_expand(reflection.rgb) * weight - replaced, 0.0);
}
