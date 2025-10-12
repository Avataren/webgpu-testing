// src/gpu_particles/shaders/physics.wgsl

struct Particle {
    position: vec3<f32>,
    lifetime: f32,
    velocity: vec3<f32>,
    max_lifetime: f32,
    rotation: vec4<f32>,
    scale: vec3<f32>,
    angular_velocity: f32,
    color: vec4<f32>,
    user_data: vec4<f32>,
}

struct Params {
    delta_time: f32,
    drag: f32,
    turbulence_strength: f32,
    turbulence_frequency: f32,
    gravity: vec3<f32>,
    particle_count: u32,
}

@group(0) @binding(0) var<storage, read_write> particles: array<Particle>;
@group(0) @binding(1) var<uniform> params: Params;

fn noise3d(p: vec3<f32>) -> vec3<f32> {
    let i = floor(p);
    let f = fract(p);
    
    return vec3<f32>(
        sin(i.x * 12.9898 + i.y * 78.233 + i.z * 37.719) * 43758.5453,
        sin(i.x * 93.989 + i.y * 67.345 + i.z * 12.456) * 43758.5453,
        sin(i.x * 45.678 + i.y * 23.456 + i.z * 89.123) * 43758.5453
    ) * 2.0 - 1.0;
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let index = global_id.x;
    var p = particles[index];
    
    // Update lifetime
    p.lifetime += params.delta_time;
    
    // Kill old particles
    if p.lifetime >= p.max_lifetime {
        p.velocity = vec3<f32>(0.0);
        particles[index] = p;
        return;
    }
    
    // Apply gravity
    var acceleration = params.gravity;
    
    // Apply turbulence
    if params.turbulence_strength > 0.0 {
        let turbulence_pos = p.position * params.turbulence_frequency;
        let turbulence = noise3d(turbulence_pos) * params.turbulence_strength;
        acceleration += turbulence;
    }
    
    // Apply drag
    acceleration -= p.velocity * params.drag;
    
    // Update velocity and position
    p.velocity += acceleration * params.delta_time;
    p.position += p.velocity * params.delta_time;
    
    // Update rotation
    p.rotation.w += p.angular_velocity * params.delta_time;
    
    // Fade out near end of life
    let life_ratio = clamp(p.lifetime / p.max_lifetime, 0.0, 1.0);
    p.color.a = 1.0 - life_ratio;
    
    particles[index] = p;
}