// Local exposure: a bilateral grid (Chen, Paris and Durand 2007; what Unreal's local
// exposure uses) of the image's exposed log brightness. Each grid cell covers a block of
// pixels and splits them into brightness bins, so after a blur the grid holds, for every
// spot and brightness, the average brightness of the nearby pixels about that bright. The
// composite pass reads it at each pixel (`apply_local_exposure`) as the brightness of the
// region the pixel belongs to, pulls that toward middle grey and keeps the pixel's detail
// around it: a dark corner opens up while the sunlit table beside it keeps its contrast,
// and the edge between them stays sharp because the two sit in different bins.
//
// Passes: build (one workgroup per cell, every other pixel each way) into grid buffer A,
// blur along x (A to B), along y (B to A), then along the bins into a 3D texture the
// composite pass samples with trilinear filtering. Cells hold the sum of log brightness
// and the pixel count (both per sampled pixel of a full cell), so a blur averages the
// region's brightness and empty cells and bins add nothing.

// Brightness bins per cell.
const LOCAL_EXPOSURE_BINS : u32 = 32u;
// The grid's largest size in cells along x and y (src/renderer/postprocess/local_exposure.rs).
const LOCAL_EXPOSURE_MAX_CELLS : u32 = 56u;
// Exposed log2 luminance the bins span; middle grey is log2(0.18), about -2.5.
const LOCAL_EXPOSURE_MIN_LOG : f32 = -14.0;
const LOCAL_EXPOSURE_MAX_LOG : f32 = 6.0;
// Fixed-point scale for summing log brightness with integer atomics.
const LOCAL_EXPOSURE_FIXED : f32 = 256.0;

@group(1) @binding(113)
var local_exposure_scene : texture_2d<f32>;
@group(1) @binding(114)
var<storage, read> local_exposure_in : array<vec2<f32>>;
@group(1) @binding(115)
var<storage, read_write> local_exposure_out : array<vec2<f32>>;
@group(1) @binding(116)
var local_exposure_grid : texture_storage_3d<rgba16float, write>;
@group(1) @binding(117)
var<storage, read> local_exposure_state : ExposureState;

var<workgroup> local_exposure_sums : array<atomic<u32>, LOCAL_EXPOSURE_BINS>;
var<workgroup> local_exposure_counts : array<atomic<u32>, LOCAL_EXPOSURE_BINS>;

// Cells in use along x and y.
fn local_exposure_cells() -> vec2<u32> {
    return vec2<u32>(post_uniform.local_exposure_grid.yz);
}

fn local_exposure_index(cell : vec3<u32>) -> u32 {
    return (cell.z * LOCAL_EXPOSURE_MAX_CELLS + cell.y) * LOCAL_EXPOSURE_MAX_CELLS + cell.x;
}

// Where `log_luminance` falls along the bins, 0 to 1.
fn local_exposure_depth(log_luminance : f32) -> f32 {
    return (log_luminance - LOCAL_EXPOSURE_MIN_LOG)
        / (LOCAL_EXPOSURE_MAX_LOG - LOCAL_EXPOSURE_MIN_LOG);
}

@compute @workgroup_size(8, 8, 1)
fn cs_local_exposure_build(
    @builtin(workgroup_id) group : vec3<u32>,
    @builtin(local_invocation_id) local : vec3<u32>,
    @builtin(local_invocation_index) local_index : u32,
) {
    if local_index < LOCAL_EXPOSURE_BINS {
        atomicStore(&local_exposure_sums[local_index], 0u);
        atomicStore(&local_exposure_counts[local_index], 0u);
    }
    workgroupBarrier();

    let cell_size = u32(post_uniform.local_exposure_grid.x);
    let dims = vec2<f32>(textureDimensions(local_exposure_scene, 0));
    let origin = vec2<i32>(post_uniform.viewport_offset * dims);
    let size = vec2<u32>(post_uniform.resolution);
    let exposure = exposure_from_state(local_exposure_state);
    let start = group.xy * cell_size;
    // Every other pixel each way, like the exposure histogram: the grid is coarse anyway.
    for (var y = local.y * 2u; y < cell_size; y = y + 16u) {
        for (var x = local.x * 2u; x < cell_size; x = x + 16u) {
            let p = start + vec2<u32>(x, y);
            if any(p >= size) {
                continue;
            }
            let color = textureLoad(local_exposure_scene, origin + vec2<i32>(p), 0).rgb;
            let luminance = luminance_rec709(max(color, vec3<f32>(0.0))) * exposure;
            if luminance != luminance {
                continue;
            }
            let l = clamp(
                log2(max(luminance, 1e-10)),
                LOCAL_EXPOSURE_MIN_LOG,
                LOCAL_EXPOSURE_MAX_LOG,
            );
            let bin = min(
                u32(local_exposure_depth(l) * f32(LOCAL_EXPOSURE_BINS)),
                LOCAL_EXPOSURE_BINS - 1u,
            );
            let fixed = u32((l - LOCAL_EXPOSURE_MIN_LOG) * LOCAL_EXPOSURE_FIXED + 0.5);
            atomicAdd(&local_exposure_sums[bin], fixed);
            atomicAdd(&local_exposure_counts[bin], 1u);
        }
    }
    workgroupBarrier();

    if local_index < LOCAL_EXPOSURE_BINS {
        let count = f32(atomicLoad(&local_exposure_counts[local_index]));
        let sum = f32(atomicLoad(&local_exposure_sums[local_index])) / LOCAL_EXPOSURE_FIXED
            + LOCAL_EXPOSURE_MIN_LOG * count;
        // Per sampled pixel of a full cell, so the values suit the half-float grid texture.
        let side = f32((cell_size + 1u) / 2u);
        local_exposure_out[local_exposure_index(vec3<u32>(group.xy, local_index))] =
            vec2<f32>(sum, count) / (side * side);
    }
}

// A cell of the input grid; nothing outside the cells in use.
fn local_exposure_tap(cell : vec3<i32>) -> vec2<f32> {
    let cells = vec3<i32>(vec2<i32>(local_exposure_cells()), i32(LOCAL_EXPOSURE_BINS));
    if any(cell < vec3<i32>(0)) || any(cell >= cells) {
        return vec2<f32>(0.0);
    }
    return local_exposure_in[local_exposure_index(vec3<u32>(cell))];
}

// Binomial blur across cells along `axis`: 9 taps, a sigma of about 1.4 cells.
fn local_exposure_blur_cells(cell : vec3<i32>, axis : vec3<i32>) -> vec2<f32> {
    let sum = 70.0 * local_exposure_tap(cell)
        + 56.0 * (local_exposure_tap(cell - axis) + local_exposure_tap(cell + axis))
        + 28.0 * (local_exposure_tap(cell - 2 * axis) + local_exposure_tap(cell + 2 * axis))
        + 8.0 * (local_exposure_tap(cell - 3 * axis) + local_exposure_tap(cell + 3 * axis))
        + local_exposure_tap(cell - 4 * axis) + local_exposure_tap(cell + 4 * axis);
    return sum / 256.0;
}

// Binomial blur across bins: 5 taps, a sigma of one bin (0.6 stops), so regions a stop or
// more apart in brightness stay apart.
fn local_exposure_blur_bins(cell : vec3<i32>) -> vec2<f32> {
    let axis = vec3<i32>(0, 0, 1);
    let sum = 6.0 * local_exposure_tap(cell)
        + 4.0 * (local_exposure_tap(cell - axis) + local_exposure_tap(cell + axis))
        + local_exposure_tap(cell - 2 * axis) + local_exposure_tap(cell + 2 * axis);
    return sum / 16.0;
}

fn local_exposure_in_use(cell : vec3<u32>) -> bool {
    return all(cell.xy < local_exposure_cells()) && cell.z < LOCAL_EXPOSURE_BINS;
}

@compute @workgroup_size(8, 8, 1)
fn cs_local_exposure_blur_x(@builtin(global_invocation_id) id : vec3<u32>) {
    if local_exposure_in_use(id) {
        local_exposure_out[local_exposure_index(id)] =
            local_exposure_blur_cells(vec3<i32>(id), vec3<i32>(1, 0, 0));
    }
}

@compute @workgroup_size(8, 8, 1)
fn cs_local_exposure_blur_y(@builtin(global_invocation_id) id : vec3<u32>) {
    if local_exposure_in_use(id) {
        local_exposure_out[local_exposure_index(id)] =
            local_exposure_blur_cells(vec3<i32>(id), vec3<i32>(0, 1, 0));
    }
}

@compute @workgroup_size(8, 8, 1)
fn cs_local_exposure_blur_bins(@builtin(global_invocation_id) id : vec3<u32>) {
    if local_exposure_in_use(id) {
        let grid = local_exposure_blur_bins(vec3<i32>(id));
        textureStore(local_exposure_grid, id, vec4<f32>(grid, 0.0, 0.0));
    }
}
