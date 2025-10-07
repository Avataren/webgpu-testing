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
};

@group(0) @binding(0)
var<uniform> post_uniform : PostUniform;

@group(1) @binding(0)
var depth_texture : texture_depth_multisampled_2d;
@group(1) @binding(1)
var noise_texture : texture_2d<f32>;
@group(1) @binding(2)
var clamp_sampler : sampler;

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
    if (uv.x <= 0.0 || uv.x >= 1.0 || uv.y <= 0.0 || uv.y >= 1.0) {
        return 1.0;
    }
    let tex_size = vec2<f32>(textureDimensions(depth_texture));
    let coord = vec2<i32>(uv * tex_size);
    return textureLoad(depth_texture, coord, 0);
}

fn view_normal(uv : vec2<f32>, view_pos : vec3<f32>) -> vec3<f32> {
    let texel = 1.0 / post_uniform.resolution;
    let depth_right = fetch_depth(uv + vec2<f32>(texel.x, 0.0));
    let depth_up = fetch_depth(uv + vec2<f32>(0.0, texel.y));
    let pos_right = reconstruct_view_position(uv + vec2<f32>(texel.x, 0.0), depth_right);
    let pos_up = reconstruct_view_position(uv + vec2<f32>(0.0, texel.y), depth_up);
    let tangent = pos_right - view_pos;
    let bitangent = pos_up - view_pos;
    return normalize(cross(tangent, bitangent));
}

fn ssao_kernel() -> array<vec3<f32>, 32> {
    return array<vec3<f32>, 32>(
        vec3<f32>(0.5381, 0.1856, 0.4319),
        vec3<f32>(0.1379, 0.2486, 0.4430),
        vec3<f32>(0.3371, 0.5679, 0.0057),
        vec3<f32>(-0.6999, -0.0451, -0.0019),
        vec3<f32>(0.0689, -0.1598, 0.8547),
        vec3<f32>(0.0560, 0.0069, -0.1843),
        vec3<f32>(-0.0146, 0.1402, 0.0762),
        vec3<f32>(0.0100, -0.1924, -0.0344),
        vec3<f32>(-0.3577, -0.5301, -0.4358),
        vec3<f32>(-0.3169, 0.1063, 0.0158),
        vec3<f32>(0.0103, -0.5869, 0.0046),
        vec3<f32>(-0.0897, -0.4940, 0.3287),
        vec3<f32>(0.7119, -0.0154, -0.0918),
        vec3<f32>(-0.0533, 0.0596, -0.5411),
        vec3<f32>(0.0352, -0.0631, 0.5460),
        vec3<f32>(-0.4776, 0.2847, -0.0271),
        vec3<f32>(0.3333, -0.3596, 0.3830),
        vec3<f32>(-0.2941, 0.2513, 0.1042),
        vec3<f32>(0.2624, 0.5570, -0.0846),
        vec3<f32>(0.1248, 0.1221, -0.5559),
        vec3<f32>(-0.6291, 0.1545, 0.2803),
        vec3<f32>(0.3933, 0.5746, -0.0978),
        vec3<f32>(-0.4925, 0.2801, -0.2511),
        vec3<f32>(-0.1279, -0.4738, -0.0977),
        vec3<f32>(-0.2346, 0.0931, 0.3024),
        vec3<f32>(0.0035, -0.1466, -0.3281),
        vec3<f32>(0.1647, 0.2177, 0.2720),
        vec3<f32>(0.4625, -0.1217, -0.4370),
        vec3<f32>(0.0702, 0.4898, -0.1250),
        vec3<f32>(-0.0441, -0.3091, 0.2510),
        vec3<f32>(-0.3645, -0.1065, 0.4305),
        vec3<f32>(0.0207, -0.1306, -0.2221)
    );
}

@fragment
fn fs_ssao(in : VertexOutput) -> @location(0) vec4<f32> {
    let depth = fetch_depth(in.uv);
    if (depth >= 1.0) {
        return vec4<f32>(1.0, 1.0, 1.0, 1.0);
    }

    let view_pos = reconstruct_view_position(in.uv, depth);
    let normal = view_normal(in.uv, view_pos);
    let noise = textureSample(noise_texture, clamp_sampler, in.uv * post_uniform.noise_scale);
    let tangent = normalize(noise.xyz * 2.0 - vec3<f32>(1.0, 1.0, 1.0));
    let bitangent = normalize(cross(normal, tangent));
    let tbn = mat3x3<f32>(tangent, bitangent, normal);

    var occlusion = 0.0;
    let samples = ssao_kernel();
    for (var i : u32 = 0u; i < 32u; i = i + 1u) {
        var sample = tbn * samples[i];
        sample = view_pos + sample * post_uniform.radius_bias.x;

        let sample_clip = post_uniform.proj * vec4<f32>(sample, 1.0);
        var offset = sample_clip.xyz / sample_clip.w;
        offset = offset * 0.5 + vec3<f32>(0.5, 0.5, 0.5);
        if (offset.z >= 1.0) {
            continue;
        }
        let sample_depth = fetch_depth(offset.xy);
        let range_check = smoothstep(0.0, 1.0, post_uniform.radius_bias.x / abs(view_pos.z - sample.z));
        let bias = post_uniform.radius_bias.y;
        if (sample_depth < offset.z - bias) {
            occlusion = occlusion + range_check;
        }
    }
    let ao = 1.0 - occlusion / 32.0;
    let ao_pow = pow(ao, post_uniform.intensity_power.y);
    let strength = clamp(post_uniform.intensity_power.x, 0.0, 5.0);
    return vec4<f32>(mix(1.0, ao_pow * 1, strength), 1.0, 1.0, 1.0);
}

// Bloom prefilter
@group(0) @binding(0)
var scene_texture : texture_2d<f32>;
@group(0) @binding(1)
var scene_sampler : sampler;

@fragment
fn fs_bloom_prefilter(in : VertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(scene_texture, scene_sampler, in.uv);
    let brightness = max(max(color.r, color.g), color.b);
    let threshold = 1.0;
    let soft = 0.8;
    let intensity = clamp((brightness - threshold * soft) / max(0.0001, 1.0 - soft), 0.0, 1.0);
    return vec4<f32>(color.rgb * intensity, 1.0);
}

// Bloom blur horizontal
@group(0) @binding(0)
var blur_texture : texture_2d<f32>;
@group(0) @binding(1)
var blur_sampler : sampler;

fn gaussian_weight(x : f32) -> f32 {
    let sigma = 4.0;
    return exp(-(x * x) / (2.0 * sigma * sigma));
}

@fragment
fn fs_bloom_blur_horizontal(in : VertexOutput) -> @location(0) vec4<f32> {
    let tex_size = vec2<f32>(textureDimensions(blur_texture, 0));
    let texel = vec2<f32>(1.0 / tex_size.x, 0.0);
    var result = vec3<f32>(0.0);
    var total = 0.0;
    for (var i : i32 = -8; i <= 8; i = i + 1) {
        let w = gaussian_weight(f32(i));
        let color = textureSample(blur_texture, blur_sampler, in.uv + texel * f32(i)).rgb;
        result = result + color * w;
        total = total + w;
    }
    return vec4<f32>(result / total, 1.0);
}

@fragment
fn fs_bloom_blur_vertical(in : VertexOutput) -> @location(0) vec4<f32> {
    let tex_size = vec2<f32>(textureDimensions(blur_texture, 0));
    let texel = vec2<f32>(0.0, 1.0 / tex_size.y);
    var result = vec3<f32>(0.0);
    var total = 0.0;
    for (var i : i32 = -8; i <= 8; i = i + 1) {
        let w = gaussian_weight(f32(i));
        let color = textureSample(blur_texture, blur_sampler, in.uv + texel * f32(i)).rgb;
        result = result + color * w;
        total = total + w;
    }
    return vec4<f32>(result / total, 1.0);
}

@fragment
fn fs_composite(
    in : VertexOutput,
    @group(1) @binding(0) ssao_texture : texture_2d<f32>,
    @group(1) @binding(1) ssao_sampler : sampler,
    @group(2) @binding(0) bloom_texture : texture_2d<f32>,
    @group(2) @binding(1) bloom_sampler : sampler
) -> @location(0) vec4<f32> {
    let scene_color = textureSample(scene_texture, scene_sampler, in.uv);
    let ssao = textureSample(ssao_texture, ssao_sampler, in.uv).r;
    let bloom = textureSample(bloom_texture, bloom_sampler, in.uv).rgb;
    let ao = ssao;
    let color = scene_color.rgb * ao;
    let final_color = color + bloom;
    return vec4<f32>(final_color, scene_color.a);
}
