// src/shader/particle_render.wgsl
// Particle rendering with full PBR lighting support, including shadows

// ============================================================================
// Camera Uniform
// ============================================================================

struct CameraUniform {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    camera_pos: vec3<f32>,
    _padding: f32,
}

@group(0) @binding(0) var<uniform> camera: CameraUniform;

// ============================================================================
// Particle Data
// ============================================================================

const MAX_COLOR_KEYS: u32 = 4u;

struct Particle {
    position: vec3<f32>,
    lifetime: f32,
    velocity: vec3<f32>,
    max_lifetime: f32,
    rotation: vec4<f32>,  // axis-angle: xyz = axis, w = angle
    scale: vec3<f32>,
    angular_velocity: f32,
    color: vec4<f32>,
    spawn_color: vec4<f32>,
    color_keys: array<vec4<f32>, MAX_COLOR_KEYS>,
    color_key_times: vec4<f32>,
    user_data: vec4<f32>,
}

@group(1) @binding(0) var<storage, read> particles: array<Particle>;
@group(1) @binding(1) var<uniform> material_data: MaterialData;

// ============================================================================
// Lighting and Environment
// ============================================================================

// Group 2 is for lights (@binding 0) and environment (@binding 8-10)
// These are imported from lighting_common.wgsl and environment.wgsl

// ============================================================================
// Vertex Shader
// ============================================================================

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) tangent: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) world_tangent: vec3<f32>,
    @location(4) world_bitangent: vec3<f32>,
    @location(5) particle_color: vec4<f32>,
}

fn billboard_matrix(particle_pos: vec3<f32>, camera_pos: vec3<f32>) -> mat3x3<f32> {
    let forward = normalize(camera_pos - particle_pos);
    let world_up = vec3<f32>(0.0, 1.0, 0.0);
    var right = normalize(cross(world_up, forward));
    
    let right_len = length(right);
    if (right_len < 0.001) {
        right = vec3<f32>(1.0, 0.0, 0.0);
    }
    
    let up = cross(forward, right);
    return mat3x3<f32>(right, up, forward);
}

// Build rotation matrix from axis-angle (Rodrigues' rotation formula)
fn axis_angle_to_matrix(axis: vec3<f32>, angle: f32) -> mat3x3<f32> {
    let c = cos(angle);
    let s = sin(angle);
    let t = 1.0 - c;
    let x = axis.x;
    let y = axis.y;
    let z = axis.z;
    
    return mat3x3<f32>(
        vec3<f32>(t*x*x + c,    t*x*y - s*z,  t*x*z + s*y),
        vec3<f32>(t*x*y + s*z,  t*y*y + c,    t*y*z - s*x),
        vec3<f32>(t*x*z - s*y,  t*y*z + s*x,  t*z*z + c)
    );
}

fn safe_normalized_axis(axis: vec3<f32>) -> vec3<f32> {
    let len = length(axis);
    if (len < 1e-5) {
        return vec3<f32>(0.0, 1.0, 0.0);
    }
    return axis / len;
}

@vertex
fn vs_main(
    vertex: VertexInput,
    @builtin(instance_index) instance_idx: u32,
) -> VertexOutput {
    let particle = particles[instance_idx];

    // ✅ CHECK BILLBOARDING FLAG
    let is_billboarded = (material_data.material_flags & FLAG_BILLBOARDED) != 0u;
    
    var rot_mat: mat3x3<f32>;
    
    if (is_billboarded) {
        // Billboard mode: face camera
        rot_mat = billboard_matrix(particle.position, camera.camera_pos);
    } else {
        // 3D mode: use particle rotation
        let axis = safe_normalized_axis(particle.rotation.xyz);
        let angle = particle.rotation.w;
        rot_mat = axis_angle_to_matrix(axis, angle);
    }
    
    // Apply scale and rotation to vertex (unchanged)
    let scaled_pos = vertex.position * particle.scale;
    let rotated_pos = rot_mat * scaled_pos;
    let world_pos = rotated_pos + particle.position;
    
    // Transform normal and tangent
    let world_normal = normalize(rot_mat * vertex.normal);
    let world_tangent = normalize(rot_mat * vertex.tangent.xyz);
    let world_bitangent = cross(world_normal, world_tangent) * vertex.tangent.w;
    
    var output: VertexOutput;
    output.clip_position = camera.view_proj * vec4<f32>(world_pos, 1.0);
    output.world_position = world_pos;
    output.world_normal = world_normal;
    output.uv = vertex.uv;
    output.world_tangent = world_tangent;
    output.world_bitangent = world_bitangent;
    output.particle_color = particle.color;
    
    return output;
}

// ============================================================================
// Fragment Shader
// ============================================================================

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Sample base color texture if enabled
    var base_color = material_data.color * in.particle_color;

    if ((material_data.material_flags & FLAG_USE_BASE_COLOR_TEXTURE) != 0u) {
        let tex_index = material_data.base_color_texture;
        let tex_color = sample_base_color_texture(tex_index, in.uv);
        base_color *= tex_color;
    }

    // Get material properties
    let metallic = material_data.metallic_factor;
    let roughness = max(material_data.roughness_factor, 0.01);
    
    // Normal mapping (if needed, particles usually don't use it but support it)
    var N = normalize(in.world_normal);
    if ((material_data.material_flags & FLAG_USE_NORMAL_TEXTURE) != 0u) {
        let normal_sample =
            sample_normal_texture(material_data.normal_texture, in.uv);
        let tangent_normal = normal_sample * 2.0 - 1.0;
        let T = normalize(in.world_tangent);
        let B = normalize(in.world_bitangent);
        let TBN = mat3x3<f32>(T, B, N);
        N = normalize(TBN * tangent_normal);
    }
    
    let V = normalize(camera.camera_pos - in.world_position);
    
    // Use shared lighting functions with full shadow support
    let Lo = calculate_scene_lighting(
        in.world_position, N, V, base_color.rgb, metallic, roughness
    );
    
    // Use environment lighting if available
    // This function is from environment.wgsl
    let environment_light = calculate_environment_lighting(
        N, V, base_color.rgb, metallic, roughness, 1.0  // no occlusion for particles
    );
    
    var color: vec3<f32>;
    if ((material_data.material_flags & FLAG_UNLIT) != 0u) {
        color = base_color.rgb;
    } else {
        color = environment_light + Lo;
    }
    
    // Tone mapping
    color = color / (color + vec3<f32>(1.0));
    
    return vec4<f32>(color, base_color.a);
}