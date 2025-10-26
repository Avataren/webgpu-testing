// src/gpu_particles/shaders/starfield.wgsl

struct Params {
    time_and_planes: vec4<f32>,
    field_and_count: vec4<f32>,
}

@group(0) @binding(0) var<storage, read_write> particles: array<Particle>;
@group(0) @binding(1) var<uniform> params: Params;
@group(0) @binding(2) var<storage, read_write> dead_list: DeadList;

fn hash(x: u32) -> f32 {
    var h = x;
    h = h ^ (h >> 16u);
    h = h * 0x7feb352du;
    h = h ^ (h >> 15u);
    h = h * 0x846ca68bu;
    h = h ^ (h >> 16u);
    return f32(h) / 4294967295.0;
}

// Convert axis-angle to quaternion
fn axis_angle_to_quat(axis: vec3<f32>, angle: f32) -> vec4<f32> {
    let half_angle = angle * 0.5;
    let s = sin(half_angle);
    return vec4<f32>(axis * s, cos(half_angle));
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let index = global_id.x;
    var p = particles[index];
    
    // Update lifetime
    p.lifetime += params.time_and_planes.x;
    
    // Move particle (velocity is in world space)
    p.position += p.velocity * params.time_and_planes.x;
    
    // Update rotation angle (stored in rotation.w)
    // Keep axis in rotation.xyz, angle in rotation.w
    p.rotation.w += p.angular_velocity * params.time_and_planes.x;
    
    // Wrap angle to keep it in reasonable range
    if p.rotation.w > 6.28318 {
        p.rotation.w -= 6.28318;
    } else if p.rotation.w < -6.28318 {
        p.rotation.w += 6.28318;
    }
    
    // Check if particle passed the near plane
    if p.position.z > params.time_and_planes.y {
        // Increment reset counter (stored in user_data.x for debugging)
        p.user_data.x += 1.0;
        
        // Create seed from index and current time
        let time_factor = u32(p.lifetime * 1000.0);
        let seed = (index * 73856093u) ^ (time_factor * 19349663u);
        
        // Generate new X,Y position
        var x = (hash(seed) * 2.0 - 1.0) * params.field_and_count.x;
        var y = (hash(seed + 1u) * 2.0 - 1.0) * params.field_and_count.x;
        
        // Ensure minimum distance from center
        let dist_sq = x * x + y * y;
        if dist_sq < params.field_and_count.y * params.field_and_count.y {
            let angle = hash(seed + 2u) * 6.28318;
            x = cos(angle) * params.field_and_count.y;
            y = sin(angle) * params.field_and_count.y;
        }
        
        // Reset to far plane with small random offset
        let z_randomness = hash(seed + 3u) * params.time_and_planes.w;
        let reset_z = -params.time_and_planes.z + z_randomness;
        p.position = vec3<f32>(x, y, reset_z);
        
        // Give it a new random rotation axis and angle
        let new_angle = hash(seed + 4u) * 6.28318;
        let axis_x = hash(seed + 5u) * 2.0 - 1.0;
        let axis_y = hash(seed + 6u) * 2.0 - 1.0;
        let axis_z = hash(seed + 7u) * 2.0 - 1.0;
        let axis = normalize(vec3<f32>(axis_x, axis_y, axis_z));
        
        // Store as [axis.xyz, angle]
        p.rotation = vec4<f32>(axis, new_angle);
    }
    
    particles[index] = p;
}
