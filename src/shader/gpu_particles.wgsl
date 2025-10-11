struct ObjectData {
    model: mat4x4<f32>,
    material_index: u32,
    _padding: array<u32, 3>,
    _padding2: array<u32, 4>,
};

struct ParticleState {
    position_speed: vec4<f32>,
    rotation: vec4<f32>,
    angular_axis_speed: vec4<f32>,
    scale_seed: vec4<u32>,
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
var<storage, read_write> objects: array<ObjectData>;

@group(0) @binding(2)
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

fn rotation_matrix(q: vec4<f32>) -> mat3x3<f32> {
    let x = q.x;
    let y = q.y;
    let z = q.z;
    let w = q.w;

    let xx = x + x;
    let yy = y + y;
    let zz = z + z;
    let xy = x * yy;
    let xz = x * zz;
    let yz = y * zz;
    let wx = w * xx;
    let wy = w * yy;
    let wz = w * zz;

    let c0 = vec3<f32>(1.0 - (y * yy + z * zz), xy + wz, xz - wy);
    let c1 = vec3<f32>(xy - wz, 1.0 - (x * xx + z * zz), yz + wx);
    let c2 = vec3<f32>(xz + wy, yz - wx, 1.0 - (x * xx + y * yy));
    return mat3x3<f32>(c0, c1, c2);
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

@compute @workgroup_size(128)
fn update_particles(@builtin(global_invocation_id) gid: vec3<u32>) {
    let index = gid.x;
    if index >= params.particle_count {
        return;
    }

    var state = particles[index];
    var position = state.position_speed.xyz;
    var speed = state.position_speed.w;
    position.z = position.z + speed * params.dt;

    var rotation = state.rotation;
    let angular_speed = state.angular_axis_speed.w;
    if angular_speed > 0.0 {
        let axis = normalize(state.angular_axis_speed.xyz);
        let angle = angular_speed * params.dt;
        let half = angle * 0.5;
        let sin_half = sin(half);
        let delta = vec4<f32>(axis * sin_half, cos(half));
        rotation = quat_normalize(quat_mul(delta, rotation));
    }

    var seed = state.scale_seed.y;
    var scale = bitcast<f32>(state.scale_seed.x);

    if position.z > -params.near_plane {
        position = respawn_position(&seed);
        speed = random_range(&seed, params.speed_min, params.speed_max);
        let new_axis = random_unit_vector(&seed);
        let new_spin = random_range(&seed, params.spin_min, params.spin_max);
        state.angular_axis_speed = vec4<f32>(new_axis, new_spin);
        scale = random_range(&seed, params.scale_min, params.scale_max);
        rotation = random_rotation(&seed);
    }

    state.position_speed = vec4<f32>(position, speed);
    state.rotation = rotation;
    state.scale_seed.x = bitcast<u32>(scale);
    state.scale_seed.y = seed;
    particles[index] = state;

    let rot = rotation_matrix(rotation) * scale;
    let model = mat4x4<f32>(
        vec4<f32>(rot[0], 0.0),
        vec4<f32>(rot[1], 0.0),
        vec4<f32>(rot[2], 0.0),
        vec4<f32>(position, 1.0),
    );

    let object_index = params.base_instance + index;
    var object = objects[object_index];
    object.model = model;
    objects[object_index] = object;
}
