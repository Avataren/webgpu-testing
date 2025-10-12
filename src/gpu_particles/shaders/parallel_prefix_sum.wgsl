// src/gpu_particles/shaders/parallel_prefix_sum.wgsl

struct CellData {
    start_index: u32,
    count: u32,
}

struct GridParams {
    bounds: f32,
    cell_size: f32,
    _padding1: vec2<u32>,
    grid_dimensions: vec3<u32>,  // vec3 in uniform = 16 bytes
    particle_count: u32,
    total_cells: u32,
    _padding2: vec2<u32>,         // Pad to 64 bytes
}

@group(0) @binding(0) var<storage, read> cell_counts: array<u32>;
@group(0) @binding(1) var<storage, read_write> spatial_grid: array<CellData>;
@group(0) @binding(2) var<uniform> params: GridParams;

var<workgroup> temp: array<u32, 256>;

@compute @workgroup_size(256)
fn compute_start_indices(@builtin(global_invocation_id) global_id: vec3<u32>,
                         @builtin(local_invocation_id) local_id: vec3<u32>) {
    let gid = global_id.x;
    let lid = local_id.x;
    
    if gid < params.total_cells {
        temp[lid] = cell_counts[gid];
    } else {
        temp[lid] = 0u;
    }
    workgroupBarrier();
    
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
    
    if lid == 0u {
        temp[255] = 0u;
    }
    workgroupBarrier();
    
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
    
    if gid < params.total_cells {
        spatial_grid[gid].start_index = temp[lid];
        spatial_grid[gid].count = cell_counts[gid];
    }
}