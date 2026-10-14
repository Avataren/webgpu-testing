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
    let count = i32(textureNumSamples(depth_texture));
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
