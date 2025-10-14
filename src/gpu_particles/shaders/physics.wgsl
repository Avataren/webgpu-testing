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
    color_mid: vec4<f32>,
    color_end: vec4<f32>,
    user_data: vec4<f32>, // [spawn_scale, end_size_ratio, mid_color_ratio, reserved]
}

struct Params {
    delta_time: f32,
    drag: f32,
    turbulence_strength: f32,
    turbulence_frequency: f32,
    gravity: vec3<f32>,
    _padding_vec3: f32,
    particle_count: u32,
    ground_level: f32,
    bounce_factor: f32,
    velocity_damping: f32,
}

struct DeadList {
    count: atomic<u32>,
    indices: array<u32>,
}

@group(0) @binding(0) var<storage, read_write> particles: array<Particle>;
@group(0) @binding(1) var<uniform> params: Params;
@group(0) @binding(2) var<storage, read_write> dead_list: DeadList;

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
    if index >= arrayLength(&particles) {
        return;
    }

    var p = particles[index];
    
    if p.lifetime < 0.0 {
        return;
    }
    
    p.lifetime += params.delta_time;
    
    if p.lifetime >= p.max_lifetime {
        p.lifetime = -1.0;
        p.position = vec3<f32>(0.0, -10000.0, 0.0);
        p.velocity = vec3<f32>(0.0);
        let slot = atomicAdd(&dead_list.count, 1u);
        if slot < arrayLength(&dead_list.indices) {
            dead_list.indices[slot] = index;
        }
        particles[index] = p;
        return;
    }
    
    let life_ratio = clamp(p.lifetime / p.max_lifetime, 0.0, 1.0);
    
    // Physics updates (gravity, drag, turbulence, collision, rotation)
    var acceleration = params.gravity;
    
    if params.turbulence_strength > 0.0 {
        let turbulence_pos = p.position * params.turbulence_frequency + vec3<f32>(p.lifetime * 0.5);
        let turbulence = noise3d(turbulence_pos) * params.turbulence_strength;
        acceleration += turbulence * 0.5;
    }
    
    acceleration -= p.velocity * params.drag;
    p.velocity += acceleration * params.delta_time;
    p.position += p.velocity * params.delta_time;
    
    if p.position.y < params.ground_level {
        p.position.y = params.ground_level;
        if p.velocity.y < -0.1 {
            p.velocity.y = -p.velocity.y * params.bounce_factor;
            p.velocity.x *= params.velocity_damping;
            p.velocity.z *= params.velocity_damping;
        } else {
            p.velocity.y = 0.0;
            p.velocity.x *= 0.95;
            p.velocity.z *= 0.95;
        }
    }
    
    p.rotation.w += p.angular_velocity * params.delta_time;
    
    // ✅ SIMPLE: Size interpolation with uniform scale
    let spawn_scale = p.user_data.x;
    let size_ratio = p.user_data.y;
    let mid_ratio = clamp(p.user_data.z, 0.001, 0.999);

    // Interpolate from 1.0x to size_ratio
    let current_size_multiplier = 1.0 + (size_ratio - 1.0) * life_ratio;

    // Apply uniformly to all axes (no drift, no aspect ratio issues)
    let new_scale = spawn_scale * current_size_multiplier;
    p.scale = vec3<f32>(new_scale, new_scale, new_scale);

    // ✅ Piecewise color interpolation with configurable midpoint
    let start_color = p.color;
    let mid_color = p.color_mid;
    let end_color = p.color_end;

    var current_color: vec4<f32>;

    if life_ratio < mid_ratio {
        let denom = max(mid_ratio, 1e-4);
        let t = life_ratio / denom;
        current_color = mix(start_color, mid_color, t);
    } else {
        let denom = max(1.0 - mid_ratio, 1e-4);
        let t = (life_ratio - mid_ratio) / denom;
        current_color = mix(mid_color, end_color, t);
    }

    p.color = current_color;

    particles[index] = p;
}