// The scene may fill only part of the post-processing targets (the editor's game view).
// Bloom steps scale with that part so a glow covers the same share of the view at any
// viewport size, and samples stay inside it so nothing outside bleeds in.
fn bloom_texel(tex_size : vec2<f32>) -> vec2<f32> {
    return post_uniform.viewport_scale / tex_size;
}

fn bloom_clamp_uv(uv : vec2<f32>) -> vec2<f32> {
    let lo = post_uniform.viewport_offset;
    return clamp(uv, lo, lo + post_uniform.viewport_scale);
}

fn bloom_tap(tex : texture_2d<f32>, smp : sampler, uv : vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(tex, smp, bloom_clamp_uv(uv), 0.0).rgb;
}

fn karis_weight(color : vec3<f32>) -> f32 {
    return 1.0 / (1.0 + luminance_rec709(color));
}

// 13-tap downsample from "Next Generation Post Processing in Call of Duty: Advanced
// Warfare" (Jimenez 2014). With `karis` set, each 2x2 block is weighted by its inverse
// brightness so single very bright pixels do not flicker as the camera moves.
fn bloom_downsample13(
    tex : texture_2d<f32>,
    smp : sampler,
    uv : vec2<f32>,
    texel : vec2<f32>,
    karis : bool,
) -> vec3<f32> {
    let a = bloom_tap(tex, smp, uv + texel * vec2<f32>(-2.0, -2.0));
    let b = bloom_tap(tex, smp, uv + texel * vec2<f32>(0.0, -2.0));
    let c = bloom_tap(tex, smp, uv + texel * vec2<f32>(2.0, -2.0));
    let d = bloom_tap(tex, smp, uv + texel * vec2<f32>(-1.0, -1.0));
    let e = bloom_tap(tex, smp, uv + texel * vec2<f32>(1.0, -1.0));
    let f = bloom_tap(tex, smp, uv + texel * vec2<f32>(-2.0, 0.0));
    let g = bloom_tap(tex, smp, uv);
    let h = bloom_tap(tex, smp, uv + texel * vec2<f32>(2.0, 0.0));
    let i = bloom_tap(tex, smp, uv + texel * vec2<f32>(-1.0, 1.0));
    let j = bloom_tap(tex, smp, uv + texel * vec2<f32>(1.0, 1.0));
    let k = bloom_tap(tex, smp, uv + texel * vec2<f32>(-2.0, 2.0));
    let l = bloom_tap(tex, smp, uv + texel * vec2<f32>(0.0, 2.0));
    let m = bloom_tap(tex, smp, uv + texel * vec2<f32>(2.0, 2.0));

    if !karis {
        return g * 0.125
            + (a + c + k + m) * 0.03125
            + (b + f + h + l) * 0.0625
            + (d + e + i + j) * 0.125;
    }

    let g0 = (d + e + i + j) * 0.25;
    let g1 = (a + b + f + g) * 0.25;
    let g2 = (b + c + g + h) * 0.25;
    let g3 = (f + g + k + l) * 0.25;
    let g4 = (g + h + l + m) * 0.25;
    let w0 = karis_weight(g0) * 0.5;
    let w1 = karis_weight(g1) * 0.125;
    let w2 = karis_weight(g2) * 0.125;
    let w3 = karis_weight(g3) * 0.125;
    let w4 = karis_weight(g4) * 0.125;
    return (g0 * w0 + g1 * w1 + g2 * w2 + g3 * w3 + g4 * w4) / max(w0 + w1 + w2 + w3 + w4, 1e-4);
}

// Soft-knee threshold: below `threshold - knee` nothing passes, above `threshold + knee`
// everything above the threshold does, with a quadratic blend in between.
fn bloom_apply_threshold(color : vec3<f32>) -> vec3<f32> {
    let threshold = max(post_uniform.bloom_params.x, 0.0);
    if threshold <= 0.0 {
        return color;
    }
    let knee = max(threshold * post_uniform.bloom_params.y, 1e-4);
    let brightness = max(max(color.r, color.g), color.b);
    let soft = clamp(brightness - threshold + knee, 0.0, 2.0 * knee);
    let soft_curve = soft * soft / (4.0 * knee + 1e-5);
    let contribution = max(soft_curve, brightness - threshold);
    return color * (contribution / max(brightness, 1e-4));
}
