// src/gpu_particles/shaders/reorder.wgsl
//
// Reorders particles into a sorted array based on their grid cell assignment.
//
// After computing cell indices and start indices, this shader places each particle
// into the correct position in the sorted array. Particles in the same cell are
// stored contiguously for cache-coherent neighbor queries.
//
// Example:
// Particle 0 -> Cell 2 -> Placed at sorted_indices[cell_2_start + offset]
// Particle 1 -> Cell 0 -> Placed at sorted_indices[cell_0_start + offset]
// Particle 2 -> Cell 2 -> Placed at sorted_indices[cell_2_start + offset]
// ...
//
// The atomic offset ensures multiple particles in the same cell don't collide.

struct ParticleGridData {
    cell_index: u32,
    particle_index: u32,
}

struct CellData {
    start_index: u32,
    count: u32,
}

@group(0) @binding(0) var<storage, read> particle_grid_data: array<ParticleGridData>;
@group(0) @binding(1) var<storage, read> spatial_grid: array<CellData>;
@group(0) @binding(2) var<storage, read_write> sorted_indices: array<u32>;
@group(0) @binding(3) var<storage, read_write> cell_offsets: array<atomic<u32>>;
@group(0) @binding(4) var<uniform> particle_count: u32;

@compute @workgroup_size(256)
fn reorder_particles(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let idx = global_id.x;
    if idx >= particle_count { return; }
    
    let cell_idx = particle_grid_data[idx].cell_index;
    let particle_idx = particle_grid_data[idx].particle_index;
    
    let cell = spatial_grid[cell_idx];
    
    // Use atomic add to get a unique offset within the cell
    // Multiple threads may write to the same cell, so we need atomics
    let offset = atomicAdd(&cell_offsets[cell_idx], 1u);
    
    // Write particle index to sorted array at: cell_start + offset
    sorted_indices[cell.start_index + offset] = particle_idx;
}