// examples/shaders/boids.wgsl - 3D Boids Simulation Shader

struct Boid {
    position: vec3<f32>,
    velocity: vec3<f32>,
}

struct Params {
    delta_time: f32,
    separation_radius: f32,
    alignment_radius: f32,
    cohesion_radius: f32,
    separation_weight: f32,
    alignment_weight: f32,
    cohesion_weight: f32,
    max_speed: f32,
    max_force: f32,
    bounds: f32,
    boid_count: u32,
}

@group(0) @binding(0) var<storage, read_write> boids: array<Boid>;
@group(0) @binding(1) var<uniform> params: Params;

// Clamp vector length
fn limit_magnitude(v: vec3<f32>, max_length: f32) -> vec3<f32> {
    let len = length(v);
    if len > max_length {
        return normalize(v) * max_length;
    }
    return v;
}

// Separation: steer to avoid crowding local flockmates
fn separation(index: u32, position: vec3<f32>) -> vec3<f32> {
    var steer = vec3<f32>(0.0);
    var count = 0u;
    
    for (var i = 0u; i < params.boid_count; i = i + 1u) {
        if i == index {
            continue;
        }
        
        let other_pos = boids[i].position;
        let distance = length(position - other_pos);
        
        if distance > 0.0 && distance < params.separation_radius {
            var diff = position - other_pos;
            diff = normalize(diff) / distance; // Weight by distance
            steer = steer + diff;
            count = count + 1u;
        }
    }
    
    if count > 0u {
        steer = steer / f32(count);
    }
    
    if length(steer) > 0.0 {
        steer = normalize(steer) * params.max_speed;
    }
    
    return steer;
}

// Alignment: steer towards the average heading of local flockmates
fn alignment(index: u32, position: vec3<f32>, velocity: vec3<f32>) -> vec3<f32> {
    var sum = vec3<f32>(0.0);
    var count = 0u;
    
    for (var i = 0u; i < params.boid_count; i = i + 1u) {
        if i == index {
            continue;
        }
        
        let other_pos = boids[i].position;
        let distance = length(position - other_pos);
        
        if distance > 0.0 && distance < params.alignment_radius {
            sum = sum + boids[i].velocity;
            count = count + 1u;
        }
    }
    
    if count > 0u {
        sum = sum / f32(count);
        sum = normalize(sum) * params.max_speed;
        let steer = sum - velocity;
        return limit_magnitude(steer, params.max_force);
    }
    
    return vec3<f32>(0.0);
}

// Cohesion: steer to move toward the average position of local flockmates
fn cohesion(index: u32, position: vec3<f32>, velocity: vec3<f32>) -> vec3<f32> {
    var sum = vec3<f32>(0.0);
    var count = 0u;
    
    for (var i = 0u; i < params.boid_count; i = i + 1u) {
        if i == index {
            continue;
        }
        
        let other_pos = boids[i].position;
        let distance = length(position - other_pos);
        
        if distance > 0.0 && distance < params.cohesion_radius {
            sum = sum + other_pos;
            count = count + 1u;
        }
    }
    
    if count > 0u {
        sum = sum / f32(count);
        return seek(sum, position, velocity);
    }
    
    return vec3<f32>(0.0);
}

// Seek a target position
fn seek(target_pos: vec3<f32>, position: vec3<f32>, velocity: vec3<f32>) -> vec3<f32> {
    let desired = target_pos - position;
    let desired_normalized = normalize(desired) * params.max_speed;
    let steer = desired_normalized - velocity;
    return limit_magnitude(steer, params.max_force);
}

// Keep boids within bounds
fn bounds_check(position: vec3<f32>, velocity: vec3<f32>) -> vec3<f32> {
    var steer = vec3<f32>(0.0);
    let margin = params.bounds * 0.9;
    let turn_factor = 0.5;
    
    if position.x < -margin {
        steer.x = steer.x + turn_factor;
    }
    if position.x > margin {
        steer.x = steer.x - turn_factor;
    }
    
    if position.y < -margin {
        steer.y = steer.y + turn_factor;
    }
    if position.y > margin {
        steer.y = steer.y - turn_factor;
    }
    
    if position.z < -margin {
        steer.z = steer.z + turn_factor;
    }
    if position.z > margin {
        steer.z = steer.z - turn_factor;
    }
    
    return steer;
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let index = global_id.x;
    
    if index >= params.boid_count {
        return;
    }
    
    let position = boids[index].position;
    var velocity = boids[index].velocity;
    
    // Apply boid rules
    var acceleration = vec3<f32>(0.0);
    
    let sep = separation(index, position);
    let ali = alignment(index, position, velocity);
    let coh = cohesion(index, position, velocity);
    let bnd = bounds_check(position, velocity);
    
    acceleration = acceleration + sep * params.separation_weight;
    acceleration = acceleration + ali * params.alignment_weight;
    acceleration = acceleration + coh * params.cohesion_weight;
    acceleration = acceleration + bnd;
    
    // Update velocity
    velocity = velocity + acceleration * params.delta_time;
    velocity = limit_magnitude(velocity, params.max_speed);
    
    // Update position
    var new_position = position + velocity * params.delta_time;
    
    // Hard boundary check (wrap around)
    if new_position.x < -params.bounds {
        new_position.x = params.bounds;
    }
    if new_position.x > params.bounds {
        new_position.x = -params.bounds;
    }
    if new_position.y < -params.bounds {
        new_position.y = params.bounds;
    }
    if new_position.y > params.bounds {
        new_position.y = -params.bounds;
    }
    if new_position.z < -params.bounds {
        new_position.z = params.bounds;
    }
    if new_position.z > params.bounds {
        new_position.z = -params.bounds;
    }
    
    // Write back results
    boids[index].position = new_position;
    boids[index].velocity = velocity;
}