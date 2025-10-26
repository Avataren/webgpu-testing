struct Globals {
    view_proj: mat4x4<f32>,
    inverse_view_proj: mat4x4<f32>,
    camera_pos: vec3<f32>,
    _padding0: f32,
    camera_forward: vec3<f32>,
    _padding1: f32,
    camera_up: vec3<f32>,
    _padding2: f32,
};
@group(0) @binding(0) var<uniform> globals: Globals;

struct EnvironmentSettings {
    flags_intensity: vec4<f32>,
    ambient_color: vec4<f32>,
};
@group(1) @binding(8) var<uniform> environment_settings: EnvironmentSettings;
@group(1) @binding(9) var environment_map: texture_2d<f32>;
@group(1) @binding(10) var environment_sampler: sampler;


struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) clip: vec3<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VsOut {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(3.0, 1.0),
    );

    let xy = positions[vertex_index];
    var out: VsOut;
    out.position = vec4<f32>(xy, 0.0, 1.0);
    out.clip = vec3<f32>(xy, 1.0);
    return out;
}

fn environment_enabled() -> bool {
    return environment_settings.flags_intensity.x > 0.5;
}

fn environment_intensity() -> f32 {
    return environment_settings.flags_intensity.y;
}

fn direction_to_equirect(direction: vec3<f32>) -> vec2<f32> {
    let dir = normalize(direction);
    let theta = atan2(dir.z, dir.x);
    let phi = acos(clamp(dir.y, -1.0, 1.0));
    let u = fract(0.5 - theta / TWO_PI);
    let v = clamp(phi / PI, 0.0, 1.0);
    return vec2<f32>(u, v);
}

fn environment_uv(direction: vec3<f32>) -> vec2<f32> {
    let base_uv = direction_to_equirect(direction);
    let dims_u32 = textureDimensions(environment_map, 0);
    let dims = vec2<f32>(f32(dims_u32.x), f32(dims_u32.y));
    let safe_dims = max(dims, vec2<f32>(1.0, 1.0));
    let inv_dims = vec2<f32>(1.0, 1.0) / safe_dims;
    let texel = inv_dims * 0.5;
    let shifted = base_uv + texel;
    let wrapped_u = fract(shifted.x);
    let clamped_v = clamp(shifted.y, texel.y, 1.0 - texel.y);
    return vec2<f32>(wrapped_u, clamped_v);
}

struct BackgroundOut {
    @location(0) color: vec4<f32>,
    @location(1) normal: vec4<f32>,
    @location(2) world_pos: vec4<f32>,
};

struct BackgroundOutPick {
    @location(0) color: vec4<f32>,
    @location(1) normal: vec4<f32>,
    @location(2) world_pos: vec4<f32>,
    @location(3) pick: vec2<u32>,
};

fn sample_background(in: VsOut) -> BackgroundOut {
    if (!environment_enabled()) {
        return BackgroundOut(
            vec4<f32>(0.0, 0.0, 0.0, 1.0),
            vec4<f32>(0.0, 0.0, 0.0, 0.0),
            vec4<f32>(0.0, 0.0, 0.0, 0.0),
        );
    }

    let inv_view_proj = globals.inverse_view_proj;
    let clip = vec4<f32>(in.clip, 1.0);
    let world = inv_view_proj * clip;
    let world_pos = world.xyz / max(world.w, 1e-4);
    let dir = normalize(world_pos - globals.camera_pos);

    let uv = environment_uv(dir);
    let color = textureSampleLevel(environment_map, environment_sampler, uv, 0.0).rgb
        * environment_intensity();

    let mapped = color / (color + vec3<f32>(1.0));
    return BackgroundOut(
        vec4<f32>(mapped, 1.0),
        vec4<f32>(0.0, 0.0, 0.0, 0.0),
        vec4<f32>(0.0, 0.0, 0.0, 0.0),
    );
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return sample_background(in).color;
}

@fragment
fn fs_main_gbuffer(in: VsOut) -> BackgroundOut {
    return sample_background(in);
}

@fragment
fn fs_main_gbuffer_pick(in: VsOut) -> BackgroundOutPick {
    let base = sample_background(in);
    return BackgroundOutPick(base.color, base.normal, base.world_pos, vec2<u32>(0u, 0u));
}
