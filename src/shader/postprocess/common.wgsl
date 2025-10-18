struct PostUniform {
    view_proj : mat4x4<f32>,
    view_proj_inv : mat4x4<f32>,
    proj : mat4x4<f32>,
    proj_inv : mat4x4<f32>,
    view : mat4x4<f32>,
    view_inv : mat4x4<f32>,
    camera_position : vec4<f32>,
    resolution : vec2<f32>,
    viewport_offset : vec2<f32>,
    viewport_scale : vec2<f32>,
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

fn viewport_to_scene_uv(uv : vec2<f32>) -> vec2<f32> {
    return post_uniform.viewport_offset + uv * post_uniform.viewport_scale;
}

fn scene_to_viewport_uv(uv : vec2<f32>) -> vec2<f32> {
    let scale = max(post_uniform.viewport_scale, vec2<f32>(1e-6, 1e-6));
    return (uv - post_uniform.viewport_offset) / scale;
}

fn viewport_texel_size() -> vec2<f32> {
    let res = max(post_uniform.resolution, vec2<f32>(1.0, 1.0));
    return vec2<f32>(1.0, 1.0) / res;
}

fn scene_texel_size() -> vec2<f32> {
    return post_uniform.viewport_scale * viewport_texel_size();
}

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

fn safe_texel_size() -> vec2<f32> {
    let width = max(post_uniform.resolution.x, 1.0);
    let height = max(post_uniform.resolution.y, 1.0);
    return vec2<f32>(1.0 / width, 1.0 / height);
}

fn view_matrix() -> mat4x4<f32> {
    return post_uniform.view;
}

fn view_inverse_matrix() -> mat4x4<f32> {
    return post_uniform.view_inv;
}

fn world_to_view_position(world : vec3<f32>) -> vec3<f32> {
    let view = view_matrix();
    let transformed = view * vec4<f32>(world, 1.0);
    return transformed.xyz;
}

fn world_to_view_normal(normal : vec3<f32>) -> vec3<f32> {
    let view = view_matrix();
    let basis = mat3x3<f32>(view[0].xyz, view[1].xyz, view[2].xyz);
    return basis * normal;
}
