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
    resolution : vec2<f32>,
    viewport_offset : vec2<f32>,
    viewport_scale : vec2<f32>,
    _padding : vec2<f32>,
};

@group(0) @binding(0)
var<uniform> grid_uniform : GridUniform;

fn viewport_to_scene_uv(uv : vec2<f32>) -> vec2<f32> {
    return grid_uniform.viewport_offset + uv * grid_uniform.viewport_scale;
}

fn reconstruct_world_position(uv : vec2<f32>, depth : f32) -> vec3<f32> {
    let ndc = vec3<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth);
    let clip = vec4<f32>(ndc, 1.0);
    let world = grid_uniform.view_proj_inv * clip;
    return world.xyz / world.w;
}

fn grid_line_mask(coord : vec2<f32>, width : f32) -> f32 {
    let cell = abs(fract(coord) - vec2<f32>(0.5, 0.5));
    let distance = min(cell.x, cell.y);
    return smoothstep(width, 0.0, distance);
}

fn grid_overlay(uv : vec2<f32>) -> vec4<f32> {
    let camera_pos = grid_uniform.camera_position.xyz;
    let world_far = reconstruct_world_position(uv, 1.0);
    var ray = world_far - camera_pos;
    let ray_len_sq = dot(ray, ray);
    if (ray_len_sq < 1e-6) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    let dir = normalize(ray);
    let denom = dir.y;
    if (abs(denom) < 1e-5) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }

    let t = -camera_pos.y / denom;
    if (t <= 0.0) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }

    let point = camera_pos + dir * t;
    let clip = grid_uniform.view_proj * vec4<f32>(point, 1.0);
    if (clip.w <= 0.0) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }

    let grid_depth = clip.z / clip.w;
    if (grid_depth < 0.0 || grid_depth > 1.0) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }

    let depth = fetch_depth(viewport_to_scene_uv(uv));
    if (depth < 1.0 && depth + 1e-4 < grid_depth) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }

    let grid_pos = point.xz;
    let minor = grid_line_mask(grid_pos, 0.015);
    let major = grid_line_mask(grid_pos / 10.0, 0.02);
    let axis_x = smoothstep(0.1, 0.0, abs(grid_pos.x));
    let axis_z = smoothstep(0.1, 0.0, abs(grid_pos.y));

    let distance = length(point - camera_pos);
    let fade = 1.0 - smoothstep(0.0, 1.0, distance / 120.0);
    if (fade <= 0.0) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }

    let minor_strength = minor * 0.6;
    let major_strength = major;
    let axis_x_strength = axis_x * 0.9;
    let axis_z_strength = axis_z * 0.9;

    var grid_color = vec3<f32>(0.0, 0.0, 0.0);
    grid_color = grid_color + vec3<f32>(0.32, 0.34, 0.38) * minor_strength;
    grid_color = grid_color + vec3<f32>(0.55, 0.57, 0.62) * major_strength;
    grid_color = grid_color + vec3<f32>(0.85, 0.3, 0.3) * axis_x_strength;
    grid_color = grid_color + vec3<f32>(0.3, 0.48, 0.85) * axis_z_strength;

    let intensity = clamp(max(max(minor_strength, major_strength), max(axis_x_strength, axis_z_strength)), 0.0, 1.0);
    let alpha = clamp(intensity * fade, 0.0, 1.0);

    grid_color = clamp(grid_color * fade, vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(grid_color, alpha);
}

@fragment
fn fs_main(in : VertexOutput) -> @location(0) vec4<f32> {
    let viewport_uv = clamp(in.uv, vec2<f32>(0.0), vec2<f32>(1.0));
    return grid_overlay(viewport_uv);
}
