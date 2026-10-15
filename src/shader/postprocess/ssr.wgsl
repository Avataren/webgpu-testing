// Screen-space reflections. A half-resolution pass reflects the view ray off every smooth
// surface and marches it through the G-buffer positions until it passes behind something,
// two separable blur passes turn the result into a glossy sheen, and a full-resolution pass
// adds it to the HDR scene (before TAA), weighted by the surface's smoothness and the
// viewing angle.
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
    let pixel = min(vec2<i32>(in.position.xy * 2.0), dims - vec2<i32>(1));
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

// 9-tap Gaussian, spread by the blur setting (up to 4 half-res texels between taps).
fn ssr_blur(uv : vec2<f32>, direction : vec2<f32>) -> vec4<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(ssr_blur_source));
    let spread = post_uniform.ssr.w * 4.0;
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
    return ssr_blur(in.uv, vec2<f32>(1.0, 0.0));
}

@fragment
fn fs_ssr_blur_vertical(in : VertexOutput) -> @location(0) vec4<f32> {
    return ssr_blur(in.uv, vec2<f32>(0.0, 1.0));
}

// Added to the scene: smoother surfaces and grazing views reflect more (Schlick's
// Fresnel for a dielectric, with a floor so a surface seen head-on still shows a sheen).
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
    let fresnel = 0.04 + 0.96 * pow(1.0 - n_dot_v, 5.0);
    let weight = post_uniform.ssr.x * smoothness * smoothness * (0.2 + 0.8 * fresnel);
    let reflection = textureSampleLevel(ssr_reflection, ssr_apply_sampler, viewport_to_scene_uv(in.uv), 0.0);
    return vec4<f32>(ssr_expand(reflection.rgb) * weight, 0.0);
}
