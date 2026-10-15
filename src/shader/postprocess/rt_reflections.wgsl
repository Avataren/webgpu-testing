// Ray-traced reflections: the hardware counterpart of fs_ssr_trace (ssr.wgsl). It writes
// the same reflection texture (half or full resolution), so SSR's temporal, blur and
// apply passes finish it.
//
// Each smooth pixel's reflected ray is traced against the scene's acceleration structure,
// so it finds what it hits even when that is off screen or hidden. Where the hit point is
// what the screen shows at that spot, the reflection takes the lit colour from the screen
// (full lighting, shadows, textures, transparency); elsewhere, and on smooth surfaces
// (whose screen colour lacks their reflection), the hit is shaded here from its material,
// the environment and the lights, each light behind a shadow ray, and a smooth one traces
// its own reflection once more (rt_second_bounce).
//
// GPU particles are in the acceleration structure too (raytracing/scene.rs): opaque ones
// stop rays like any surface; alpha-blended and additive ones are gathered along the
// reflection ray up to what it hits (rt_blended_particles).
//
// Needs `enable wgpu_ray_query;` (prepended by the Rust side), post-processing's common
// and reflection_common modules, the main pass's constants, material, lighting and
// environment modules (the lights bind group is group 2, as in the main pass), and one
// of rt_textures_bindless.wgsl / rt_textures_none.wgsl for `rt_sample_texture`.

// The scene, hit and material helpers are in rt_common.wgsl.

@group(1) @binding(170)
var rt_scene_color : texture_2d<f32>;
@group(1) @binding(171)
var rt_gbuffer_normal : texture_2d<f32>;
@group(1) @binding(172)
var rt_gbuffer_position : texture_2d<f32>;
@group(1) @binding(173)
var rt_scene_sampler : sampler;
// The scene before and after particles were drawn over it, while particles are traced
// (otherwise both are the scene): where they differ, the screen shows particles.
@group(1) @binding(174)
var rt_opaque_scene : texture_2d<f32>;
@group(1) @binding(175)
var rt_particle_scene : texture_2d<f32>;

// Hit surfaces at least this smooth get a second reflection ray (a mirror seen in a
// mirror). It is not blurred by the surface's roughness, so rougher surfaces would show a
// reflection far sharper than their own.
const RT_BOUNCE_MIN_SMOOTHNESS : f32 = 0.7;

// Most blended particles one ray gathers; past these it stops looking.
const RT_MAX_BLENDED_PARTICLES : u32 = 32u;

// Light from the alpha-blended and additive particles a ray passes on its way from
// `origin` to `max_t` (rgb), and how much of what lies beyond still shows through them
// (a). The acceleration structure reports them in no particular order, so alpha-blended
// ones are combined order-independently: their colours averaged by alpha, covering as
// much as all of them together would (weighted blended order-independent transparency,
// McGuire and Bavoil 2013, without its depth weight). `pixel_angle` and `travelled` (the
// path's length up to `origin`) size the ray cone for texture levels.
fn rt_blended_particles(
    origin : vec3<f32>,
    direction : vec3<f32>,
    max_t : f32,
    pixel_angle : f32,
    travelled : f32,
) -> vec4<f32> {
    var query : ray_query;
    rayQueryInitialize(
        &query,
        rt_tlas,
        RayDesc(RAY_FLAG_NONE, RT_MASK_BLENDED_PARTICLES, 0.0, max_t, origin, direction),
    );
    var added = vec3<f32>(0.0);
    var blended = vec3<f32>(0.0);
    var coverage = 0.0;
    var transmittance = 1.0;
    var gathered = 0u;
    loop {
        if gathered >= RT_MAX_BLENDED_PARTICLES || !rayQueryProceed(&query) {
            break;
        }
        // Never confirmed: every particle along the ray comes up as a candidate.
        let hit = rayQueryGetCandidateIntersection(&query);
        if hit.kind != RAY_QUERY_INTERSECTION_TRIANGLE {
            continue;
        }
        gathered += 1u;
        let material = rt_material(hit, direction, pixel_angle * (travelled + hit.t));
        let light = rt_light(material, origin + direction * hit.t, direction);
        if (rt_instances[hit.instance_custom_data].flags & RT_INSTANCE_ADDITIVE) != 0u {
            // Additive particles are drawn adding their colour, whatever their alpha.
            added += light;
        } else {
            let alpha = clamp(material.alpha, 0.0, 1.0);
            blended += light * alpha;
            coverage += alpha;
            transmittance *= 1.0 - alpha;
        }
    }
    var color = added;
    if coverage > 1e-4 {
        color += blended / coverage * (1.0 - transmittance);
    }
    return vec4<f32>(color, transmittance);
}

// Random numbers for rt_gi_light, seeded per pixel and frame by the entry point.
var<private> rt_rng_state : u32;

fn rt_rand() -> f32 {
    let state = rt_rng_state * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    rt_rng_state = (word >> 22u) ^ word;
    return f32(rt_rng_state >> 8u) * (1.0 / 16777216.0);
}

// With traced global illumination (environment_diffuse_traced), the diffuse light at a
// reflected hit the screen does not show, from one cosine-weighted ray: the sky where it
// leaves the scene, the screen's lit colour where it lands on what the screen shows, and
// nothing where it lands on an unseen surface. The surfaces around the hit thus shade it
// as the traced global illumination shades what the screen shows (a room's walls are not
// lit by the sky through them). Noisy; the reflection's temporal filter averages it.
fn rt_gi_light(position : vec3<f32>, n : vec3<f32>) -> vec3<f32> {
    let u = vec2<f32>(rt_rand(), rt_rand());
    let r = sqrt(u.x);
    let phi = TWO_PI * u.y;
    let t = normalize(select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), abs(n.x) > 0.9) - n * dot(n, select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), abs(n.x) > 0.9)));
    let b = cross(n, t);
    let l = normalize(t * (r * cos(phi)) + b * (r * sin(phi)) + n * sqrt(max(1.0 - u.x, 0.0)));
    let start = position + n * RT_SHADOW_BIAS;
    let hit = rt_closest_hit(start, l, 1.0e3);
    if hit.kind == RAY_QUERY_INTERSECTION_NONE {
        let tint = select(vec3<f32>(1.0), environment_settings.ambient_color.rgb, environment_hdr_enabled());
        var sky = tint * environment_ambient_intensity();
        if environment_hdr_enabled() {
            sky += sample_environment_hdr(l, environment_settings.flags_intensity.w) * environment_hdr_intensity();
        }
        return sky;
    }
    let screen = rt_screen_radiance(start + l * hit.t);
    return screen.rgb * screen.w;
}

// What the main pass would have drawn at a hit: lights (each behind a shadow ray),
// environment and emission, without the surface's own reflection. With traced global
// illumination the environment's light comes from rt_gi_light instead.
fn rt_light(material : RtMaterial, position : vec3<f32>, direction : vec3<f32>) -> vec3<f32> {
    if material.unlit {
        return material.base_color + material.emissive;
    }
    let v = -direction;
    // As the main pass: light passing through a transmissive surface is not scattered
    // back diffusely (what shows through it is not traced from here).
    let lit_color = material.base_color * (1.0 - material.transmission);
    let direct = rt_direct_lighting(
        position, material.normal, v, lit_color, material.metallic, material.roughness
    );
    var ambient : vec3<f32>;
    if environment_diffuse_traced() {
        ambient = rt_gi_light(position, material.normal) * lit_color * (1.0 - material.metallic);
    } else {
        ambient = calculate_environment_lighting(
            material.normal, v, lit_color, material.metallic, material.roughness,
            material.occlusion
        );
    }
    return ambient + direct + material.emissive;
}

// The lit colour the screen shows at `position`, if the screen shows that very point
// (w = 1); w = 0 when it is off screen or something else is in front of it.
fn rt_screen_radiance(position : vec3<f32>) -> vec4<f32> {
    let projected = ssr_viewport_uv(position);
    if projected.z < 0.5 || !ssr_on_viewport(projected.xy) {
        return vec4<f32>(0.0);
    }
    let scene_uv = viewport_to_scene_uv(projected.xy);
    let dims = vec2<i32>(textureDimensions(rt_gbuffer_position));
    let pixel = clamp(vec2<i32>(scene_uv * vec2<f32>(dims)), vec2<i32>(0), dims - vec2<i32>(1));
    let surface = textureLoad(rt_gbuffer_position, pixel, 0);
    if surface.w < 0.5 {
        return vec4<f32>(0.0);
    }
    // The screen shows smooth surfaces before their reflections are added; those are
    // shaded here instead, so their own reflection can be traced. Transmissive ones
    // (marked by a negative alpha) are taken from the screen anyway: it shows what is seen
    // through them, which they are mostly made of.
    let screen_color = textureSampleLevel(rt_scene_color, rt_scene_sampler, scene_uv, 0.0);
    if ssr_smoothness(surface) >= RT_BOUNCE_MIN_SMOOTHNESS && screen_color.a >= 0.0 {
        return vec4<f32>(0.0);
    }
    // Particles drawn over the surface there are not on it (they are traced where they
    // are), so the surface is shaded here instead.
    let before = textureLoad(rt_opaque_scene, pixel, 0).rgb;
    let after = textureLoad(rt_particle_scene, pixel, 0).rgb;
    if any(abs(after - before) > vec3<f32>(1e-3) * (vec3<f32>(1.0) + before)) {
        return vec4<f32>(0.0);
    }
    // A pixel covers more of the world further away; allow for that and for TAA jitter.
    // Tuned at 1080 viewport pixels; a lower resolution (upscaling) has bigger pixels.
    let pixel_scale = max(1.0, 1080.0 / max(post_uniform.resolution.y, 1.0));
    let tolerance = 0.03 + 0.01 * ssr_view_depth(position) * pixel_scale;
    if distance(surface.xyz, position) > tolerance {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(screen_color.rgb, 1.0);
}

// What a reflected ray sees through transmissive geometry it hits off screen, straight
// on past it (opaque geometry only, no bending: refraction at reflection scale), tinted by
// its colour and weighted by transmission and 1 - Fresnel. On-screen glass comes from the
// screen instead, which has the full refraction.
fn rt_seen_through(
    hit : RayIntersection,
    material : RtMaterial,
    position : vec3<f32>,
    direction : vec3<f32>,
    cone_width : f32,
) -> vec3<f32> {
    let surface = rt_surface(hit, direction);
    let data = rt_surface_material(surface);
    let ior = max(data.ior, 1.0);
    let f0 = pow((ior - 1.0) / (ior + 1.0), 2.0);
    let fresnel = f0 + (1.0 - f0) * pow(1.0 - clamp(-dot(direction, material.normal), 0.0, 1.0), 5.0);
    let behind = rt_closest_hit_masked(position + direction * RT_SHADOW_BIAS, direction, 1.0e3, 0x01u);
    var color : vec3<f32>;
    if behind.kind == RAY_QUERY_INTERSECTION_NONE {
        color = rt_environment(direction);
    } else {
        let behind_position = position + direction * (RT_SHADOW_BIAS + behind.t);
        let screen = rt_screen_radiance(behind_position);
        if screen.w > 0.5 {
            color = screen.rgb;
        } else {
            color = rt_light(rt_material(behind, direction, cone_width), behind_position, direction);
        }
    }
    let tint = data.color.rgb * surface.color.rgb;
    return color * tint * material.transmission * (1.0 - fresnel);
}

// The reflection a smooth hit surface shows, weighted as the apply pass (fs_ssr_apply)
// weights the first one: Fresnel from the surface's specular colour, faded with roughness,
// and for metals minus the flat ambient the reflection replaces. Its own hits are shaded
// without a further bounce. `travelled` is the distance from the camera to the surface.
fn rt_second_bounce(
    material : RtMaterial,
    position : vec3<f32>,
    direction : vec3<f32>,
    pixel_angle : f32,
    travelled : f32,
    max_distance : f32,
) -> vec3<f32> {
    let n = material.normal;
    let r = reflect(direction, n);
    if dot(r, n) <= 0.0 {
        return vec3<f32>(0.0);
    }
    let start = position + n * RT_SHADOW_BIAS;
    let hit = rt_closest_hit(start, r, max_distance);
    if hit.kind == RAY_QUERY_INTERSECTION_NONE {
        return vec3<f32>(0.0);
    }
    let hit_position = start + r * hit.t;
    var color : vec3<f32>;
    let screen = rt_screen_radiance(hit_position);
    if screen.w > 0.5 {
        color = screen.rgb;
    } else {
        color = rt_light(rt_material(hit, r, pixel_angle * (travelled + hit.t)), hit_position, r);
    }

    let smoothness = 1.0 - material.roughness;
    let n_dot_v = clamp(dot(n, -direction), 0.0, 1.0);
    let f0 = mix(vec3<f32>(0.04), material.base_color, material.metallic);
    let fresnel = f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - n_dot_v, 5.0);
    let sheen = 0.2 * (1.0 - material.metallic);
    let fade = smoothness * smoothness * (1.0 - smoothstep(0.8, 1.0, hit.t / max_distance));
    let replaced = post_uniform.reflection.yzw * f0 * material.metallic * material.occlusion;
    return (color * (sheen + (1.0 - sheen) * fresnel) - replaced) * fade;
}

@fragment
fn fs_rt_reflection_trace(in : VertexOutput) -> @location(0) vec4<f32> {
    let dims = vec2<i32>(textureDimensions(rt_gbuffer_position));
    let pixel = ssr_source_pixel(in.position.xy, dims);
    let position = textureLoad(rt_gbuffer_position, pixel, 0);
    let smoothness = ssr_smoothness(position);
    if position.w < 0.5 || smoothness < SSR_MIN_SMOOTHNESS {
        return vec4<f32>(0.0);
    }
    let n = decode_normal_oct(textureLoad(rt_gbuffer_normal, pixel, 0).xy);
    let origin = position.xyz;
    rt_rng_state = (u32(pixel.x) * 1973u + u32(pixel.y) * 9277u + u32(post_uniform.output.z) * 26699u) | 1u;
    let v = normalize(origin - post_uniform.camera_position.xyz);
    let r = reflect(v, n);
    if dot(r, n) <= 0.0 {
        return vec4<f32>(0.0);
    }

    let max_distance = post_uniform.ssr.y;
    // Start a little off the surface so it does not hit itself.
    let start = origin + n * (0.01 + 0.001 * ssr_view_depth(origin));
    let hit = rt_closest_hit(start, r, max_distance);
    let missed = hit.kind == RAY_QUERY_INTERSECTION_NONE;

    // Angle one trace texel covers, from the projection and the viewport's height (not
    // the target's, which the editor draws into part of), in output pixels so reflected
    // textures keep the output's detail while upscaling.
    let output_height = max(post_uniform.resolution.y, 1.0) / render_scale();
    let pixel_angle = 2.0 * ssr_texel_scale() / (post_uniform.proj[1][1] * output_height);
    let to_origin = distance(post_uniform.camera_position.xyz, origin);
    let particles = rt_blended_particles(
        start, r, select(hit.t, max_distance, missed), pixel_angle, to_origin
    );

    if missed {
        // Misses reflect the environment, which the main pass's lighting already adds;
        // particles in front of it add their light to that (with no weight, the rest of
        // the reflection stays the main pass's). With traced global illumination the main
        // pass leaves the sky's reflection to this pass (environment_specular_traced), so
        // it is added here, where the ray reaches it.
        var light = particles.rgb;
        if environment_specular_traced() {
            light += rt_environment(r) * particles.a;
        }
        if all(light <= vec3<f32>(0.0)) {
            return vec4<f32>(0.0);
        }
        light = select(light, vec3<f32>(0.0), light != light);
        light = clamp(light, vec3<f32>(0.0), vec3<f32>(SSR_MAX_RADIANCE));
        return vec4<f32>(ssr_compress(light), 0.0);
    }

    let hit_position = start + r * hit.t;
    var color : vec3<f32>;
    let screen = rt_screen_radiance(hit_position);
    if screen.w > 0.5 {
        color = screen.rgb;
    } else {
        let travelled = to_origin + hit.t;
        let material = rt_material(hit, r, pixel_angle * travelled);
        color = rt_light(material, hit_position, r);
        let hit_smoothness = 1.0 - material.roughness;
        if !material.unlit && hit_smoothness >= RT_BOUNCE_MIN_SMOOTHNESS {
            color += rt_second_bounce(material, hit_position, r, pixel_angle, travelled + hit.t, max_distance);
        }
        if material.transmission > 0.0 {
            color += rt_seen_through(hit, material, hit_position, r, pixel_angle * (travelled + hit.t));
        }
    }
    color = color * particles.a + particles.rgb;
    color = select(color, vec3<f32>(0.0), color != color);
    color = clamp(color, vec3<f32>(0.0), vec3<f32>(SSR_MAX_RADIANCE));

    let weight = 1.0 - smoothstep(0.8, 1.0, hit.t / max_distance);
    return vec4<f32>(ssr_compress(color) * weight, weight);
}

// ---------------------------------------------------------------------------------------
// Ray-traced refraction
// ---------------------------------------------------------------------------------------

// Path events (surfaces crossed or internal reflections) a refracted ray may take before
// it gives up; a cut gem needs several.
const RT_REFRACTION_MAX_EVENTS : u32 = 8u;
// Distance through a transmissive medium over which its colour tints light fully
// (Beer-Lambert): thin glass stays nearly clear, a thick gem saturates.
const RT_REFRACTION_ABSORPTION_DISTANCE : f32 = 0.5;


// Rays per pixel, at rotated-grid sub-pixel positions: refraction through a cut gem
// changes completely across a facet edge, and one ray per pixel would make those edges
// crawl under TAA's jitter.
const RT_REFRACTION_SAMPLES : u32 = 4u;

// The world-space view ray through `scene_uv` (scene texture uv), through the same
// jittered camera the frame was drawn with.
fn rt_view_ray(scene_uv : vec2<f32>) -> vec3<f32> {
    let viewport_uv = scene_to_viewport_uv(scene_uv);
    let ndc = vec2<f32>(viewport_uv.x * 2.0 - 1.0, 1.0 - viewport_uv.y * 2.0);
    let near = post_uniform.view_proj_inv * vec4<f32>(ndc, 0.25, 1.0);
    let far = post_uniform.view_proj_inv * vec4<f32>(ndc, 0.75, 1.0);
    let a = near.xyz / near.w;
    let b = far.xyz / far.w;
    var direction = normalize(b - a);
    if dot(direction, a - post_uniform.camera_position.xyz) < 0.0 {
        direction = -direction;
    }
    return direction;
}

// Dispersion: wavelengths a dispersive pixel traces per sub-pixel sample (so
// RT_REFRACTION_SAMPLES * RT_DISPERSION_WAVELENGTHS across the visible spectrum per pixel,
// shifted every frame for TAA to average).
const RT_DISPERSION_WAVELENGTHS : u32 = 2u;
// The traced part of the spectrum, in nanometres.
const RT_SPECTRUM_SHORT : f32 = 400.0;
const RT_SPECTRUM_LONG : f32 = 700.0;

// Linear RGB response to light of `wavelength` nm (smooth bumps, violet with a little
// red), scaled so a uniform spectrum averages to white.
fn rt_spectral_rgb(wavelength : f32) -> vec3<f32> {
    let bump = vec4<f32>(605.0, 545.0, 455.0, 415.0);
    let width = vec4<f32>(40.0, 38.0, 32.0, 18.0);
    let x = (vec4<f32>(wavelength) - bump) / width;
    let g = exp(-0.5 * x * x);
    // The bumps' averages over 400-700 nm.
    return vec3<f32>(g.x + 0.18 * g.w, g.y, g.z) / vec3<f32>(0.35288, 0.31748, 0.25592);
}

// Where `wavelength` sits in the spread of the index of refraction, as Cauchy's equation
// (n = A + B / wavelength^2) puts it: -1 at the red C line (656.3 nm), 0 at the d line
// (587.6 nm, where `ior` is given), +1 at the blue F line (486.1 nm).
fn rt_dispersion_position(wavelength : f32) -> f32 {
    let inv_sq = 1.0 / (wavelength * wavelength);
    return 2.0 * (inv_sq - 2.8963e-6) / 1.9103e-6;
}

// Index of refraction at a dispersion position (above), as glTF KHR_materials_dispersion:
// the C and F lines are (ior - 1) * 0.025 * dispersion either side of ior.
fn rt_dispersed_ior(ior : f32, dispersion : f32, position : f32) -> f32 {
    let base = max(ior, 1.0);
    return max(base + (base - 1.0) * 0.025 * dispersion * position, 1.0);
}

// The light a ray brings back after refracting into a transmissive surface at `entry`
// (normal `n` facing the ray `v`, which travelled `travelled` from the camera): it
// crosses the object (bending, absorbing `tint`, reflecting internally where it cannot
// leave), refracts out and lights what it reaches. `position` is the ray's wavelength as
// a dispersion position (0 for none).
fn rt_refraction_path(
    entry : vec3<f32>,
    n : vec3<f32>,
    v : vec3<f32>,
    entry_travelled : f32,
    entry_ior : f32,
    entry_tint : vec3<f32>,
    position : f32,
    bias : f32,
    pixel_angle : f32,
) -> vec3<f32> {
    var ior = entry_ior;
    var tint = entry_tint;
    var direction = refract(v, n, 1.0 / ior);
    var origin = entry - n * bias;
    var travelled = entry_travelled;
    var throughput = vec3<f32>(1.0);
    var inside = true;
    var color = vec3<f32>(0.0);
    for (var event = 0u; event < RT_REFRACTION_MAX_EVENTS; event = event + 1u) {
        let hit = rt_closest_hit(origin, direction, 1.0e3);
        if hit.kind == RAY_QUERY_INTERSECTION_NONE {
            color = rt_environment(direction);
            break;
        }
        let hit_position = origin + direction * hit.t;
        travelled = travelled + hit.t;
        // Normal facing the ray: back into the medium the ray travels in.
        let surface = rt_surface(hit, direction);
        let material = rt_surface_material(surface);
        if inside {
            throughput *= pow(max(tint, vec3<f32>(1e-4)), vec3<f32>(hit.t / RT_REFRACTION_ABSORPTION_DISTANCE));
            let out = refract(direction, surface.normal, ior);
            if dot(out, out) < 1e-8 {
                // Total internal reflection: the ray stays inside.
                direction = reflect(direction, surface.normal);
                origin = hit_position + surface.normal * bias;
            } else {
                direction = out;
                origin = hit_position - surface.normal * bias;
                inside = false;
            }
            continue;
        }
        if material.transmission > 0.0 {
            // Into glass again (another object, or this one's far side seen past it).
            ior = rt_dispersed_ior(material.ior, material.dispersion, position);
            tint = material.color.rgb * surface.color.rgb;
            let into = refract(direction, surface.normal, 1.0 / ior);
            inside = dot(into, into) >= 1e-8;
            direction = select(reflect(direction, surface.normal), into, inside);
            origin = hit_position - surface.normal * bias * select(-1.0, 1.0, inside);
            continue;
        }
        let screen = rt_screen_radiance(hit_position);
        if screen.w > 0.5 {
            color = screen.rgb;
        } else {
            color = rt_light(rt_material(hit, direction, pixel_angle * travelled), hit_position, direction);
        }
        break;
    }
    color = select(color, vec3<f32>(0.0), color != color);
    return min(color, vec3<f32>(SSR_MAX_RADIANCE)) * throughput;
}

// Light that reaches the camera through transmissive geometry along the view ray `v`
// (zero when the ray's first hit is not transmissive), weighted by 1 - Fresnel at entry.
// A dispersive surface (one with `dispersion`) splits the ray into RT_DISPERSION_WAVELENGTHS
// wavelengths, each refracting with its own index: `sample` (this ray's sub-pixel sample)
// and `spectrum_shift` (0-1, per pixel and frame) pick which.
fn rt_refracted(v : vec3<f32>, pixel_angle : f32, sample : u32, spectrum_shift : f32) -> vec3<f32> {
    // From the near plane, as the main pass draws: anything closer (a spark drifting past
    // the lens) is clipped on screen and must not block the glass behind it.
    let forward = -normalize(post_uniform.view_inv[2].xyz);
    let near = post_uniform.near_far.x / max(dot(v, forward), 1e-3);
    let start = post_uniform.camera_position.xyz + v * near;
    let primary = rt_closest_hit(start, v, 1.0e3);
    if primary.kind == RAY_QUERY_INTERSECTION_NONE {
        return vec3<f32>(0.0);
    }
    let entry_surface = rt_surface(primary, v);
    let entry_material = rt_surface_material(entry_surface);
    if entry_material.transmission <= 0.0 {
        return vec3<f32>(0.0);
    }
    let ior = max(entry_material.ior, 1.0);
    let tint = entry_material.color.rgb * entry_surface.color.rgb;
    let n = entry_surface.normal;
    let f0 = pow((ior - 1.0) / (ior + 1.0), 2.0);
    let fresnel = f0 + (1.0 - f0) * pow(1.0 - clamp(-dot(v, n), 0.0, 1.0), 5.0);
    let weight = entry_material.transmission * (1.0 - fresnel);

    let entry = start + v * primary.t;
    let travelled = near + primary.t;
    let bias = 0.002 + 0.0005 * travelled;
    var color = vec3<f32>(0.0);
    if entry_material.dispersion <= 0.0 {
        color = rt_refraction_path(entry, n, v, travelled, ior, tint, 0.0, bias, pixel_angle);
    } else {
        // Stratified over the spectrum: the pixel's samples interleave, so each covers
        // red to violet and together they cover it evenly.
        let strata = f32(RT_REFRACTION_SAMPLES * RT_DISPERSION_WAVELENGTHS);
        for (var k = 0u; k < RT_DISPERSION_WAVELENGTHS; k = k + 1u) {
            let stratum = f32(k * RT_REFRACTION_SAMPLES + sample);
            let wavelength = mix(RT_SPECTRUM_SHORT, RT_SPECTRUM_LONG, (stratum + spectrum_shift) / strata);
            let position = rt_dispersion_position(wavelength);
            let path_ior = rt_dispersed_ior(ior, entry_material.dispersion, position);
            color += rt_spectral_rgb(wavelength)
                * rt_refraction_path(entry, n, v, travelled, path_ior, tint, position, bias, pixel_angle);
        }
        color /= f32(RT_DISPERSION_WAVELENGTHS);
    }
    return clamp(color * weight, vec3<f32>(0.0), vec3<f32>(SSR_MAX_RADIANCE));
}

// Light passing through transmissive surfaces (glass, gems), at the pixels the main pass
// marked as one (colour alpha = -transmission, see gbuffer_color in common.wgsl), averaged
// over RT_REFRACTION_SAMPLES rays across the pixel. Added to the scene; the reflection
// passes add the reflected part, and the main pass already drew the surface's highlights.
@fragment
fn fs_rt_refraction_trace(in : VertexOutput) -> @location(0) vec4<f32> {
    let dims = vec2<i32>(textureDimensions(rt_gbuffer_position));
    let pixel = min(vec2<i32>(in.position.xy), dims - vec2<i32>(1));
    if textureLoad(rt_scene_color, pixel, 0).a >= 0.0 {
        return vec4<f32>(0.0);
    }
    // An output pixel's angle (see fs_rt_reflection_trace).
    let output_height = max(post_uniform.resolution.y, 1.0) / render_scale();
    let pixel_angle = 2.0 / (post_uniform.proj[1][1] * output_height);
    // Rotated grid: four distinct rows and columns within the pixel.
    var offsets = array<vec2<f32>, 4>(
        vec2<f32>(-0.375, -0.125),
        vec2<f32>(0.125, -0.375),
        vec2<f32>(0.375, 0.125),
        vec2<f32>(-0.125, 0.375),
    );
    // Which wavelengths dispersive surfaces trace, new every frame for TAA to average.
    let spectrum_shift = noise_2d(vec2<u32>(pixel), u32(post_uniform.output.z));
    var sum = vec3<f32>(0.0);
    for (var i = 0u; i < RT_REFRACTION_SAMPLES; i = i + 1u) {
        let scene_uv = (in.position.xy + offsets[i]) / vec2<f32>(dims);
        sum += rt_refracted(rt_view_ray(scene_uv), pixel_angle * 0.5, i, spectrum_shift);
    }
    return vec4<f32>(sum / f32(RT_REFRACTION_SAMPLES), 0.0);
}
