// Temporal anti-aliasing and motion blur. Both read the motion vectors the main pass
// packs into the normal G-buffer (zw, in viewport UV units) and run first in the
// post-processing chain, on the HDR scene.

@group(1) @binding(140)
var taa_current : texture_2d<f32>;
@group(1) @binding(141)
var taa_history : texture_2d<f32>;
@group(1) @binding(142)
var taa_motion : texture_2d<f32>;
@group(1) @binding(143)
var taa_position : texture_2d<f32>;
@group(1) @binding(144)
var taa_sampler : sampler;

@group(1) @binding(150)
var blur_color : texture_2d<f32>;
@group(1) @binding(151)
var blur_motion : texture_2d<f32>;
@group(1) @binding(152)
var blur_position : texture_2d<f32>;
@group(1) @binding(153)
var blur_sampler : sampler;

const TEMPORAL_FAR : f32 = 1.0e6;
const TAA_BLEND : f32 = 0.1;
const MOTION_BLUR_SAMPLES : i32 = 12;

fn temporal_view_depth(position : vec4<f32>) -> f32 {
    if position.w < 0.5 {
        return TEMPORAL_FAR;
    }
    return -(post_uniform.view * vec4<f32>(position.xyz, 1.0)).z;
}

fn temporal_safe(color : vec3<f32>) -> vec3<f32> {
    let finite = select(color, vec3<f32>(0.0), color != color);
    return clamp(finite, vec3<f32>(0.0), vec3<f32>(65000.0));
}

// Neighbourhood statistics work on compressed colour so a few very bright pixels do not
// dominate (and flicker); the result is expanded back to HDR.
fn taa_compress(color : vec3<f32>) -> vec3<f32> {
    return color / (1.0 + luminance_rec709(color));
}

fn taa_expand(color : vec3<f32>) -> vec3<f32> {
    return color / max(1.0 - luminance_rec709(color), 1e-4);
}

fn rgb_to_ycocg(c : vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        dot(c, vec3<f32>(0.25, 0.5, 0.25)),
        dot(c, vec3<f32>(0.5, 0.0, -0.5)),
        dot(c, vec3<f32>(-0.25, 0.5, -0.25)),
    );
}

fn ycocg_to_rgb(c : vec3<f32>) -> vec3<f32> {
    let t = c.x - c.z;
    return vec3<f32>(t + c.y, c.x + c.z, t - c.y);
}

// Pull a history colour into the neighbourhood's box (Playdead's clip toward the centre).
fn clip_to_box(box_min : vec3<f32>, box_max : vec3<f32>, history : vec3<f32>) -> vec3<f32> {
    let center = 0.5 * (box_max + box_min);
    let extent = 0.5 * (box_max - box_min) + vec3<f32>(1e-5);
    let offset = history - center;
    let units = abs(offset / extent);
    let largest = max(units.x, max(units.y, units.z));
    if largest > 1.0 {
        return center + offset / largest;
    }
    return history;
}

// Catmull-Rom history fetch in nine bilinear taps: sharper than plain bilinear, so the
// accumulated image does not soften frame after frame.
fn sample_history(uv : vec2<f32>) -> vec3<f32> {
    let dims = vec2<f32>(textureDimensions(taa_history));
    let sample_pos = uv * dims;
    let p1 = floor(sample_pos - 0.5) + 0.5;
    let f = sample_pos - p1;
    let w0 = f * (-0.5 + f * (1.0 - 0.5 * f));
    let w1 = 1.0 + f * f * (-2.5 + 1.5 * f);
    let w2 = f * (0.5 + f * (2.0 - 1.5 * f));
    let w3 = f * f * (-0.5 + 0.5 * f);
    let w12 = w1 + w2;
    let p0 = (p1 - 1.0) / dims;
    let p3 = (p1 + 2.0) / dims;
    let p12 = (p1 + w2 / w12) / dims;
    var color = vec3<f32>(0.0);
    color += textureSampleLevel(taa_history, taa_sampler, vec2<f32>(p0.x, p0.y), 0.0).rgb * w0.x * w0.y;
    color += textureSampleLevel(taa_history, taa_sampler, vec2<f32>(p12.x, p0.y), 0.0).rgb * w12.x * w0.y;
    color += textureSampleLevel(taa_history, taa_sampler, vec2<f32>(p3.x, p0.y), 0.0).rgb * w3.x * w0.y;
    color += textureSampleLevel(taa_history, taa_sampler, vec2<f32>(p0.x, p12.y), 0.0).rgb * w0.x * w12.y;
    color += textureSampleLevel(taa_history, taa_sampler, vec2<f32>(p12.x, p12.y), 0.0).rgb * w12.x * w12.y;
    color += textureSampleLevel(taa_history, taa_sampler, vec2<f32>(p3.x, p12.y), 0.0).rgb * w3.x * w12.y;
    color += textureSampleLevel(taa_history, taa_sampler, vec2<f32>(p0.x, p3.y), 0.0).rgb * w0.x * w3.y;
    color += textureSampleLevel(taa_history, taa_sampler, vec2<f32>(p12.x, p3.y), 0.0).rgb * w12.x * w3.y;
    color += textureSampleLevel(taa_history, taa_sampler, vec2<f32>(p3.x, p3.y), 0.0).rgb * w3.x * w3.y;
    return max(color, vec3<f32>(0.0));
}

fn inside_viewport(uv : vec2<f32>) -> bool {
    let lo = post_uniform.viewport_offset;
    let hi = lo + post_uniform.viewport_scale;
    return all(uv >= lo) && all(uv <= hi);
}

@fragment
fn fs_taa(in : VertexOutput) -> @location(0) vec4<f32> {
    let dims = vec2<i32>(textureDimensions(taa_current));
    let pixel = vec2<i32>(in.position.xy);
    let current = temporal_safe(textureLoad(taa_current, pixel, 0).rgb);

    // Neighbourhood box in YCoCg, and the closest surface's motion (so edges of moving
    // objects reproject with the object, not the background behind them).
    var mean = vec3<f32>(0.0);
    var second = vec3<f32>(0.0);
    var closest = TEMPORAL_FAR * 2.0;
    var closest_pixel = pixel;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let p = clamp(pixel + vec2<i32>(x, y), vec2<i32>(0), dims - vec2<i32>(1));
            let c = rgb_to_ycocg(taa_compress(temporal_safe(textureLoad(taa_current, p, 0).rgb)));
            mean += c;
            second += c * c;
            let depth = temporal_view_depth(textureLoad(taa_position, p, 0));
            if depth < closest {
                closest = depth;
                closest_pixel = p;
            }
        }
    }
    mean = mean / 9.0;
    let sigma = sqrt(max(second / 9.0 - mean * mean, vec3<f32>(0.0)));

    let velocity = textureLoad(taa_motion, closest_pixel, 0).zw;
    let uv = (vec2<f32>(pixel) + 0.5) / vec2<f32>(dims);
    let previous_uv = uv - velocity * post_uniform.viewport_scale;
    if post_uniform.misc.y < 0.5 || !inside_viewport(previous_uv) {
        return vec4<f32>(current, 1.0);
    }

    let history = rgb_to_ycocg(taa_compress(sample_history(previous_uv)));
    let clipped = clip_to_box(mean - sigma, mean + sigma, history);
    let blended = mix(clipped, rgb_to_ycocg(taa_compress(current)), TAA_BLEND);
    return vec4<f32>(temporal_safe(taa_expand(max(ycocg_to_rgb(blended), vec3<f32>(0.0)))), 1.0);
}

@fragment
fn fs_motion_blur(in : VertexOutput) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(blur_color));
    let pixel = vec2<i32>(in.position.xy);
    let center = textureLoad(blur_color, pixel, 0).rgb;
    // Viewport UV per frame, scaled by the shutter, as viewport pixels.
    var motion = textureLoad(blur_motion, pixel, 0).zw * post_uniform.misc.z;
    let motion_px = motion * post_uniform.resolution;
    let length_px = length(motion_px);
    if length_px < 0.5 {
        return vec4<f32>(center, 1.0);
    }
    let max_px = post_uniform.misc.w;
    if length_px > max_px {
        motion = motion * (max_px / length_px);
    }
    let step = motion * post_uniform.viewport_scale;
    let center_depth = temporal_view_depth(textureLoad(blur_position, pixel, 0));
    let streak_px = min(length_px, max_px);
    // Per-pixel offset hides the banding of a fixed sample pattern.
    let jitter = noise_2d(vec2<u32>(pixel), u32(post_uniform.output.z)) - 0.5;

    var sum = center;
    var weight = 1.0;
    for (var i = 0; i < MOTION_BLUR_SAMPLES; i = i + 1) {
        let t = (f32(i) + 0.5 + jitter) / f32(MOTION_BLUR_SAMPLES) - 0.5;
        if abs(t) < 1e-3 {
            continue;
        }
        let uv = clamp(
            in.uv + step * t,
            post_uniform.viewport_offset,
            post_uniform.viewport_offset + post_uniform.viewport_scale,
        );
        let sample_pixel = vec2<i32>(uv * dims);
        let depth = temporal_view_depth(textureLoad(blur_position, sample_pixel, 0));
        // A still foreground in front of this pixel does not smear over it; a moving
        // one only as far as its own streak reaches.
        var w = 1.0;
        if depth < center_depth - 0.05 {
            let own = length(textureLoad(blur_motion, sample_pixel, 0).zw * post_uniform.misc.z * post_uniform.resolution);
            w = clamp(own / max(abs(t) * streak_px, 1e-3), 0.0, 1.0);
        }
        sum += temporal_safe(textureSampleLevel(blur_color, blur_sampler, uv, 0.0).rgb) * w;
        weight += w;
    }
    return vec4<f32>(sum / weight, 1.0);
}
