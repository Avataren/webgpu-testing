// Caustic photons: light the path tracer's camera paths cannot find. A path from the camera
// that crosses glass (or bounces off a mirror) never reaches a point, spot or directional
// light, which have no surface to hit, so light focused through glass or off a mirror onto
// a diffuse surface is traced from the lights instead (photon tracing, Jensen 1996, splatted
// to the screen as in real-time ray-traced caustics).
//
// Every fragment of the photon grid is one photon. Photons are shared out between each
// light and each caster (the bounding spheres of transmissive and smooth metal surfaces,
// raytracing/scene.rs) by the square root of the light reaching the caster (see
// photon_starts in path_tracing.rs), each aimed at its caster's sphere: over the sun's disc for
// a directional light, within the cone the sphere subtends for a point or spot light, in a
// low-discrepancy pattern that moves every frame. A photon that reaches a diffuse surface
// after at least one refraction or mirror reflection deposits its power there: projected
// to the screen, if that pixel shows that very point, it adds the irradiance it brings
// (fixed point, stochastically rounded, atomically) to the pixel's caustic sum.
// pt_caustic_resolve.wgsl gathers the sums into the diffuse light the denoiser smooths.
//
// While caustics are on, shadow rays stop at glass too (pt_visible): the light through it
// arrives focused, as photons, instead of passing straight through.
//
// Part of the trace shader module (pt_trace.wgsl's helpers and bindings).

const CAUSTIC_MAX_CASTERS : u32 = 32u;
// The lights that shine photons (path_tracing.rs MAX_CAUSTIC_LIGHTS: those bringing the
// most light to the casters).
const CAUSTIC_MAX_LIGHTS : u32 = 12u;
// Light-caster pairs' first photons, four to a vector (path_tracing.rs PAIR_START_VECTORS:
// 12 lights times 32 casters, plus the total).
const CAUSTIC_PAIR_START_VECTORS : u32 = 97u;
// Fixed-point steps per unit of demodulated diffuse light.
const CAUSTIC_SCALE : f32 = 256.0;
// Most light one photon may add to a pixel (keeps the sums from overflowing).
const CAUSTIC_MAX_DEPOSIT : f32 = 10000.0;
const CAUSTIC_MAX_EVENTS : u32 = 10u;
// Mirrors: metals at least this metallic and this smooth (as raytracing/scene.rs picks).
const CAUSTIC_MAX_ROUGHNESS : f32 = 0.2;

// Mirrors `PtCaustics` in src/renderer/postprocess/path_tracing.rs.
struct PtCaustics {
    // Bounding spheres: centre (xyz), radius (w).
    casters : array<vec4<f32>, CAUSTIC_MAX_CASTERS>,
    // Pair p (light * casters + caster) traces photons pair_start(p) .. pair_start(p + 1).
    pair_starts : array<vec4<u32>, CAUSTIC_PAIR_START_VECTORS>,
    // casters, light-caster pairs, photons, photon grid width
    counts : vec4<u32>,
    // The caustic lights, as indices into the directional lights and then the local
    // lights (an index past the directional lights' count is a local light's plus it);
    // x: how many.
    lights : array<vec4<u32>, 3>,
    light_count : vec4<u32>,
};

@group(1) @binding(194)
var<uniform> pt_caustics : PtCaustics;
// Per region pixel: red, green, blue sums in CAUSTIC_SCALE steps.
@group(1) @binding(195)
var<storage, read_write> pt_caustic_sums : array<atomic<u32>>;

// A uniformly distributed direction on the unit sphere.
fn pt_sphere_direction(u : vec2<f32>) -> vec3<f32> {
    let z = 1.0 - 2.0 * u.x;
    let r = sqrt(max(1.0 - z * z, 0.0));
    let phi = TWO_PI * u.y;
    return vec3<f32>(r * cos(phi), r * sin(phi), z);
}

struct PtPhoton {
    origin : vec3<f32>,
    direction : vec3<f32>,
    // Power (flux) it carries.
    power : vec3<f32>,
    // The light's range, for the main pass's range falloff.
    range : f32,
    valid : bool,
};

// Caustic light `slot`'s index into the directional lights, then the local lights.
fn pt_caustic_light(slot : u32) -> u32 {
    return pt_caustics.lights[slot / 4u][slot % 4u];
}

// Photon `k` of the pair (`light`, `caster`): aimed at the caster's bounding sphere,
// carrying an equal share of the light that reaches the sphere.
fn pt_emit_photon(light : u32, caster : vec4<f32>, u : vec2<f32>, per_pair : f32) -> PtPhoton {
    var photon : PtPhoton;
    let center = caster.xyz;
    let radius = max(caster.w, 1e-3);
    let dir_count = min(lights.counts.x, MAX_DIRECTIONAL_LIGHTS);
    if light < dir_count {
        // A parallel beam through the sphere's cross-section, from far away (what blocks
        // the sun still does), spread over the sun's disc.
        let source = lights.directionals[light];
        let travel = normalize(source.direction.xyz);
        let r = sqrt(u.x) * radius;
        let phi = TWO_PI * u.y;
        let on_disc = center + pt_basis(travel) * vec3<f32>(r * cos(phi), r * sin(phi), 0.0);
        photon.direction = pt_cone_direction(travel, pt.params.z, pt_rand2());
        photon.origin = on_disc - photon.direction * PT_MAX_DISTANCE * 0.5;
        photon.power = source.color_intensity.xyz * source.color_intensity.w
            * (PI * radius * radius / per_pair);
        photon.valid = true;
        return photon;
    }

    let source = lights.locals[light - dir_count];
    let position = source.position_range.xyz;
    photon.range = source.position_range.w;
    let color = source.color_intensity;
    let spot = source.direction.w > -1.5;
    let spot_direction = normalize(source.direction.xyz);
    let cone = vec2<f32>(source.params.x, source.direction.w);
    // From a point on the lamp (its radius softens the caustic as it softens shadows).
    photon.origin = position + pt_sphere_direction(pt_rand2()) * (pt.params.w * sqrt(pt_rand()));
    let to_center = center - photon.origin;
    let distance = length(to_center);
    var solid_angle = 4.0 * PI;
    if distance > radius {
        let cos_max = sqrt(max(1.0 - (radius * radius) / (distance * distance), 0.0));
        solid_angle = TWO_PI * (1.0 - cos_max);
        photon.direction = pt_cone_direction(to_center / distance, acos(cos_max), u);
    } else {
        photon.direction = pt_sphere_direction(u);
    }
    var intensity = color.w;
    if spot {
        let cos_theta = dot(spot_direction, photon.direction);
        let factor = clamp((cos_theta - cone.y) / max(cone.x - cone.y, 1e-4), 0.0, 1.0);
        intensity *= factor * factor;
    }
    photon.power = color.xyz * (intensity * solid_angle / per_pair);
    photon.valid = intensity > 0.0;
    return photon;
}

// Add a photon's light at `position` (normal `n` facing where it came from) to the pixel
// that shows it, if one does.
fn pt_deposit(position : vec3<f32>, n : vec3<f32>, power : vec3<f32>) {
    let camera = post_uniform.camera_position.xyz;
    if dot(n, camera - position) <= 0.0 {
        return;
    }
    let clip = post_uniform.view_proj * vec4<f32>(position, 1.0);
    if clip.w <= 1e-5 {
        return;
    }
    let ndc = clip.xy / clip.w;
    let viewport_uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    if any(viewport_uv < vec2<f32>(0.0)) || any(viewport_uv > vec2<f32>(1.0)) {
        return;
    }
    let dims = vec2<f32>(textureDimensions(pt_gbuffer_position));
    let pixel = vec2<i32>(viewport_to_scene_uv(viewport_uv) * dims);
    let local = pixel - vec2<i32>(pt.region.xy);
    let size = vec2<i32>(pt.region.zw);
    if any(local < vec2<i32>(0)) || any(local >= size) {
        return;
    }
    let surface = textureLoad(pt_gbuffer_position, pixel, 0);
    let depth = pt_view_depth(position);
    if surface.w < 0.5 || distance(surface.xyz, position) > 0.03 + 0.01 * depth {
        return;
    }
    // Irradiance over the pixel's patch of surface, as demodulated diffuse light (E / pi).
    let footprint = pt_footprint(depth);
    let cos_view = max(abs(dot(n, normalize(camera - position))), 0.2);
    let area = footprint * footprint / cos_view;
    let light = min(power / (area * PI), vec3<f32>(CAUSTIC_MAX_DEPOSIT));
    let steps = vec3<u32>(light * CAUSTIC_SCALE + vec3<f32>(pt_rand(), pt_rand(), pt_rand()));
    let index = u32(local.y * size.x + local.x) * 3u;
    if steps.r > 0u {
        atomicAdd(&pt_caustic_sums[index], steps.r);
    }
    if steps.g > 0u {
        atomicAdd(&pt_caustic_sums[index + 1u], steps.g);
    }
    if steps.b > 0u {
        atomicAdd(&pt_caustic_sums[index + 2u], steps.b);
    }
}

// Follow a photon through glass and off mirrors until it reaches a diffuse surface (where
// it deposits, if it was bent or reflected on the way) or leaves.
fn pt_trace_photon(photon_in : PtPhoton) {
    var origin = photon_in.origin;
    var direction = photon_in.direction;
    var power = photon_in.power;
    var travelled = 0.0;
    var focused = false;
    var inside = false;
    var medium = vec3<f32>(1.0);
    for (var event = 0u; event < CAUSTIC_MAX_EVENTS; event = event + 1u) {
        let hit = rt_closest_hit(origin, direction, PT_MAX_DISTANCE);
        if hit.kind == RAY_QUERY_INTERSECTION_NONE {
            return;
        }
        let position = origin + direction * hit.t;
        travelled += hit.t;
        if inside {
            power *= pow(max(medium, vec3<f32>(1e-4)), vec3<f32>(hit.t / PT_ABSORPTION_DISTANCE));
        }
        let material = rt_material(hit, direction, 0.0);
        let n = material.normal;
        let bias = RT_SHADOW_BIAS + 1e-4 * hit.t;

        if material.transmission > 0.0 && pt_rand() < material.transmission {
            let eta = select(1.0 / material.ior, material.ior, inside);
            let r0 = (material.ior - 1.0) / (material.ior + 1.0);
            let f0 = r0 * r0;
            let fresnel = f0 + (1.0 - f0) * pow(1.0 - clamp(dot(n, -direction), 0.0, 1.0), 5.0);
            let refracted = refract(direction, n, eta);
            if dot(refracted, refracted) < 1e-8 || pt_rand() < fresnel {
                direction = reflect(direction, n);
                origin = position + n * bias;
            } else {
                direction = refracted;
                origin = position - n * bias;
                inside = !inside;
                medium = material.base_color;
            }
            focused = true;
            continue;
        }
        inside = false;
        if material.unlit {
            return;
        }
        if pt_rand() < material.metallic && material.roughness <= CAUSTIC_MAX_ROUGHNESS {
            let alpha = material.roughness * material.roughness;
            let h = pt_sample_ggx_normal(n, -direction, alpha, pt_rand2());
            let next = reflect(direction, h);
            if dot(next, n) <= 0.0 {
                return;
            }
            power *= fresnel_schlick(max(dot(-direction, h), 0.0), material.base_color);
            direction = next;
            origin = position + n * bias;
            focused = true;
            continue;
        }
        // A diffuse surface: light that came straight from the light is direct light,
        // which the camera paths have.
        if focused {
            var falloff = 1.0;
            if photon_in.range > 0.0 {
                let f = clamp(1.0 - travelled / photon_in.range, 0.0, 1.0);
                falloff = f * f;
            }
            pt_deposit(position, n, power * falloff);
        }
        return;
    }
}

fn pt_pair_start(pair : u32) -> u32 {
    return pt_caustics.pair_starts[pair / 4u][pair % 4u];
}

@fragment
fn fs_pt_photons(in : VertexOutput) -> @location(0) vec4<f32> {
    let grid = pt_caustics.counts.w;
    let index = u32(in.position.y) * grid + u32(in.position.x);
    let casters = pt_caustics.counts.x;
    let pairs = pt_caustics.counts.y;
    if casters == 0u || pairs == 0u || index >= pt_caustics.counts.z {
        return vec4<f32>(0.0);
    }
    // The pair whose range holds this photon: the last start at or below it.
    var low = 0u;
    var high = pairs;
    while high - low > 1u {
        let middle = (low + high) / 2u;
        if pt_pair_start(middle) <= index {
            low = middle;
        } else {
            high = middle;
        }
    }
    let pair = low;
    let first = pt_pair_start(pair);
    let per_pair = pt_pair_start(pair + 1u) - first;
    if per_pair == 0u {
        return vec4<f32>(0.0);
    }
    let k = index - first;
    let slot = pair / casters;
    if slot >= pt_caustics.light_count.x {
        return vec4<f32>(0.0);
    }
    let light = pt_caustic_light(slot);
    pt_rng_state = pt_hash(index + pt_hash(pt.frame.x * 7919u + 1u));
    let offset = vec2<u32>(pt_hash(pair * 2u + 101u), pt_hash(pair * 2u + 102u));
    let u = pt_r2(offset, pt.frame.x * per_pair + k);
    let photon = pt_emit_photon(light, pt_caustics.casters[pair % casters], u, f32(per_pair));
    if photon.valid {
        pt_trace_photon(photon);
    }
    return vec4<f32>(0.0);
}
