// Shared by every post-processing pass. Mirrors `PostProcessUniform` in
// src/renderer/postprocess/mod.rs: matrices, then vec2 pairs, then vec4 blocks, all
// 16-byte aligned (720 bytes).
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
    // SSAO radius, bias
    radius_bias : vec2<f32>,
    // SSAO intensity, power
    intensity_power : vec2<f32>,
    noise_scale : vec2<f32>,
    near_far : vec2<f32>,
    // seconds (wrapped), frame delta in seconds
    time_delta : vec2<f32>,
    // ssao, bloom, fxaa, auto exposure (1.0 = on)
    effects : vec4<f32>,
    // threshold, knee, scatter, intensity
    bloom_params : vec4<f32>,
    // bloom tint rgb, unused
    bloom_tint : vec4<f32>,
    // compensation multiplier, post-exposure multiplier, min EV, max EV
    exposure_params : vec4<f32>,
    // speed up, speed down (EV/s), low percent, high percent
    exposure_adapt : vec4<f32>,
    // white balance LMS multipliers, unused
    white_balance : vec4<f32>,
    // colour filter rgb, hue shift in turns
    color_filter : vec4<f32>,
    // contrast, saturation, vibrance, tonemapper index
    grading : vec4<f32>,
    // per-channel shadow offset, unused
    lift : vec4<f32>,
    // per-channel inverse midtone exponent, unused
    gamma : vec4<f32>,
    // per-channel highlight multiplier, unused
    gain : vec4<f32>,
    // intensity, smoothness, roundness, viewport aspect
    vignette : vec4<f32>,
    // vignette colour rgb, unused
    vignette_color : vec4<f32>,
    // vignette centre uv, unused
    vignette_center : vec4<f32>,
    // chromatic aberration, grain intensity, grain response, sharpen strength
    lens : vec4<f32>,
    // encode sRGB in the shader (surface without an sRGB format), dither, frame index, unused
    output : vec4<f32>,
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

fn luminance_rec709(color : vec3<f32>) -> f32 {
    return dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn linear_to_srgb(color : vec3<f32>) -> vec3<f32> {
    let c = max(color, vec3<f32>(0.0));
    let low = c * 12.92;
    let high = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(high, low, c <= vec3<f32>(0.0031308));
}

fn srgb_to_linear(color : vec3<f32>) -> vec3<f32> {
    let c = max(color, vec3<f32>(0.0));
    let low = c / 12.92;
    let high = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(high, low, c <= vec3<f32>(0.04045));
}

// Integer hash (PCG) for per-pixel noise that does not tile.
fn pcg_hash(input : u32) -> u32 {
    let state = input * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

fn noise_2d(pixel : vec2<u32>, frame : u32) -> f32 {
    let seed = pcg_hash(pixel.x + pcg_hash(pixel.y + pcg_hash(frame)));
    return f32(seed) / 4294967295.0;
}
