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
    user_data: vec4<f32>, // [start_size, end_size, unused, unused]
}

struct Params {
    delta_time: f32,
    drag: f32,
    turbulence_strength: f32,
    turbulence_frequency: f32,
    gravity: vec3<f32>,
    particle_count: u32,
    ground_level: f32,
    bounce_factor: f32,
    velocity_damping: f32,
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
    
    // Skip already dead particles
    if p.lifetime < 0.0 {
        return;
    }
    
    // Update lifetime
    p.lifetime += params.delta_time;
    
    // Kill old particles and mark for recycling
    if p.lifetime >= p.max_lifetime {
        p.lifetime = -1.0;
        p.position = vec3<f32>(0.0, -10000.0, 0.0); // Hide offscreen
        p.velocity = vec3<f32>(0.0);
        particles[index] = p;
        return;
    }
    
    // Calculate life ratio for effects
    let life_ratio = clamp(p.lifetime / p.max_lifetime, 0.0, 1.0);
    
    // Apply gravity
    var acceleration = params.gravity;
    
    // Apply turbulence if enabled
    if params.turbulence_strength > 0.0 {
        let turbulence_pos = p.position * params.turbulence_frequency + vec3<f32>(p.lifetime * 0.5);
        let turbulence = noise3d(turbulence_pos) * params.turbulence_strength;
        acceleration += turbulence;
    }
    
    // Apply drag
    acceleration -= p.velocity * params.drag;
    
    // Update velocity and position
    p.velocity += acceleration * params.delta_time;
    p.position += p.velocity * params.delta_time;
    
    // Ground collision with bounce
    if p.position.y < params.ground_level {
        p.position.y = params.ground_level;
        
        // Bounce if velocity is significant
        if p.velocity.y < -0.1 {
            p.velocity.y = -p.velocity.y * params.bounce_factor;
            p.velocity.x *= params.velocity_damping;
            p.velocity.z *= params.velocity_damping;
        } else {
            // Stop bouncing if velocity is too small
            p.velocity.y = 0.0;
            p.velocity.x *= 0.95;
            p.velocity.z *= 0.95;
        }
    }
    
    // Update rotation
    p.rotation.w += p.angular_velocity * params.delta_time;
    
    // Interpolate size based on user_data (start_size, end_size)
    let start_size = p.user_data.x;
    let end_size = p.user_data.y;
    let current_size = start_size + (end_size - start_size) * life_ratio;
    
    // Apply size to scale (preserve aspect ratio)
    let base_scale = length(p.scale) / 1.732; // Normalize by sqrt(3)
    p.scale = vec3<f32>(base_scale * current_size);
    
    // Fade out near end of life (affects alpha in color)
    // We can't modify color gradient here easily, but we can dim the alpha
    // The color is set at spawn time with the gradient
    let fade_start = 0.8;
    if life_ratio > fade_start {
        let fade_factor = 1.0 - (life_ratio - fade_start) / (1.0 - fade_start);
        p.color.a *= fade_factor;
    }
    
    particles[index] = p;
}