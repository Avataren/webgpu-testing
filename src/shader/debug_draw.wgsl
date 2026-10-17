// Debug geometry (renderer/debug_draw.rs): translucent triangles and anti-aliased
// lines a few pixels wide, in world space, drawn over the finished image and depth
// tested against the scene. A second "x-ray" pass draws what is hidden faintly.
// Prepended by camera.wgsl (bind group 0).

struct DebugUniform {
    viewport_size : vec2<f32>,
    // Multiplies every alpha: 1 for the visible pass, the x-ray strength for the hidden one.
    alpha_scale : f32,
    _padding : f32,
};
@group(1) @binding(0) var<uniform> debug_uniform : DebugUniform;

// Clip-space w below which a point counts as behind the camera.
const NEAR_W : f32 = 1e-4;

// Pull a point toward the camera, so geometry lying on a surface wins the depth test
// against it.
fn lifted(p : vec3<f32>) -> vec3<f32> {
    let to_camera = globals.camera_pos - p;
    let dist = length(to_camera);
    return p + to_camera / max(dist, 1e-4) * (0.015 + dist * 0.0015);
}

struct TriangleOutput {
    @builtin(position) position : vec4<f32>,
    @location(0) color : vec4<f32>,
};

@vertex
fn vs_triangle(@location(0) position : vec4<f32>, @location(1) color : vec4<f32>) -> TriangleOutput {
    var out : TriangleOutput;
    out.position = globals.view_proj * vec4<f32>(lifted(position.xyz), 1.0);
    out.color = color;
    return out;
}

@fragment
fn fs_triangle(in : TriangleOutput) -> @location(0) vec4<f32> {
    let alpha = in.color.a * debug_uniform.alpha_scale;
    // Premultiplied alpha.
    return vec4<f32>(in.color.rgb * alpha, alpha);
}

struct LineOutput {
    @builtin(position) position : vec4<f32>,
    @location(0) color : vec4<f32>,
    // Pixels from the line's centre, across it.
    @location(1) across : f32,
    @location(2) @interpolate(flat) half_width : f32,
};

fn culled_line() -> LineOutput {
    var out : LineOutput;
    out.position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    out.color = vec4<f32>(0.0);
    out.across = 0.0;
    out.half_width = 0.0;
    return out;
}

@vertex
fn vs_line(
    @builtin(vertex_index) vertex_index : u32,
    // xyz: the line's ends; a.w: its width in pixels.
    @location(0) a : vec4<f32>,
    @location(1) b : vec4<f32>,
    @location(2) color : vec4<f32>,
) -> LineOutput {
    // Two triangles along the segment: (end, side) per corner.
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0),
    );
    let corner = corners[vertex_index];
    var ca = globals.view_proj * vec4<f32>(lifted(a.xyz), 1.0);
    var cb = globals.view_proj * vec4<f32>(lifted(b.xyz), 1.0);
    if (ca.w < NEAR_W && cb.w < NEAR_W) {
        return culled_line();
    }
    // Clip the part behind the camera.
    if (ca.w < NEAR_W) {
        ca = mix(ca, cb, (NEAR_W - ca.w) / (cb.w - ca.w));
    } else if (cb.w < NEAR_W) {
        cb = mix(cb, ca, (NEAR_W - cb.w) / (ca.w - cb.w));
    }
    let half_viewport = debug_uniform.viewport_size * 0.5;
    let sa = ca.xy / ca.w * half_viewport;
    let sb = cb.xy / cb.w * half_viewport;
    let delta = sb - sa;
    let length_px = length(delta);
    let dir = select(vec2<f32>(1.0, 0.0), delta / length_px, length_px > 1e-4);
    let normal = vec2<f32>(-dir.y, dir.x);
    // Room for anti-aliasing beyond the line's edge, and caps as long as it is wide.
    let half_width = 0.5 * a.w;
    let reach = half_width + 1.0;
    let end = select(-1.0, 1.0, corner.x > 0.5);
    let offset_px = normal * (corner.y * reach) + dir * (end * half_width);
    let clip = select(ca, cb, corner.x > 0.5);
    var out : LineOutput;
    out.position = vec4<f32>(clip.xy + offset_px / half_viewport * clip.w, clip.z, clip.w);
    out.color = color;
    out.across = corner.y * reach;
    out.half_width = half_width;
    return out;
}

@fragment
fn fs_line(in : LineOutput) -> @location(0) vec4<f32> {
    let coverage = clamp(in.half_width + 0.5 - abs(in.across), 0.0, 1.0);
    if (coverage <= 0.0) {
        discard;
    }
    let alpha = in.color.a * coverage * debug_uniform.alpha_scale;
    return vec4<f32>(in.color.rgb * alpha, alpha);
}
