@group(1) @binding(50)
var composite_scene : texture_2d<f32>;
@group(1) @binding(51)
var composite_ssao : texture_2d<f32>;
@group(1) @binding(52)
var composite_bloom : texture_2d<f32>;
@group(1) @binding(53)
var composite_sampler : sampler;

const FXAA_REDUCE_MIN : f32 = 1.0 / 128.0;
const FXAA_REDUCE_MUL : f32 = 1.0 / 8.0;
const FXAA_SPAN_MAX : f32 = 8.0;

fn sample_lit_color(uv : vec2<f32>) -> vec3<f32> {
    let scene_uv = viewport_to_scene_uv(uv);
    let uv_clamped = clamp(scene_uv, vec2<f32>(0.0), vec2<f32>(1.0));
    let base = textureSampleLevel(composite_scene, composite_sampler, uv_clamped, 0.0);
    let ssao_enabled = post_uniform.effects.x > 0.5;
    let bloom_enabled = post_uniform.effects.y > 0.5;
    var ssao = 1.0;
    if ssao_enabled {
        ssao = textureSampleLevel(composite_ssao, composite_sampler, uv_clamped, 0.0).r;
    }
    var bloom = vec3<f32>(0.0);
    if bloom_enabled {
        bloom = textureSampleLevel(composite_bloom, composite_sampler, uv_clamped, 0.0).rgb;
    }
    return base.rgb * ssao + bloom;
}

fn luminance(color : vec3<f32>) -> f32 {
    return dot(color, vec3<f32>(0.299, 0.587, 0.114));
}

fn fxaa(uv : vec2<f32>) -> vec3<f32> {
    let texel = safe_texel_size();

    let rgb_m = sample_lit_color(uv);
    let luma_m = luminance(rgb_m);

    let rgb_nw = sample_lit_color(uv + texel * vec2<f32>(-1.0, -1.0));
    let rgb_ne = sample_lit_color(uv + texel * vec2<f32>(1.0, -1.0));
    let rgb_sw = sample_lit_color(uv + texel * vec2<f32>(-1.0, 1.0));
    let rgb_se = sample_lit_color(uv + texel * vec2<f32>(1.0, 1.0));

    let luma_nw = luminance(rgb_nw);
    let luma_ne = luminance(rgb_ne);
    let luma_sw = luminance(rgb_sw);
    let luma_se = luminance(rgb_se);

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
    dir = clamp(
        dir * rcp_dir_min,
        vec2<f32>(-FXAA_SPAN_MAX, -FXAA_SPAN_MAX),
        vec2<f32>(FXAA_SPAN_MAX, FXAA_SPAN_MAX),
    );
    dir = dir * texel;

    let rgb_a = 0.5
        * (sample_lit_color(uv + dir * (1.0 / 3.0 - 0.5))
            + sample_lit_color(uv + dir * (2.0 / 3.0 - 0.5)));
    let rgb_b = rgb_a * 0.5
        + 0.25
            * (sample_lit_color(uv + dir * -0.5) + sample_lit_color(uv + dir * 0.5));
    let luma_b = luminance(rgb_b);

    if (luma_b < luma_min || luma_b > luma_max) {
        return rgb_a;
    }
    return rgb_b;
}

@fragment
fn fs_composite(in : VertexOutput) -> @location(0) vec4<f32> {
    let viewport_uv = clamp(in.uv, vec2<f32>(0.0), vec2<f32>(1.0));
    let scene_uv = viewport_to_scene_uv(viewport_uv);
    let base = textureSampleLevel(
        composite_scene,
        composite_sampler,
        clamp(scene_uv, vec2<f32>(0.0), vec2<f32>(1.0)),
        0.0,
    );
    var color = sample_lit_color(viewport_uv);
    if post_uniform.effects.z > 0.5 {
        color = fxaa(viewport_uv);
    }
    return vec4<f32>(color, base.a);
}
