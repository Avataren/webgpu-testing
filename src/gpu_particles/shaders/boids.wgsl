// src/gpu_particles/shaders/boids.wgsl

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
    particle_count: u32,
}

@group(0) @binding(0) var<storage, read_write> particles: array<Particle>;
@group(0) @binding(1) var<uniform> params: Params;
@group(0) @binding(2) var<storage, read_write> dead_list: DeadList;

fn limit_magnitude(v: vec3<f32>, max_length: f32) -> vec3<f32> {
    let len = length(v);
    if len > max_length {
        return normalize(v) * max_length;
    }
    return v;
}

fn separation(index: u32, position: vec3<f32>) -> vec3<f32> {
    var steer = vec3<f32>(0.0);
    var count = 0u;
    
    for (var i = 0u; i < params.particle_count; i++) {
        if i == index { continue; }
        
        let other_pos = particles[i].position;
        let distance = length(position - other_pos);
        
        if distance > 0.0 && distance < params.separation_radius {
            var diff = position - other_pos;
            diff = normalize(diff) / distance;
            steer += diff;
            count++;
        }
    }
    
    if count > 0u {
        steer /= f32(count);
        if length(steer) > 0.0 {
            steer = normalize(steer) * params.max_speed;
        }
    }
    
    return steer;
}

fn alignment(index: u32, position: vec3<f32>, velocity: vec3<f32>) -> vec3<f32> {
    var sum = vec3<f32>(0.0);
    var count = 0u;
    
    for (var i = 0u; i < params.particle_count; i++) {
        if i == index { continue; }
        
        let other_pos = particles[i].position;
        let distance = length(position - other_pos);
        
        if distance > 0.0 && distance < params.alignment_radius {
            sum += particles[i].velocity;
            count++;
        }
    }
    
    if count > 0u {
        sum /= f32(count);
        sum = normalize(sum) * params.max_speed;
        return limit_magnitude(sum - velocity, params.max_force);
    }
    
    return vec3<f32>(0.0);
}

fn cohesion(index: u32, position: vec3<f32>, velocity: vec3<f32>) -> vec3<f32> {
    var sum = vec3<f32>(0.0);
    var count = 0u;
    
    for (var i = 0u; i < params.particle_count; i++) {
        if i == index { continue; }
        
        let other_pos = particles[i].position;
        let distance = length(position - other_pos);
        
        if distance > 0.0 && distance < params.cohesion_radius {
            sum += other_pos;
            count++;
        }
    }
    
    if count > 0u {
        sum /= f32(count);
        let desired = sum - position;
        let desired_normalized = normalize(desired) * params.max_speed;
        return limit_magnitude(desired_normalized - velocity, params.max_force);
    }
    
    return vec3<f32>(0.0);
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let index = global_id.x;
    if index >= params.particle_count { return; }
    
    var p = particles[index];
    
    // Apply boid rules
    var acceleration = vec3<f32>(0.0);
    
    let sep = separation(index, p.position);
    let ali = alignment(index, p.position, p.velocity);
    let coh = cohesion(index, p.position, p.velocity);
    
    acceleration += sep * params.separation_weight;
    acceleration += ali * params.alignment_weight;
    acceleration += coh * params.cohesion_weight;
    
    // Boundary forces
    let margin = params.bounds * 0.7;
    let edge_distance = params.bounds - margin;
    
    if p.position.x < -margin {
        let dist = (-margin - p.position.x) / edge_distance;
        acceleration.x += pow(dist, 3.0) * 8.0;
    }
    if p.position.x > margin {
        let dist = (p.position.x - margin) / edge_distance;
        acceleration.x -= pow(dist, 3.0) * 8.0;
    }
    
    if p.position.y < -margin {
        let dist = (-margin - p.position.y) / edge_distance;
        acceleration.y += pow(dist, 3.0) * 8.0;
    }
    if p.position.y > margin {
        let dist = (p.position.y - margin) / edge_distance;
        acceleration.y -= pow(dist, 3.0) * 8.0;
    }
    
    if p.position.z < -margin {
        let dist = (-margin - p.position.z) / edge_distance;
        acceleration.z += pow(dist, 3.0) * 8.0;
    }
    if p.position.z > margin {
        let dist = (p.position.z - margin) / edge_distance;
        acceleration.z -= pow(dist, 3.0) * 8.0;
    }
    
    // Emergency boundary clamping
    let emergency_margin = params.bounds * 0.95;
    if abs(p.position.x) > emergency_margin {
        acceleration.x -= sign(p.position.x) * 15.0;
    }
    if abs(p.position.y) > emergency_margin {
        acceleration.y -= sign(p.position.y) * 15.0;
    }
    if abs(p.position.z) > emergency_margin {
        acceleration.z -= sign(p.position.z) * 15.0;
    }
    
    // Update velocity and position
    p.velocity += acceleration * params.delta_time;
    p.velocity = limit_magnitude(p.velocity, params.max_speed);
    p.position += p.velocity * params.delta_time;

    // Hard clamp to bounds
    p.position = clamp(p.position, vec3<f32>(-params.bounds), vec3<f32>(params.bounds));

    // Align orientation with velocity direction for instanced rendering
    let speed_sq = dot(p.velocity, p.velocity);
    if speed_sq > 1e-6 {
        let forward = normalize(p.velocity);
        let base_forward = vec3<f32>(0.0, 0.0, 1.0);
        let dot_value = clamp(dot(base_forward, forward), -1.0, 1.0);
        var axis = cross(base_forward, forward);
        let axis_length = length(axis);

        if axis_length < 1e-5 {
            axis = vec3<f32>(0.0, 1.0, 0.0);
            if dot_value > 0.0 {
                p.rotation = vec4<f32>(axis, 0.0);
            } else {
                p.rotation = vec4<f32>(axis, 3.14159265);
            }
        } else {
            axis = axis / axis_length;
            let angle = acos(dot_value);
            p.rotation = vec4<f32>(axis, angle);
        }
    } else {
        p.rotation = vec4<f32>(0.0, 1.0, 0.0, 0.0);
    }

    particles[index] = p;
}
