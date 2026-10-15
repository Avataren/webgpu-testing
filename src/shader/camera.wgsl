// The camera of the scene passes (renderer/uniforms.rs, CameraUniform): bind group 0.
struct Globals {
    view_proj: mat4x4<f32>,
    inverse_view_proj: mat4x4<f32>,
    camera_pos: vec3<f32>,
    // Texture LOD bias: log2 of the render scale while upscaling, else 0.
    mip_bias: f32,
    camera_forward: vec3<f32>,
    _padding1: f32,
    camera_up: vec3<f32>,
    _padding2: f32,
    // view_proj without the TAA jitter, and last frame's: motion vectors compare them.
    unjittered_view_proj: mat4x4<f32>,
    prev_view_proj: mat4x4<f32>,
};
@group(0) @binding(0) var<uniform> globals: Globals;
