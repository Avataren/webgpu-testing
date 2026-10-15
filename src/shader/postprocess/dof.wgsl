// Depth of field: circle of confusion (CoC) from the G-buffer positions, a half-resolution
// prefilter, and a bokeh gather along a golden-angle spiral (after Dennis Gustafsson,
// "Bokeh depth of field in a single pass"). CoC is signed (negative in front of the
// focus plane) and measured in full-resolution pixels.

@group(1) @binding(120)
var dof_scene : texture_2d<f32>;
@group(1) @binding(121)
var dof_position : texture_2d<f32>;

@group(1) @binding(130)
var dof_half : texture_2d<f32>;
@group(1) @binding(131)
var dof_half_sampler : sampler;

const DOF_GOLDEN_ANGLE : f32 = 2.39996323;
// Spiral step: larger is faster and coarser.
const DOF_RADIUS_SCALE : f32 = 1.0;
const DOF_MAX_SAMPLES : i32 = 256;
// Height of the virtual sensor the physical mode's lens projects onto (full frame).
const DOF_SENSOR_HEIGHT : f32 = 0.024;
const DOF_FAR_AWAY : f32 = 1.0e6;

// `pixel` at output resolution; the G-buffer is at the scene's (smaller when upscaling).
fn dof_view_depth(pixel : vec2<i32>) -> f32 {
    let dims = vec2<f32>(textureDimensions(dof_position));
    let position = textureLoad(dof_position, display_to_scene_pixel(vec2<f32>(pixel) + 0.5, dims), 0);
    // Background pixels carry no position: treat them as infinitely far.
    if position.w < 0.5 {
        return DOF_FAR_AWAY;
    }
    return -(post_uniform.view * vec4<f32>(position.xyz, 1.0)).z;
}

fn dof_focus_distance() -> f32 {
    if post_uniform.misc.x > 0.5 {
        // Autofocus on whatever is at the centre of the view.
        let dims = vec2<f32>(textureDimensions(dof_position));
        let center = (post_uniform.viewport_offset + 0.5 * post_uniform.viewport_scale) * dims;
        let depth = dof_view_depth(vec2<i32>(center));
        if depth < DOF_FAR_AWAY {
            return max(depth, 0.01);
        }
    }
    return max(post_uniform.dof_params.y, 0.01);
}

fn dof_coc(depth : f32, focus : f32) -> f32 {
    let max_blur = post_uniform.dof_params.w;
    if post_uniform.dof_params.x > 1.5 {
        // Manual: blur ramps up between the near and far ranges.
        let range = post_uniform.dof_ranges;
        if depth < range.y {
            return -max_blur * clamp((range.y - depth) / max(range.y - range.x, 1e-3), 0.0, 1.0);
        }
        if depth > range.z {
            return max_blur * clamp((depth - range.z) / max(range.w - range.z, 1e-3), 0.0, 1.0);
        }
        return 0.0;
    }
    // Physical thin lens: focal length from the field of view, aperture from the f-stop.
    let focal = post_uniform.proj[1][1] * DOF_SENSOR_HEIGHT * 0.5;
    let aperture = focal / max(post_uniform.dof_params.z, 0.5);
    let coc_on_sensor = aperture * focal / max(focus - focal, 1e-4) * (depth - focus) / max(depth, 1e-4);
    let coc_pixels = coc_on_sensor / DOF_SENSOR_HEIGHT * post_uniform.resolution.y;
    return clamp(coc_pixels, -max_blur, max_blur);
}

// Half resolution: average each 2x2 block and keep the CoC of its nearest pixel, so
// in-focus edges in front stay sharp.
@fragment
fn fs_dof_prefilter(in : VertexOutput) -> @location(0) vec4<f32> {
    let full = vec2<i32>(in.position.xy * 2.0);
    let dims = vec2<i32>(textureDimensions(dof_scene)) - vec2<i32>(1);
    let focus = dof_focus_distance();
    var color = vec3<f32>(0.0);
    var nearest = DOF_FAR_AWAY * 2.0;
    for (var i = 0; i < 4; i = i + 1) {
        let pixel = min(full + vec2<i32>(i % 2, i / 2), dims);
        var tap = textureLoad(dof_scene, pixel, 0).rgb;
        tap = select(tap, vec3<f32>(0.0), tap != tap);
        color = color + min(tap, vec3<f32>(65000.0));
        nearest = min(nearest, dof_view_depth(pixel));
    }
    return vec4<f32>(color * 0.25, dof_coc(nearest, focus));
}

fn dof_clamp_uv(uv : vec2<f32>) -> vec2<f32> {
    let lo = post_uniform.viewport_offset;
    return clamp(uv, lo, lo + post_uniform.viewport_scale);
}

// Gather along a spiral. A sample contributes where its own blur disc reaches this
// pixel; samples behind a sharper centre may not blur over it, while blurry near samples
// do (foreground bleeding). Alpha returns the blur size to blend by, in full-res pixels.
@fragment
fn fs_dof_blur(in : VertexOutput) -> @location(0) vec4<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(dof_half));
    let center = textureSampleLevel(dof_half, dof_half_sampler, in.uv, 0.0);
    let center_size = abs(center.a) * 0.5;
    let max_radius = post_uniform.dof_params.w * 0.5;
    var color = center.rgb;
    var total = 1.0;
    var near_size = 0.0;
    var radius = DOF_RADIUS_SCALE;
    var angle = 0.0;
    for (var i = 0; i < DOF_MAX_SAMPLES; i = i + 1) {
        if radius >= max_radius {
            break;
        }
        let offset = vec2<f32>(cos(angle), sin(angle)) * texel * radius;
        let tap = textureSampleLevel(dof_half, dof_half_sampler, dof_clamp_uv(in.uv + offset), 0.0);
        var size = abs(tap.a) * 0.5;
        if tap.a > center.a {
            size = min(size, center_size * 2.0);
        }
        let weight = smoothstep(radius - 0.5, radius + 0.5, size);
        color = color + mix(color / total, tap.rgb, weight);
        total = total + 1.0;
        if tap.a < 0.0 && tap.a < center.a {
            near_size = max(near_size, size * weight);
        }
        radius = radius + DOF_RADIUS_SCALE / radius;
        angle = angle + DOF_GOLDEN_ANGLE;
    }
    return vec4<f32>(color / total, max(center_size, near_size) * 2.0);
}
