// Final pass: anti-aliasing, sharpening, film grain and dithering on the tonemapped image,
// written to the surface.

@group(1) @binding(100)
var final_source : texture_2d<f32>;
@group(1) @binding(101)
var final_sampler : sampler;

const FXAA_REDUCE_MIN : f32 = 1.0 / 128.0;
const FXAA_REDUCE_MUL : f32 = 1.0 / 8.0;
const FXAA_SPAN_MAX : f32 = 8.0;

fn sample_display(viewport_uv : vec2<f32>) -> vec3<f32> {
    let scene_uv = viewport_to_scene_uv(clamp(viewport_uv, vec2<f32>(0.0), vec2<f32>(1.0)));
    return textureSampleLevel(final_source, final_sampler, scene_uv, 0.0).rgb;
}

// Edge detection works on perceptual brightness, not linear light.
fn perceptual_luma(color : vec3<f32>) -> f32 {
    return sqrt(max(luminance_rec709(color), 0.0));
}

fn fxaa(uv : vec2<f32>, rgb_m : vec3<f32>) -> vec3<f32> {
    let texel = safe_texel_size();
    let luma_m = perceptual_luma(rgb_m);
    let luma_nw = perceptual_luma(sample_display(uv + texel * vec2<f32>(-1.0, -1.0)));
    let luma_ne = perceptual_luma(sample_display(uv + texel * vec2<f32>(1.0, -1.0)));
    let luma_sw = perceptual_luma(sample_display(uv + texel * vec2<f32>(-1.0, 1.0)));
    let luma_se = perceptual_luma(sample_display(uv + texel * vec2<f32>(1.0, 1.0)));

    let luma_min = min(luma_m, min(min(luma_nw, luma_ne), min(luma_sw, luma_se)));
    let luma_max = max(luma_m, max(max(luma_nw, luma_ne), max(luma_sw, luma_se)));

    var dir = vec2<f32>(
        -((luma_nw + luma_ne) - (luma_sw + luma_se)),
        ((luma_nw + luma_sw) - (luma_ne + luma_se)),
    );
    let dir_reduce = max(
        (luma_nw + luma_ne + luma_sw + luma_se) * (0.25 * FXAA_REDUCE_MUL),
        FXAA_REDUCE_MIN,
    );
    let rcp_dir_min = 1.0 / (min(abs(dir.x), abs(dir.y)) + dir_reduce);
    dir = clamp(dir * rcp_dir_min, vec2<f32>(-FXAA_SPAN_MAX), vec2<f32>(FXAA_SPAN_MAX)) * texel;

    let rgb_a = 0.5 * (sample_display(uv + dir * (1.0 / 3.0 - 0.5))
        + sample_display(uv + dir * (2.0 / 3.0 - 0.5)));
    let rgb_b = rgb_a * 0.5
        + 0.25 * (sample_display(uv + dir * -0.5) + sample_display(uv + dir * 0.5));
    let luma_b = perceptual_luma(rgb_b);
    if luma_b < luma_min || luma_b > luma_max {
        return rgb_a;
    }
    return rgb_b;
}

// Contrast-adaptive sharpening (after AMD FidelityFX CAS): sharpens less where the
// neighbourhood already has strong contrast, so edges do not ring.
fn cas_delta(uv : vec2<f32>, center : vec3<f32>, strength : f32) -> vec3<f32> {
    let texel = safe_texel_size();
    let n = sample_display(uv + vec2<f32>(0.0, -texel.y));
    let s = sample_display(uv + vec2<f32>(0.0, texel.y));
    let w = sample_display(uv + vec2<f32>(-texel.x, 0.0));
    let e = sample_display(uv + vec2<f32>(texel.x, 0.0));
    let min_rgb = min(center, min(min(n, s), min(w, e)));
    let max_rgb = max(center, max(max(n, s), max(w, e)));
    let amp = sqrt(clamp(min(min_rgb, 1.0 - max_rgb) / max(max_rgb, vec3<f32>(1e-5)), vec3<f32>(0.0), vec3<f32>(1.0)));
    let peak = -1.0 / mix(8.0, 5.0, clamp(strength, 0.0, 1.0));
    let weight = amp * peak;
    let sharpened = ((n + s + w + e) * weight + center) / (1.0 + 4.0 * weight);
    return clamp(sharpened, vec3<f32>(0.0), vec3<f32>(1.0)) - center;
}

@fragment
fn fs_final(in : VertexOutput) -> @location(0) vec4<f32> {
    let viewport_uv = clamp(in.uv, vec2<f32>(0.0), vec2<f32>(1.0));
    let center = sample_display(viewport_uv);
    var color = center;
    if post_uniform.effects.z > 0.5 {
        color = fxaa(viewport_uv, center);
    }

    let sharpen = post_uniform.lens.w;
    if sharpen > 0.0 {
        color = color + cas_delta(viewport_uv, center, sharpen);
    }

    let pixel = vec2<u32>(max(in.position.xy, vec2<f32>(0.0)));
    let frame = u32(post_uniform.output.z);

    // Work in display encoding from here: grain and dither are perceptual.
    var encoded = linear_to_srgb(clamp(color, vec3<f32>(0.0), vec3<f32>(1.0)));

    let grain = post_uniform.lens.y;
    if grain > 0.0 {
        let noise = noise_2d(pixel, frame) + noise_2d(pixel + vec2<u32>(7919u, 104729u), frame) - 1.0;
        let luma = luminance_rec709(encoded);
        let response = mix(1.0, 1.0 - luma, post_uniform.lens.z);
        encoded = encoded + vec3<f32>(noise * grain * 0.25 * response);
    }

    // Triangular dither of one 8-bit step hides banding in gradients.
    let dither = post_uniform.output.y;
    if dither > 0.0 {
        let d = noise_2d(pixel, frame + 1u) + noise_2d(pixel + vec2<u32>(3571u, 2903u), frame + 1u) - 1.0;
        encoded = encoded + vec3<f32>(d * dither / 255.0);
    }
    encoded = clamp(encoded, vec3<f32>(0.0), vec3<f32>(1.0));

    if post_uniform.output.x > 0.5 {
        return vec4<f32>(encoded, 1.0);
    }
    // An sRGB surface encodes on write, so hand it linear values.
    return vec4<f32>(srgb_to_linear(encoded), 1.0);
}
