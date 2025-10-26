// src/gpu_particles/shaders/spatial_grid_build.wgsl
//
// First pass: Assigns each particle to a grid cell and counts particles per cell.
//
// This shader:
// 1. Converts particle 3D position to grid cell coordinates
// 2. Stores cell index and particle index for later sorting
// 3. Atomically increments the count for that cell
//
// The spatial hash grid divides 3D space into uniform cubic cells.
// Cell size is chosen as ~1.5x the largest interaction radius.

struct GridParams {
    bounds_and_cell: vec4<f32>,
    grid_info: vec4<u32>,
    totals: vec4<u32>,
}

struct ParticleGridData {
    cell_index: u32,
    particle_index: u32,
}

@group(0) @binding(0) var<storage, read> particles: array<Particle>;
@group(0) @binding(1) var<uniform> params: GridParams;
@group(0) @binding(2) var<storage, read_write> particle_grid_data: array<ParticleGridData>;
@group(0) @binding(3) var<storage, read_write> cell_counts: array<atomic<u32>>;

/// Convert 3D position to grid cell coordinates
fn pos_to_cell(pos: vec3<f32>) -> vec3<u32> {
    // Shift position from [-bounds, +bounds] to [0, 2*bounds]
    let offset_pos = pos + vec3<f32>(params.bounds_and_cell.x);
    
    // Divide by cell size to get grid coordinates
    let grid_pos = offset_pos / params.bounds_and_cell.y;
    
    // Clamp to valid grid range [0, grid_dimensions-1]
    return clamp(
        vec3<u32>(grid_pos),
        vec3<u32>(0u),
        params.grid_info.xyz - vec3<u32>(1u)
    );
}

/// Convert 3D cell coordinates to 1D grid index
/// Uses row-major ordering: index = x + y*width + z*width*height
fn cell_to_index(cell: vec3<u32>) -> u32 {
    return cell.x + 
           cell.y * params.grid_info.x + 
           cell.z * params.grid_info.x * params.grid_info.y;
}

@compute @workgroup_size(256)
fn compute_cell_indices(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let idx = global_id.x;
    if idx >= params.grid_info.w { return; }
    
    // Get particle position
    let pos = particles[idx].position;
    
    // Compute which cell this particle belongs to
    let cell = pos_to_cell(pos);
    let cell_idx = cell_to_index(cell);
    
    // Store cell and particle indices for later sorting
    particle_grid_data[idx].cell_index = cell_idx;
    particle_grid_data[idx].particle_index = idx;
    
    // Atomically increment count for this cell
    // Multiple particles may hash to the same cell, so we need atomics
    atomicAdd(&cell_counts[cell_idx], 1u);
}
