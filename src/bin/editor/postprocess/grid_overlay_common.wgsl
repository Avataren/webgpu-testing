struct VertexOutput {
    @builtin(position) position : vec4<f32>,
    @location(0) uv : vec2<f32>,
};

@vertex
fn vs_fullscreen(@builtin(vertex_index) vertex_index : u32) -> VertexOutput {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(3.0, 1.0)
    );
    let pos = positions[vertex_index];
    var out : VertexOutput;
    out.position = vec4<f32>(pos, 0.0, 1.0);
    out.uv = vec2<f32>(0.5 * (pos.x + 1.0), 0.5 * (1.0 - pos.y));
    return out;
}

struct GridUniform {
    view_proj : mat4x4<f32>,
    view_proj_inv : mat4x4<f32>,
    camera_position : vec4<f32>,
    camera_params : vec4<f32>,
    resolution : vec2<f32>,
    viewport_offset : vec2<f32>,
    viewport_scale : vec2<f32>,
    _padding : vec2<f32>,
};

@group(0) @binding(0) var<uniform> grid_uniform : GridUniform;

// Spacing (in pixels) at which a grid level starts to appear, and where it
// reaches full strength. Levels are powers of ten metres, so the coarsest
// visible level is always at least 100x MIN_SPACING_PX apart and fully faded in.
const MIN_SPACING_PX : f32 = 8.0;
const FULL_SPACING_PX : f32 = 400.0;
// Finest level drawn: 10^-1 = 10 cm.
const MIN_LEVEL : f32 = -1.0;

const LINE_COLOR : vec3<f32> = vec3<f32>(0.78, 0.80, 0.84);
const LINE_ALPHA : f32 = 0.42;
const X_AXIS_COLOR : vec3<f32> = vec3<f32>(0.92, 0.22, 0.24);
const Z_AXIS_COLOR : vec3<f32> = vec3<f32>(0.22, 0.46, 0.96);
const AXIS_ALPHA : f32 = 0.85;
const AXIS_WIDTH_PX : f32 = 1.75;

fn viewport_to_scene_uv(uv : vec2<f32>) -> vec2<f32> {
    return grid_uniform.viewport_offset + uv * grid_uniform.viewport_scale;
}

fn viewport_pixel_size() -> vec2<f32> {
    return 1.0 / max(grid_uniform.resolution, vec2<f32>(1.0, 1.0));
}

fn scene_pixel_size() -> vec2<f32> {
    return viewport_pixel_size() * grid_uniform.viewport_scale;
}

fn reconstruct_world_position(uv : vec2<f32>, depth : f32) -> vec3<f32> {
    let ndc = vec3<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth);
    let world = grid_uniform.view_proj_inv * vec4<f32>(ndc, 1.0);
    return world.xyz / world.w;
}

struct PlaneHit {
    origin : vec3<f32>,
    point : vec3<f32>,
    dir : vec3<f32>,
    t : f32,
    valid : bool,
};

// Intersects the pixel's view ray (near plane to far plane, so orthographic
// views work too) with the y = 0 ground plane.
fn intersect_ground(uv : vec2<f32>) -> PlaneHit {
    var hit : PlaneHit;
    hit.origin = reconstruct_world_position(uv, 0.0);
    let ray = reconstruct_world_position(uv, 1.0) - hit.origin;
    let ray_len = length(ray);
    hit.dir = ray / max(ray_len, 1e-6);
    hit.t = select(-1.0, -hit.origin.y / hit.dir.y, abs(hit.dir.y) > 1e-6);
    hit.valid = hit.t > 0.0 && hit.t < ray_len;
    hit.point = hit.origin + hit.dir * hit.t;
    return hit;
}

// World-space footprint of one pixel step on the plane, from the intersection
// of a neighbouring pixel's ray (falling back to the opposite neighbour).
fn plane_step(uv : vec2<f32>, center : vec3<f32>, offset : vec2<f32>) -> vec3<f32> {
    let forward = intersect_ground(uv + offset);
    if (forward.valid) {
        return forward.point - center;
    }
    let backward = intersect_ground(uv - offset);
    if (backward.valid) {
        return center - backward.point;
    }
    return vec3<f32>(1e6);
}

// How visible the grid is through one depth sample: 1 where the scene surface
// lies behind the plane (or on it), fading to 0 as it rises above the plane on
// the camera's side. Along a ray crossing the plane, "closer than the grid" is
// exactly "on the camera's side of y = 0", which is far more robust than
// comparing depths: floors at y = 0 never z-fight and intersections stay crisp.
fn sample_visibility(scene_uv : vec2<f32>, viewport_uv : vec2<f32>, side : f32, tolerance : f32) -> f32 {
    let depth = fetch_depth(scene_uv);
    if (depth >= 1.0) {
        return 1.0;
    }
    let scene_pos = reconstruct_world_position(viewport_uv, depth);
    let height = scene_pos.y * side;
    return 1.0 - smoothstep(tolerance, tolerance * 3.0, height);
}

// Takes the minimum over a cross of samples so the grid never bleeds onto the
// anti-aliased edges of geometry standing in front of it.
fn grid_visibility(scene_uv : vec2<f32>, viewport_uv : vec2<f32>, side : f32, tolerance : f32) -> f32 {
    let scene_px = scene_pixel_size();
    let view_px = viewport_pixel_size();
    let offsets = array<vec2<f32>, 5>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(-1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, -1.0),
    );
    var visibility = 1.0;
    for (var i : i32 = 0; i < 5; i = i + 1) {
        let offset = offsets[i];
        visibility = min(
            visibility,
            sample_visibility(scene_uv + offset * scene_px, viewport_uv + offset * view_px, side, tolerance),
        );
    }
    return visibility;
}

// Anti-aliased lines of constant pixel width at every multiple of `spacing`
// along one axis. `px_per_unit` is pixels per world unit across the lines.
fn grid_lines(coord : f32, spacing : f32, px_per_unit : f32, width_px : f32) -> f32 {
    let dist_px = abs(fract(coord / spacing + 0.5) - 0.5) * spacing * px_per_unit;
    let half_width = 0.5 * width_px;
    return 1.0 - smoothstep(half_width - 0.5, half_width + 0.5, dist_px);
}

// Lines of constant `coord` drawn as power-of-ten levels. Only the three levels
// whose lines are at least MIN_SPACING_PX apart are drawn; coarser ones
// coincide with the third and are equally strong, so switching levels never
// pops. Each axis picks its levels on its own, so lines running towards the
// horizon stay visible while the ones crossing it fade.
fn grid_axis(coord : f32, px_per_unit : f32) -> f32 {
    let units_per_px = 1.0 / px_per_unit;
    let base_level = max(ceil(log2(MIN_SPACING_PX * units_per_px) / log2(10.0)), MIN_LEVEL);
    var alpha = 0.0;
    for (var i : i32 = 0; i < 3; i = i + 1) {
        let spacing = pow(10.0, base_level + f32(i));
        let strength = level_strength(spacing * px_per_unit);
        let lines = grid_lines(coord, spacing, px_per_unit, mix(1.0, 1.5, strength));
        alpha = max(alpha, lines * strength);
    }
    return alpha * LINE_ALPHA;
}

fn axis_line(coord : f32, px_per_unit : f32) -> f32 {
    let dist_px = abs(coord) * px_per_unit;
    let half_width = 0.5 * AXIS_WIDTH_PX;
    return 1.0 - smoothstep(half_width - 0.5, half_width + 0.5, dist_px);
}

// 0 for a level whose lines are MIN_SPACING_PX apart, 1 at FULL_SPACING_PX,
// measured on a log scale so each power of ten steps evenly.
fn level_strength(spacing_px : f32) -> f32 {
    let t = log2(max(spacing_px, 1e-6) / MIN_SPACING_PX) / log2(FULL_SPACING_PX / MIN_SPACING_PX);
    return smoothstep(0.0, 1.0, t);
}

fn grid_overlay(uv : vec2<f32>) -> vec4<f32> {
    let hit = intersect_ground(uv);
    if (!hit.valid) {
        return vec4<f32>(0.0);
    }

    let px = viewport_pixel_size();
    let step_x = plane_step(uv, hit.point, vec2<f32>(px.x, 0.0));
    let step_y = plane_step(uv, hit.point, vec2<f32>(0.0, px.y));
    // Pixels per world unit along x and z, i.e. how fast each coordinate
    // changes across the screen.
    let units_per_px = max(
        vec2<f32>(length(vec2<f32>(step_x.x, step_y.x)), length(vec2<f32>(step_x.z, step_y.z))),
        vec2<f32>(1e-7),
    );
    let px_per_unit = 1.0 / units_per_px;

    // Fade out where the plane is seen almost edge-on and near the far plane.
    let grazing_fade = smoothstep(0.004, 0.04, abs(hit.dir.y));
    let far = grid_uniform.camera_params.y;
    let far_fade = 1.0 - smoothstep(0.6 * far, 0.95 * far, hit.t);
    let fade = grazing_fade * far_fade;
    if (fade <= 0.0) {
        return vec4<f32>(0.0);
    }

    // Allow for depth precision and sub-pixel camera jitter when deciding
    // whether a surface sits on the plane: about a pixel's width across the ray
    // (not along the plane, which is huge at grazing angles).
    let lateral = min(
        length(step_x - hit.dir * dot(step_x, hit.dir)),
        length(step_y - hit.dir * dot(step_y, hit.dir)),
    );
    let side = select(-1.0, 1.0, hit.origin.y >= 0.0);
    let tolerance = max(lateral * 0.75, 1e-4);
    let visibility = grid_visibility(viewport_to_scene_uv(uv), uv, side, tolerance);
    if (visibility <= 0.0) {
        return vec4<f32>(0.0);
    }

    let pos = hit.point.xz;
    let line_alpha = max(grid_axis(pos.x, px_per_unit.x), grid_axis(pos.y, px_per_unit.y));

    // The X axis runs along z = 0, the Z axis along x = 0.
    let x_axis = axis_line(pos.y, px_per_unit.y);
    let z_axis = axis_line(pos.x, px_per_unit.x);

    var color = LINE_COLOR;
    var alpha = line_alpha;
    color = mix(color, X_AXIS_COLOR, x_axis);
    alpha = max(alpha, x_axis * AXIS_ALPHA);
    color = mix(color, Z_AXIS_COLOR, z_axis);
    alpha = max(alpha, z_axis * AXIS_ALPHA);

    // Seen from below the plane the grid is only a faint reference.
    let underside = select(0.4, 1.0, side > 0.0);
    return vec4<f32>(color, alpha * fade * visibility * underside);
}

@fragment
fn fs_main(in : VertexOutput) -> @location(0) vec4<f32> {
    let viewport_uv = clamp(in.uv, vec2<f32>(0.0), vec2<f32>(1.0));
    return grid_overlay(viewport_uv);
}
