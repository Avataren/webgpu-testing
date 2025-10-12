// src/gpu_particles/shaders/parallel_prefix_sum.wgsl
//
// Parallel prefix sum using work-efficient scan (simplified Blelloch)
// Much faster than the naive sequential version: O(log n) vs O(n²)

struct CellData {
    start_index: u32,
    count: u32,
}

@group(0) @binding(0) var<storage, read> cell_counts: array<u32>;
@group(0) @binding(1) var<storage, read_write> spatial_grid: array<CellData>;
@group(0) @binding(2) var<uniform> total_cells: u32;

// Shared memory for workgroup-level reduction
var<workgroup> temp: array<u32, 256>;

@compute @workgroup_size(256)
fn compute_start_indices(@builtin(global_invocation_id) global_id: vec3<u32>,
                         @builtin(local_invocation_id) local_id: vec3<u32>) {
    let gid = global_id.x;
    let lid = local_id.x;
    
    // Load into shared memory
    if gid < total_cells {
        temp[lid] = cell_counts[gid];
    } else {
        temp[lid] = 0u;
    }
    workgroupBarrier();
    
    // Up-sweep (reduce) phase
    var stride = 1u;
    for (var d = 128u; d > 0u; d >>= 1u) {
        if lid < d {
            let ai = stride * (2u * lid + 1u) - 1u;
            let bi = stride * (2u * lid + 2u) - 1u;
            temp[bi] += temp[ai];
        }
        stride *= 2u;
        workgroupBarrier();
    }
    
    // Clear last element (for exclusive scan)
    if lid == 0u {
        temp[255] = 0u;
    }
    workgroupBarrier();
    
    // Down-sweep phase
    for (var d = 1u; d < 256u; d *= 2u) {
        stride >>= 1u;
        if lid < d {
            let ai = stride * (2u * lid + 1u) - 1u;
            let bi = stride * (2u * lid + 2u) - 1u;
            let t = temp[ai];
            temp[ai] = temp[bi];
            temp[bi] += t;
        }
        workgroupBarrier();
    }
    
    // Write results
    if gid < total_cells {
        spatial_grid[gid].start_index = temp[lid];
        spatial_grid[gid].count = cell_counts[gid];
    }
}