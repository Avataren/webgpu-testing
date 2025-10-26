// src/gpu_particles/shaders/boids_optimized.wgsl
//
// Optimized boids simulation using spatial hash grid for O(n) neighbor queries.
//
// Instead of checking all N particles for each particle (O(n²)), we only check
// particles in nearby grid cells (typically 27 cells in 3D), reducing complexity
// to approximately O(n).

struct Params {
    radii: vec4<f32>,
    weights_and_speed: vec4<f32>,
    force_bounds_and_cell: vec4<f32>,
    grid_info: vec4<u32>,
}

struct CellData {
    start_index: u32,
    count: u32,
}

// NEW: Sorted particle data structure (output from radix sort)
struct ParticleGridData {
    cell_index: u32,
    particle_index: u32,
}

@group(0) @binding(0) var<storage, read_write> particles: array<Particle>;
@group(0) @binding(1) var<uniform> params: Params;
@group(0) @binding(2) var<storage, read_write> dead_list: DeadList;
@group(0) @binding(3) var<storage, read> spatial_grid: array<CellData>;
@group(0) @binding(4) var<storage, read> sorted_particle_data: array<ParticleGridData>;

// ============================================================================
// Spatial Grid Helper Functions
// ============================================================================

/// Convert 3D position to grid cell coordinates
fn pos_to_cell(pos: vec3<f32>) -> vec3<u32> {
    let offset_pos = pos + vec3<f32>(params.force_bounds_and_cell.y);
    let grid_pos = offset_pos / params.force_bounds_and_cell.z;
    return clamp(
        vec3<u32>(grid_pos),
        vec3<u32>(0u),
        params.grid_info.xyz - vec3<u32>(1u)
    );
}

/// Convert 3D cell coordinates to 1D grid index
fn cell_to_index(cell: vec3<u32>) -> u32 {
    return cell.x +
           cell.y * params.grid_info.x +
           cell.z * params.grid_info.x * params.grid_info.y;
}

/// Clamp cell coordinates to the valid grid range
fn clamp_cell_coords(cell: vec3<i32>) -> vec3<i32> {
    let max_dims = vec3<i32>(
        i32(params.grid_info.x) - 1,
        i32(params.grid_info.y) - 1,
        i32(params.grid_info.z) - 1,
    );
    return clamp(cell, vec3<i32>(0), max_dims);
}

/// Convert a world-space position to a clamped cell coordinate
fn pos_to_cell_clamped_i32(pos: vec3<f32>) -> vec3<i32> {
    let offset_pos = pos + vec3<f32>(params.force_bounds_and_cell.y);
    let grid_pos = offset_pos / params.force_bounds_and_cell.z;
    let floored = floor(grid_pos);
    return clamp_cell_coords(vec3<i32>(floored));
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

    let min_cell = pos_to_cell_clamped_i32(position - vec3<f32>(params.radii.y));
    let max_cell = pos_to_cell_clamped_i32(position + vec3<f32>(params.radii.y));

    // Search neighboring cells that overlap the interaction sphere
    for (var cx = min_cell.x; cx <= max_cell.x; cx++) {
        for (var cy = min_cell.y; cy <= max_cell.y; cy++) {
            for (var cz = min_cell.z; cz <= max_cell.z; cz++) {
                let neighbor_cell = vec3<i32>(cx, cy, cz);
                let grid_idx = cell_to_index(vec3<u32>(neighbor_cell));
                let cell_data = spatial_grid[grid_idx];

                // Check all particles in this cell
                for (var i = 0u; i < cell_data.count; i++) {
                    let particle_idx = sorted_particle_data[cell_data.start_index + i].particle_index;
                    if particle_idx == index { continue; }

                    let other_pos = particles[particle_idx].position;
                    let distance = length(position - other_pos);

                    if distance > 0.0 && distance < params.radii.y {
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
            steer = normalize(steer) * params.weights_and_speed.w;
        }
    }

    return steer;
}

/// Alignment: Steer towards average heading of nearby boids
fn alignment(index: u32, position: vec3<f32>, velocity: vec3<f32>) -> vec3<f32> {
    var sum = vec3<f32>(0.0);
    var count = 0u;

    let min_cell = pos_to_cell_clamped_i32(position - vec3<f32>(params.radii.z));
    let max_cell = pos_to_cell_clamped_i32(position + vec3<f32>(params.radii.z));

    for (var cx = min_cell.x; cx <= max_cell.x; cx++) {
        for (var cy = min_cell.y; cy <= max_cell.y; cy++) {
            for (var cz = min_cell.z; cz <= max_cell.z; cz++) {
                let neighbor_cell = vec3<i32>(cx, cy, cz);
                let grid_idx = cell_to_index(vec3<u32>(neighbor_cell));
                let cell_data = spatial_grid[grid_idx];

                for (var i = 0u; i < cell_data.count; i++) {
                    let particle_idx = sorted_particle_data[cell_data.start_index + i].particle_index;
                    if particle_idx == index { continue; }

                    let other_pos = particles[particle_idx].position;
                    let distance = length(position - other_pos);

                    if distance > 0.0 && distance < params.radii.z {
                        sum += particles[particle_idx].velocity;
                        count++;
                    }
                }
            }
        }
    }

    if count > 0u {
        sum /= f32(count);
        sum = normalize(sum) * params.weights_and_speed.w;
        return limit_magnitude(sum - velocity, params.force_bounds_and_cell.x);
    }

    return vec3<f32>(0.0);
}

/// Cohesion: Steer towards average position of nearby boids
fn cohesion(index: u32, position: vec3<f32>, velocity: vec3<f32>) -> vec3<f32> {
    var sum = vec3<f32>(0.0);
    var count = 0u;

    let min_cell = pos_to_cell_clamped_i32(position - vec3<f32>(params.radii.w));
    let max_cell = pos_to_cell_clamped_i32(position + vec3<f32>(params.radii.w));

    for (var cx = min_cell.x; cx <= max_cell.x; cx++) {
        for (var cy = min_cell.y; cy <= max_cell.y; cy++) {
            for (var cz = min_cell.z; cz <= max_cell.z; cz++) {
                let neighbor_cell = vec3<i32>(cx, cy, cz);
                let grid_idx = cell_to_index(vec3<u32>(neighbor_cell));
                let cell_data = spatial_grid[grid_idx];

                for (var i = 0u; i < cell_data.count; i++) {
                    let particle_idx = sorted_particle_data[cell_data.start_index + i].particle_index;
                    if particle_idx == index { continue; }

                    let other_pos = particles[particle_idx].position;
                    let distance = length(position - other_pos);

                    if distance > 0.0 && distance < params.radii.w {
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
        let desired_normalized = normalize(desired) * params.weights_and_speed.w;
        return limit_magnitude(desired_normalized - velocity, params.force_bounds_and_cell.x);
    }

    return vec3<f32>(0.0);
}

// ============================================================================
// Main Compute Shader
// ============================================================================

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let index = global_id.x;
    if index >= params.grid_info.w { return; }
    
    var p = particles[index];
    
    // Apply boid rules using spatial grid for fast neighbor queries
    var acceleration = vec3<f32>(0.0);
    
    let sep = separation(index, p.position);
    let ali = alignment(index, p.position, p.velocity);
    let coh = cohesion(index, p.position, p.velocity);
    
    acceleration += sep * params.weights_and_speed.x;
    acceleration += ali * params.weights_and_speed.y;
    acceleration += coh * params.weights_and_speed.z;
    
    // Boundary forces - soft repulsion from edges
    let margin = params.force_bounds_and_cell.y * 0.7;
    let edge_distance = params.force_bounds_and_cell.y - margin;
    
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
    let emergency_margin = params.force_bounds_and_cell.y * 0.95;
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

    // Hard clamp to bounds (safety net)
    p.position = clamp(p.position, vec3<f32>(-params.force_bounds_and_cell.y), vec3<f32>(params.force_bounds_and_cell.y));

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
