// src/shader/particle_shadow.wgsl

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

struct ParticleShadowUniform {
    view_proj: mat4x4<f32>
}

@group(0) @binding(0) var<uniform> shadow_uniform: ParticleShadowUniform;
@group(1) @binding(0) var<storage, read> particles: array<Particle>;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) tangent: vec4<f32>,
}

fn axis_angle_to_matrix(axis: vec3<f32>, angle: f32) -> mat3x3<f32> {
    let c = cos(angle);
    let s = sin(angle);
    let t = 1.0 - c;
    let x = axis.x;
    let y = axis.y;
    let z = axis.z;

    return mat3x3<f32>(
        vec3<f32>(t * x * x + c, t * x * y - s * z, t * x * z + s * y),
        vec3<f32>(t * x * y + s * z, t * y * y + c, t * y * z - s * x),
        vec3<f32>(t * x * z - s * y, t * y * z + s * x, t * z * z + c),
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
fn vs_main(vertex: VertexInput, @builtin(instance_index) instance_idx: u32) -> @builtin(position) vec4<f32> {
    let particle = particles[instance_idx];
    let axis = safe_normalized_axis(particle.rotation.xyz);
    let angle = particle.rotation.w;
    let rot_mat = axis_angle_to_matrix(axis, angle);

    let scaled_pos = vertex.position * particle.scale;
    let rotated_pos = rot_mat * scaled_pos;
    let world_pos = rotated_pos + particle.position;

    return shadow_uniform.view_proj * vec4<f32>(world_pos, 1.0);
}
