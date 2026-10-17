// src/shader/sky_generate.wgsl
//
// Renders the procedural sky (sky.wgsl) for the renderer: the transmittance table
// (fs_transmittance), the multiple scattering table (fs_multiple_scattering), then the
// sky's scattered light into the equirectangular environment map that lights and
// reflects the scene (fs_environment). The settings are the environment uniform
// (environment.wgsl's EnvironmentSettings).

struct SkyGenerateSettings {
    flags_intensity: vec4<f32>,
    ambient_color: vec4<f32>,
    traced: vec4<f32>,
    sky: SkyParams,
};

@group(0) @binding(0) var<uniform> settings: SkyGenerateSettings;
@group(1) @binding(0) var transmittance_lut: texture_2d<f32>;
@group(1) @binding(1) var lut_sampler: sampler;
@group(2) @binding(0) var multiple_scattering_lut: texture_2d<f32>;

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VsOut {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let xy = positions[vertex_index];
    var out: VsOut;
    out.position = vec4<f32>(xy, 0.0, 1.0);
    // v grows downward, as texture rows do.
    out.uv = vec2<f32>(xy.x * 0.5 + 0.5, 0.5 - xy.y * 0.5);
    return out;
}

@fragment
fn fs_transmittance(in: VsOut) -> @location(0) vec4<f32> {
    let params = sky_transmittance_params(in.uv);
    return vec4<f32>(sky_transmittance_texel(settings.sky, params.x, params.y), 1.0);
}

@fragment
fn fs_multiple_scattering(in: VsOut) -> @location(0) vec4<f32> {
    let light = sky_multiple_scattering_texel(settings.sky, transmittance_lut, lut_sampler, in.uv);
    return vec4<f32>(light, 1.0);
}

@fragment
fn fs_environment(in: VsOut) -> @location(0) vec4<f32> {
    // The inverse of environment.wgsl's direction_to_equirect.
    let theta = (0.5 - in.uv.x) * 2.0 * SKY_PI;
    let phi = in.uv.y * SKY_PI;
    let direction = vec3<f32>(sin(phi) * cos(theta), cos(phi), sin(phi) * sin(theta));
    let light = sky_scattering(
        settings.sky,
        transmittance_lut,
        lut_sampler,
        multiple_scattering_lut,
        direction,
    );
    return vec4<f32>(min(light, vec3<f32>(60000.0)), 1.0);
}
