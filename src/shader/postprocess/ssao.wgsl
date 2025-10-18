@group(1) @binding(0)
var depth_texture : texture_depth_2d;
@group(1) @binding(1)
var noise_texture : texture_2d<f32>;
@group(1) @binding(2)
var noise_sampler : sampler;

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
    let texel_viewport = viewport_texel_size();
    let texel_scene = scene_texel_size();
    let scene_uv = viewport_to_scene_uv(uv);

    let depth_left = fetch_depth(scene_uv - vec2<f32>(texel_scene.x, 0.0));
    let depth_right = fetch_depth(scene_uv + vec2<f32>(texel_scene.x, 0.0));
    let depth_down = fetch_depth(scene_uv - vec2<f32>(0.0, texel_scene.y));
    let depth_up = fetch_depth(scene_uv + vec2<f32>(0.0, texel_scene.y));

    var pos_left = view_pos;
    if (depth_left < 1.0) {
        pos_left = reconstruct_view_position(uv - vec2<f32>(texel_viewport.x, 0.0), depth_left);
    }

    var pos_right = view_pos;
    if (depth_right < 1.0) {
        pos_right = reconstruct_view_position(uv + vec2<f32>(texel_viewport.x, 0.0), depth_right);
    }

    var pos_down = view_pos;
    if (depth_down < 1.0) {
        pos_down = reconstruct_view_position(uv - vec2<f32>(0.0, texel_viewport.y), depth_down);
    }

    var pos_up = view_pos;
    if (depth_up < 1.0) {
        pos_up = reconstruct_view_position(uv + vec2<f32>(0.0, texel_viewport.y), depth_up);
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
        vec3<f32>(0.7516, 0.6225, 0.2181),
    );
}

@fragment
fn fs_ssao(in : VertexOutput) -> @location(0) vec4<f32> {
    if (post_uniform.effects.x < 0.5) {
        return vec4<f32>(1.0, 1.0, 1.0, 1.0);
    }

    let scene_uv = in.uv;
    let viewport_uv = scene_to_viewport_uv(scene_uv);
    let depth = fetch_depth(scene_uv);
    let noise_sample = textureSample(
        noise_texture,
        noise_sampler,
        clamp(viewport_uv, vec2<f32>(0.0), vec2<f32>(1.0)) * post_uniform.noise_scale,
    );
    if (depth >= 1.0) {
        return vec4<f32>(1.0, 1.0, 1.0, 1.0);
    }

    let view_pos = reconstruct_view_position(viewport_uv, depth);
    let normal = view_normal(viewport_uv, view_pos);
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
        let sample_depth = fetch_depth(viewport_to_scene_uv(offset_uv));
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
