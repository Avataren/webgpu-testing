struct VertexOutput {
    @builtin(position) position : vec4<f32>,
    @location(0) uv : vec2<f32>,
};

@vertex
fn vs_fullscreen(@builtin(vertex_index) vertex_index : u32) -> VertexOutput {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(3.0, 1.0)
    );
    let pos = positions[vertex_index];
    var out : VertexOutput;
    out.position = vec4<f32>(pos, 0.0, 1.0);
    // Flip Y coordinate: top of screen (clip Y=1) maps to UV.y=0
    out.uv = vec2<f32>(0.5 * (pos.x + 1.0), 0.5 * (1.0 - pos.y));
    return out;
}

struct PostUniform {
    proj : mat4x4<f32>,
    proj_inv : mat4x4<f32>,
    resolution : vec2<f32>,
    radius_bias : vec2<f32>,
    intensity_power : vec2<f32>,
    noise_scale : vec2<f32>,
    near_far : vec2<f32>,
    effects : vec4<f32>,
};

@group(0) @binding(0)
var<uniform> post_uniform : PostUniform;

@group(1) @binding(0)
var depth_texture : texture_depth_multisampled_2d;
@group(1) @binding(1)
var noise_texture : texture_2d<f32>;
@group(1) @binding(2)
var noise_sampler : sampler;

fn linearize_depth(depth: f32) -> f32 {
    let near = post_uniform.near_far.x;
    let far = post_uniform.near_far.y;
    return (2.0 * near * far) / (far + near - depth * (far - near));
}

fn reconstruct_view_position(uv : vec2<f32>, depth : f32) -> vec3<f32> {
    // For wgpu, depth is already in [0, 1], don't remap it
    let clip = vec4<f32>(uv * 2.0 - 1.0, depth, 1.0);  // Changed: removed "depth * 2.0 - 1.0"
    let view = post_uniform.proj_inv * clip;
    return view.xyz / view.w;
}

fn fetch_depth(uv : vec2<f32>) -> f32 {
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {
        return 1.0;
    }
    let tex_size = vec2<f32>(textureDimensions(depth_texture));
    let max_uv = (tex_size - vec2<f32>(1.0)) / tex_size;
    let clamped_uv = clamp(uv, vec2<f32>(0.0), max_uv);
    let coord = vec2<i32>(clamped_uv * tex_size);
    return textureLoad(depth_texture, coord, 0);
}

fn view_normal(uv : vec2<f32>, view_pos : vec3<f32>) -> vec3<f32> {
    let texel = 1.0 / post_uniform.resolution;

    let depth_left = fetch_depth(uv - vec2<f32>(texel.x, 0.0));
    let depth_right = fetch_depth(uv + vec2<f32>(texel.x, 0.0));
    let depth_down = fetch_depth(uv - vec2<f32>(0.0, texel.y));
    let depth_up = fetch_depth(uv + vec2<f32>(0.0, texel.y));

    var pos_left = view_pos;
    if (depth_left < 1.0) {
        pos_left = reconstruct_view_position(uv - vec2<f32>(texel.x, 0.0), depth_left);
    }

    var pos_right = view_pos;
    if (depth_right < 1.0) {
        pos_right = reconstruct_view_position(uv + vec2<f32>(texel.x, 0.0), depth_right);
    }

    var pos_down = view_pos;
    if (depth_down < 1.0) {
        pos_down = reconstruct_view_position(uv - vec2<f32>(0.0, texel.y), depth_down);
    }

    var pos_up = view_pos;
    if (depth_up < 1.0) {
        pos_up = reconstruct_view_position(uv + vec2<f32>(0.0, texel.y), depth_up);
    }

    var dx = pos_right - pos_left;
    var dy = pos_up - pos_down;
    let eps = 1e-5;
    if (dot(dx, dx) < eps) {
        dx = vec3<f32>(1.0, 0.0, 0.0);
    }
    if (dot(dy, dy) < eps) {
        dy = vec3<f32>(0.0, 1.0, 0.0);
    }

    var normal = normalize(cross(dx, dy));
    var view_dir = -view_pos;
    if (dot(view_dir, view_dir) < 1e-6) {
        view_dir = vec3<f32>(0.0, 0.0, 1.0);
    }
    view_dir = normalize(view_dir);
    if (dot(normal, view_dir) <= 0.0) {
        normal = -normal;
    }
    return normal;
}

fn ssao_kernel() -> array<vec3<f32>, 32> {
    return array<vec3<f32>, 32>(
        vec3<f32>(-0.0559, 0.0179, 0.0810),
        vec3<f32>(-0.0735, 0.0456, 0.0520),
        vec3<f32>(-0.0271, -0.0585, 0.0813),
        vec3<f32>(-0.0401, -0.1000, 0.0119),
        vec3<f32>(0.0736, 0.0220, 0.0855),
        vec3<f32>(0.0526, 0.0083, 0.1113),
        vec3<f32>(-0.0373, 0.0160, 0.1274),
        vec3<f32>(0.0362, 0.1105, 0.0882),
        vec3<f32>(0.0640, -0.1422, 0.0357),
        vec3<f32>(-0.0765, -0.1526, 0.0423),
        vec3<f32>(-0.1389, -0.0773, 0.1106),
        vec3<f32>(-0.1343, -0.1290, 0.1041),
        vec3<f32>(-0.0925, 0.1734, 0.1286),
        vec3<f32>(0.0560, -0.1689, 0.1872),
        vec3<f32>(-0.1564, -0.0560, 0.2298),
        vec3<f32>(0.1163, 0.0473, 0.2842),
        vec3<f32>(0.2561, 0.2062, 0.0856),
        vec3<f32>(-0.3332, -0.1314, 0.0953),
        vec3<f32>(-0.1698, 0.2602, 0.2574),
        vec3<f32>(-0.2598, 0.2179, 0.2773),
        vec3<f32>(0.4501, -0.0447, 0.1438),
        vec3<f32>(-0.4453, 0.1078, 0.2309),
        vec3<f32>(0.1033, 0.4858, 0.2439),
        vec3<f32>(-0.2672, 0.4736, 0.2425),
        vec3<f32>(-0.4269, -0.4726, 0.0572),
        vec3<f32>(0.2285, 0.5237, 0.3785),
        vec3<f32>(-0.4756, -0.1290, 0.5427),
        vec3<f32>(0.0357, 0.5773, 0.5274),
        vec3<f32>(-0.6415, 0.2899, 0.4476),
        vec3<f32>(0.0824, -0.5200, 0.7146),
        vec3<f32>(-0.8058, -0.1353, 0.4706),
        vec3<f32>(0.7516, 0.6225, 0.2181)
    );
}

@fragment
fn fs_ssao(in : VertexOutput) -> @location(0) vec4<f32> {
    if (post_uniform.effects.x < 0.5) {
        return vec4<f32>(1.0, 1.0, 1.0, 1.0);
    }

    let depth = fetch_depth(in.uv);
    if (depth >= 1.0) {
        return vec4<f32>(1.0, 1.0, 1.0, 1.0);
    }

    let view_pos = reconstruct_view_position(in.uv, depth);
    let normal = view_normal(in.uv, view_pos);
    let noise_sample = textureSample(noise_texture, noise_sampler, in.uv * post_uniform.noise_scale);
    var tangent = vec3<f32>(noise_sample.xy, 0.0);
    if (dot(tangent, tangent) < 1e-4) {
        tangent = vec3<f32>(1.0, 0.0, 0.0);
    }
    tangent = normalize(tangent);
    let bitangent = normalize(cross(normal, tangent));
    let tbn = mat3x3<f32>(tangent, bitangent, normal);

    let radius = post_uniform.radius_bias.x;
    let bias = post_uniform.radius_bias.y;

    var occlusion = 0.0;
    let samples = ssao_kernel();
    let sample_count = 32.0;
    for (var i : u32 = 0u; i < 32u; i = i + 1u) {
        let rotated = tbn * samples[i];
        let sample_pos = view_pos + normal * bias + rotated * radius;

        let sample_clip = post_uniform.proj * vec4<f32>(sample_pos, 1.0);
        var offset = sample_clip.xyz / sample_clip.w;
        offset = offset * 0.5 + vec3<f32>(0.5, 0.5, 0.5);
        if (offset.z >= 1.0) {
            continue;
        }
        let sample_depth = fetch_depth(offset.xy);
        if (sample_depth >= 1.0) {
            continue;
        }

        let sample_view_pos = reconstruct_view_position(offset.xy, sample_depth);
        let range_check = smoothstep(
            0.0,
            1.0,
            radius / (abs(view_pos.z - sample_view_pos.z) + 1e-4),
        );
        if (sample_view_pos.z >= sample_pos.z) {
            occlusion = occlusion + range_check;
        }
    }
    let ao = 1.0 - occlusion / sample_count;
    let ao_pow = pow(ao, max(post_uniform.intensity_power.y, 0.01));
    let strength = clamp(post_uniform.intensity_power.x, 0.0, 1.0);
    let ao_result = mix(1.0, ao_pow, strength);
    return vec4<f32>(ao_result, ao_result, ao_result, 1.0);
}

// Bloom prefilter - uses group 0 for scene texture
@group(0) @binding(0)
var scene_texture : texture_2d<f32>;
@group(0) @binding(1)
var scene_sampler : sampler;

@fragment
fn fs_bloom_prefilter(in : VertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(scene_texture, scene_sampler, in.uv).rgb;
    let brightness = max(max(color.r, color.g), color.b);
    let threshold = 0.8;
    let knee = threshold * 0.5;
    let soft = brightness - threshold + knee;
    let clamped = clamp(soft, 0.0, 2.0 * knee);
    let soft_curve = clamped * clamped / (4.0 * max(knee, 1e-4) + 1e-5);
    let contribution = max(soft_curve, brightness - threshold);
    let weight = contribution / max(brightness, 1e-4);
    return vec4<f32>(color * weight, 1.0);
}

// Bloom downsample
@group(0) @binding(0)
var bloom_down_texture : texture_2d<f32>;
@group(0) @binding(1)
var bloom_down_sampler : sampler;

fn bloom_gaussian_weight(offset : vec2<f32>, sigma : f32) -> f32 {
    return exp(-(dot(offset, offset)) / (2.0 * sigma * sigma));
}

@fragment
fn fs_bloom_downsample(in : VertexOutput) -> @location(0) vec4<f32> {
    let tex_size = vec2<f32>(textureDimensions(bloom_down_texture, 0));
    let texel = 1.0 / tex_size;
    var result = vec3<f32>(0.0);
    var total = 0.0;
    for (var x : i32 = -2; x <= 2; x = x + 1) {
        for (var y : i32 = -2; y <= 2; y = y + 1) {
            let offset = vec2<f32>(f32(x), f32(y));
            let weight = bloom_gaussian_weight(offset, 2.5);
            let sample_uv = clamp(in.uv + offset * texel, vec2<f32>(0.0), vec2<f32>(1.0));
            let color = textureSampleLevel(
                bloom_down_texture,
                bloom_down_sampler,
                sample_uv,
                0.0,
            )
                .rgb;
            result = result + color * weight;
            total = total + weight;
        }
    }
    return vec4<f32>(result / max(total, 1e-4), 1.0);
}

// Bloom upsample
@group(0) @binding(0)
var bloom_upsample_texture : texture_2d<f32>;
@group(0) @binding(1)
var bloom_upsample_base : texture_2d<f32>;
@group(0) @binding(2)
var bloom_upsample_sampler : sampler;

@fragment
fn fs_bloom_upsample(in : VertexOutput) -> @location(0) vec4<f32> {
    let tex_size = vec2<f32>(textureDimensions(bloom_upsample_texture, 0));
    let texel = 1.0 / tex_size;
    var filtered = vec3<f32>(0.0);
    var total = 0.0;
    for (var x : i32 = -1; x <= 1; x = x + 1) {
        for (var y : i32 = -1; y <= 1; y = y + 1) {
            let offset = vec2<f32>(f32(x), f32(y));
            let weight = bloom_gaussian_weight(offset, 1.5);
            let sample_uv = clamp(in.uv + offset * texel, vec2<f32>(0.0), vec2<f32>(1.0));
            let color = textureSampleLevel(
                bloom_upsample_texture,
                bloom_upsample_sampler,
                sample_uv,
                0.0,
            )
                .rgb;
            filtered = filtered + color * weight;
            total = total + weight;
        }
    }
    filtered = filtered / max(total, 1e-4);
    let base = textureSampleLevel(
        bloom_upsample_base,
        bloom_upsample_sampler,
        clamp(in.uv, vec2<f32>(0.0), vec2<f32>(1.0)),
        0.0,
    )
        .rgb;
    let scatter = 0.95;
    return vec4<f32>(base + filtered * scatter, 1.0);
}

// Composite - uses group 0 for all textures with one sampler (matching non-MSAA version)
@group(0) @binding(0)
var composite_scene : texture_2d<f32>;
@group(0) @binding(1)
var composite_ssao : texture_2d<f32>;
@group(0) @binding(2)
var composite_bloom : texture_2d<f32>;
@group(0) @binding(3)
var composite_sampler : sampler;

@group(1) @binding(0)
var<uniform> composite_uniform : PostUniform;

const FXAA_REDUCE_MIN : f32 = 1.0 / 128.0;
const FXAA_REDUCE_MUL : f32 = 1.0 / 8.0;
const FXAA_SPAN_MAX : f32 = 8.0;

fn safe_texel_size() -> vec2<f32> {
    let width = max(composite_uniform.resolution.x, 1.0);
    let height = max(composite_uniform.resolution.y, 1.0);
    return vec2<f32>(1.0 / width, 1.0 / height);
}

fn sample_lit_color(uv : vec2<f32>) -> vec3<f32> {
    let uv_clamped = clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0));
    let base = textureSampleLevel(composite_scene, composite_sampler, uv_clamped, 0.0);
    let ssao_enabled = composite_uniform.effects.x > 0.5;
    let bloom_enabled = composite_uniform.effects.y > 0.5;
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
    let base = textureSampleLevel(
        composite_scene,
        composite_sampler,
        clamp(in.uv, vec2<f32>(0.0), vec2<f32>(1.0)),
        0.0,
    );
    var color = sample_lit_color(in.uv);
    if composite_uniform.effects.z > 0.5 {
        color = fxaa(in.uv);
    }
    return vec4<f32>(color, base.a);
}
