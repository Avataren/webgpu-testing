// HDR composite: scene * AO + bloom, exposure, vignette, colour correction and
// tonemapping. Writes display-referred linear colour (0-1) to the intermediate target the
// final pass anti-aliases, sharpens and outputs.

@group(1) @binding(90)
var composite_scene : texture_2d<f32>;
@group(1) @binding(91)
var composite_ssao : texture_2d<f32>;
@group(1) @binding(92)
var composite_bloom : texture_2d<f32>;
@group(1) @binding(93)
var composite_sampler : sampler;
@group(1) @binding(94)
var<storage, read> composite_exposure : ExposureState;
@group(1) @binding(95)
var composite_lut : texture_3d<f32>;
@group(1) @binding(96)
var composite_dof : texture_2d<f32>;

fn sample_hdr(scene_uv : vec2<f32>) -> vec3<f32> {
    let uv = clamp(scene_uv, vec2<f32>(0.0), vec2<f32>(1.0));
    var color = textureSampleLevel(composite_scene, composite_sampler, uv, 0.0).rgb;
    // Broken materials can produce NaN or inf; keep them from spreading.
    color = select(color, vec3<f32>(0.0), color != color);
    color = min(color, vec3<f32>(65000.0));
    if post_uniform.effects.x > 0.5 {
        let dims = vec2<f32>(textureDimensions(composite_ssao));
        let ssao_uv = display_to_scene_uv(uv, dims);
        color = color * textureSampleLevel(composite_ssao, composite_sampler, ssao_uv, 0.0).r;
    }
    if post_uniform.dof_params.x > 0.5 {
        // Blend toward the blurred image once the blur passes about a pixel.
        let blurred = textureSampleLevel(composite_dof, composite_sampler, uv, 0.0);
        color = mix(color, blurred.rgb, clamp(blurred.a - 1.0, 0.0, 1.0));
    }
    if post_uniform.effects.y > 0.5 {
        let bloom = textureSampleLevel(composite_bloom, composite_sampler, uv, 0.0).rgb;
        color = color + bloom * post_uniform.bloom_tint.rgb * post_uniform.bloom_params.w;
    }
    return color;
}

fn sample_with_aberration(viewport_uv : vec2<f32>) -> vec3<f32> {
    let amount = post_uniform.lens.x;
    let center = sample_hdr(viewport_to_scene_uv(viewport_uv));
    if amount <= 0.0 {
        return center;
    }
    // Red is pushed outward and blue inward, growing toward the edges.
    let from_center = viewport_uv - vec2<f32>(0.5);
    // About 1% of the frame width at the corners at full intensity.
    let offset = from_center * dot(from_center, from_center) * amount * 0.02;
    let red = sample_hdr(viewport_to_scene_uv(clamp(viewport_uv - offset, vec2<f32>(0.0), vec2<f32>(1.0)))).r;
    let blue = sample_hdr(viewport_to_scene_uv(clamp(viewport_uv + offset, vec2<f32>(0.0), vec2<f32>(1.0)))).b;
    return vec3<f32>(red, center.g, blue);
}

fn exposure_multiplier() -> f32 {
    var exposure = post_uniform.exposure_params.x * post_uniform.exposure_params.y;
    if post_uniform.effects.w > 0.5 && composite_exposure.initialized > 0.5 {
        // Map the adapted average luminance to middle grey.
        exposure = exposure * 0.18 / exp2(composite_exposure.average_ev);
    }
    return exposure;
}

fn apply_vignette(color : vec3<f32>, viewport_uv : vec2<f32>) -> vec3<f32> {
    let intensity = post_uniform.vignette.x;
    if intensity <= 0.0 {
        return color;
    }
    let smoothness = max(post_uniform.vignette.y, 0.01);
    let roundness = post_uniform.vignette.z;
    let aspect = post_uniform.vignette.w;
    // Same shape as Unity's post-processing vignette.
    var d = abs(viewport_uv - post_uniform.vignette_center.xy) * intensity * 3.0;
    // "Rounded" keeps it a circle on screen instead of following the frame's shape.
    if post_uniform.vignette_center.z > 0.5 {
        d.x = d.x * aspect;
    }
    // Roundness 1 gives round corners, 0 square ones.
    d = pow(clamp(d, vec2<f32>(0.0), vec2<f32>(1.0)), vec2<f32>((1.0 - roundness) * 6.0 + roundness));
    let factor = pow(clamp(1.0 - dot(d, d), 0.0, 1.0), smoothness * 5.0);
    return color * mix(post_uniform.vignette_color.rgb, vec3<f32>(1.0), factor);
}

const LINEAR_TO_LMS = mat3x3<f32>(
    vec3<f32>(3.90405e-1, 7.08416e-2, 2.31082e-2),
    vec3<f32>(5.49941e-1, 9.63172e-1, 1.28021e-1),
    vec3<f32>(8.92632e-3, 1.35775e-3, 9.36245e-1),
);
const LMS_TO_LINEAR = mat3x3<f32>(
    vec3<f32>(2.85847e+0, -2.10182e-1, -4.18120e-2),
    vec3<f32>(-1.62879e+0, 1.15820e+0, -1.18169e-1),
    vec3<f32>(-2.48910e-2, 3.24281e-4, 1.06867e+0),
);

fn rotate_hue(color : vec3<f32>, turns : f32) -> vec3<f32> {
    // Rodrigues rotation around the grey axis keeps luminance roughly constant.
    let angle = turns * 6.28318530718;
    let k = vec3<f32>(0.57735026919);
    let cos_a = cos(angle);
    return color * cos_a + cross(k, color) * sin(angle) + k * dot(k, color) * (1.0 - cos_a);
}

// Colour correction in scene-referred (HDR) linear space, before tonemapping.
fn grade_hdr(input : vec3<f32>) -> vec3<f32> {
    var color = input;

    let wb = post_uniform.white_balance.xyz;
    if any(abs(wb - vec3<f32>(1.0)) > vec3<f32>(1e-4)) {
        color = LMS_TO_LINEAR * ((LINEAR_TO_LMS * color) * wb);
    }

    color = color * post_uniform.color_filter.rgb;

    let hue_turns = post_uniform.color_filter.w;
    if abs(hue_turns) > 1e-5 {
        color = max(rotate_hue(color, hue_turns), vec3<f32>(0.0));
    }

    // Contrast pivots on middle grey in log2 space, so it behaves the same at any
    // exposure and never pushes values negative.
    let contrast = post_uniform.grading.x;
    if abs(contrast - 1.0) > 1e-4 {
        let middle_grey = log2(0.18);
        let log_color = log2(max(color, vec3<f32>(1e-6)));
        color = exp2((log_color - middle_grey) * contrast + middle_grey);
    }

    let luma = luminance_rec709(color);
    let vibrance = post_uniform.grading.z;
    if abs(vibrance) > 1e-4 {
        let max_c = max(color.r, max(color.g, color.b));
        let min_c = min(color.r, min(color.g, color.b));
        let current = (max_c - min_c) / max(max_c, 1e-5);
        color = mix(vec3<f32>(luma), color, 1.0 + vibrance * (1.0 - current));
    }

    color = mix(vec3<f32>(luma), color, post_uniform.grading.y);
    return max(color, vec3<f32>(0.0));
}

fn tonemap_reinhard(color : vec3<f32>) -> vec3<f32> {
    let luma = luminance_rec709(color);
    return color / (1.0 + luma);
}

// ACES filmic fit by Stephen Hill (BakingLab).
const ACES_INPUT = mat3x3<f32>(
    vec3<f32>(0.59719, 0.07600, 0.02840),
    vec3<f32>(0.35458, 0.90834, 0.13383),
    vec3<f32>(0.04823, 0.01566, 0.83777),
);
const ACES_OUTPUT = mat3x3<f32>(
    vec3<f32>(1.60475, -0.10208, -0.00327),
    vec3<f32>(-0.53108, 1.10813, -0.07276),
    vec3<f32>(-0.07367, -0.00605, 1.07602),
);

fn tonemap_aces(color : vec3<f32>) -> vec3<f32> {
    let v = ACES_INPUT * color;
    let a = v * (v + 0.0245786) - 0.000090537;
    let b = v * (0.983729 * v + 0.4329510) + 0.238081;
    return ACES_OUTPUT * (a / b);
}

// AgX (Troy Sobotka), minimal fit by Benjamin Wrensch.
const AGX_INSET = mat3x3<f32>(
    vec3<f32>(0.842479062253094, 0.0423282422610123, 0.0423756549057051),
    vec3<f32>(0.0784335999999992, 0.878468636469772, 0.0784336),
    vec3<f32>(0.0792237451477643, 0.0791661274605434, 0.879142973793104),
);
const AGX_OUTSET = mat3x3<f32>(
    vec3<f32>(1.19687900512017, -0.0528968517574562, -0.0529716355144438),
    vec3<f32>(-0.0980208811401368, 1.15190312990417, -0.0980434501171241),
    vec3<f32>(-0.0990297440797205, -0.0989611768448433, 1.15107367264116),
);

fn agx_contrast(x : vec3<f32>) -> vec3<f32> {
    let x2 = x * x;
    let x4 = x2 * x2;
    return 15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4 - 6.868 * x2 * x + 0.4298 * x2
        + 0.1191 * x - 0.00232;
}

fn tonemap_agx(color : vec3<f32>) -> vec3<f32> {
    let min_ev = -12.47393;
    let max_ev = 4.026069;
    var c = AGX_INSET * max(color, vec3<f32>(1e-10));
    c = clamp(log2(max(c, vec3<f32>(1e-10))), vec3<f32>(min_ev), vec3<f32>(max_ev));
    c = (c - min_ev) / (max_ev - min_ev);
    c = agx_contrast(c);
    c = AGX_OUTSET * c;
    // The curve outputs display-encoded values; return linear like the other curves.
    return pow(max(c, vec3<f32>(0.0)), vec3<f32>(2.2));
}

// Khronos PBR Neutral: identity up to ~0.8, then compresses highlights toward white.
fn tonemap_neutral(input : vec3<f32>) -> vec3<f32> {
    let start_compression = 0.8 - 0.04;
    let desaturation = 0.15;
    var color = input;
    let x = min(color.r, min(color.g, color.b));
    let offset = select(0.04, x - 6.25 * x * x, x < 0.08);
    color = color - offset;
    let peak = max(color.r, max(color.g, color.b));
    if peak < start_compression {
        return color;
    }
    let d = 1.0 - start_compression;
    let new_peak = 1.0 - d * d / (peak + d - start_compression);
    color = color * (new_peak / peak);
    let g = 1.0 - 1.0 / (desaturation * (peak - new_peak) + 1.0);
    return mix(color, vec3<f32>(new_peak), g);
}

// Khronos PBR Neutral's shoulder without its toe: identity below 0.76.
fn tonemap_rolloff(input : vec3<f32>) -> vec3<f32> {
    let start_compression = 0.76;
    let desaturation = 0.15;
    var color = max(input, vec3<f32>(0.0));
    let peak = max(color.r, max(color.g, color.b));
    if peak < start_compression {
        return color;
    }
    let d = 1.0 - start_compression;
    let new_peak = 1.0 - d * d / (peak + d - start_compression);
    color = color * (new_peak / peak);
    let g = 1.0 - 1.0 / (desaturation * (peak - new_peak) + 1.0);
    return mix(color, vec3<f32>(new_peak), g);
}

fn hdr_output() -> bool {
    return post_uniform.display.y > 0.5;
}

// HDR displays: keep everything up to 0.8 (around paper white) as authored and roll
// highlights off toward the display's peak instead of toward 1.0.
fn tonemap_to_peak(input : vec3<f32>, display_peak : f32) -> vec3<f32> {
    let start_compression = 0.8;
    let desaturation = 0.15;
    var color = max(input, vec3<f32>(0.0));
    let peak = max(color.r, max(color.g, color.b));
    if peak < start_compression {
        return color;
    }
    let d = display_peak - start_compression;
    let new_peak = display_peak - d * d / (peak + d - start_compression);
    color = color * (new_peak / peak);
    let g = 1.0 - 1.0 / (desaturation * (peak - new_peak) + 1.0);
    return mix(color, vec3<f32>(new_peak), g);
}

fn tonemap(color : vec3<f32>) -> vec3<f32> {
    if hdr_output() {
        // The SDR curves are built for a 0-1 display; HDR output keeps the headroom.
        return tonemap_to_peak(color, max(post_uniform.display.x, 1.0));
    }
    let mode = u32(post_uniform.grading.w + 0.5);
    switch mode {
        case 1u: { return tonemap_reinhard(color); }
        case 2u: { return tonemap_aces(color); }
        case 3u: { return tonemap_agx(color); }
        case 4u: { return tonemap_neutral(color); }
        case 5u: { return tonemap_rolloff(color); }
        default: { return color; }
    }
}

// Lift, gamma and gain on the display image (0-1). On HDR output they grade the
// paper-white range the same way, and highlights above it pass through.
fn grade_ldr(input : vec3<f32>) -> vec3<f32> {
    let headroom = max(input - vec3<f32>(1.0), vec3<f32>(0.0));
    return grade_display(input) + select(vec3<f32>(0.0), headroom, hdr_output());
}

fn grade_display(input : vec3<f32>) -> vec3<f32> {
    var color = clamp(input, vec3<f32>(0.0), vec3<f32>(1.0));
    color = color * post_uniform.gain.xyz + post_uniform.lift.xyz * (1.0 - color);
    color = pow(max(color, vec3<f32>(0.0)), post_uniform.gamma.xyz);
    return clamp(color, vec3<f32>(0.0), vec3<f32>(1.0));
}

// A .cube LUT maps display-encoded (sRGB) colour to display-encoded colour.
fn apply_lut(color : vec3<f32>) -> vec3<f32> {
    let amount = post_uniform.output.w;
    if amount <= 0.0 {
        return color;
    }
    let encoded = linear_to_srgb(clamp(color, vec3<f32>(0.0), vec3<f32>(1.0)));
    let size = f32(textureDimensions(composite_lut).x);
    // Sample texel centres: 0 and 1 land on the first and last entries.
    let coord = encoded * ((size - 1.0) / size) + 0.5 / size;
    let graded = textureSampleLevel(composite_lut, composite_sampler, coord, 0.0).rgb;
    let headroom = max(color - vec3<f32>(1.0), vec3<f32>(0.0));
    return srgb_to_linear(mix(encoded, graded, amount)) + headroom;
}

@fragment
fn fs_composite(in : VertexOutput) -> @location(0) vec4<f32> {
    let viewport_uv = clamp(in.uv, vec2<f32>(0.0), vec2<f32>(1.0));
    var color = sample_with_aberration(viewport_uv);
    color = color * exposure_multiplier();
    color = apply_vignette(color, viewport_uv);
    color = grade_hdr(color);
    color = tonemap(color);
    color = grade_ldr(color);
    color = apply_lut(color);
    return vec4<f32>(color, 1.0);
}
