// src/gpu_particles/shaders/physics.wgsl

struct Params {
    time_and_turbulence: vec4<f32>,
    gravity_and_count: vec4<f32>,
    collision: vec4<f32>,
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
    let delta_time = params.time_and_turbulence.x;
    let drag = params.time_and_turbulence.y;
    let turbulence_strength = params.time_and_turbulence.z;
    let turbulence_frequency = params.time_and_turbulence.w;
    let gravity = params.gravity_and_count.xyz;
    let ground_level = params.collision.x;
    let bounce_factor = params.collision.y;
    let velocity_damping = params.collision.z;
    
    if p.lifetime < 0.0 {
        return;
    }
    
    p.lifetime += delta_time;
    
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
    var acceleration = gravity;
    
    if turbulence_strength > 0.0 {
        let turbulence_pos = p.position * turbulence_frequency + vec3<f32>(p.lifetime * 0.5);
        let turbulence = noise3d(turbulence_pos) * turbulence_strength;
        acceleration += turbulence * 0.5;
    }
    
    acceleration -= p.velocity * drag;
    p.velocity += acceleration * delta_time;
    p.position += p.velocity * delta_time;
    
    if p.position.y < ground_level {
        p.position.y = ground_level;
        if p.velocity.y < -0.1 {
            p.velocity.y = -p.velocity.y * bounce_factor;
            p.velocity.x *= velocity_damping;
            p.velocity.z *= velocity_damping;
        } else {
            p.velocity.y = 0.0;
            p.velocity.x *= 0.95;
            p.velocity.z *= 0.95;
        }
    }
    
    p.rotation.w += p.angular_velocity * delta_time;
    
    // ✅ SIMPLE: Size interpolation with uniform scale
    let spawn_scale = p.user_data.x;
    let size_ratio = p.user_data.y;
    
    // Interpolate from 1.0x to size_ratio
    let current_size_multiplier = 1.0 + (size_ratio - 1.0) * life_ratio;
    
    // Apply uniformly to all axes (no drift, no aspect ratio issues)
    let new_scale = spawn_scale * current_size_multiplier;
    p.scale = vec3<f32>(new_scale, new_scale, new_scale);
    
    // Gradient interpolation driven by stored keyframes
    let raw_count = u32(p.user_data.z);
    let key_count = max(min(raw_count, MAX_COLOR_KEYS), 1u);
    let color_times = p.color_key_times;

    var lower_index = 0u;
    var upper_index = key_count - 1u;
    var found_span = false;

    for (var i = 0u; i + 1u < key_count; i = i + 1u) {
        let start_time = sample_time(color_times, i);
        let end_time = sample_time(color_times, i + 1u);

        if life_ratio <= start_time {
            lower_index = i;
            upper_index = i;
            found_span = true;
            break;
        }

        if life_ratio < end_time {
            lower_index = i;
            upper_index = i + 1u;
            found_span = true;
            break;
        }
    }

    if !found_span {
        lower_index = key_count - 1u;
        upper_index = key_count - 1u;
    }

    var lower_color = p.color_keys[lower_index];
    var upper_color = p.color_keys[upper_index];

    if lower_index == 0u {
        lower_color = p.spawn_color;
    }
    if upper_index == 0u {
        upper_color = p.spawn_color;
    }

    if lower_index == upper_index {
        p.color = lower_color;
    } else {
        let lower_time = sample_time(color_times, lower_index);
        let upper_time = sample_time(color_times, upper_index);
        let span = max(upper_time - lower_time, 1e-5);
        let mix_t = clamp((life_ratio - lower_time) / span, 0.0, 1.0);
        p.color = lower_color + (upper_color - lower_color) * mix_t;
    }

    particles[index] = p;
}
