// Auto exposure: a 64-bin histogram of log2 luminance over the visible scene, then one
// workgroup averages the bins between the low and high percentiles and eases the stored
// exposure toward it.

const HISTOGRAM_BINS : u32 = 64u;
// Luminance range the histogram covers, in EV (log2 luminance).
const HISTOGRAM_MIN_EV : f32 = -12.0;
const HISTOGRAM_RANGE_EV : f32 = 20.0;

struct ExposureState {
    // Adapted average scene luminance in EV.
    average_ev : f32,
    // 1.0 once a frame has been measured.
    initialized : f32,
    _padding : vec2<f32>,
};

// The exposure the composite applies: compensation and post exposure, and with auto
// exposure the adapted average luminance mapped to middle grey.
fn exposure_from_state(state : ExposureState) -> f32 {
    var exposure = post_uniform.exposure_params.x * post_uniform.exposure_params.y;
    if post_uniform.effects.w > 0.5 && state.initialized > 0.5 {
        exposure = exposure * 0.18 / exp2(state.average_ev);
    }
    return exposure;
}

@group(1) @binding(110)
var histogram_scene : texture_2d<f32>;
@group(1) @binding(111)
var<storage, read_write> histogram_bins : array<atomic<u32>, 64>;
@group(1) @binding(112)
var<storage, read_write> exposure_state : ExposureState;

var<workgroup> local_bins : array<atomic<u32>, 64>;
var<workgroup> bin_counts : array<u32, 64>;

fn luminance_bin(luminance : f32) -> u32 {
    // Bin 0 collects black and near-black pixels and is left out of the average, so an
    // empty background does not blow the exposure up.
    if luminance < exp2(HISTOGRAM_MIN_EV) {
        return 0u;
    }
    let t = clamp((log2(luminance) - HISTOGRAM_MIN_EV) / HISTOGRAM_RANGE_EV, 0.0, 1.0);
    return u32(t * f32(HISTOGRAM_BINS - 2u)) + 1u;
}

fn bin_center_ev(bin : u32) -> f32 {
    let t = (f32(bin) - 0.5) / f32(HISTOGRAM_BINS - 2u);
    return HISTOGRAM_MIN_EV + t * HISTOGRAM_RANGE_EV;
}

// One thread per 2x2 pixel block of the viewport.
@compute @workgroup_size(16, 16, 1)
fn cs_exposure_histogram(
    @builtin(global_invocation_id) global_id : vec3<u32>,
    @builtin(local_invocation_index) local_index : u32,
) {
    if local_index < HISTOGRAM_BINS {
        atomicStore(&local_bins[local_index], 0u);
    }
    workgroupBarrier();

    let dims = vec2<f32>(textureDimensions(histogram_scene, 0));
    let origin = post_uniform.viewport_offset * dims;
    let size = post_uniform.resolution;
    let local = vec2<f32>(global_id.xy * 2u);
    if local.x < size.x && local.y < size.y {
        let pixel = vec2<i32>(origin + local);
        let color = textureLoad(histogram_scene, pixel, 0).rgb;
        let luminance = luminance_rec709(max(color, vec3<f32>(0.0)));
        if luminance == luminance {
            atomicAdd(&local_bins[luminance_bin(luminance)], 1u);
        }
    }
    workgroupBarrier();

    if local_index < HISTOGRAM_BINS {
        let count = atomicLoad(&local_bins[local_index]);
        if count > 0u {
            atomicAdd(&histogram_bins[local_index], count);
        }
    }
}

@compute @workgroup_size(64, 1, 1)
fn cs_exposure_average(@builtin(local_invocation_index) local_index : u32) {
    // Read and reset this frame's bins.
    bin_counts[local_index] = atomicExchange(&histogram_bins[local_index], 0u);
    workgroupBarrier();

    if local_index != 0u {
        return;
    }

    var total = 0.0;
    for (var bin = 1u; bin < HISTOGRAM_BINS; bin = bin + 1u) {
        total = total + f32(bin_counts[bin]);
    }
    let low = total * post_uniform.exposure_adapt.z;
    let high = total * post_uniform.exposure_adapt.w;

    var cumulative = 0.0;
    var weight = 0.0;
    var sum_ev = 0.0;
    for (var bin = 1u; bin < HISTOGRAM_BINS; bin = bin + 1u) {
        let count = f32(bin_counts[bin]);
        let taken = max(0.0, min(cumulative + count, high) - max(cumulative, low));
        weight = weight + taken;
        sum_ev = sum_ev + taken * bin_center_ev(bin);
        cumulative = cumulative + count;
    }

    let previous = exposure_state.average_ev;
    var measured = previous;
    if weight > 0.0 {
        measured = sum_ev / weight;
    } else if exposure_state.initialized < 0.5 {
        measured = log2(0.18);
    }
    let target_ev = clamp(measured, post_uniform.exposure_params.z, post_uniform.exposure_params.w);

    if exposure_state.initialized < 0.5 {
        exposure_state.average_ev = target_ev;
        exposure_state.initialized = 1.0;
        return;
    }

    let delta = target_ev - previous;
    // Brighter scene -> darken at "speed up", darker scene -> brighten at "speed down".
    let speed = select(post_uniform.exposure_adapt.y, post_uniform.exposure_adapt.x, delta > 0.0);
    let dt = clamp(post_uniform.time_delta.y, 0.0, 0.25);
    let blend = 1.0 - exp(-dt * max(speed, 0.0));
    exposure_state.average_ev = previous + delta * blend;
}
