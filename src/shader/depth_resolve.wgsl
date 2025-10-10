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

fn fetch_depth(uv : vec2<f32>) -> f32 {
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {
        return 1.0;
    }
    let tex_size = vec2<f32>(textureDimensions(depth_texture));
    let max_uv = (tex_size - vec2<f32>(1.0)) / tex_size;
    let clamped_uv = clamp(uv, vec2<f32>(0.0), max_uv);
    let coord = vec2<i32>(clamped_uv * tex_size);
    let max_samples = 8;
    let count_f = clamp(post_uniform.effects.w, 1.0, f32(max_samples));
    let count = i32(count_f);
    var d = 1.0;
    var i : i32 = 0;
    loop {
        if (i >= count) { break; }
        let s = textureLoad(depth_texture, coord, i);
        d = min(d, s);
        i = i + 1;
    }
    return d;
}

struct DepthOutput {
    @builtin(frag_depth) depth : f32,
};

@fragment
fn fs_resolve_depth(in : VertexOutput) -> DepthOutput {
    let depth = fetch_depth(in.uv);
    return DepthOutput(depth);
}
