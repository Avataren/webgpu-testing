// src/gpu_particles/shaders/radix_sort.wgsl
//
// GPU radix sort for particle reordering
// Sorts particles by their cell index in parallel without atomic contention

struct ParticleGridData {
    cell_index: u32,
    particle_index: u32,
}

struct SortParams {
    count: u32,
    bit_offset: u32,
}

const WORKGROUP_SIZE: u32 = 256u;
const RADIX: u32 = 16u;  // Sort 4 bits at a time (0-15)

/// Extract 4-bit digit from cell index at given bit offset
fn get_digit(value: u32, bit_offset: u32) -> u32 {
    return (value >> bit_offset) & 0xFu;
}

// ============================================================================
// COUNT PHASE
// ============================================================================

@group(0) @binding(0) var<storage, read> count_input_data: array<ParticleGridData>;
@group(0) @binding(1) var<storage, read_write> count_histogram: array<atomic<u32>>;
@group(0) @binding(2) var<uniform> count_params: SortParams;

// Shared memory for local histogram - MUST be atomic!
var<workgroup> local_histogram: array<atomic<u32>, 16>;

@compute @workgroup_size(256)
fn count_phase(
    @builtin(global_invocation_id) global_id: vec3<u32>,
    @builtin(local_invocation_id) local_id: vec3<u32>
) {
    let tid = local_id.x;
    let gid = global_id.x;
    
    // Initialize local histogram
    if tid < RADIX {
        atomicStore(&local_histogram[tid], 0u);
    }
    workgroupBarrier();
    
    // Count digits in this thread's elements
    if gid < count_params.count {
        let cell_idx = count_input_data[gid].cell_index;
        let digit = get_digit(cell_idx, count_params.bit_offset);
        atomicAdd(&local_histogram[digit], 1u);
    }
    workgroupBarrier();
    
    // Write local histogram to global
    if tid < RADIX {
        let local_count = atomicLoad(&local_histogram[tid]);
        if local_count > 0u {
            atomicAdd(&count_histogram[tid], local_count);
        }
    }
}

// ============================================================================
// PREFIX SUM PHASE
// ============================================================================

@group(0) @binding(0) var<storage, read_write> prefix_histogram: array<atomic<u32>>;

@compute @workgroup_size(16)
fn prefix_sum_phase(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let tid = global_id.x;
    if tid >= RADIX { return; }
    
    // Sequential exclusive prefix sum (only 16 elements, very fast)
    var sum = 0u;
    for (var i = 0u; i < tid; i++) {
        sum += atomicLoad(&prefix_histogram[i]);
    }
    
    // Store the exclusive prefix sum
    atomicStore(&prefix_histogram[tid], sum);
}

// ============================================================================
// SCATTER PHASE
// ============================================================================

@group(0) @binding(0) var<storage, read> scatter_input_data: array<ParticleGridData>;
@group(0) @binding(1) var<storage, read_write> scatter_output_data: array<ParticleGridData>;
@group(0) @binding(2) var<storage, read_write> scatter_histogram: array<atomic<u32>>;
@group(0) @binding(3) var<uniform> scatter_params: SortParams;

@compute @workgroup_size(256)
fn scatter_phase(
    @builtin(global_invocation_id) global_id: vec3<u32>
) {
    let gid = global_id.x;
    
    // Each thread scatters its element
    if gid < scatter_params.count {
        let data = scatter_input_data[gid];
        let digit = get_digit(data.cell_index, scatter_params.bit_offset);
        
        // Atomically get output position and increment for next element
        let output_pos = atomicAdd(&scatter_histogram[digit], 1u);
        scatter_output_data[output_pos] = data;
    }
}