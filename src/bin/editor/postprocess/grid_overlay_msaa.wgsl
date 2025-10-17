@group(0) @binding(1)
var depth_texture : texture_depth_multisampled_2d;

fn fetch_depth(uv : vec2<f32>) -> f32 {
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {
        return 1.0;
    }
    let tex_size = vec2<f32>(textureDimensions(depth_texture));
    let max_uv = (tex_size - vec2<f32>(1.0)) / tex_size;
    let clamped_uv = clamp(uv, vec2<f32>(0.0), max_uv);
    let coord = vec2<i32>(clamped_uv * tex_size);
    let sample_count = i32(textureNumSamples(depth_texture));
    if (sample_count <= 1) {
        return textureLoad(depth_texture, coord, 0);
    }
    var min_depth = 1.0;
    for (var i : i32 = 0; i < sample_count; i = i + 1) {
        let sample_depth = textureLoad(depth_texture, coord, i);
        min_depth = min(min_depth, sample_depth);
    }
    return min_depth;
}
