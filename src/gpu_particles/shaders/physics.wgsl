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
    user_data: vec4<f32>, // [start_size, end_size, original_scale_magnitude, end_alpha]
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
    
    // Skip already dead particles
    if p.lifetime < 0.0 {
        return;
    }
    
    // ✅ CRITICAL FIX: Store original scale BEFORE updating lifetime
    // This ensures it's available on the first frame
    let is_first_frame = p.lifetime == 0.0 && p.user_data.z == 0.0;
    if is_first_frame {
        // Store original scale magnitude
        let scale_magnitude = (p.scale.x + p.scale.y + p.scale.z) / 3.0;
        p.user_data.z = scale_magnitude;
    }
    
    // Store start values for interpolation
    let start_alpha = p.color.a;
    let start_rgb = p.color.rgb;
    
    // Update lifetime
    p.lifetime += params.delta_time;
    
    // Kill old particles and mark for recycling
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
    
    // Calculate life ratio for effects
    let life_ratio = clamp(p.lifetime / p.max_lifetime, 0.0, 1.0);
    
    // Apply gravity
    var acceleration = params.gravity;
    
    // ✅ FIX: Reduce turbulence impact significantly
    if params.turbulence_strength > 0.0 {
        let turbulence_pos = p.position * params.turbulence_frequency + vec3<f32>(p.lifetime * 0.5);
        let turbulence = noise3d(turbulence_pos) * params.turbulence_strength;
        // Scale down turbulence to prevent chaotic motion
        acceleration += turbulence * 0.5;
    }
    
    // Apply drag
    acceleration -= p.velocity * params.drag;
    
    // Update velocity and position
    p.velocity += acceleration * params.delta_time;
    p.position += p.velocity * params.delta_time;
    
    // Ground collision with bounce
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
    
    // Update rotation
    p.rotation.w += p.angular_velocity * params.delta_time;
    
    // ✅ FIX: Properly interpolate size using original scale
    let start_size = p.user_data.x;
    let end_size = p.user_data.y;
    let original_scale = p.user_data.z;
    
    // Interpolate size multiplier
    let current_size_multiplier = start_size + (end_size - start_size) * life_ratio;
    
    // Apply to original spawn scale (prevents drift and compounding)
    if original_scale > 0.0 {
        p.scale = vec3<f32>(original_scale * current_size_multiplier);
    }
    
    // ✅ FIX: Properly interpolate color and alpha
    let end_alpha = p.user_data.w;
    
    // Linear interpolation from start to end alpha
    let current_alpha = start_alpha + (end_alpha - start_alpha) * life_ratio;
    
    // Darken RGB over time (smoke darkens as it dissipates)
    let darken_factor = 1.0 - life_ratio * 0.6;
    let current_rgb = start_rgb * darken_factor;
    
    // Set the interpolated color (direct assignment, not multiplication!)
    p.color = vec4<f32>(current_rgb, current_alpha);
    
    particles[index] = p;
}