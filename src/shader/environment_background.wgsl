// The camera (`globals`) comes from camera.wgsl.

struct EnvironmentSettings {
    flags_intensity: vec4<f32>,
    ambient_color: vec4<f32>,
    traced: vec4<f32>,
    sky: SkyParams,
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

// What the procedural sky adds to its environment map per pixel: the stars and the Milky
// Way, the moon and the sun's disc, through the air.
fn procedural_sky_details(dir: vec3<f32>, pixel: f32) -> vec3<f32> {
    let sky = environment_settings.sky;
    let through = sky_view_transmittance(sky, dir);
    let moon = sky_moon_disc(sky, dir, pixel);
    // The moon hides the stars behind it.
    let stars = sky_starlight(sky, dir, pixel) * sky.look.x * through * (1.0 - moon.a);
    return stars + moon.rgb * moon.a + sky_sun_disc(sky, dir, pixel);
}

fn sample_background(in: VsOut) -> BackgroundOut {
    let inv_view_proj = globals.inverse_view_proj;
    let clip = vec4<f32>(in.clip, 1.0);
    let world = inv_view_proj * clip;
    let world_pos = world.xyz / max(world.w, 1e-4);
    let dir = normalize(world_pos - globals.camera_pos);
    // A pixel's angular size, for the stars and the discs' edges (before any branch: the
    // derivatives need every pixel of the quad).
    let pixel = length(fwidth(dir));

    if (!environment_enabled()) {
        return BackgroundOut(
            vec4<f32>(0.0, 0.0, 0.0, 1.0),
            vec4<f32>(0.0, 0.0, 0.0, 0.0),
            vec4<f32>(0.0, 0.0, 0.0, 0.0),
        );
    }

    let uv = environment_uv(dir);
    var color = textureSampleLevel(environment_map, environment_sampler, uv, 0.0).rgb
        * environment_intensity();
    if sky_enabled(environment_settings.sky) {
        color += procedural_sky_details(dir, pixel);
    }

    // The sky stays HDR: post-processing exposes, blooms and tonemaps it.
    // Directions (w = 0) reproject without translation, like a sky at infinity.
    let curr = globals.unjittered_view_proj * vec4<f32>(dir, 0.0);
    let prev = globals.prev_view_proj * vec4<f32>(dir, 0.0);
    var velocity = vec2<f32>(0.0);
    if curr.w > 0.0 && prev.w > 0.0 {
        velocity = (curr.xy / curr.w - prev.xy / prev.w) * vec2<f32>(0.5, -0.5);
    }
    // No surface here: the normal slot carries only the motion vector (zw).
    return BackgroundOut(
        vec4<f32>(color, 1.0),
        vec4<f32>(0.0, 0.0, velocity),
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
