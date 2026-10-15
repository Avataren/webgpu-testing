// Upscaling: depth and pick ids at output resolution, each output pixel taking the scene
// pixel nearest to it, for what is drawn after post-processing.

@group(1) @binding(0)
var upscale_depth_source : texture_depth_2d;
@group(1) @binding(1)
var upscale_pick_source : texture_2d<u32>;

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

struct UpscaledDepthPick {
    @location(0) id : vec2<u32>,
    @builtin(frag_depth) depth : f32,
};

@fragment
fn fs_upscale_depth_pick(in : VertexOutput) -> UpscaledDepthPick {
    let pixel = upscale_source_pixel(in.position.xy);
    return UpscaledDepthPick(
        textureLoad(upscale_pick_source, pixel, 0).xy,
        textureLoad(upscale_depth_source, pixel, 0),
    );
}
