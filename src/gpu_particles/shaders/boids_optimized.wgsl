// src/gpu_particles/shaders/boids_optimized.wgsl
//
// Optimized boids simulation using spatial hash grid for O(n) neighbor queries.
//
// Instead of checking all N particles for each particle (O(n²)), we only check
// particles in nearby grid cells (typically 27 cells in 3D), reducing complexity
// to approximately O(n).

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
    cell_size: f32,
    grid_dimensions: vec3<u32>,
}

struct CellData {
    start_index: u32,
    count: u32,
}

@group(0) @binding(0) var<storage, read_write> particles: array<Particle>;
@group(0) @binding(1) var<uniform> params: Params;
@group(0) @binding(2) var<storage, read> spatial_grid: array<CellData>;
@group(0) @binding(3) var<storage, read> sorted_indices: array<u32>;

// ============================================================================
// Spatial Grid Helper Functions
// ============================================================================

/// Convert 3D position to grid cell coordinates
fn pos_to_cell(pos: vec3<f32>) -> vec3<u32> {
    let offset_pos = pos + vec3<f32>(params.bounds);
    let grid_pos = offset_pos / params.cell_size;
    return clamp(
        vec3<u32>(grid_pos),
        vec3<u32>(0u),
        params.grid_dimensions - vec3<u32>(1u)
    );
}

/// Convert 3D cell coordinates to 1D grid index
fn cell_to_index(cell: vec3<u32>) -> u32 {
    return cell.x + 
           cell.y * params.grid_dimensions.x + 
           cell.z * params.grid_dimensions.x * params.grid_dimensions.y;
}

/// Clamp magnitude of a vector
fn limit_magnitude(v: vec3<f32>, max_length: f32) -> vec3<f32> {
    let len = length(v);
    if len > max_length {
        return normalize(v) * max_length;
    }
    return v;
}

// ============================================================================
// Boid Rules (Optimized with Spatial Grid)
// ============================================================================

/// Separation: Steer away from nearby boids
fn separation(index: u32, position: vec3<f32>) -> vec3<f32> {
    var steer = vec3<f32>(0.0);
    var count = 0u;
    
    let cell = pos_to_cell(position);
    let search_radius = u32(ceil(params.separation_radius / params.cell_size));
    
    // Search neighboring cells only (typically 27 cells in 3D)
    for (var dx = -i32(search_radius); dx <= i32(search_radius); dx++) {
        for (var dy = -i32(search_radius); dy <= i32(search_radius); dy++) {
            for (var dz = -i32(search_radius); dz <= i32(search_radius); dz++) {
                let neighbor_cell = vec3<i32>(cell) + vec3<i32>(dx, dy, dz);
                
                // Check if neighbor cell is within grid bounds
                if neighbor_cell.x < 0 || neighbor_cell.y < 0 || neighbor_cell.z < 0 ||
                   neighbor_cell.x >= i32(params.grid_dimensions.x) ||
                   neighbor_cell.y >= i32(params.grid_dimensions.y) ||
                   neighbor_cell.z >= i32(params.grid_dimensions.z) {
                    continue;
                }
                
                let grid_idx = cell_to_index(vec3<u32>(neighbor_cell));
                let cell_data = spatial_grid[grid_idx];
                
                // Check all particles in this cell
                for (var i = 0u; i < cell_data.count; i++) {
                    let particle_idx = sorted_indices[cell_data.start_index + i];
                    if particle_idx == index { continue; }
                    
                    let other_pos = particles[particle_idx].position;
                    let distance = length(position - other_pos);
                    
                    if distance > 0.0 && distance < params.separation_radius {
                        var diff = position - other_pos;
                        diff = normalize(diff) / distance;  // Weight by distance
                        steer += diff;
                        count++;
                    }
                }
            }
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

/// Alignment: Steer towards average heading of nearby boids
fn alignment(index: u32, position: vec3<f32>, velocity: vec3<f32>) -> vec3<f32> {
    var sum = vec3<f32>(0.0);
    var count = 0u;
    
    let cell = pos_to_cell(position);
    let search_radius = u32(ceil(params.alignment_radius / params.cell_size));
    
    for (var dx = -i32(search_radius); dx <= i32(search_radius); dx++) {
        for (var dy = -i32(search_radius); dy <= i32(search_radius); dy++) {
            for (var dz = -i32(search_radius); dz <= i32(search_radius); dz++) {
                let neighbor_cell = vec3<i32>(cell) + vec3<i32>(dx, dy, dz);
                
                if neighbor_cell.x < 0 || neighbor_cell.y < 0 || neighbor_cell.z < 0 ||
                   neighbor_cell.x >= i32(params.grid_dimensions.x) ||
                   neighbor_cell.y >= i32(params.grid_dimensions.y) ||
                   neighbor_cell.z >= i32(params.grid_dimensions.z) {
                    continue;
                }
                
                let grid_idx = cell_to_index(vec3<u32>(neighbor_cell));
                let cell_data = spatial_grid[grid_idx];
                
                for (var i = 0u; i < cell_data.count; i++) {
                    let particle_idx = sorted_indices[cell_data.start_index + i];
                    if particle_idx == index { continue; }
                    
                    let other_pos = particles[particle_idx].position;
                    let distance = length(position - other_pos);
                    
                    if distance > 0.0 && distance < params.alignment_radius {
                        sum += particles[particle_idx].velocity;
                        count++;
                    }
                }
            }
        }
    }
    
    if count > 0u {
        sum /= f32(count);
        sum = normalize(sum) * params.max_speed;
        return limit_magnitude(sum - velocity, params.max_force);
    }
    
    return vec3<f32>(0.0);
}

/// Cohesion: Steer towards average position of nearby boids
fn cohesion(index: u32, position: vec3<f32>, velocity: vec3<f32>) -> vec3<f32> {
    var sum = vec3<f32>(0.0);
    var count = 0u;
    
    let cell = pos_to_cell(position);
    let search_radius = u32(ceil(params.cohesion_radius / params.cell_size));
    
    for (var dx = -i32(search_radius); dx <= i32(search_radius); dx++) {
        for (var dy = -i32(search_radius); dy <= i32(search_radius); dy++) {
            for (var dz = -i32(search_radius); dz <= i32(search_radius); dz++) {
                let neighbor_cell = vec3<i32>(cell) + vec3<i32>(dx, dy, dz);
                
                if neighbor_cell.x < 0 || neighbor_cell.y < 0 || neighbor_cell.z < 0 ||
                   neighbor_cell.x >= i32(params.grid_dimensions.x) ||
                   neighbor_cell.y >= i32(params.grid_dimensions.y) ||
                   neighbor_cell.z >= i32(params.grid_dimensions.z) {
                    continue;
                }
                
                let grid_idx = cell_to_index(vec3<u32>(neighbor_cell));
                let cell_data = spatial_grid[grid_idx];
                
                for (var i = 0u; i < cell_data.count; i++) {
                    let particle_idx = sorted_indices[cell_data.start_index + i];
                    if particle_idx == index { continue; }
                    
                    let other_pos = particles[particle_idx].position;
                    let distance = length(position - other_pos);
                    
                    if distance > 0.0 && distance < params.cohesion_radius {
                        sum += other_pos;
                        count++;
                    }
                }
            }
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

// ============================================================================
// Main Compute Shader
// ============================================================================

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let index = global_id.x;
    if index >= params.particle_count { return; }
    
    var p = particles[index];
    
    // Apply boid rules using spatial grid for fast neighbor queries
    var acceleration = vec3<f32>(0.0);
    
    let sep = separation(index, p.position);
    let ali = alignment(index, p.position, p.velocity);
    let coh = cohesion(index, p.position, p.velocity);
    
    acceleration += sep * params.separation_weight;
    acceleration += ali * params.alignment_weight;
    acceleration += coh * params.cohesion_weight;
    
    // Boundary forces - soft repulsion from edges
    let margin = params.bounds * 0.7;
    let edge_distance = params.bounds - margin;
    
    // X boundaries
    if p.position.x < -margin {
        let dist = (-margin - p.position.x) / edge_distance;
        acceleration.x += pow(dist, 3.0) * 8.0;
    }
    if p.position.x > margin {
        let dist = (p.position.x - margin) / edge_distance;
        acceleration.x -= pow(dist, 3.0) * 8.0;
    }
    
    // Y boundaries
    if p.position.y < -margin {
        let dist = (-margin - p.position.y) / edge_distance;
        acceleration.y += pow(dist, 3.0) * 8.0;
    }
    if p.position.y > margin {
        let dist = (p.position.y - margin) / edge_distance;
        acceleration.y -= pow(dist, 3.0) * 8.0;
    }
    
    // Z boundaries
    if p.position.z < -margin {
        let dist = (-margin - p.position.z) / edge_distance;
        acceleration.z += pow(dist, 3.0) * 8.0;
    }
    if p.position.z > margin {
        let dist = (p.position.z - margin) / edge_distance;
        acceleration.z -= pow(dist, 3.0) * 8.0;
    }
    
    // Emergency boundary clamping - strong repulsion near absolute bounds
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

    // Hard clamp to bounds (safety net)
    p.position = clamp(p.position, vec3<f32>(-params.bounds), vec3<f32>(params.bounds));

    // Align boid orientation with velocity direction for instanced rendering
    let speed_sq = dot(p.velocity, p.velocity);
    if speed_sq > 1e-6 {
        let forward = normalize(p.velocity);
        let base_forward = vec3<f32>(0.0, 0.0, 1.0);
        let dot_value = clamp(dot(base_forward, forward), -1.0, 1.0);
        var axis = cross(base_forward, forward);
        let axis_length = length(axis);

        if axis_length < 1e-5 {
            // Parallel or anti-parallel
            axis = vec3<f32>(0.0, 1.0, 0.0);
            if dot_value > 0.0 {
                p.rotation = vec4<f32>(axis, 0.0);
            } else {
                p.rotation = vec4<f32>(axis, 3.14159265);
            }
        } else {
            // Normal case
            axis = axis / axis_length;
            let angle = acos(dot_value);
            p.rotation = vec4<f32>(axis, angle);
        }
    } else {
        // No movement, use identity rotation
        p.rotation = vec4<f32>(0.0, 1.0, 0.0, 0.0);
    }

    particles[index] = p;
}