// src/gpu_particles/shaders/boids.wgsl

struct Params {
    radii: vec4<f32>,
    weights_and_speed: vec4<f32>,
    force_bounds_and_count: vec4<f32>,
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
    
    for (var i = 0u; i < u32(params.force_bounds_and_count.z); i++) {
        if i == index { continue; }
        
        let other_pos = particles[i].position;
        let distance = length(position - other_pos);
        
        if distance > 0.0 && distance < params.radii.y {
            var diff = position - other_pos;
            diff = normalize(diff) / distance;
            steer += diff;
            count++;
        }
    }
    
    if count > 0u {
        steer /= f32(count);
        if length(steer) > 0.0 {
            steer = normalize(steer) * params.weights_and_speed.w;
        }
    }
    
    return steer;
}

fn alignment(index: u32, position: vec3<f32>, velocity: vec3<f32>) -> vec3<f32> {
    var sum = vec3<f32>(0.0);
    var count = 0u;
    
    for (var i = 0u; i < u32(params.force_bounds_and_count.z); i++) {
        if i == index { continue; }
        
        let other_pos = particles[i].position;
        let distance = length(position - other_pos);
        
        if distance > 0.0 && distance < params.radii.z {
            sum += particles[i].velocity;
            count++;
        }
    }
    
    if count > 0u {
        sum /= f32(count);
        sum = normalize(sum) * params.weights_and_speed.w;
        return limit_magnitude(sum - velocity, params.force_bounds_and_count.x);
    }
    
    return vec3<f32>(0.0);
}

fn cohesion(index: u32, position: vec3<f32>, velocity: vec3<f32>) -> vec3<f32> {
    var sum = vec3<f32>(0.0);
    var count = 0u;
    
    for (var i = 0u; i < u32(params.force_bounds_and_count.z); i++) {
        if i == index { continue; }
        
        let other_pos = particles[i].position;
        let distance = length(position - other_pos);
        
        if distance > 0.0 && distance < params.radii.w {
            sum += other_pos;
            count++;
        }
    }
    
    if count > 0u {
        sum /= f32(count);
        let desired = sum - position;
        let desired_normalized = normalize(desired) * params.weights_and_speed.w;
        return limit_magnitude(desired_normalized - velocity, params.force_bounds_and_count.x);
    }
    
    return vec3<f32>(0.0);
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let index = global_id.x;
    if index >= u32(params.force_bounds_and_count.z) { return; }
    
    var p = particles[index];
    
    // Apply boid rules
    var acceleration = vec3<f32>(0.0);
    
    let sep = separation(index, p.position);
    let ali = alignment(index, p.position, p.velocity);
    let coh = cohesion(index, p.position, p.velocity);
    
    acceleration += sep * params.weights_and_speed.x;
    acceleration += ali * params.weights_and_speed.y;
    acceleration += coh * params.weights_and_speed.z;
    
    // Boundary forces
    let margin = params.force_bounds_and_count.y * 0.7;
    let edge_distance = params.force_bounds_and_count.y - margin;
    
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
    let emergency_margin = params.force_bounds_and_count.y * 0.95;
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
    p.velocity += acceleration * params.radii.x;
    p.velocity = limit_magnitude(p.velocity, params.weights_and_speed.w);
    p.position += p.velocity * params.radii.x;

    // Hard clamp to bounds
    p.position = clamp(p.position, vec3<f32>(-params.force_bounds_and_count.y), vec3<f32>(params.force_bounds_and_count.y));

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
