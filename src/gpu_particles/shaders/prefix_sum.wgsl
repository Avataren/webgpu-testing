// src/gpu_particles/shaders/prefix_sum.wgsl
//
// Computes prefix sum (cumulative sum) to determine start indices for each grid cell.
// 
// After counting particles per cell, we need to know where each cell's particles
// begin in the sorted array. A prefix sum gives us these start indices.
//
// Example:
// Cell counts:     [3, 0, 2, 1, 0, 4]
// Prefix sum:      [0, 3, 3, 5, 6, 6]  <- These are the start indices
//
// Note: This is a naive sequential implementation. For large grids (>100k cells),
// consider implementing a parallel prefix sum algorithm like Blelloch scan.

struct CellData {
    start_index: u32,
    count: u32,
}

@group(0) @binding(0) var<storage, read> cell_counts: array<u32>;
@group(0) @binding(1) var<storage, read_write> spatial_grid: array<CellData>;
@group(0) @binding(2) var<uniform> total_cells: u32;

// @compute @workgroup_size(256)
// fn compute_start_indices(@builtin(global_invocation_id) global_id: vec3<u32>) {
//     let idx = global_id.x;
//     if idx >= total_cells { return; }
    
//     // Compute prefix sum: each cell's start index is the sum of all previous counts
//     var prefix_sum = 0u;
//     for (var i = 0u; i < idx; i++) {
//         prefix_sum += cell_counts[i];
//     }
    
//     spatial_grid[idx].start_index = prefix_sum;
//     spatial_grid[idx].count = cell_counts[idx];
// }

//Simplified atomic-based prefix sum for small grids
@compute @workgroup_size(1)  // Single thread!
fn compute_start_indices(@builtin(global_invocation_id) global_id: vec3<u32>) {
    // Only one thread runs this
    if global_id.x > 0u { return; }
    
    var running_sum = 0u;
    for (var i = 0u; i < arrayLength(&cell_counts); i++) {
        spatial_grid[i].start_index = running_sum;
        spatial_grid[i].count = cell_counts[i];
        running_sum += cell_counts[i];
    }
}