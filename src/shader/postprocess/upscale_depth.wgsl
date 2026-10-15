// Upscaling: depth at output resolution, each output pixel taking the scene pixel
// nearest to it, for what is drawn after post-processing.

@group(1) @binding(0)
var upscale_depth_source : texture_depth_2d;

fn upscale_source_pixel(position : vec2<f32>) -> vec2<i32> {
    let dims = vec2<f32>(textureDimensions(upscale_depth_source));
    return display_to_scene_pixel(position, dims);
}

struct UpscaledDepth {
    @builtin(frag_depth) depth : f32,
};

@fragment
fn fs_upscale_depth(in : VertexOutput) -> UpscaledDepth {
    let pixel = upscale_source_pixel(in.position.xy);
    return UpscaledDepth(textureLoad(upscale_depth_source, pixel, 0));
}
