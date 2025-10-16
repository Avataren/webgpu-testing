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

@group(0) @binding(0) var<uniform> grid_uniform : GridUniform;

fn viewport_to_scene_uv(uv : vec2<f32>) -> vec2<f32> {
    return grid_uniform.viewport_offset + uv * grid_uniform.viewport_scale;
}

fn reconstruct_world_position(uv : vec2<f32>, depth : f32) -> vec3<f32> {
    let ndc = vec3<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth);
    let clip = vec4<f32>(ndc, 1.0);
    let world = grid_uniform.view_proj_inv * clip;
    return world.xyz / world.w;
}

// Enhanced grid line with perspective-aware thickness
fn grid_line_mask_aa(coord : vec2<f32>, line_width : f32, distance_factor : f32) -> f32 {
    let derivative = fwidth(coord);
    
    // Perspective-aware line width - thinner in distance
    let adaptive_width = line_width * (1.0 + distance_factor * 0.5);
    
    let grid = abs(fract(coord - 0.5) - 0.5) / derivative;
    let line = min(grid.x, grid.y);
    
    // Improved aliasing prevention with smoother cutoff
    let max_derivative = max(derivative.x, derivative.y);
    let fade_filter = smoothstep(0.3, 1.2, max_derivative);
    
    // Sharper line edges with smooth falloff
    let line_alpha = 1.0 - smoothstep(0.0, adaptive_width, line);
    
    return line_alpha * (1.0 - fade_filter);
}

// Enhanced axis line with glow effect
fn axis_line_with_glow(coord : f32, width : f32, glow_width : f32) -> vec2<f32> {
    let derivative = fwidth(coord);
    let dist = abs(coord) / derivative;
    
    // Core line
    let core = 1.0 - smoothstep(0.0, width, dist);
    
    // Soft glow around the line
    let glow = exp(-dist * dist / (glow_width * glow_width)) * 0.4;
    
    return vec2<f32>(core, glow);
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
    let distance = length(point - camera_pos);
    let camera_height = abs(camera_pos.y);
    
    // Enhanced fade with exponential falloff for better depth perception
    let fade_start = 400.0;
    let fade_end = 1000.0;
    let distance_fade = 1.0 - smoothstep(fade_start, fade_end, distance);
    
    // Atmospheric perspective - subtle fog effect
    let fog_density = 0.0008;
    let fog_fade = exp(-distance * fog_density);
    
    // Height-based fade with better near-ground handling
    let height_fade = smoothstep(0.02, 1.0, camera_height);
    let combined_fade = distance_fade * height_fade * fog_fade;
    
    if (combined_fade <= 0.01) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    
    // Improved scale progression starting from 0.1 units
    let log_distance = log2(max(distance, 0.1));
    let scale_power = floor((log_distance / log2(10.0)) - 1.0);
    let scale1 = pow(10.0, scale_power);
    let scale2 = scale1 * 10.0;
    
    // Smoother transition curve
    let scale_t = fract((log_distance / log2(10.0)) - 1.0);
    let blend = smoothstep(0.15, 0.85, scale_t);
    
    // Distance factor for perspective-aware rendering
    let distance_factor = clamp(distance / 100.0, 0.0, 1.0);
    
    // Generate grid patterns
    let grid1 = grid_line_mask_aa(grid_pos / scale1, 0.8, distance_factor);
    let grid1_major = grid_line_mask_aa(grid_pos / scale2, 1.2, distance_factor);
    let grid2 = grid_line_mask_aa(grid_pos / scale2, 0.8, distance_factor);
    let grid2_major = grid_line_mask_aa(grid_pos / (scale2 * 10.0), 1.2, distance_factor);
    
    // Improved cross-fade between scale levels
    let scale1_fade = 1.0 - smoothstep(0.3, 0.95, scale_t);
    let scale2_fade = smoothstep(0.05, 0.7, scale_t);
    
    let minor = grid1 * scale1_fade + grid2 * scale2_fade;
    let major = grid1_major * scale1_fade + grid2_major * scale2_fade;
    
    // Axis lines with glow effect
    let axis_x_result = axis_line_with_glow(grid_pos.x, 1.5, 3.0);
    let axis_z_result = axis_line_with_glow(grid_pos.y, 1.5, 3.0);
    let axis_x = axis_x_result.x;
    let axis_x_glow = axis_x_result.y;
    let axis_z = axis_z_result.x;
    let axis_z_glow = axis_z_result.y;
    
    // Modern color palette with better contrast
    let minor_color = vec3<f32>(0.18, 0.20, 0.22);      // Subtle blue-gray
    let major_color = vec3<f32>(0.35, 0.38, 0.42);      // Medium blue-gray
    let axis_x_color = vec3<f32>(1.0, 0.25, 0.20);      // Vibrant red (X-axis)
    let axis_z_color = vec3<f32>(0.20, 0.55, 1.0);      // Vibrant blue (Z-axis)
    let axis_x_glow_color = vec3<f32>(1.0, 0.4, 0.3);   // Red glow
    let axis_z_glow_color = vec3<f32>(0.4, 0.65, 1.0);  // Blue glow
    
    // Enhanced distance-based intensity with exponential falloff
    let minor_distance_fade = 1.0 - smoothstep(80.0, 250.0, distance);
    let major_distance_fade = 1.0 - smoothstep(100.0, 400.0, distance);
    let axis_distance_fade = 1.0 - smoothstep(50.0, 300.0, distance);
    
    // Calculate layer strengths with improved visibility
    let minor_strength = minor * minor_distance_fade * 0.7;
    let major_strength = major * major_distance_fade * 0.9;
    let axis_x_strength = axis_x * axis_distance_fade;
    let axis_z_strength = axis_z * axis_distance_fade;
    let axis_x_glow_strength = axis_x_glow * axis_distance_fade * 0.6;
    let axis_z_glow_strength = axis_z_glow * axis_distance_fade * 0.6;
    
    // Composite layers with proper alpha blending
    var grid_color = vec3<f32>(0.0);
    var total_alpha = 0.0;
    
    // Minor grid lines (base layer)
    grid_color = minor_color * minor_strength;
    total_alpha = minor_strength;
    
    // Major grid lines (overlay)
    grid_color = mix(grid_color, major_color, major_strength);
    total_alpha = max(total_alpha, major_strength);
    
    // Axis glows (subtle background)
    let glow_contrib = axis_x_glow_color * axis_x_glow_strength + 
                       axis_z_glow_color * axis_z_glow_strength;
    grid_color = mix(grid_color, glow_contrib, axis_x_glow_strength + axis_z_glow_strength);
    
    // Axis core lines (top layer)
    grid_color = mix(grid_color, axis_x_color, axis_x_strength);
    total_alpha = max(total_alpha, axis_x_strength);
    
    grid_color = mix(grid_color, axis_z_color, axis_z_strength);
    total_alpha = max(total_alpha, axis_z_strength);
    
    // Apply overall environmental fade
    let final_alpha = total_alpha * combined_fade;
    
    // Subtle depth darkening for better 3D perception
    let depth_darken = mix(1.0, 0.85, distance_factor);
    grid_color *= depth_darken;
    
    return vec4<f32>(grid_color, final_alpha);
}

@fragment
fn fs_main(in : VertexOutput) -> @location(0) vec4<f32> {
    let viewport_uv = clamp(in.uv, vec2<f32>(0.0), vec2<f32>(1.0));
    return grid_overlay(viewport_uv);
}