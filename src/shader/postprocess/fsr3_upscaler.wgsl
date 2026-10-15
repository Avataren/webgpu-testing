// AMD FidelityFX Super Resolution 3.1 upscaler (FSR 3.1.5), ported to WGSL.
//
// Ported from the FidelityFX SDK v2.3.0 (Kits/FidelityFX/upscalers/fsr3/include/gpu/
// fsr3upscaler/*.h and fsr1/ffx_fsr1.h), which carries this notice:
//
//   Copyright (C) 2026 Advanced Micro Devices, Inc.
//
//   Permission is hereby granted, free of charge, to any person obtaining a copy
//   of this software and associated documentation files(the "Software"), to deal
//   in the Software without restriction, including without limitation the rights
//   to use, copy, modify, merge, publish, distribute, sublicense, and /or sell
//   copies of the Software, and to permit persons to whom the Software is
//   furnished to do so, subject to the following conditions :
//
//   The above copyright notice and this permission notice shall be included in
//   all copies or substantial portions of the Software.
//
//   THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
//   IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
//   FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
//   AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
//   LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
//   OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
//   THE SOFTWARE.
//
// The permutation ported: HDR colour input, low-resolution (render size) motion vectors
// without jitter, standard (not inverted) depth, the reference Lanczos history reprojection,
// the approximate Lanczos upsample, FSR's own auto exposure, no reactive or
// transparency-and-composition masks, 32-bit math. Differences from the original, all
// for portability to WebGPU:
// - The single-pass downsamplers (luma and shading-change pyramids) are separate passes:
//   WebGPU cannot make one workgroup's texture writes visible to another within a
//   dispatch. Only the levels FSR reads are made (the luma average, farthest depth at half
//   size, shading change levels 0-2).
// - The reconstructed previous depth is a storage buffer of `1 - depth` bits with
//   atomicMax (WGSL has no texture atomics).
// - Formats are what WebGPU can store to (r32float for the 8- and 16-bit single-channel
//   targets, rg32float for rg16float); r32float and rg32float are not filterable, so they
//   are sampled with a manual bilinear fetch.
// - Resets are handled in the shaders (frame index 0) instead of by clearing textures.
// - Inputs and output live at an offset in the engine's targets (the editor viewport).

// ============================================================================
// Constants (mirrors FsrConstants in src/renderer/postprocess/fsr.rs)
// ============================================================================

struct FsrConstants {
    render_size : vec2<i32>,
    previous_render_size : vec2<i32>,
    upscale_size : vec2<i32>,
    previous_upscale_size : vec2<i32>,
    // Size of FSR's own render- and output-sized textures.
    max_render_size : vec2<i32>,
    // Where the render region starts in the input textures, and the output region in the
    // output texture.
    input_offset : vec2<i32>,
    output_offset : vec2<i32>,
    // Luma reduction workgroups along x, and in total.
    luma_groups : vec2<i32>,
    device_to_view_depth : vec4<f32>,
    jitter : vec2<f32>,
    previous_jitter : vec2<f32>,
    downscale_factor : vec2<f32>,
    jitter_sequence_length : f32,
    delta_time : f32,
    delta_pre_exposure : f32,
    frame_index : f32,
    velocity_factor : f32,
    reactiveness_scale : f32,
    shading_change_scale : f32,
    accumulation_added_per_frame : f32,
    min_disocclusion_accumulation : f32,
    // RCAS sharpness as a linear factor (exp2(-stops)); sharpen = 1 runs RCAS.
    rcas_sharpness : f32,
    sharpen : u32,
    _pad0 : u32,
    _pad1 : u32,
    _pad2 : u32,
};

@group(0) @binding(0) var<uniform> fsr : FsrConstants;

// Inputs, at the render region of the engine's targets.
@group(0) @binding(1) var input_color : texture_2d<f32>;
@group(0) @binding(2) var input_depth : texture_depth_2d;
// The normal G-buffer: screen motion since last frame in viewport uv units in zw
// (current - previous, jitter excluded).
@group(0) @binding(3) var input_motion : texture_2d<f32>;
@group(0) @binding(4) var linear_clamp : sampler;

// Prepare inputs.
@group(0) @binding(10) var w_dilated_depth : texture_storage_2d<r32float, write>;
@group(0) @binding(11) var w_dilated_motion : texture_storage_2d<rg32float, write>;
@group(0) @binding(12) var w_farthest_depth : texture_storage_2d<r32float, write>;
@group(0) @binding(13) var w_current_luma : texture_storage_2d<r32float, write>;
@group(0) @binding(14) var<storage, read_write> rw_reconstructed_depth : array<atomic<u32>>;

// Luma pyramid.
@group(0) @binding(20) var r_current_luma : texture_2d<f32>;
@group(0) @binding(21) var r_farthest_depth : texture_2d<f32>;
@group(0) @binding(22) var w_farthest_depth_mip1 : texture_storage_2d<r32float, write>;
@group(0) @binding(23) var<storage, read_write> rw_luma_partials : array<vec4<f32>>;
@group(0) @binding(24) var<storage, read_write> rw_frame_info : array<vec4<f32>>;
@group(0) @binding(25) var<storage, read> r_frame_info : array<vec4<f32>>;

// Shading change.
@group(0) @binding(30) var r_dilated_motion : texture_2d<f32>;
@group(0) @binding(31) var r_previous_luma : texture_2d<f32>;
@group(0) @binding(32) var w_spd_mip0 : texture_storage_2d<rg32float, write>;
@group(0) @binding(33) var w_spd_mip1 : texture_storage_2d<rg32float, write>;
@group(0) @binding(34) var w_spd_mip2 : texture_storage_2d<rg32float, write>;
@group(0) @binding(35) var r_spd_mip0 : texture_2d<f32>;
@group(0) @binding(36) var r_spd_mip1 : texture_2d<f32>;
@group(0) @binding(37) var r_spd_mip2 : texture_2d<f32>;
@group(0) @binding(38) var w_shading_change : texture_storage_2d<r32float, write>;

// Prepare reactivity.
@group(0) @binding(40) var r_dilated_depth : texture_2d<f32>;
@group(0) @binding(41) var<storage, read> r_reconstructed_depth : array<u32>;
@group(0) @binding(42) var r_shading_change : texture_2d<f32>;
@group(0) @binding(43) var r_previous_accumulation : texture_2d<f32>;
@group(0) @binding(44) var w_accumulation : texture_storage_2d<r32float, write>;
@group(0) @binding(45) var w_dilated_reactive : texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(46) var w_new_locks : texture_storage_2d<r32float, write>;

// Luma instability.
@group(0) @binding(50) var r_dilated_reactive : texture_2d<f32>;
@group(0) @binding(51) var r_previous_luma_history : texture_2d<f32>;
@group(0) @binding(52) var r_farthest_depth_mip1 : texture_2d<f32>;
@group(0) @binding(53) var w_luma_history : texture_storage_2d<rgba16float, write>;
@group(0) @binding(54) var w_luma_instability : texture_storage_2d<r32float, write>;

// Accumulate.
@group(0) @binding(60) var r_history : texture_2d<f32>;
@group(0) @binding(61) var r_luma_instability : texture_2d<f32>;
@group(0) @binding(62) var w_history : texture_storage_2d<rgba16float, write>;
@group(0) @binding(63) var w_output : texture_storage_2d<rgba16float, write>;
@group(0) @binding(64) var rw_new_locks : texture_storage_2d<r32float, read_write>;

// RCAS.
@group(0) @binding(70) var r_rcas_input : texture_2d<f32>;

// ============================================================================
// ffx_fsr3upscaler_common.h
// ============================================================================

const FP16_MIN : f32 = 6.10e-05;
const FP16_MAX : f32 = 65504.0;
const EPSILON : f32 = 6.10e-05;
const TONEMAP_EPSILON : f32 = 6.10e-05;
const FP32_MAX : f32 = 3.4028234e38;
const FP32_MIN : f32 = 1.175494351e-38;

const RECONSTRUCTED_DEPTH_BILINEAR_WEIGHT_THRESHOLD : f32 = 6.10e-04;

fn reconstructed_depth_mv_px_threshold(nearest_depth_in_meters : f32) -> f32 {
    return mix(0.25, 0.75, saturate(nearest_depth_in_meters / 100.0));
}

const UPSAMPLE_LANCZOS_WEIGHT_SCALE : f32 = 1.0 / 16.0;
const AVERAGE_LANCZOS_WEIGHT_PER_FRAME : f32 = 0.74 / 16.0;

const SHADING_CHANGE_SET_SIZE : i32 = 5;
const SHADING_CHANGE_MIP_START : i32 = 0;
const SHADING_CHANGE_MIP_LEVELS : i32 = 3;

const LOCK_THRESHOLD : f32 = 1.0;
const LOCK_MAX : f32 = 2.0;

const REACTIVE : i32 = 0;
const DISOCCLUSION : i32 = 1;
const SHADING_CHANGE : i32 = 2;
const ACCUMULATION : i32 = 3;

const FRAME_INFO_EXPOSURE : i32 = 0;
const FRAME_INFO_LOG_LUMA : i32 = 1;
const FRAME_INFO_SCENE_AVERAGE_LUMA : i32 = 2;

const RESET_AUTO_EXPOSURE_AVERAGE_SMOOTHING : f32 = 1e4;

fn render_size() -> vec2<i32> {
    return fsr.render_size;
}

fn shading_change_render_size() -> vec2<i32> {
    return vec2<i32>(vec2<f32>(fsr.render_size) * 0.5);
}

fn shading_change_max_render_size() -> vec2<i32> {
    return vec2<i32>(vec2<f32>(fsr.max_render_size) * 0.5);
}

fn jitter() -> vec2<f32> {
    return fsr.jitter;
}

fn exposure() -> f32 {
    var value = r_frame_info[0][FRAME_INFO_EXPOSURE];
    if value < 0.000030517578 {
        value = 1.0;
    }
    if value == 0.0 {
        value = 1.0;
    }
    return value;
}

fn get_4k_velocity(motion_vector : vec2<f32>) -> f32 {
    return length(motion_vector * vec2<f32>(3840.0, 2160.0));
}

struct RectificationBox {
    box_center : vec3<f32>,
    box_vec : vec3<f32>,
    aabb_min : vec3<f32>,
    aabb_max : vec3<f32>,
    box_center_weight : f32,
};

fn rectification_box_add_sample(
    initial_sample : bool,
    box_in : RectificationBox,
    color_sample : vec3<f32>,
    sample_weight : f32,
) -> RectificationBox {
    var b = box_in;
    let weighted_sample = color_sample * sample_weight;
    if initial_sample {
        b.aabb_min = color_sample;
        b.aabb_max = color_sample;
        b.box_center = weighted_sample;
        b.box_vec = color_sample * weighted_sample;
        b.box_center_weight = sample_weight;
    } else {
        b.aabb_min = min(b.aabb_min, color_sample);
        b.aabb_max = max(b.aabb_max, color_sample);
        b.box_center += weighted_sample;
        b.box_vec += color_sample * weighted_sample;
        b.box_center_weight += sample_weight;
    }
    return b;
}

fn rectification_box_compute_variance_box_data(box_in : RectificationBox) -> RectificationBox {
    var b = box_in;
    b.box_center_weight = select(1.0, b.box_center_weight, abs(b.box_center_weight) > FP32_MIN);
    b.box_center /= b.box_center_weight;
    b.box_vec /= b.box_center_weight;
    b.box_vec = sqrt(abs(b.box_vec - b.box_center * b.box_center));
    return b;
}

fn min_divided_by_max_or(v0 : f32, v1 : f32, on_zero : f32) -> f32 {
    let m = max(v0, v1);
    return select(on_zero, min(v0, v1) / m, m != 0.0);
}

fn min_divided_by_max(v0 : f32, v1 : f32) -> f32 {
    return min_divided_by_max_or(v0, v1, 0.0);
}

fn ycocg_to_rgb(c : vec3<f32>) -> vec3<f32> {
    return vec3<f32>(c.x + c.y - c.z, c.x + c.z, c.x - c.y - c.z);
}

fn rgb_to_ycocg(c : vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        0.25 * c.r + 0.5 * c.g + 0.25 * c.b,
        0.5 * c.r - 0.5 * c.b,
        -0.25 * c.r + 0.5 * c.g - 0.25 * c.b,
    );
}

fn rgb_to_luma(c : vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn tonemap(c : vec3<f32>) -> vec3<f32> {
    return c / (max(max(0.0, c.r), max(c.g, c.b)) + 1.0);
}

fn inverse_tonemap(c : vec3<f32>) -> vec3<f32> {
    return c / max(TONEMAP_EPSILON, 1.0 - max(c.r, max(c.g, c.b)));
}

fn is_uv_inside(uv : vec2<f32>) -> bool {
    return uv.x >= 0.0 && uv.x <= 1.0 && uv.y >= 0.0 && uv.y <= 1.0;
}

fn clamp_load(sample_pos : vec2<i32>, offset : vec2<i32>, size : vec2<i32>) -> vec2<i32> {
    return clamp(sample_pos + offset, vec2<i32>(0), size - vec2<i32>(1));
}

fn clamp_uv(uv : vec2<f32>, texture_size : vec2<i32>, resource_size : vec2<i32>) -> vec2<f32> {
    let location = uv * vec2<f32>(texture_size);
    let clamped = max(vec2<f32>(0.5), min(location, vec2<f32>(texture_size) - vec2<f32>(0.5)));
    return clamped / vec2<f32>(resource_size);
}

fn is_on_screen(pos : vec2<i32>, size : vec2<i32>) -> bool {
    return all(vec2<u32>(pos) < vec2<u32>(size));
}

fn compute_auto_exposure_from_lavg(lavg_log : f32) -> f32 {
    let lavg = exp(lavg_log);
    let s = 100.0;
    let k = 12.5;
    let exposure_iso100 = log2((lavg * s) / k);
    let q = 0.65;
    let lmax = (78.0 / (q * s)) * pow(2.0, exposure_iso100);
    return 1.0 / lmax;
}

fn compute_hr_pos_from_lr_pos(lr_pos : vec2<i32>) -> vec2<i32> {
    let src_jittered_pos = vec2<f32>(lr_pos) + 0.5 - jitter();
    let lr_pos_in_hr = (src_jittered_pos / vec2<f32>(render_size())) * vec2<f32>(fsr.upscale_size);
    return vec2<i32>(floor(lr_pos_in_hr));
}

fn get_view_space_depth(device_depth : f32) -> f32 {
    let d = fsr.device_to_view_depth;
    return d.y / (device_depth - d.x);
}

// The engine works in meters.
fn get_view_space_depth_in_meters(device_depth : f32) -> f32 {
    return get_view_space_depth(device_depth);
}

struct BilinearSamplingData {
    offsets : array<vec2<i32>, 4>,
    weights : array<f32, 4>,
    base_pos : vec2<i32>,
};

fn get_bilinear_sampling_data(uv : vec2<f32>, size : vec2<i32>) -> BilinearSamplingData {
    var data : BilinearSamplingData;
    let px_sample = uv * vec2<f32>(size) - vec2<f32>(0.5);
    data.base_pos = vec2<i32>(floor(px_sample));
    let frac = fract(px_sample);
    data.offsets = array<vec2<i32>, 4>(
        vec2<i32>(0, 0),
        vec2<i32>(1, 0),
        vec2<i32>(0, 1),
        vec2<i32>(1, 1),
    );
    data.weights = array<f32, 4>(
        (1.0 - frac.x) * (1.0 - frac.y),
        frac.x * (1.0 - frac.y),
        (1.0 - frac.x) * frac.y,
        frac.x * frac.y,
    );
    return data;
}

// ============================================================================
// Resource access (ffx_fsr3upscaler_callbacks_hlsl.h)
// ============================================================================

// Bilinear filtering with clamp-to-edge addressing for the formats WebGPU cannot filter.
fn bilinear_load_4(tex : texture_2d<f32>, uv : vec2<f32>) -> vec4<f32> {
    let size = vec2<i32>(textureDimensions(tex));
    let px = uv * vec2<f32>(size) - vec2<f32>(0.5);
    let base = vec2<i32>(floor(px));
    let f = fract(px);
    let last = size - vec2<i32>(1);
    let c00 = textureLoad(tex, clamp(base, vec2<i32>(0), last), 0);
    let c10 = textureLoad(tex, clamp(base + vec2<i32>(1, 0), vec2<i32>(0), last), 0);
    let c01 = textureLoad(tex, clamp(base + vec2<i32>(0, 1), vec2<i32>(0), last), 0);
    let c11 = textureLoad(tex, clamp(base + vec2<i32>(1, 1), vec2<i32>(0), last), 0);
    return mix(mix(c00, c10, f.x), mix(c01, c11, f.x), f.y);
}

fn input_pixel(pos : vec2<i32>) -> vec2<i32> {
    let size = vec2<i32>(textureDimensions(input_color));
    return clamp(pos + fsr.input_offset, vec2<i32>(0), size - vec2<i32>(1));
}

fn load_input_color(pos : vec2<i32>) -> vec3<f32> {
    return textureLoad(input_color, input_pixel(pos), 0).rgb;
}

fn load_input_depth(pos : vec2<i32>) -> f32 {
    return textureLoad(input_depth, input_pixel(pos), 0);
}

// FSR motion vectors point from the current to the previous position (uv units).
fn load_input_motion_vector(pos : vec2<i32>) -> vec2<f32> {
    return -textureLoad(input_motion, input_pixel(pos), 0).zw;
}

fn load_dilated_motion_vector(pos : vec2<i32>) -> vec2<f32> {
    return textureLoad(r_dilated_motion, pos, 0).xy;
}

fn load_dilated_depth(pos : vec2<i32>) -> f32 {
    return textureLoad(r_dilated_depth, pos, 0).x;
}

fn load_current_luma(pos : vec2<i32>) -> f32 {
    return textureLoad(r_current_luma, pos, 0).x;
}

fn sample_current_luma(uv : vec2<f32>) -> f32 {
    return bilinear_load_4(r_current_luma, uv).x;
}

fn load_previous_luma(pos : vec2<i32>) -> f32 {
    return textureLoad(r_previous_luma, pos, 0).x;
}

fn reconstructed_depth_index(pos : vec2<i32>) -> u32 {
    return u32(pos.y * fsr.max_render_size.x + pos.x);
}

// Stores keep the nearest (smallest) depth: as 1 - depth with atomicMax, so the buffer
// cleared to 0 reads as depth 1 (far), like the original's clear.
fn store_reconstructed_depth(pos : vec2<i32>, depth : f32) {
    atomicMax(&rw_reconstructed_depth[reconstructed_depth_index(pos)], bitcast<u32>(1.0 - depth));
}

fn load_reconstructed_prev_depth(pos : vec2<i32>) -> f32 {
    return 1.0 - bitcast<f32>(r_reconstructed_depth[reconstructed_depth_index(pos)]);
}

fn sample_dilated_reactive_masks(uv : vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(r_dilated_reactive, linear_clamp, uv, 0.0);
}

fn sample_farthest_depth_mip1(uv : vec2<f32>) -> f32 {
    return bilinear_load_4(r_farthest_depth_mip1, uv).x;
}

fn farthest_depth_mip1_dimensions() -> vec2<i32> {
    return vec2<i32>(textureDimensions(r_farthest_depth_mip1));
}

// ============================================================================
// Prepare inputs (ffx_fsr3upscaler_prepare_inputs.h)
// ============================================================================

fn reconstruct_prev_depth(pos : vec2<i32>, depth : f32, motion_vector_in : vec2<f32>) {
    let nearest_depth_in_meters = min(get_view_space_depth_in_meters(depth), FP16_MAX);
    let threshold = reconstructed_depth_mv_px_threshold(nearest_depth_in_meters);
    // Discard small motion vectors.
    let motion_vector = motion_vector_in * f32(get_4k_velocity(motion_vector_in) > threshold);

    let uv = (vec2<f32>(pos) + 0.5) / vec2<f32>(render_size());
    let reprojected_uv = uv + motion_vector;
    let bilinear = get_bilinear_sampling_data(reprojected_uv, render_size());

    // Project the current depth into the previous frame, to every pixel a bilinear
    // reprojection would read.
    for (var i = 0; i < 4; i++) {
        if bilinear.weights[i] > RECONSTRUCTED_DEPTH_BILINEAR_WEIGHT_THRESHOLD {
            let store_pos = bilinear.base_pos + bilinear.offsets[i];
            if is_on_screen(store_pos, render_size()) {
                store_reconstructed_depth(store_pos, depth);
            }
        }
    }
}

struct DepthExtents {
    nearest : f32,
    nearest_coord : vec2<i32>,
    farthest : f32,
};

fn find_depth_extents(pos : vec2<i32>) -> DepthExtents {
    var offsets = array<vec2<i32>, 9>(
        vec2<i32>(0, 0),
        vec2<i32>(1, 0),
        vec2<i32>(0, 1),
        vec2<i32>(0, -1),
        vec2<i32>(-1, 0),
        vec2<i32>(-1, 1),
        vec2<i32>(1, 1),
        vec2<i32>(-1, -1),
        vec2<i32>(1, -1),
    );
    var depth : array<f32, 9>;
    for (var i = 0; i < 9; i++) {
        depth[i] = load_input_depth(pos + offsets[i]);
    }
    var extents : DepthExtents;
    extents.nearest_coord = pos;
    extents.nearest = depth[0];
    extents.farthest = depth[0];
    for (var i = 1; i < 9; i++) {
        let sample_pos = pos + offsets[i];
        if is_on_screen(sample_pos, render_size()) {
            let nd_depth = depth[i];
            if nd_depth < extents.nearest {
                extents.farthest = max(extents.farthest, nd_depth);
                extents.nearest_coord = sample_pos;
                extents.nearest = nd_depth;
            }
        }
    }
    return extents;
}

@compute @workgroup_size(8, 8, 1)
fn cs_prepare_inputs(@builtin(global_invocation_id) id : vec3<u32>) {
    let pos = vec2<i32>(id.xy);
    if !is_on_screen(pos, render_size()) {
        return;
    }
    let extents = find_depth_extents(pos);
    let dilated_motion_vector = load_input_motion_vector(extents.nearest_coord);

    reconstruct_prev_depth(pos, extents.nearest, dilated_motion_vector);

    textureStore(w_dilated_motion, pos, vec4<f32>(dilated_motion_vector, 0.0, 0.0));
    textureStore(w_dilated_depth, pos, vec4<f32>(extents.nearest, 0.0, 0.0, 0.0));

    let farthest_in_meters = min(get_view_space_depth_in_meters(extents.farthest), FP16_MAX);
    textureStore(w_farthest_depth, pos, vec4<f32>(farthest_in_meters, 0.0, 0.0, 0.0));

    // Linear input assumed.
    let luma = rgb_to_luma(max(vec3<f32>(0.0), load_input_color(pos)));
    textureStore(w_current_luma, pos, vec4<f32>(luma, 0.0, 0.0, 0.0));
}

// ============================================================================
// Luma pyramid (ffx_fsr3upscaler_luma_pyramid.h): the farthest depth at half size and
// the scene's average (log) luma, which drives FSR's auto exposure.
// ============================================================================

var<workgroup> luma_shared : array<vec4<f32>, 256>;

@compute @workgroup_size(16, 16, 1)
fn cs_luma_pyramid(
    @builtin(global_invocation_id) id : vec3<u32>,
    @builtin(local_invocation_index) local_index : u32,
    @builtin(workgroup_id) group : vec3<u32>,
) {
    let half_pos = vec2<i32>(id.xy);
    let half_size = (render_size() + vec2<i32>(1)) / 2;
    var sums = vec4<f32>(0.0);
    if is_on_screen(half_pos, half_size) {
        var log_luma = 0.0;
        var luma = 0.0;
        var depth = 0.0;
        for (var i = 0; i < 4; i++) {
            let source = clamp_load(half_pos * 2, vec2<i32>(i % 2, i / 2), render_size());
            let l = load_current_luma(source);
            log_luma += max(EPSILON, log(l));
            luma += l;
            depth += textureLoad(r_farthest_depth, source, 0).x;
        }
        textureStore(w_farthest_depth_mip1, half_pos, vec4<f32>(depth * 0.25, 0.0, 0.0, 0.0));
        sums = vec4<f32>(log_luma * 0.25, luma * 0.25, 1.0, 0.0);
    }
    luma_shared[local_index] = sums;
    workgroupBarrier();
    for (var stride = 128u; stride > 0u; stride = stride / 2u) {
        if local_index < stride {
            luma_shared[local_index] += luma_shared[local_index + stride];
        }
        workgroupBarrier();
    }
    if local_index == 0u {
        let index = i32(group.y) * fsr.luma_groups.x + i32(group.x);
        rw_luma_partials[index] = luma_shared[0];
    }
}

@compute @workgroup_size(256, 1, 1)
fn cs_luma_finalize(@builtin(local_invocation_index) local_index : u32) {
    var sums = vec4<f32>(0.0);
    for (var i = i32(local_index); i < fsr.luma_groups.y; i += 256) {
        sums += rw_luma_partials[i];
    }
    luma_shared[local_index] = sums;
    workgroupBarrier();
    for (var stride = 128u; stride > 0u; stride = stride / 2u) {
        if local_index < stride {
            luma_shared[local_index] += luma_shared[local_index + stride];
        }
        workgroupBarrier();
    }
    if local_index == 0u {
        let total = luma_shared[0];
        let count = max(total.z, 1.0);
        var frame_info = rw_frame_info[0];
        if fsr.frame_index == 0.0 {
            // As the original's reset clear.
            frame_info = vec4<f32>(-1.0, 1.0, 0.0, 0.0);
        }
        let scene_avg_luma = total.y / count;
        let prev_log_luma = frame_info[FRAME_INFO_LOG_LUMA];
        var log_luma = total.x / count;
        if prev_log_luma < RESET_AUTO_EXPOSURE_AVERAGE_SMOOTHING {
            log_luma = prev_log_luma + (log_luma - prev_log_luma) * (1.0 - exp(-fsr.delta_time));
            log_luma = max(0.0, log_luma);
        }
        frame_info[FRAME_INFO_EXPOSURE] = compute_auto_exposure_from_lavg(log_luma);
        frame_info[FRAME_INFO_LOG_LUMA] = log_luma;
        frame_info[FRAME_INFO_SCENE_AVERAGE_LUMA] = scene_avg_luma;
        rw_frame_info[0] = frame_info;
    }
}

// ============================================================================
// Shading change pyramid (ffx_fsr3upscaler_shading_change_pyramid.h)
// ============================================================================

fn compare_swap(values : ptr<function, array<f32, 5>>, i : i32, j : i32) {
    let tmp = min((*values)[i], (*values)[j]);
    (*values)[j] = max((*values)[i], (*values)[j]);
    (*values)[i] = tmp;
}

fn sort_set(values : ptr<function, array<f32, 5>>) {
    compare_swap(values, 0, 3);
    compare_swap(values, 1, 4);
    compare_swap(values, 0, 2);
    compare_swap(values, 1, 3);
    compare_swap(values, 0, 1);
    compare_swap(values, 2, 4);
    compare_swap(values, 1, 2);
    compare_swap(values, 3, 4);
    compare_swap(values, 2, 3);
}

fn compute_minimum_difference(set0_in : array<f32, 5>, set1_in : array<f32, 5>) -> f32 {
    var set0 = set0_in;
    var set1 = set1_in;
    var min_diff = FP16_MAX - 1.0;
    var a = 0;
    var b = 0;
    sort_set(&set0);
    sort_set(&set1);
    let max_value = min(set0[SHADING_CHANGE_SET_SIZE - 1], set1[SHADING_CHANGE_SET_SIZE - 1]);
    if max_value > FP32_MIN {
        for (var i = 0; i < SHADING_CHANGE_SET_SIZE && min_diff < FP16_MAX; i++) {
            var diff = set0[a] - set1[b];
            if abs(diff) > FP16_MIN {
                diff = sign(diff) * (1.0 - min_divided_by_max(set0[a], set1[b]));
                min_diff = select(min_diff, diff, abs(diff) < abs(min_diff));
                a += i32(set0[a] < set1[b]);
                // The original reads past the set here; keep the index in range.
                a = min(a, SHADING_CHANGE_SET_SIZE - 1);
                b += i32(set0[a] >= set1[b]);
                b = min(b, SHADING_CHANGE_SET_SIZE - 1);
            } else {
                min_diff = FP16_MAX;
            }
        }
    }
    return min_diff * f32(min_diff < FP16_MAX - 1.0);
}

const SHADING_SAMPLE_OFFSETS = array<vec2<i32>, 5>(
    vec2<i32>(0, 0),
    vec2<i32>(-1, 0),
    vec2<i32>(1, 0),
    vec2<i32>(0, -1),
    vec2<i32>(0, 1),
);

fn compute_shading_diff(uv : vec2<f32>, motion_vector : vec2<f32>) -> f32 {
    let exposure_value = exposure();
    var current : array<f32, 5>;
    let uv_jittered = uv + jitter() / vec2<f32>(render_size());
    let base_pos = vec2<i32>(floor(uv_jittered * vec2<f32>(render_size())));
    for (var i = 0; i < SHADING_CHANGE_SET_SIZE; i++) {
        let p = clamp_load(base_pos, SHADING_SAMPLE_OFFSETS[i], render_size());
        current[i] = max(load_current_luma(p) * exposure_value, EPSILON);
    }

    let prev_uv_jittered = uv + fsr.previous_jitter / vec2<f32>(fsr.previous_render_size);
    let reprojected_uv = prev_uv_jittered + motion_vector;
    if !is_uv_inside(reprojected_uv) {
        return 0.0;
    }
    var previous : array<f32, 5>;
    let prev_base = vec2<i32>(floor(reprojected_uv * vec2<f32>(fsr.previous_render_size)));
    for (var i = 0; i < SHADING_CHANGE_SET_SIZE; i++) {
        let p = clamp_load(prev_base, SHADING_SAMPLE_OFFSETS[i], fsr.previous_render_size);
        previous[i] = max(load_previous_luma(p) * fsr.delta_pre_exposure * exposure_value, EPSILON);
    }
    return compute_minimum_difference(current, previous);
}

fn shading_pyramid_source(pos : vec2<i32>) -> vec2<f32> {
    let p = clamp_load(pos, vec2<i32>(0), render_size());
    let motion_vector = load_dilated_motion_vector(p);
    let uv = (vec2<f32>(p) + 0.5) / vec2<f32>(render_size());
    let diff = compute_shading_diff(uv, motion_vector);
    return vec2<f32>(diff, select(0.0, sign(diff), diff != 0.0));
}

var<workgroup> shading_shared : array<vec2<f32>, 64>;

// Levels 0-2 of the (difference, sign sum) pyramid, each a 2x2 average of the one above:
// an 8x8 group makes 8x8 of level 0, 4x4 of level 1 and 2x2 of level 2.
@compute @workgroup_size(8, 8, 1)
fn cs_shading_change_pyramid(
    @builtin(global_invocation_id) id : vec3<u32>,
    @builtin(local_invocation_id) local : vec3<u32>,
) {
    let pos = vec2<i32>(id.xy);
    var value = vec2<f32>(0.0);
    for (var i = 0; i < 4; i++) {
        value += shading_pyramid_source(pos * 2 + vec2<i32>(i % 2, i / 2));
    }
    value *= 0.25;
    if is_on_screen(pos, vec2<i32>(textureDimensions(w_spd_mip0))) {
        textureStore(w_spd_mip0, pos, vec4<f32>(value, 0.0, 0.0));
    }
    let l = vec2<i32>(local.xy);
    shading_shared[l.y * 8 + l.x] = value;
    workgroupBarrier();

    var mip1 = vec2<f32>(0.0);
    let is_mip1 = all(l % 2 == vec2<i32>(0));
    if is_mip1 {
        mip1 = (shading_shared[l.y * 8 + l.x] + shading_shared[l.y * 8 + l.x + 1]
            + shading_shared[(l.y + 1) * 8 + l.x] + shading_shared[(l.y + 1) * 8 + l.x + 1]) * 0.25;
        let mip1_pos = pos / 2;
        if is_on_screen(mip1_pos, vec2<i32>(textureDimensions(w_spd_mip1))) {
            textureStore(w_spd_mip1, mip1_pos, vec4<f32>(mip1, 0.0, 0.0));
        }
    }
    workgroupBarrier();
    if is_mip1 {
        shading_shared[l.y * 8 + l.x] = mip1;
    }
    workgroupBarrier();
    if all(l % 4 == vec2<i32>(0)) {
        let mip2 = (shading_shared[l.y * 8 + l.x] + shading_shared[l.y * 8 + l.x + 2]
            + shading_shared[(l.y + 2) * 8 + l.x] + shading_shared[(l.y + 2) * 8 + l.x + 2]) * 0.25;
        let mip2_pos = pos / 4;
        if is_on_screen(mip2_pos, vec2<i32>(textureDimensions(w_spd_mip2))) {
            textureStore(w_spd_mip2, mip2_pos, vec4<f32>(mip2, 0.0, 0.0));
        }
    }
}

// ============================================================================
// Shading change (ffx_fsr3upscaler_shading_change.h)
// ============================================================================

@compute @workgroup_size(8, 8, 1)
fn cs_shading_change(@builtin(global_invocation_id) id : vec3<u32>) {
    let pos = vec2<i32>(id.xy);
    let size = shading_change_render_size();
    if !is_on_screen(pos, size) {
        return;
    }
    let uv = (vec2<f32>(pos) + 0.5) / vec2<f32>(size);
    let uv_jittered = uv + jitter() / vec2<f32>(render_size());
    let mip_uv = clamp_uv(uv_jittered, size, vec2<i32>(textureDimensions(r_spd_mip0)));

    var samples = array<f32, 3>(0.0, 0.0, 0.0);
    let s0 = bilinear_load_4(r_spd_mip0, mip_uv).xy;
    let s1 = bilinear_load_4(r_spd_mip1, mip_uv).xy;
    let s2 = bilinear_load_4(r_spd_mip2, mip_uv).xy;
    samples[0] = abs(s0.x * s0.y);
    samples[1] = abs(s1.x * s1.y);
    samples[2] = abs(s2.x * s2.y);

    let scale = 1.0 + f32(SHADING_CHANGE_MIP_START / SHADING_CHANGE_MIP_LEVELS);
    var shading_change = 0.0;
    for (var i = SHADING_CHANGE_MIP_START; i < SHADING_CHANGE_MIP_LEVELS; i++) {
        if samples[i] > 0.0 {
            shading_change = max(shading_change, samples[i]) * scale;
        }
    }
    textureStore(w_shading_change, pos, vec4<f32>(saturate(shading_change), 0.0, 0.0, 0.0));
}

// ============================================================================
// Prepare reactivity (ffx_fsr3upscaler_prepare_reactivity.h)
// ============================================================================

fn compute_disocclusions(uv : vec2<f32>, motion_vector_in : vec2<f32>, current_depth_view_space : f32) -> f32 {
    let nearest_depth_in_meters = min(current_depth_view_space, FP16_MAX);
    let threshold = reconstructed_depth_mv_px_threshold(nearest_depth_in_meters);
    let motion_vector = motion_vector_in * f32(get_4k_velocity(motion_vector_in) > threshold);

    let reprojected_uv = uv + motion_vector;
    let bilinear = get_bilinear_sampling_data(reprojected_uv, render_size());

    var disocclusion = 0.0;
    var weight_sum = 0.0;
    var potential_disocclusion = true;
    for (var i = 0; i < 4 && potential_disocclusion; i++) {
        let sample_pos = clamp_load(bilinear.base_pos, bilinear.offsets[i], render_size());
        if is_on_screen(sample_pos, render_size()) {
            let weight = bilinear.weights[i];
            if weight > RECONSTRUCTED_DEPTH_BILINEAR_WEIGHT_THRESHOLD {
                let prev_nearest_depth = get_view_space_depth(load_reconstructed_prev_depth(sample_pos));
                let depth_difference = current_depth_view_space - prev_nearest_depth;
                potential_disocclusion = potential_disocclusion && depth_difference > FP32_MIN;
                if potential_disocclusion {
                    let half_viewport_width = length(vec2<f32>(render_size()) * 0.5);
                    let depth_threshold = max(current_depth_view_space, prev_nearest_depth);
                    let ksep = 1.37e-05;
                    let required_separation = ksep * half_viewport_width * depth_threshold;
                    disocclusion += saturate(required_separation / depth_difference) * weight;
                    weight_sum += weight;
                }
            }
        }
    }
    if potential_disocclusion && weight_sum > 0.0 {
        return saturate(1.0 - disocclusion / weight_sum);
    }
    return 0.0;
}

fn compute_motion_divergence(uv : vec2<f32>, motion_vector : vec2<f32>, current_depth : f32) -> f32 {
    let reprojected_pos = clamp(
        vec2<i32>((uv + motion_vector) * vec2<f32>(render_size())),
        vec2<i32>(0),
        render_size() - vec2<i32>(1),
    );
    let reprojected_depth = load_dilated_depth(reprojected_pos);
    let reprojected_motion_vector = load_dilated_motion_vector(reprojected_pos);
    let reprojected_velocity = get_4k_velocity(reprojected_motion_vector);
    let velocity_4k = get_4k_velocity(motion_vector);
    // Still pixels have no divergence (the original gets 0 through saturate(NaN) = 0,
    // which WGSL does not promise).
    if velocity_4k <= 0.0 {
        return 0.0;
    }
    let nucleus_depth = get_view_space_depth_in_meters(reprojected_depth);
    let current_depth_meters = get_view_space_depth_in_meters(current_depth);
    let distance_factor = min_divided_by_max(nucleus_depth, current_depth_meters);
    let velocity_factor = saturate(velocity_4k / 10.0);
    return (1.0 - saturate(reprojected_velocity / velocity_4k)) * distance_factor * velocity_factor;
}

fn compute_thin_feature_confidence(pos : vec2<i32>) -> f32 {
    //  1 2 3
    //  4 0 5
    //  6 7 8
    var offsets = array<vec2<i32>, 9>(
        vec2<i32>(0, 0),
        vec2<i32>(-1, -1),
        vec2<i32>(0, -1),
        vec2<i32>(1, -1),
        vec2<i32>(-1, 0),
        vec2<i32>(1, 0),
        vec2<i32>(-1, 1),
        vec2<i32>(0, 1),
        vec2<i32>(1, 1),
    );
    let exposure_value = exposure();
    var samples : array<f32, 9>;
    var luma_min = FP32_MAX;
    var luma_max = FP32_MIN;
    for (var i = 0; i < 9; i++) {
        let p = clamp_load(pos, offsets[i], render_size());
        samples[i] = load_current_luma(p) * exposure_value;
        luma_min = min(luma_min, samples[i]);
        luma_max = max(luma_max, samples[i]);
    }

    // A flat neighbourhood has no ridge (the original reaches 0 through NaN compares).
    if luma_max - luma_min <= 0.0 {
        return 0.0;
    }
    let threshold = 0.9;
    var dissimilar_min = FP32_MAX;
    var dissimilar_max = 0.0;
    // The nucleus is similar to itself.
    var pattern_mask = 1u;
    let rejection_masks = array<u32, 4>(
        (1u << 1u) | (1u << 2u) | (1u << 4u) | 1u, // upper left
        (1u << 2u) | (1u << 3u) | (1u << 5u) | 1u, // upper right
        (1u << 4u) | (1u << 6u) | (1u << 7u) | 1u, // lower left
        (1u << 5u) | (1u << 7u) | (1u << 8u) | 1u, // lower right
    );
    for (var i = 1; i < 9; i++) {
        let difference = abs(samples[i] - samples[0]) / (luma_max - luma_min);
        if difference < threshold {
            pattern_mask |= 1u << u32(i);
        } else {
            dissimilar_min = min(dissimilar_min, samples[i]);
            dissimilar_max = max(dissimilar_max, samples[i]);
        }
    }
    let is_ridge = samples[0] > dissimilar_max || samples[0] < dissimilar_min;
    if !is_ridge {
        return 0.0;
    }
    for (var i = 0; i < 4; i++) {
        if (pattern_mask & rejection_masks[i]) == rejection_masks[i] {
            return 0.0;
        }
    }
    return 1.0 - luma_min / luma_max;
}

fn update_accumulation(
    pos : vec2<i32>,
    uv : vec2<f32>,
    motion_vector : vec2<f32>,
    disocclusion : f32,
    shading_change : f32,
) -> f32 {
    let reprojected_uv = uv + motion_vector;
    var accumulation = 0.0;
    // Frame 0 starts from nothing (the original clears the previous accumulation).
    if is_uv_inside(reprojected_uv) && fsr.frame_index > 0.0 {
        let uv_hw = clamp_uv(reprojected_uv, fsr.previous_render_size, fsr.max_render_size);
        accumulation = saturate(bilinear_load_4(r_previous_accumulation, uv_hw).x);
    }
    // A shading change restarts accumulation; a disocclusion sets it back below zero, so
    // it takes a frame longer to build up again.
    accumulation = mix(accumulation, 0.0, shading_change);
    accumulation = mix(accumulation, min(fsr.min_disocclusion_accumulation, accumulation), disocclusion);
    accumulation *= f32(round(accumulation * 100.0) > 1.0);
    // Stored as unorm in the original.
    let to_store = saturate(accumulation + fsr.accumulation_added_per_frame);
    textureStore(w_accumulation, pos, vec4<f32>(to_store, 0.0, 0.0, 0.0));
    return accumulation;
}

fn compute_shading_change(uv : vec2<f32>) -> f32 {
    // Jitter is applied here and taken off again when the accumulation pass samples it.
    let shading_uv = clamp_uv(uv - jitter() / vec2<f32>(render_size()), shading_change_render_size(), shading_change_max_render_size());
    return saturate(bilinear_load_4(r_shading_change, shading_uv).x * fsr.shading_change_scale);
}

@compute @workgroup_size(8, 8, 1)
fn cs_prepare_reactivity(@builtin(global_invocation_id) id : vec3<u32>) {
    let pos = vec2<i32>(id.xy);
    if !is_on_screen(pos, render_size()) {
        return;
    }
    let uv = (vec2<f32>(pos) + 0.5) / vec2<f32>(render_size());
    let motion_vector = load_dilated_motion_vector(pos);
    let dilated_depth = load_dilated_depth(pos);
    let disocclusion = compute_disocclusions(uv, motion_vector, get_view_space_depth(dilated_depth));
    // No reactive mask is given (it would be max-dilated over 3x3 here).
    let shading_change = max(0.0, compute_shading_change(uv));
    let motion_divergence = compute_motion_divergence(uv, motion_vector, dilated_depth);
    // No transparency-and-composition mask either.
    let final_reactiveness = max(motion_divergence, 0.0);
    let accumulation = update_accumulation(pos, uv, motion_vector, disocclusion, shading_change);

    var output : vec4<f32>;
    output[REACTIVE] = final_reactiveness;
    output[DISOCCLUSION] = disocclusion;
    output[SHADING_CHANGE] = shading_change;
    output[ACCUMULATION] = accumulation;
    textureStore(w_dilated_reactive, pos, output);

    let lock_strength = compute_thin_feature_confidence(pos);
    if lock_strength > 1.0 / 100.0 {
        let hr_pos = compute_hr_pos_from_lr_pos(pos);
        if is_on_screen(hr_pos, fsr.upscale_size) {
            textureStore(w_new_locks, hr_pos, vec4<f32>(lock_strength, 0.0, 0.0, 0.0));
        }
    }
}

// ============================================================================
// Luma instability (ffx_fsr3upscaler_luma_instability.h)
// ============================================================================

@compute @workgroup_size(8, 8, 1)
fn cs_luma_instability(@builtin(global_invocation_id) id : vec3<u32>) {
    let pos = vec2<i32>(id.xy);
    if !is_on_screen(pos, render_size()) {
        return;
    }
    var luma_history = vec4<f32>(0.0);
    var instability_factor = 0.0;

    let dilated_motion_vector = load_dilated_motion_vector(pos);
    let uv = (vec2<f32>(pos) + 0.5) / vec2<f32>(render_size());
    let uv_curr_jittered = uv + jitter() / vec2<f32>(render_size());
    let uv_prev_jittered = uv + fsr.previous_jitter / vec2<f32>(fsr.previous_render_size);
    let reprojected_uv = uv_prev_jittered + dilated_motion_vector;

    if is_uv_inside(reprojected_uv) {
        let uv_reactive = clamp_uv(uv_curr_jittered, render_size(), fsr.max_render_size);
        let masks = sample_dilated_reactive_masks(uv_reactive);
        let reactive = saturate(masks[REACTIVE]);
        let disocclusion = saturate(masks[DISOCCLUSION]);
        let shading_change = saturate(masks[SHADING_CHANGE]);
        let accumulation = saturate(masks[ACCUMULATION]);

        if accumulation > 0.9 {
            let exposure_value = exposure();
            let uv_hw = clamp_uv(uv_curr_jittered, render_size(), fsr.max_render_size);
            let current_luma = sample_current_luma(uv_hw) * exposure_value;
            let reprojected_uv_hw = clamp_uv(reprojected_uv, fsr.previous_render_size, fsr.max_render_size);
            luma_history = textureSampleLevel(r_previous_luma_history, linear_clamp, reprojected_uv_hw, 0.0)
                * fsr.delta_pre_exposure * exposure_value;

            // Is the luma oscillating: closer to an older frame's than to last frame's,
            // on the same side?
            let diffs0 = current_luma - luma_history[0];
            let similarity0 = min_divided_by_max_or(current_luma, luma_history[0], 1.0);
            var max_similarity = similarity0;
            var instability = 0.0;
            if similarity0 < 1.0 {
                for (var i = 1; i <= 3; i++) {
                    let diffs1 = current_luma - luma_history[i];
                    let similarity1 = min_divided_by_max(current_luma, luma_history[i]);
                    if sign(diffs0) == sign(diffs1) {
                        max_similarity = max(max_similarity, similarity1);
                    }
                }
                instability = f32(max_similarity > similarity0);
            }
            luma_history = vec4<f32>(current_luma, luma_history[0], luma_history[1], luma_history[2]);
            luma_history /= exposure_value;
            instability_factor = instability * f32(luma_history[3] != 0.0);

            let velocity_weight = 1.0 - saturate(get_4k_velocity(dilated_motion_vector) / 20.0);
            instability_factor *= velocity_weight * (1.0 - disocclusion) * (1.0 - reactive) * (1.0 - shading_change);
        }
    }
    textureStore(w_luma_history, pos, luma_history);
    textureStore(w_luma_instability, pos, vec4<f32>(instability_factor, 0.0, 0.0, 0.0));
}

// ============================================================================
// Accumulate (ffx_fsr3upscaler_accumulate.h, _upsample.h, _reproject.h, _sample.h)
// ============================================================================

fn lanczos2_no_clamp(x : f32) -> f32 {
    let pi = 3.141592653589793;
    if abs(x) < EPSILON {
        return 1.0;
    }
    return (sin(pi * x) / (pi * x)) * (sin(0.5 * pi * x) / (0.5 * pi * x));
}

fn lanczos2(x : f32) -> f32 {
    return lanczos2_no_clamp(min(abs(x), 2.0));
}

// FSR1's Lanczos approximation; takes x * x.
fn lanczos2_approx_sq(x2_in : f32) -> f32 {
    let x2 = min(x2_in, 4.0);
    let a = (2.0 / 5.0) * x2 - 1.0;
    let b = (1.0 / 4.0) * x2 - 1.0;
    return ((25.0 / 16.0) * a * a - (25.0 / 16.0 - 1.0)) * (b * b);
}

fn lanczos2_row(c0 : vec4<f32>, c1 : vec4<f32>, c2 : vec4<f32>, c3 : vec4<f32>, t : f32) -> vec4<f32> {
    let w0 = lanczos2(-1.0 - t);
    let w1 = lanczos2(-0.0 - t);
    let w2 = lanczos2(1.0 - t);
    let w3 = lanczos2(2.0 - t);
    return (w0 * c0 + w1 * c1 + w2 * c2 + w3 * c3) / (w0 + w1 + w2 + w3);
}

// Clamp by offset direction, as ClampCoord.
fn clamp_coord(sample_pos : vec2<i32>, offset : vec2<i32>, size : vec2<i32>) -> vec2<i32> {
    let p = sample_pos + offset;
    return vec2<i32>(max(1, min(p.x, size.x - 2)), max(1, min(p.y, size.y - 2)));
}

fn load_history(pos : vec2<i32>, size : vec2<i32>, offset : vec2<i32>) -> vec4<f32> {
    return textureLoad(r_history, clamp_coord(pos, offset, size), 0);
}

// Lanczos 2 over 4x4 history texels, deringed to the middle 2x2.
fn history_sample(uv : vec2<f32>, size : vec2<i32>) -> vec4<f32> {
    var px = uv * vec2<f32>(size) - vec2<f32>(0.5);
    let frac = fract(px);
    px = clamp(px, vec2<f32>(0.0), vec2<f32>(size - vec2<i32>(1)));
    let p = vec2<i32>(floor(px));

    var rows : array<vec4<f32>, 4>;
    for (var y = 0; y < 4; y++) {
        rows[y] = lanczos2_row(
            load_history(p, size, vec2<i32>(-1, y - 1)),
            load_history(p, size, vec2<i32>(0, y - 1)),
            load_history(p, size, vec2<i32>(1, y - 1)),
            load_history(p, size, vec2<i32>(2, y - 1)),
            frac.x,
        );
    }
    var color = lanczos2_row(rows[0], rows[1], rows[2], rows[3], frac.y);

    let s11 = load_history(p, size, vec2<i32>(0, 0));
    let s21 = load_history(p, size, vec2<i32>(1, 0));
    let s12 = load_history(p, size, vec2<i32>(0, 1));
    let s22 = load_history(p, size, vec2<i32>(1, 1));
    let dering_min = min(min(s11, s21), min(s12, s22));
    let dering_max = max(max(s11, s21), max(s12, s22));
    return clamp(color, dering_min, dering_max);
}

struct AccumulationParams {
    hr_pos : vec2<i32>,
    hr_uv : vec2<f32>,
    lr_uv_jittered : vec2<f32>,
    lr_uv_hw : vec2<f32>,
    motion_vector : vec2<f32>,
    reprojected_hr_uv : vec2<f32>,
    velocity_4k : f32,
    disocclusion : f32,
    reactive_mask : f32,
    shading_change : f32,
    accumulation : f32,
    luma_instability : f32,
    farthest_depth_in_meters : f32,
    is_existing_sample : bool,
    is_new_sample : bool,
};

struct AccumulationData {
    clipping_box : RectificationBox,
    upsampled_color : vec3<f32>,
    upsampled_weight : f32,
    history_color : vec3<f32>,
    history_weight : f32,
    lock : f32,
    lock_contribution : f32,
};

fn init_params(hr_pos : vec2<i32>) -> AccumulationParams {
    var params : AccumulationParams;
    params.hr_pos = hr_pos;
    let hr_uv = (vec2<f32>(hr_pos) + 0.5) / vec2<f32>(fsr.upscale_size);
    params.hr_uv = hr_uv;
    params.lr_uv_jittered = hr_uv + jitter() / vec2<f32>(render_size());
    params.lr_uv_hw = clamp_uv(params.lr_uv_jittered, render_size(), fsr.max_render_size);
    // Render-size motion vectors.
    let mv_pos = clamp(vec2<i32>(hr_uv * vec2<f32>(render_size())), vec2<i32>(0), render_size() - vec2<i32>(1));
    params.motion_vector = load_dilated_motion_vector(mv_pos);
    params.velocity_4k = get_4k_velocity(params.motion_vector);
    params.reprojected_hr_uv = hr_uv + params.motion_vector;
    params.is_existing_sample = is_uv_inside(params.reprojected_hr_uv);

    let instability_uv = clamp_uv(hr_uv, render_size(), fsr.max_render_size);
    params.luma_instability = bilinear_load_4(r_luma_instability, instability_uv).x;
    let farthest_uv = clamp_uv(params.lr_uv_jittered, render_size() / 2, farthest_depth_mip1_dimensions());
    params.farthest_depth_in_meters = sample_farthest_depth_mip1(farthest_uv);
    params.is_new_sample = !params.is_existing_sample || fsr.frame_index == 0.0;

    let masks = sample_dilated_reactive_masks(params.lr_uv_hw);
    params.reactive_mask = saturate(masks[REACTIVE]);
    params.disocclusion = saturate(masks[DISOCCLUSION]);
    params.shading_change = saturate(masks[SHADING_CHANGE]);
    params.accumulation = saturate(masks[ACCUMULATION]);
    params.accumulation *= f32(round(params.accumulation * 100.0) > 1.0);
    return params;
}

fn reproject_history_color(params : AccumulationParams, data : ptr<function, AccumulationData>) {
    let history = history_sample(params.reprojected_hr_uv, fsr.previous_upscale_size);
    (*data).history_color = rgb_to_ycocg(history.rgb * fsr.delta_pre_exposure * exposure());
    (*data).lock = history.w;
}

fn update_lock_status(params : AccumulationParams, data : ptr<function, AccumulationData>) {
    var lock = (*data).lock * f32(!params.is_new_sample);
    let lifetime_decrease_factor = max(saturate(params.shading_change), max(params.reactive_mask, params.disocclusion));
    lock = max(0.0, lock - lifetime_decrease_factor * LOCK_MAX);

    // This frame's lock contribution.
    (*data).lock_contribution = saturate(saturate(lock - LOCK_THRESHOLD) * (LOCK_MAX - LOCK_THRESHOLD));

    let new_lock = textureLoad(rw_new_locks, params.hr_pos).x * (1.0 - max(params.shading_change * 0.0, params.reactive_mask));
    lock = max(0.0, min(lock + new_lock, LOCK_MAX));

    // For the next frame.
    let lifetime_decrease = (0.1 / fsr.jitter_sequence_length) * (1.0 - lifetime_decrease_factor);
    lock = max(0.0, lock - lifetime_decrease);

    // Expect similar motion next frame; drop locks that would leave the screen.
    let estimated_uv_next_frame = params.hr_uv - params.motion_vector;
    lock *= f32(is_uv_inside(estimated_uv_next_frame));
    (*data).lock = lock;
}

fn compute_base_accumulation_weight(params : AccumulationParams, data : ptr<function, AccumulationData>) {
    var base = params.accumulation;
    base = min(base, mix(base, 0.15, saturate(max(0.0, (params.velocity_4k * fsr.velocity_factor) / 0.5))));
    (*data).history_weight = base;
}

fn load_prepared_color(pos : vec2<i32>) -> vec3<f32> {
    let rgb = max(vec3<f32>(0.0), load_input_color(pos)) * exposure();
    return rgb_to_ycocg(rgb);
}

fn compute_upsampled_color_and_weight(params : AccumulationParams, data : ptr<function, AccumulationData>) {
    let dst_output_pos = vec2<f32>(params.hr_pos) + 0.5;
    let src_output_pos = dst_output_pos * fsr.downscale_factor;
    let src_input_pos = vec2<i32>(floor(src_output_pos));
    // The unjittered position of the sample at offset (0, 0).
    let src_unjittered_pos = (vec2<f32>(src_input_pos) + 0.5) - jitter();
    let base_sample_offset = src_unjittered_pos - src_output_pos;

    let flip_col = src_unjittered_pos.x > src_output_pos.x;
    let flip_row = src_unjittered_pos.y > src_output_pos.y;
    let offset_tl = vec2<i32>(select(-1, -2, flip_col), select(-1, -2, flip_row));

    let is_initial_sample = params.accumulation == 0.0;

    var samples : array<vec3<f32>, 9>;
    var index = 0;
    for (var row = 0; row < 3; row++) {
        for (var col = 0; col < 3; col++) {
            let col_row = vec2<i32>(select(col, 3 - col, flip_col), select(row, 3 - row, flip_row));
            let src_sample_pos = src_input_pos + offset_tl + col_row;
            let coord = clamp_load(src_sample_pos, vec2<i32>(0), render_size());
            samples[index] = load_prepared_color(coord);
            index++;
        }
    }
    // HDR input: a fresh sample's neighbourhood is tonemapped before it is averaged.
    if is_initial_sample {
        for (var i = 0; i < 9; i++) {
            samples[i] = rgb_to_ycocg(tonemap(ycocg_to_rgb(samples[i])));
        }
    }

    let kernel_bias_max = min(1.99, 1.0 + (1.0 / fsr.downscale_factor.x - 1.0));
    let kernel_bias_min = max(1.0, (1.0 + kernel_bias_max) * 0.3);
    let kernel_bias_weight = min(
        1.0 - params.disocclusion * 0.5,
        min(1.0 - params.shading_change, saturate((*data).history_weight * 5.0)),
    );
    let kernel_bias = mix(kernel_bias_min, kernel_bias_max, kernel_bias_weight);

    var upsampled_color = vec3<f32>(0.0);
    var upsampled_weight = 0.0;
    var clipping_box : RectificationBox;
    index = 0;
    for (var row = 0; row < 3; row++) {
        for (var col = 0; col < 3; col++) {
            let col_row = vec2<i32>(select(col, 3 - col, flip_col), select(row, 3 - row, flip_row));
            let offset = vec2<f32>(offset_tl) + vec2<f32>(col_row);
            let src_sample_offset = base_sample_offset + offset;
            let src_sample_pos = src_input_pos + offset_tl + col_row;
            let on_screen = f32(is_on_screen(src_sample_pos, render_size()));
            if !is_initial_sample {
                let biased = src_sample_offset * kernel_bias;
                let weight = on_screen * lanczos2_approx_sq(dot(biased, biased));
                upsampled_color += samples[index] * weight;
                upsampled_weight += weight;
            }
            let rectification_curve_bias = -2.3;
            let offset_sq = dot(src_sample_offset, src_sample_offset);
            let box_weight = exp(rectification_curve_bias * offset_sq) * on_screen;
            clipping_box = rectification_box_add_sample(row == 0 && col == 0, clipping_box, samples[index], box_weight);
            index++;
        }
    }
    clipping_box = rectification_box_compute_variance_box_data(clipping_box);

    upsampled_weight *= f32(upsampled_weight > EPSILON);
    if upsampled_weight > EPSILON {
        upsampled_color = upsampled_color / upsampled_weight;
        upsampled_weight *= AVERAGE_LANCZOS_WEIGHT_PER_FRAME;
        // Deringing.
        upsampled_color = clamp(upsampled_color, clipping_box.aabb_min, clipping_box.aabb_max);
    }
    if is_initial_sample {
        upsampled_color = rgb_to_ycocg(inverse_tonemap(ycocg_to_rgb(clipping_box.box_center)));
        upsampled_weight = 1.0;
        (*data).history_weight = 0.0;
    }
    (*data).clipping_box = clipping_box;
    (*data).upsampled_color = upsampled_color;
    (*data).upsampled_weight = upsampled_weight;
}

fn rectify_history(params : AccumulationParams, data : ptr<function, AccumulationData>) {
    let velocity_factor = saturate(params.velocity_4k / 20.0);
    let distance_factor = saturate(0.75 - params.farthest_depth_in_meters / 20.0);
    let accumulation_factor = 1.0 - params.accumulation;
    let reactive_factor = pow(params.reactive_mask, 1.0 / 2.0);
    let shading_change_factor = params.shading_change;
    let box_scale_t = max(velocity_factor, max(distance_factor, max(accumulation_factor, max(reactive_factor, shading_change_factor))));

    let box_scale = mix(3.0, 1.0, box_scale_t);
    let box = (*data).clipping_box;
    let scaled_box_vec = box.box_vec * vec3<f32>(1.7, 1.0, 1.0) * box_scale;
    let clamped_scaled_box_vec = max(scaled_box_vec, vec3<f32>(1.193e-7));
    let transformed_history = ((*data).history_color - box.box_center) / clamped_scaled_box_vec;
    if length(transformed_history) > 1.0 {
        let clamped_history = normalize(transformed_history);
        let final_clamped = clamped_history * scaled_box_vec + box.box_center;
        // Keep more history where luma oscillates or a thin feature is locked; never
        // where it is unsafe (disocclusion, little accumulation).
        let history_contribution = max(params.luma_instability, (*data).lock_contribution)
            * params.accumulation * (1.0 - params.disocclusion);
        (*data).history_color = mix(final_clamped, (*data).history_color, saturate(history_contribution));
    }
}

fn accumulate_color(data : ptr<function, AccumulationData>) {
    // Avoid invalid values when accumulation and upsampled weight are 0.
    var history_weight = (*data).history_weight * f32((*data).history_weight > FP16_MIN);
    history_weight = max(EPSILON, history_weight + (*data).upsampled_weight);
    (*data).history_weight = history_weight;

    // HDR input: YCoCg -> RGB -> tonemap -> YCoCg (the RGB tonemapper keeps saturation).
    let upsampled = rgb_to_ycocg(tonemap(ycocg_to_rgb((*data).upsampled_color)));
    let history = rgb_to_ycocg(tonemap(ycocg_to_rgb((*data).history_color)));

    let alpha = saturate((*data).upsampled_weight / history_weight);
    let blended = mix(history, upsampled, alpha);
    (*data).history_color = inverse_tonemap(ycocg_to_rgb(blended));
}

@compute @workgroup_size(8, 8, 1)
fn cs_accumulate(@builtin(global_invocation_id) id : vec3<u32>) {
    let hr_pos = vec2<i32>(id.xy);
    if !is_on_screen(hr_pos, fsr.upscale_size) {
        return;
    }
    let params = init_params(hr_pos);
    var data : AccumulationData;
    data.upsampled_color = vec3<f32>(0.0);
    data.history_color = vec3<f32>(0.0);
    data.history_weight = 1.0;
    data.upsampled_weight = 0.0;
    data.lock = 0.0;
    data.lock_contribution = 0.0;

    if params.is_existing_sample && !params.is_new_sample {
        reproject_history_color(params, &data);
    }
    update_lock_status(params, &data);
    compute_base_accumulation_weight(params, &data);
    compute_upsampled_color_and_weight(params, &data);
    rectify_history(params, &data);
    accumulate_color(&data);

    let color = max(data.history_color / exposure(), vec3<f32>(0.0));
    textureStore(w_history, hr_pos, vec4<f32>(color, data.lock));
    if fsr.sharpen == 0u {
        textureStore(w_output, hr_pos + fsr.output_offset, vec4<f32>(color, 1.0));
    }
    textureStore(rw_new_locks, hr_pos, vec4<f32>(0.0));
}

// ============================================================================
// RCAS (ffx_fsr3upscaler_rcas.h, FsrRcasF in fsr1/ffx_fsr1.h), with denoise
// ============================================================================

const RCAS_LIMIT : f32 = 0.25 - (1.0 / 16.0);

fn rcas_load(pos : vec2<i32>) -> vec3<f32> {
    let p = clamp(pos, vec2<i32>(0), fsr.upscale_size - vec2<i32>(1));
    return textureLoad(r_rcas_input, p, 0).rgb * exposure();
}

fn max3(a : f32, b : f32, c : f32) -> f32 {
    return max(a, max(b, c));
}

fn min3(a : f32, b : f32, c : f32) -> f32 {
    return min(a, min(b, c));
}

@compute @workgroup_size(8, 8, 1)
fn cs_rcas(@builtin(global_invocation_id) id : vec3<u32>) {
    let sp = vec2<i32>(id.xy);
    if !is_on_screen(sp, fsr.upscale_size) {
        return;
    }
    //    b
    //  d e f
    //    h
    let b = rcas_load(sp + vec2<i32>(0, -1));
    let d = rcas_load(sp + vec2<i32>(-1, 0));
    let e = rcas_load(sp);
    let f = rcas_load(sp + vec2<i32>(1, 0));
    let h = rcas_load(sp + vec2<i32>(0, 1));

    // Luma times 2.
    let b_l = b.b * 0.5 + (b.r * 0.5 + b.g);
    let d_l = d.b * 0.5 + (d.r * 0.5 + d.g);
    let e_l = e.b * 0.5 + (e.r * 0.5 + e.g);
    let f_l = f.b * 0.5 + (f.r * 0.5 + f.g);
    let h_l = h.b * 0.5 + (h.r * 0.5 + h.g);

    // Noise detection.
    var nz = 0.25 * b_l + 0.25 * d_l + 0.25 * f_l + 0.25 * h_l - e_l;
    let luma_range = max3(max3(b_l, d_l, e_l), f_l, h_l) - min3(min3(b_l, d_l, e_l), f_l, h_l);
    // The divisions below are guarded where the original's approximate reciprocals of 0
    // give inf and its min/max drop NaN.
    nz = select(0.0, saturate(abs(nz) / luma_range), luma_range > 0.0);
    nz = -0.5 * nz + 1.0;

    // Min and max of the ring.
    let mn4 = min(min(min(b, d), f), h);
    let mx4 = max(max(max(b, d), f), h);
    let peak = vec2<f32>(1.0, -4.0);
    let lower_limiter = saturate(e_l / max(min(min3(b_l, d_l, f_l), h_l), 1e-6));
    let hit_min = mn4 / max(4.0 * mx4, vec3<f32>(1e-6)) * lower_limiter;
    let max_denominator = 4.0 * mn4 + peak.y;
    let hit_max = (peak.x - mx4) / select(max_denominator, vec3<f32>(-1e-6), abs(max_denominator) < vec3<f32>(1e-6));
    let lobe_rgb = max(-hit_min, hit_max);
    var lobe = max(-RCAS_LIMIT, min(max3(lobe_rgb.r, lobe_rgb.g, lobe_rgb.b), 0.0)) * fsr.rcas_sharpness;
    lobe *= nz;

    let rcp_l = 1.0 / (4.0 * lobe + 1.0);
    let color = (lobe * b + lobe * d + lobe * h + lobe * f + e) * rcp_l;
    textureStore(w_output, sp + fsr.output_offset, vec4<f32>(max(color / exposure(), vec3<f32>(0.0)), 1.0));
}
