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
    view_proj : mat4x4<f32>,
    view_proj_inv : mat4x4<f32>,
    proj : mat4x4<f32>,
    proj_inv : mat4x4<f32>,
    camera_position : vec4<f32>,
    resolution : vec2<f32>,
    radius_bias : vec2<f32>,
    intensity_power : vec2<f32>,
    noise_scale : vec2<f32>,
    near_far : vec2<f32>,
    _padding0 : vec2<f32>,
    color_adjust : vec4<f32>,
    bloom_params : vec4<f32>,
    effects : vec4<f32>,
};

@group(0) @binding(0)
var<uniform> post_uniform : PostUniform;

// SSAO inputs (group 1 bindings 0-2)
@group(1) @binding(0)
var depth_texture : texture_depth_2d;
@group(1) @binding(1)
var noise_texture : texture_2d<f32>;
@group(1) @binding(2)
var noise_sampler : sampler;

// SSAO blur inputs (group 1 bindings 60-61)
@group(1) @binding(60)
var ssao_blur_texture : texture_2d<f32>;
@group(1) @binding(61)
var ssao_blur_sampler : sampler;

// Color grading inputs (group 1 bindings 10-11)
@group(1) @binding(10)
var color_source : texture_2d<f32>;
@group(1) @binding(11)
var color_sampler : sampler;

fn linearize_depth(depth: f32) -> f32 {
    let near = post_uniform.near_far.x;
    let far = post_uniform.near_far.y;
    return (2.0 * near * far) / (far + near - depth * (far - near));
}

fn reconstruct_view_position(uv : vec2<f32>, depth : f32) -> vec3<f32> {
    // Convert UV (origin top-left) to NDC (origin center, +Y up)
    let ndc = vec3<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth);
    let clip = vec4<f32>(ndc, 1.0);
    let view = post_uniform.proj_inv * clip;
    return view.xyz / view.w;
}

fn reconstruct_world_position(uv : vec2<f32>, depth : f32) -> vec3<f32> {
    let ndc = vec3<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth);
    let clip = vec4<f32>(ndc, 1.0);
    let world = post_uniform.view_proj_inv * clip;
    return world.xyz / world.w;
}

fn fetch_depth(uv : vec2<f32>) -> f32 {
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {
        return 1.0;
    }
    let tex_size = vec2<f32>(textureDimensions(depth_texture, 0));
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

fn apply_color_controls(color: vec3<f32>) -> vec3<f32> {
    let exposure = max(post_uniform.color_adjust.x, 0.0);
    let saturation = max(post_uniform.color_adjust.y, 0.0);
    let contrast = max(post_uniform.color_adjust.z, 0.0);

    var adjusted = color * exposure;
    let luma = dot(adjusted, vec3<f32>(0.2126, 0.7152, 0.0722));
    adjusted = mix(vec3<f32>(luma), adjusted, saturation);
    let pivot = vec3<f32>(0.5, 0.5, 0.5);
    adjusted = (adjusted - pivot) * contrast + pivot;
    return adjusted;
}

@fragment
fn fs_color_adjust(in : VertexOutput) -> @location(0) vec4<f32> {
    let uv = clamp(in.uv, vec2<f32>(0.0), vec2<f32>(1.0));
    let base = textureSample(color_source, color_sampler, uv);
    let adjusted = apply_color_controls(base.rgb);
    return vec4<f32>(adjusted, base.a);
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

// @fragment
// fn fs_ssao(in : VertexOutput) -> @location(0) vec4<f32> {
//     let depth = fetch_depth(in.uv);
//     let inverted = (1.0 - depth) * 100.0;
//     return vec4<f32>(inverted, inverted, inverted, 1.0);
// }
@fragment
fn fs_ssao(in : VertexOutput) -> @location(0) vec4<f32> {
    if (post_uniform.effects.x < 0.5) {
        return vec4<f32>(1.0, 1.0, 1.0, 1.0);
    }

    let depth = fetch_depth(in.uv);
    let noise_sample = textureSample(noise_texture, noise_sampler, in.uv * post_uniform.noise_scale);
    if (depth >= 1.0) {
        return vec4<f32>(1.0, 1.0, 1.0, 1.0);
    }

    let view_pos = reconstruct_view_position(in.uv, depth);
    let normal = view_normal(in.uv, view_pos);
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
        let offset_ndc = sample_clip.xyz / sample_clip.w;
        // Convert NDC to UV (origin top-left)
        let offset_uv = vec2<f32>(offset_ndc.x * 0.5 + 0.5, 0.5 - offset_ndc.y * 0.5);
        if (offset_ndc.z >= 1.0) {
            continue;
        }
        let sample_depth = fetch_depth(offset_uv);
        if (sample_depth >= 1.0) {
            continue;
        }

        let sample_view_pos = reconstruct_view_position(offset_uv, sample_depth);
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

fn ssao_texel_size() -> vec2<f32> {
    let width = max(post_uniform.resolution.x, 1.0);
    let height = max(post_uniform.resolution.y, 1.0);
    return vec2<f32>(1.0 / width, 1.0 / height);
}

fn ssao_blur_value(uv : vec2<f32>, direction : vec2<f32>) -> f32 {
    let texel = ssao_texel_size();
    let sigma = 2.0;
    var accum = 0.0;
    var weight_sum = 0.0;

    for (var i : i32 = -2; i <= 2; i = i + 1) {
        let offset = f32(i);
        let weight = exp(-(offset * offset) / (2.0 * sigma * sigma));
        let sample_uv = clamp(
            uv + direction * vec2<f32>(texel.x * offset, texel.y * offset),
            vec2<f32>(0.0),
            vec2<f32>(1.0),
        );
        let sample_value = textureSampleLevel(
            ssao_blur_texture,
            ssao_blur_sampler,
            sample_uv,
            0.0,
        )
            .r;
        accum = accum + sample_value * weight;
        weight_sum = weight_sum + weight;
    }

    return accum / max(weight_sum, 1e-4);
}

@fragment
fn fs_ssao_blur_horizontal(in : VertexOutput) -> @location(0) vec4<f32> {
    let ao = ssao_blur_value(in.uv, vec2<f32>(1.0, 0.0));
    return vec4<f32>(ao, ao, ao, 1.0);
}

@fragment
fn fs_ssao_blur_vertical(in : VertexOutput) -> @location(0) vec4<f32> {
    let ao = ssao_blur_value(in.uv, vec2<f32>(0.0, 1.0));
    return vec4<f32>(ao, ao, ao, 1.0);
}

// Bloom prefilter (group 1 bindings 20-21)
@group(1) @binding(20)
var scene_texture : texture_2d<f32>;
@group(1) @binding(21)
var scene_sampler : sampler;

@fragment
fn fs_bloom_prefilter(in : VertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(scene_texture, scene_sampler, in.uv).rgb;
    let brightness = max(max(color.r, color.g), color.b);
    let threshold = max(post_uniform.bloom_params.x, 0.0);
    let knee = max(post_uniform.bloom_params.y, 1e-4);
    let soft = brightness - threshold + knee;
    let clamped = clamp(soft, 0.0, 2.0 * knee);
    let soft_curve = clamped * clamped / (4.0 * knee + 1e-5);
    let contribution = max(soft_curve, brightness - threshold);
    let weight = contribution / max(brightness, 1e-4);
    return vec4<f32>(color * weight, 1.0);
}

// Bloom downsample (group 1 bindings 30-31)
@group(1) @binding(30)
var bloom_down_texture : texture_2d<f32>;
@group(1) @binding(31)
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

// Bloom upsample (group 1 bindings 40-42)
@group(1) @binding(40)
var bloom_upsample_texture : texture_2d<f32>;
@group(1) @binding(41)
var bloom_upsample_base : texture_2d<f32>;
@group(1) @binding(42)
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
    let scatter = clamp(post_uniform.bloom_params.z, 0.0, 1.0);
    return vec4<f32>(base + filtered * scatter, 1.0);
}

// Composite stage inputs (group 1 bindings 50-53)
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

fn safe_texel_size() -> vec2<f32> {
    let width = max(post_uniform.resolution.x, 1.0);
    let height = max(post_uniform.resolution.y, 1.0);
    return vec2<f32>(1.0 / width, 1.0 / height);
}

fn sample_lit_color(uv : vec2<f32>) -> vec3<f32> {
    let uv_clamped = clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0));
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

fn grid_line_mask(coord : vec2<f32>, width : f32) -> f32 {
    let cell = abs(fract(coord) - vec2<f32>(0.5, 0.5));
    let distance = min(cell.x, cell.y);
    return smoothstep(width, 0.0, distance);
}

fn grid_overlay(uv : vec2<f32>) -> vec4<f32> {
    let camera_pos = post_uniform.camera_position.xyz;
    let world_far = reconstruct_world_position(uv, 1.0);
    var ray = world_far - camera_pos;
    let ray_len_sq = dot(ray, ray);
    if (ray_len_sq < 1e-6) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    let dir = normalize(ray);
    let denom = dir.y;
    if (abs(denom) < 1e-5) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }

    let t = -camera_pos.y / denom;
    if (t <= 0.0) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }

    let point = camera_pos + dir * t;
    let clip = post_uniform.view_proj * vec4<f32>(point, 1.0);
    if (clip.w <= 0.0) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }

    let grid_depth = clip.z / clip.w;
    if (grid_depth < 0.0 || grid_depth > 1.0) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }

    let depth = fetch_depth(uv);
    if (depth < 1.0 && depth + 1e-4 < grid_depth) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }

    let grid_pos = point.xz;
    let minor = grid_line_mask(grid_pos, 0.015);
    let major = grid_line_mask(grid_pos / 10.0, 0.02);
    let axis_x = smoothstep(0.1, 0.0, abs(grid_pos.x));
    let axis_z = smoothstep(0.1, 0.0, abs(grid_pos.y));

    let distance = length(point - camera_pos);
    let fade = 1.0 - smoothstep(0.0, 1.0, distance / 120.0);
    if (fade <= 0.0) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }

    let minor_strength = minor * 0.6;
    let major_strength = major;
    let axis_x_strength = axis_x * 0.9;
    let axis_z_strength = axis_z * 0.9;

    var grid_color = vec3<f32>(0.0, 0.0, 0.0);
    grid_color = grid_color + vec3<f32>(0.32, 0.34, 0.38) * minor_strength;
    grid_color = grid_color + vec3<f32>(0.55, 0.57, 0.62) * major_strength;
    grid_color = grid_color + vec3<f32>(0.85, 0.3, 0.3) * axis_x_strength;
    grid_color = grid_color + vec3<f32>(0.3, 0.48, 0.85) * axis_z_strength;

    let intensity = clamp(max(max(minor_strength, major_strength), max(axis_x_strength, axis_z_strength)), 0.0, 1.0);
    let alpha = clamp(intensity * fade, 0.0, 1.0);

    grid_color = clamp(grid_color * fade, vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(grid_color, alpha);
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
    if post_uniform.effects.z > 0.5 {
        color = fxaa(in.uv);
    }
    let grid = grid_overlay(in.uv);
    color = mix(color, grid.rgb, grid.a);
    return vec4<f32>(color, base.a);
}


// fn fs_composite(in : VertexOutput) -> @location(0) vec4<f32> {
//     let ssao = textureSample(composite_ssao, composite_sampler, in.uv);
//     // Temporarily show just SSAO (should see dark areas in corners/crevices)
//     return vec4<f32>(ssao.r, ssao.g, ssao.b, 1.0);
    
//     // Original composite code commented out:
//     // let base = textureSample(composite_scene, composite_sampler, in.uv);
//     // let bloom = textureSample(composite_bloom, composite_sampler, in.uv).rgb;
//     // let shaded = base.rgb * ssao;
//     // let result = shaded + bloom;
//     // return vec4<f32>(result, base.a);
// }
