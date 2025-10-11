struct ParticleState {
    position: vec3<f32>,
    speed: f32,
    rotation: vec4<f32>,
    angular_axis: vec3<f32>,
    angular_speed: f32,
    scale: f32,
    seed: u32,
    _padding: vec2<u32>,
};

struct Params {
    dt: f32,
    near_plane: f32,
    far_plane: f32,
    far_reset_band: f32,
    field_half_size: f32,
    min_radius: f32,
    speed_min: f32,
    speed_max: f32,
    spin_min: f32,
    spin_max: f32,
    scale_min: f32,
    scale_max: f32,
    base_instance: u32,
    particle_count: u32,
    _padding: vec2<u32>,
};

@group(0) @binding(0)
var<storage, read_write> particles: array<ParticleState>;

@group(0) @binding(1)
var<uniform> params: Params;

fn lcg_rand(seed: ptr<function, u32>) -> f32 {
    (*seed) = (*seed) * 1664525u + 1013904223u;
    return f32(*seed) / 4294967295.0;
}

fn random_range(seed: ptr<function, u32>, min_value: f32, max_value: f32) -> f32 {
    return mix(min_value, max_value, lcg_rand(seed));
}

fn random_unit_vector(seed: ptr<function, u32>) -> vec3<f32> {
    let z = random_range(seed, -1.0, 1.0);
    let theta = random_range(seed, 0.0, 6.28318530718);
    let r = sqrt(max(0.0, 1.0 - z * z));
    return vec3<f32>(r * cos(theta), r * sin(theta), z);
}

fn random_rotation(seed: ptr<function, u32>) -> vec4<f32> {
    let axis = normalize(random_unit_vector(seed));
    let angle = random_range(seed, 0.0, 6.28318530718);
    let half = angle * 0.5;
    let sin_half = sin(half);
    return vec4<f32>(axis * sin_half, cos(half));
}

fn quat_mul(a: vec4<f32>, b: vec4<f32>) -> vec4<f32> {
    let axb = cross(a.xyz, b.xyz);
    let xyz = a.w * b.xyz + b.w * a.xyz + axb;
    let w = a.w * b.w - dot(a.xyz, b.xyz);
    return vec4<f32>(xyz, w);
}

fn quat_normalize(q: vec4<f32>) -> vec4<f32> {
    let len = sqrt(max(dot(q, q), 1e-8));
    return q / len;
}

fn respawn_position(seed: ptr<function, u32>) -> vec3<f32> {
    var pos: vec2<f32>;
    loop {
        let x = random_range(seed, -params.field_half_size, params.field_half_size);
        let y = random_range(seed, -params.field_half_size, params.field_half_size);
        pos = vec2<f32>(x, y);
        if length(pos) >= params.min_radius {
            break;
        }
    }

    let depth = random_range(seed, 0.0, params.far_reset_band);
    return vec3<f32>(pos, -(params.far_plane + depth));
}

@compute @workgroup_size(256)
fn update_particles(@builtin(global_invocation_id) gid: vec3<u32>) {
    let index = gid.x;
    if index >= params.particle_count {
        return;
    }

    var state = particles[index];
    var position = state.position;
    var speed = state.speed;
    
    // Update position
    position.z = position.z + speed * params.dt;

    // Respawn check (early exit for performance)
    if position.z > -params.near_plane {
        var seed = state.seed;
        position = respawn_position(&seed);
        speed = random_range(&seed, params.speed_min, params.speed_max);
        let new_axis = random_unit_vector(&seed);
        let new_spin = random_range(&seed, params.spin_min, params.spin_max);
        let scale = random_range(&seed, params.scale_min, params.scale_max);
        let rotation = random_rotation(&seed);
        
        state.position = position;
        state.speed = speed;
        state.rotation = rotation;
        state.angular_axis = new_axis;
        state.angular_speed = new_spin;
        state.scale = scale;
        state.seed = seed;
        particles[index] = state;
        return;
    }

    // Rotation update (only if angular speed is significant)
    var rotation = state.rotation;
    let angular_speed = state.angular_speed;
    if angular_speed > 0.001 {
        let axis = normalize(state.angular_axis);
        let angle = angular_speed * params.dt;
        let half = angle * 0.5;
        let sin_half = sin(half);
        let delta = vec4<f32>(axis * sin_half, cos(half));
        rotation = quat_normalize(quat_mul(delta, rotation));
    }

    // Write back
    state.position = position;
    state.rotation = rotation;
    particles[index] = state;
}