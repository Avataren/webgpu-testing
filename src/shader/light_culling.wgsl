// src/shader/light_culling.wgsl
// Sorts the point and spot lights into the clusters of the camera's view and the cells
// of a world-space grid (src/renderer/light_culling.rs). One invocation per cluster or
// cell: it tests every light against its box (lights are loaded 64 at a time into
// workgroup memory, and only those that reach the workgroup's 64 boxes together are
// tested one by one), takes room for the lights it found from the shared pool with one
// atomic add, and writes their indices there in light order. Lit passes then loop over
// a surface's cluster (lighting_common.wgsl, light_cluster) and ray-traced passes over
// a hit's grid cell (light_grid_cell) instead of over every light.
//
// light_lists layout (u32): [0] the pool's fill (it keeps counting past the capacity,
// so the renderer can grow the pool), [1] lights left out for lack of room, then one
// (offset, count) pair per cluster from offsets.y, one per grid cell from offsets.z, and
// the pool from offsets.w. A cluster lists at most MAX_LIGHTS_PER_CLUSTER lights.

// Mirrors `CullUniform` in src/renderer/light_culling.rs.
struct CullParams {
    // World to view space of the camera (without TAA jitter), and its inverse projection.
    view : mat4x4<f32>,
    inverse_projection : mat4x4<f32>,
    // Clusters along x, y and depth, and their count.
    cluster_dims : vec4<u32>,
    // Depth slices as in lighting_common.wgsl (cluster_slices): x scale, y bias, z 1 for
    // linear (orthographic) slices, w the camera's far plane.
    slices : vec4<f32>,
    // The world grid's lowest corner (xyz) and cell size (w).
    grid_origin : vec4<f32>,
    // Grid cells along x, y and z, and their count (0: no grid this frame).
    grid_dims : vec4<u32>,
    // x: local lights, y: the first cluster entry, z: the first grid entry, w: the pool.
    offsets : vec4<u32>,
    // x: the pool's capacity, y: workgroups that handle clusters (the rest do cells),
    // z: the camera's near plane.
    pool : vec4<f32>,
};

// As `LocalLight` in lighting_common.wgsl.
struct CullLight {
    position_range : vec4<f32>,
    color_intensity : vec4<f32>,
    direction : vec4<f32>,
    params : vec4<f32>,
};

@group(0) @binding(0) var<uniform> params : CullParams;
// The lights buffer from its local lights on (LOCAL_LIGHTS_OFFSET).
@group(0) @binding(1) var<storage, read> cull_lights : array<CullLight>;
@group(0) @binding(2) var<storage, read_write> lists : array<atomic<u32>>;

const CULL_WORKGROUP : u32 = 64u;
const MAX_PER_CLUSTER : u32 = MAX_LIGHTS_PER_CLUSTER_VALUE;

// A batch of lights: centre and reach, then the cone axis and the cosine of its outer
// angle (below -1: no cone), in view space for clusters and world space for cells.
var<workgroup> batch_spheres : array<vec4<f32>, CULL_WORKGROUP>;
var<workgroup> batch_cones : array<vec4<f32>, CULL_WORKGROUP>;
// The batch's lights that reach the workgroup's box, one bit per slot.
var<workgroup> batch_mask : array<atomic<u32>, 2>;
// The workgroup's boxes, and the box around them all.
var<workgroup> boxes_lo : array<vec3<f32>, CULL_WORKGROUP>;
var<workgroup> boxes_hi : array<vec3<f32>, CULL_WORKGROUP>;
var<workgroup> group_lo : vec3<f32>;
var<workgroup> group_hi : vec3<f32>;

// The view-space point at view depth `depth` on the line through two unprojected
// points (straight through the eye for a perspective camera, parallel for an
// orthographic one).
fn cull_point_at_depth(a : vec3<f32>, b : vec3<f32>, depth : f32) -> vec3<f32> {
    let da = -a.z;
    let db = -b.z;
    let t = select(0.0, (depth - da) / (db - da), abs(db - da) > 1e-6);
    return mix(a, b, t);
}

fn cull_unproject(ndc : vec3<f32>) -> vec3<f32> {
    let p = params.inverse_projection * vec4<f32>(ndc, 1.0);
    return p.xyz / p.w;
}

// The view depth where slice `slice` starts.
fn cull_slice_depth(slice : f32) -> f32 {
    let value = (slice - params.slices.y) / params.slices.x;
    return select(exp(value), value, params.slices.z > 0.5);
}

struct CullBox {
    lo : vec3<f32>,
    hi : vec3<f32>,
};

// Cluster `index`'s box in view space: its tile of the screen between its slice's depths
// (the first slice from the near plane, the last to the far plane).
fn cluster_box(index : u32) -> CullBox {
    let dims = params.cluster_dims;
    let x = index % dims.x;
    let y = (index / dims.x) % dims.y;
    let slice = index / (dims.x * dims.y);
    var near = cull_slice_depth(f32(slice));
    var far = cull_slice_depth(f32(slice + 1u));
    if slice == 0u {
        near = params.pool.z;
    }
    if slice + 1u >= dims.z {
        far = params.slices.w;
    }
    // Tiles run left to right and top to bottom (+y down, as the fragment's pixel).
    let x0 = -1.0 + 2.0 * f32(x) / f32(dims.x);
    let x1 = -1.0 + 2.0 * f32(x + 1u) / f32(dims.x);
    let y0 = 1.0 - 2.0 * f32(y + 1u) / f32(dims.y);
    let y1 = 1.0 - 2.0 * f32(y) / f32(dims.y);
    var lo = vec3<f32>(1.0e30);
    var hi = vec3<f32>(-1.0e30);
    for (var corner = 0u; corner < 4u; corner = corner + 1u) {
        let ndc = vec2<f32>(select(x0, x1, (corner & 1u) != 0u), select(y0, y1, (corner & 2u) != 0u));
        let a = cull_unproject(vec3<f32>(ndc, 0.0));
        let b = cull_unproject(vec3<f32>(ndc, 1.0));
        let p0 = cull_point_at_depth(a, b, near);
        let p1 = cull_point_at_depth(a, b, far);
        lo = min(lo, min(p0, p1));
        hi = max(hi, max(p0, p1));
    }
    return CullBox(lo, hi);
}

// Grid cell `index`'s box in world space.
fn cell_box(index : u32) -> CullBox {
    let dims = params.grid_dims;
    let cell = vec3<u32>(index % dims.x, (index / dims.x) % dims.y, index / (dims.x * dims.y));
    let lo = params.grid_origin.xyz + vec3<f32>(cell) * params.grid_origin.w;
    return CullBox(lo, lo + vec3<f32>(params.grid_origin.w));
}

// Whether light `j` of the batch can reach box `b`: its sphere touches the box and, for a
// spot light, its cone touches the box's bounding sphere (Wronski, "Cull that cone!").
fn cull_light_hits(j : u32, b : CullBox) -> bool {
    let sphere = batch_spheres[j];
    let nearest = clamp(sphere.xyz, b.lo, b.hi);
    let offset = sphere.xyz - nearest;
    if dot(offset, offset) > sphere.w * sphere.w {
        return false;
    }
    let cone = batch_cones[j];
    if cone.w < -1.5 {
        return true;
    }
    let center = (b.lo + b.hi) * 0.5;
    let radius = length(b.hi - b.lo) * 0.5;
    let v = center - sphere.xyz;
    let along = dot(v, cone.xyz);
    let sin_outer = sqrt(max(1.0 - cone.w * cone.w, 0.0));
    let across = sqrt(max(dot(v, v) - along * along, 0.0));
    let beside = cone.w * across - along * sin_outer;
    return !(beside > radius || along > radius + sphere.w || along < -radius);
}

// Load light `index` (if there is one) into batch slot `slot`, and flag it in
// `batch_mask` when its sphere reaches the workgroup's box.
fn cull_load(index : u32, slot : u32, grid : bool) {
    if index >= params.offsets.x {
        return;
    }
    let light = cull_lights[index];
    var center = light.position_range.xyz;
    var axis = light.direction.xyz;
    if !grid {
        center = (params.view * vec4<f32>(center, 1.0)).xyz;
        axis = (params.view * vec4<f32>(axis, 0.0)).xyz;
    }
    let reach = light.position_range.w;
    batch_spheres[slot] = vec4<f32>(center, reach);
    batch_cones[slot] = vec4<f32>(axis, light.direction.w);
    let offset = center - clamp(center, group_lo, group_hi);
    if dot(offset, offset) <= reach * reach {
        atomicOr(&batch_mask[slot / 32u], 1u << (slot % 32u));
    }
}

// The flagged lights of the batch reaching box `b`.
fn cull_count_batch(b : CullBox) -> u32 {
    var count = 0u;
    for (var half = 0u; half < 2u; half = half + 1u) {
        var mask = atomicLoad(&batch_mask[half]);
        while mask != 0u {
            let j = half * 32u + countTrailingZeros(mask);
            mask = mask & (mask - 1u);
            if cull_light_hits(j, b) {
                count = count + 1u;
            }
        }
    }
    return count;
}

@compute @workgroup_size(64)
fn cs_cull(
    @builtin(workgroup_id) group : vec3<u32>,
    @builtin(local_invocation_index) local : u32,
) {
    let cluster_groups = u32(params.pool.y);
    let grid = group.x >= cluster_groups;
    var index : u32;
    var entry : u32;
    var in_use : bool;
    var b : CullBox;
    if grid {
        index = (group.x - cluster_groups) * CULL_WORKGROUP + local;
        in_use = index < params.grid_dims.w;
        entry = params.offsets.z + 2u * index;
        b = cell_box(index);
    } else {
        index = group.x * CULL_WORKGROUP + local;
        in_use = index < params.cluster_dims.w;
        entry = params.offsets.y + 2u * index;
        b = cluster_box(index);
    }
    let light_count = params.offsets.x;

    // The box around the workgroup's boxes, which lights must reach to be tested
    // against each of them.
    boxes_lo[local] = select(vec3<f32>(1.0e30), b.lo, in_use);
    boxes_hi[local] = select(vec3<f32>(-1.0e30), b.hi, in_use);
    workgroupBarrier();
    if local == 0u {
        var lo = boxes_lo[0];
        var hi = boxes_hi[0];
        for (var i = 1u; i < CULL_WORKGROUP; i = i + 1u) {
            lo = min(lo, boxes_lo[i]);
            hi = max(hi, boxes_hi[i]);
        }
        group_lo = lo;
        group_hi = hi;
    }

    // Count the lights reaching the box.
    var count = 0u;
    for (var base = 0u; base < light_count; base = base + CULL_WORKGROUP) {
        if local < 2u {
            atomicStore(&batch_mask[local], 0u);
        }
        workgroupBarrier();
        cull_load(base + local, local, grid);
        workgroupBarrier();
        if in_use {
            count = count + cull_count_batch(b);
        }
        workgroupBarrier();
    }
    count = min(count, MAX_PER_CLUSTER);

    // Take room for them in the pool; what does not fit is left out (and counted).
    let capacity = u32(params.pool.x);
    var stored = 0u;
    var offset = 0u;
    if in_use && count > 0u {
        offset = atomicAdd(&lists[0], count);
        if offset < capacity {
            stored = min(count, capacity - offset);
        }
        if stored < count {
            atomicAdd(&lists[1], count - stored);
        }
    }

    // Write their indices, in light order.
    var written = 0u;
    for (var base = 0u; base < light_count; base = base + CULL_WORKGROUP) {
        if local < 2u {
            atomicStore(&batch_mask[local], 0u);
        }
        workgroupBarrier();
        cull_load(base + local, local, grid);
        workgroupBarrier();
        if in_use && written < stored {
            for (var half = 0u; half < 2u; half = half + 1u) {
                var mask = atomicLoad(&batch_mask[half]);
                while mask != 0u && written < stored {
                    let j = half * 32u + countTrailingZeros(mask);
                    mask = mask & (mask - 1u);
                    if cull_light_hits(j, b) {
                        atomicStore(&lists[params.offsets.w + offset + written], base + j);
                        written = written + 1u;
                    }
                }
            }
        }
        workgroupBarrier();
    }
    if in_use {
        atomicStore(&lists[entry], params.offsets.w + offset);
        atomicStore(&lists[entry + 1u], stored);
    }
}
