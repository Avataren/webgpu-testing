// src/shader/particle_render.wgsl - Standalone version with texture sampling

// Camera uniform
struct CameraUniform {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    camera_pos: vec3<f32>,
}

// Material data
struct MaterialData {
    color: vec4<f32>,
    base_color_texture: u32,
    metallic_roughness_texture: u32,
    normal_texture: u32,
    emissive_texture: u32,
    occlusion_texture: u32,
    material_flags: u32,
    metallic_factor: f32,
    roughness_factor: f32,
    emissive_strength: f32,
}

// Particle structure
struct Particle {
    position: vec3<f32>,
    lifetime: f32,
    velocity: vec3<f32>,
    max_lifetime: f32,
    rotation: vec4<f32>,
    scale: vec3<f32>,
    angular_velocity: f32,
    color: vec4<f32>,
    user_data: vec4<f32>,
}

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
    @location(4) tangent_handedness: f32,
    @location(5) particle_color: vec4<f32>,
}

// Material flags
const FLAG_USE_BASE_COLOR_TEXTURE: u32 = 1u;

@group(0) @binding(0) var<uniform> camera: CameraUniform;
@group(1) @binding(0) var<storage, read> particles: array<Particle>;
@group(1) @binding(1) var<uniform> material_data: MaterialData;
// @group(2) is lights (not used in this simple shader yet)
@group(3) @binding(0) var textures: binding_array<texture_2d<f32>>;
@group(3) @binding(1) var texture_sampler: sampler;

@vertex
fn vs_main(
    vertex: VertexInput,
    @builtin(instance_index) instance_idx: u32,
) -> VertexOutput {
    let particle = particles[instance_idx];
    
    // Build model matrix from particle data
    let scale_mat = mat4x4<f32>(
        vec4<f32>(particle.scale.x, 0.0, 0.0, 0.0),
        vec4<f32>(0.0, particle.scale.y, 0.0, 0.0),
        vec4<f32>(0.0, 0.0, particle.scale.z, 0.0),
        vec4<f32>(0.0, 0.0, 0.0, 1.0)
    );
    
    // Rotation from axis-angle (stored as [axis.xyz, angle])
    let axis = particle.rotation.xyz;
    let angle = particle.rotation.w;
    
    // Build rotation matrix from axis-angle (Rodrigues' rotation formula)
    let c = cos(angle);
    let s = sin(angle);
    let t = 1.0 - c;
    let x = axis.x;
    let y = axis.y;
    let z = axis.z;
    
    let rot_mat = mat4x4<f32>(
        vec4<f32>(t*x*x + c,    t*x*y - s*z,  t*x*z + s*y,  0.0),
        vec4<f32>(t*x*y + s*z,  t*y*y + c,    t*y*z - s*x,  0.0),
        vec4<f32>(t*x*z - s*y,  t*y*z + s*x,  t*z*z + c,    0.0),
        vec4<f32>(0.0,          0.0,          0.0,          1.0)
    );
    
    let translation_mat = mat4x4<f32>(
        vec4<f32>(1.0, 0.0, 0.0, 0.0),
        vec4<f32>(0.0, 1.0, 0.0, 0.0),
        vec4<f32>(0.0, 0.0, 1.0, 0.0),
        vec4<f32>(particle.position, 1.0)
    );
    
    let model = translation_mat * rot_mat * scale_mat;
    
    let world_position = model * vec4<f32>(vertex.position, 1.0);
    
    var output: VertexOutput;
    output.clip_position = camera.view_proj * world_position;
    output.world_position = world_position.xyz;
    output.world_normal = normalize((model * vec4<f32>(vertex.normal, 0.0)).xyz);
    output.uv = vertex.uv;
    output.world_tangent = normalize((model * vec4<f32>(vertex.tangent.xyz, 0.0)).xyz);
    output.tangent_handedness = vertex.tangent.w;
    output.particle_color = particle.color;
    
    return output;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Sample base color texture if enabled
    var base_color = material_data.color * in.particle_color;
    
    if (material_data.material_flags & FLAG_USE_BASE_COLOR_TEXTURE) != 0u {
        let tex_index = material_data.base_color_texture;
        let tex_color = textureSample(textures[tex_index], texture_sampler, in.uv);
        base_color *= tex_color;
    }
    
    // Basic diffuse lighting
    let light_dir = normalize(vec3<f32>(0.3, -1.0, -0.5));
    let ndotl = max(dot(in.world_normal, -light_dir), 0.0);
    let ambient = 0.3;
    let lighting = ambient + (1.0 - ambient) * ndotl;
    
    return vec4<f32>(base_color.rgb * lighting, base_color.a);
}