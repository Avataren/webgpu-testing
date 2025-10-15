// src/gpu_particles/shaders/common.wgsl
//
// Shared particle data definitions used across GPU particle compute and render shaders.

const MAX_COLOR_KEYS: u32 = 4u;

struct Particle {
    position: vec3<f32>,
    lifetime: f32,
    velocity: vec3<f32>,
    max_lifetime: f32,
    rotation: vec4<f32>,
    scale: vec3<f32>,
    angular_velocity: f32,
    color: vec4<f32>,
    color_keys: array<vec4<f32>, MAX_COLOR_KEYS>,
    color_key_times: vec4<f32>,
    user_data: vec4<f32>,
};

struct DeadList {
    count: atomic<u32>,
    indices: array<u32>,
};

fn sample_time(times: vec4<f32>, index: u32) -> f32 {
    if index == 0u { return times.x; }
    if index == 1u { return times.y; }
    if index == 2u { return times.z; }
    return times.w;
}
