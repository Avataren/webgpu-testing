// src/shader/fog.wgsl
// Fog for passes that bind the lights group (group 2): the opaque scene's fog pass
// (fog_apply.wgsl), transparent surfaces, particles and the froxel volume's own lighting
// (fog_volume.wgsl). Needs fog_common.wgsl, lighting_common.wgsl and environment.wgsl.
//
// The fog between the camera and a point comes from the froxel volume up to the depth
// the volume reaches, and past it (or all the way, without the volume) from the height
// fog's closed form, lit by the ambient light and the directional lights without shadows.

@group(2) @binding(16) var<uniform> fog: FogParams;
// The fog between the camera and each slice's far end (fog_integrate.wgsl): rgb the light
// it scatters toward the camera, a the share of light from behind that gets through. A
// single texel of no fog while the volume does not run.
@group(2) @binding(17) var fog_volume: texture_3d<f32>;

// Farther than any fog reaches: the distance to the background.
const FOG_FAR_AWAY: f32 = 1.0e7;

// The environment's ambient light, alike in every direction: the flat ambient and the
// sky's (or HDR image's) mean over up and down, as diffuse surfaces get it.
fn fog_environment_ambient() -> vec3<f32> {
    let hdr = environment_hdr_enabled();
    var light = select(vec3<f32>(1.0), environment_settings.ambient_color.rgb, hdr)
        * environment_ambient_intensity();
    if (hdr) {
        let lod = environment_settings.flags_intensity.w;
        light += (sample_environment_hdr(vec3<f32>(0.0, 1.0, 0.0), lod)
            + sample_environment_hdr(vec3<f32>(0.0, -1.0, 0.0), lod))
            * (0.5 * environment_hdr_intensity());
    }
    return light;
}

// The light the fog sends toward the eye along `view_dir` per unit of optical depth, away
// from the volume: the ambient light and the directional lights (unshadowed), scattered
// by the albedo, plus its own glow.
fn fog_distant_light(view_dir: vec3<f32>) -> vec3<f32> {
    var light = fog_environment_ambient() * fog.emission.w;
    let count = min(lights.counts.x, MAX_DIRECTIONAL_LIGHTS);
    for (var i = 0u; i < count; i = i + 1u) {
        let directional = lights.directionals[i];
        let to_light = normalize(-directional.direction.xyz);
        light += directional.color_intensity.xyz * directional.color_intensity.w
            * (fog.volume.z * fog_phase(to_light, view_dir));
    }
    return fog.albedo.rgb * light + fog.emission.rgb;
}

// The height fog along the camera's ray `dir` from t0 to t1 meters, in closed form.
fn fog_distant_segment(dir: vec3<f32>, t0: f32, t1: f32) -> vec4<f32> {
    let through = exp(-fog_optical_depth(fog.camera_position.xyz, dir, t0, t1));
    return vec4<f32>(fog_distant_light(dir) * (1.0 - through), through);
}

// The fog along the camera's ray `dir` out to `distance` meters (`depth` meters of view
// depth; `p` is any point on the ray, for where it lies on screen): rgb the light it
// scatters toward the camera, a the share of light from behind that gets through.
fn fog_along(dir: vec3<f32>, distance: f32, depth: f32, p: vec3<f32>) -> vec4<f32> {
    var near = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    var start = 0.0;
    if (fog_volumetric()) {
        let far = fog.volume.x;
        near = textureSampleLevel(
            fog_volume, environment_sampler, fog_integrated_uvw(p, min(depth, far)), 0.0
        );
        if (depth <= far) {
            return near;
        }
        // Where the ray leaves the volume.
        start = far * distance / depth;
    }
    let end = min(distance, fog.density.w);
    if (end <= start) {
        return near;
    }
    let beyond = fog_distant_segment(dir, start, end);
    return vec4<f32>(near.rgb + near.a * beyond.rgb, near.a * beyond.a);
}

// The fog between the camera and world point `p`.
fn fog_at(p: vec3<f32>) -> vec4<f32> {
    if (!fog_enabled()) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    let to_point = p - fog.camera_position.xyz;
    let distance = length(to_point);
    return fog_along(
        to_point / max(distance, 1e-6), distance, dot(to_point, fog.camera_forward.xyz), p
    );
}

// The fog in front of the background, along the camera's ray `dir`.
fn fog_toward(dir: vec3<f32>) -> vec4<f32> {
    if (!fog_enabled()) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    let depth = FOG_FAR_AWAY * max(dot(dir, fog.camera_forward.xyz), 1e-4);
    return fog_along(dir, FOG_FAR_AWAY, depth, fog.camera_position.xyz + dir);
}

// The share of light from world point `p` that gets through the fog to the camera (for
// light added after the fog pass: reflections, refraction).
fn fog_transmittance_at(p: vec3<f32>) -> f32 {
    if (!fog_enabled()) {
        return 1.0;
    }
    var volume = 1.0;
    if (fog_volumetric()) {
        let depth = min(fog_view_depth(p), fog.volume.x);
        volume = textureSampleLevel(
            fog_volume, environment_sampler, fog_integrated_uvw(p, depth), 0.0
        ).a;
    }
    return fog_transmittance_to(p, volume);
}

// `color` seen through the fog between the camera and world point `p`.
fn fog_apply(color: vec3<f32>, p: vec3<f32>) -> vec3<f32> {
    let f = fog_at(p);
    return color * f.a + f.rgb;
}
