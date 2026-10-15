// Path tracing: the light at every opaque pixel, traced against the scene's acceleration
// structure, replacing the main pass's lighting (pt_composite.wgsl writes it over the
// scene before particles and transparent surfaces are drawn).
//
// The surface comes from the G-buffer (its shading normal, normal maps included); a short
// ray along the pixel's view ray finds the same surface in the acceleration structure for
// its material and a full-precision position. From there:
//
// - direct light from every light, each sampled over its area (the sun's disc, a lamp's
//   radius) behind its own shadow ray, so shadows soften with distance;
// - one diffuse path (cosine-weighted) and one specular path (GGX visible normals,
//   Heitz 2018 / Dupuy and Benyoub 2023) per sample, each bouncing `bounces` times. Every
//   vertex adds its emission and one light picked in proportion to what it would add
//   unshadowed (resampled importance sampling over the lights) behind a shadow ray, then
//   continues by sampling its BRDF (Russian roulette from the third vertex). Rays leaving
//   the scene see the environment. Glass refracts or reflects by Fresnel and absorbs by
//   distance; emissive surfaces light the scene.
//
// Diffuse and specular light are written separately and demodulated (pt_common.wgsl),
// with their rays' hit distances, which the denoiser (pt_denoise.wgsl) uses to keep
// contact shadows and sharp reflections sharp. Pixels the path tracer does not handle
// (sky, transmissive surfaces the refraction pass draws, geometry not in the acceleration
// structure) are marked with emission alpha -1 and keep the main pass's colour.
//
// Global illumination (fs_gi_trace, fs_gi_composite) traces only the diffuse paths, at
// one pixel per target pixel (every scene pixel, or one of every 2x2 at half resolution),
// with no direct light at the primary surface: the main pass lights it directly and the
// reflection pass adds its specular light. After the denoiser, the composite adds the
// light times the surface's diffuse colour to the main pass's colour at full resolution.
//
// Needs `enable wgpu_ray_query;`, post-processing's fullscreen and common modules, the main
// pass's constants, material, lighting and environment modules, an rt_textures_* module,
// rt_common.wgsl and pt_common.wgsl.

@group(1) @binding(190)
var<uniform> pt : PtUniform;
@group(1) @binding(191)
var pt_gbuffer_normal : texture_2d<f32>;
@group(1) @binding(192)
var pt_gbuffer_position : texture_2d<f32>;
@group(1) @binding(193)
var pt_scene_color : texture_2d<f32>;
// Last frame's finished light (region-sized) and geometry (pt_denoise.wgsl's geometry
// target): the radiance cache.
@group(1) @binding(196)
var pt_radiance_cache : texture_2d<f32>;
@group(1) @binding(197)
var pt_prev_geometry : texture_2d<f32>;
// Global illumination's composite: the denoised indirect diffuse light and the geometry
// it was traced for (pt_denoise.wgsl's geometry target), at the targets' resolution.
@group(1) @binding(198)
var pt_gi_light : texture_2d<f32>;
@group(1) @binding(199)
var pt_gi_geometry : texture_2d<f32>;

struct PtTraceOutput {
    // Demodulated diffuse light (rgb) and its rays' mean hit distance (a).
    @location(0) diffuse : vec4<f32>,
    // Demodulated specular light (rgb) and its rays' mean hit distance (a).
    @location(1) specular : vec4<f32>,
    // Base colour (rgb) and roughness (a; 0 where nothing is path traced). Rgba8Unorm.
    @location(2) material : vec4<f32>,
    // Emitted light (rgb) and metallic (a; -1 where nothing is path traced).
    @location(3) emission : vec4<f32>,
};

// Most surfaces a path meets, glass crossings included.
const PT_MAX_EVENTS : u32 = 12u;
const PT_MAX_DISTANCE : f32 = 1.0e4;
// Brightest value a single sample may carry (keeps half floats finite).
const PT_MAX_RADIANCE : f32 = 1000.0;
// Ray cone spread after a diffuse bounce: hits after one sample coarse texture levels.
const PT_DIFFUSE_SPREAD : f32 = 0.3;
// Distance through a transmissive medium over which its colour tints light fully
// (as RT_REFRACTION_ABSORPTION_DISTANCE in rt_reflections.wgsl).
const PT_ABSORPTION_DISTANCE : f32 = 0.5;
const PT_MAX_LIGHTS : u32 = 12u;

// ---------------------------------------------------------------------------------------
// Random numbers
// ---------------------------------------------------------------------------------------

var<private> pt_rng_state : u32;

fn pt_hash(value : u32) -> u32 {
    let state = value * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

fn pt_rand() -> f32 {
    pt_rng_state = pt_hash(pt_rng_state);
    return f32(pt_rng_state >> 8u) * (1.0 / 16777216.0);
}

fn pt_rand2() -> vec2<f32> {
    return vec2<f32>(pt_rand(), pt_rand());
}

// The `index`th point of the R2 sequence (Roberts 2018) shifted by a per-pixel `offset`:
// over frames, each pixel's first-bounce directions cover the hemisphere evenly, which
// accumulation converges on faster than independent samples. Fixed point, so a large
// frame index keeps full precision.
fn pt_r2(offset : vec2<u32>, index : u32) -> vec2<f32> {
    let point = offset + index * vec2<u32>(3242174889u, 2447445413u);
    return vec2<f32>(point >> vec2<u32>(8u)) * (1.0 / 16777216.0);
}

// ---------------------------------------------------------------------------------------
// Sampling
// ---------------------------------------------------------------------------------------

// An orthonormal basis with `n` as z (Duff et al. 2017).
fn pt_basis(n : vec3<f32>) -> mat3x3<f32> {
    let s = select(-1.0, 1.0, n.z >= 0.0);
    let a = -1.0 / (s + n.z);
    let b = n.x * n.y * a;
    let t = vec3<f32>(1.0 + s * n.x * n.x * a, s * b, -s * n.x);
    let bt = vec3<f32>(b, s + n.y * n.y * a, -n.y);
    return mat3x3<f32>(t, bt, n);
}

fn pt_cosine_direction(n : vec3<f32>, u : vec2<f32>) -> vec3<f32> {
    let r = sqrt(u.x);
    let phi = TWO_PI * u.y;
    let local = vec3<f32>(r * cos(phi), r * sin(phi), sqrt(max(1.0 - u.x, 0.0)));
    return pt_basis(n) * local;
}

// A direction within `half_angle` of `axis`, uniformly over the cone's solid angle.
fn pt_cone_direction(axis : vec3<f32>, half_angle : f32, u : vec2<f32>) -> vec3<f32> {
    if half_angle <= 0.0 {
        return axis;
    }
    let cos_theta = 1.0 - u.x * (1.0 - cos(half_angle));
    let sin_theta = sqrt(max(1.0 - cos_theta * cos_theta, 0.0));
    let phi = TWO_PI * u.y;
    return pt_basis(axis) * vec3<f32>(sin_theta * cos(phi), sin_theta * sin(phi), cos_theta);
}

// A GGX microfacet normal, sampled from the normals visible from `v` (spherical caps,
// Dupuy and Benyoub 2023). `alpha` is roughness squared, as distribution_ggx uses.
fn pt_sample_ggx_normal(n : vec3<f32>, v : vec3<f32>, alpha : f32, u : vec2<f32>) -> vec3<f32> {
    let basis = pt_basis(n);
    let ve = transpose(basis) * v;
    let vh = normalize(vec3<f32>(alpha * ve.x, alpha * ve.y, ve.z));
    let phi = TWO_PI * u.x;
    let z = (1.0 - u.y) * (1.0 + vh.z) - vh.z;
    let sin_theta = sqrt(clamp(1.0 - z * z, 0.0, 1.0));
    let cap = vec3<f32>(sin_theta * cos(phi), sin_theta * sin(phi), z);
    let nh = cap + vh;
    let ne = normalize(vec3<f32>(alpha * nh.x, alpha * nh.y, max(nh.z, 0.0)));
    return basis * ne;
}

// Smith's masking for GGX in one direction.
fn pt_smith_g1(n_dot_x : f32, alpha : f32) -> f32 {
    let a2 = alpha * alpha;
    let c = max(n_dot_x, 1e-4);
    return 2.0 * c / (c + sqrt(a2 + (1.0 - a2) * c * c));
}

// ---------------------------------------------------------------------------------------
// Lights
// ---------------------------------------------------------------------------------------

fn pt_light_count() -> u32 {
    return min(lights.counts.x, MAX_DIRECTIONAL_LIGHTS)
        + min(lights.counts.y, MAX_POINT_LIGHTS)
        + min(lights.counts.z, MAX_SPOT_LIGHTS);
}

struct PtLightSample {
    direction : vec3<f32>,
    distance : f32,
    // What arrives from the light, unshadowed.
    radiance : vec3<f32>,
    // The light's centre and angular radius, for its specular highlight.
    center : vec3<f32>,
    angular_radius : f32,
};

// A point on light `index` (directional lights first, then point, then spot lights, as
// the lights buffer counts them) seen from `position`: on the sun's disc or the lamp's
// sphere, by `u`. Attenuated as the main pass attenuates.
fn pt_light_sample(index : u32, position : vec3<f32>, u : vec2<f32>) -> PtLightSample {
    var out : PtLightSample;
    let dir_count = min(lights.counts.x, MAX_DIRECTIONAL_LIGHTS);
    let point_count = min(lights.counts.y, MAX_POINT_LIGHTS);
    if index < dir_count {
        let light = lights.directionals[index];
        out.center = normalize(-light.direction.xyz);
        out.angular_radius = pt.params.z;
        out.direction = pt_cone_direction(out.center, pt.params.z, u);
        out.distance = PT_MAX_DISTANCE;
        out.radiance = light.color_intensity.xyz * light.color_intensity.w;
        return out;
    }

    var center : vec3<f32>;
    var range : f32;
    var color : vec4<f32>;
    var spot = false;
    var spot_direction = vec3<f32>(0.0);
    var cone = vec2<f32>(0.0);
    if index < dir_count + point_count {
        let light = lights.points[index - dir_count];
        center = light.position_range.xyz;
        range = light.position_range.w;
        color = light.color_intensity;
    } else {
        let light = lights.spots[index - dir_count - point_count];
        center = light.position_range.xyz;
        range = light.position_range.w;
        color = light.color_intensity;
        spot = true;
        spot_direction = normalize(light.direction.xyz);
        cone = light.cone_params.xy;
    }
    let to_center = center - position;
    let center_distance = length(to_center);
    if center_distance < 1e-4 || (range > 0.0 && center_distance > range) {
        return out;
    }
    let axis = to_center / center_distance;
    // A point on the disc of the lamp's sphere that faces `position`.
    let radius = min(pt.params.w, center_distance * 0.5);
    out.center = axis;
    out.angular_radius = asin(radius / center_distance);
    let r = sqrt(u.x) * radius;
    let phi = TWO_PI * u.y;
    let on_light = center + pt_basis(axis) * vec3<f32>(r * cos(phi), r * sin(phi), 0.0);
    let to_light = on_light - position;
    out.distance = length(to_light);
    out.direction = to_light / max(out.distance, 1e-6);

    var attenuation = 1.0 / max(center_distance * center_distance, 1e-4);
    if range > 0.0 {
        let range_factor = clamp(1.0 - center_distance / range, 0.0, 1.0);
        attenuation = attenuation * range_factor * range_factor;
    }
    if spot {
        let cos_theta = dot(spot_direction, -axis);
        let factor = clamp((cos_theta - cone.y) / max(cone.x - cone.y, 1e-4), 0.0, 1.0);
        attenuation = attenuation * factor * factor;
    }
    out.radiance = color.xyz * color.w * attenuation;
    return out;
}

// ---------------------------------------------------------------------------------------
// BRDF
// ---------------------------------------------------------------------------------------

// The main pass's BRDF for light from `l`, times n.l: diffuse without the base colour
// (it is multiplied in later), and specular.
struct PtBrdf {
    diffuse : vec3<f32>,
    specular : vec3<f32>,
};

fn pt_brdf(n : vec3<f32>, v : vec3<f32>, l : vec3<f32>, f0 : vec3<f32>, roughness : f32) -> PtBrdf {
    var out : PtBrdf;
    let n_dot_l = dot(n, l);
    if n_dot_l <= 0.0 {
        return out;
    }
    let n_dot_v = max(dot(n, v), 1e-4);
    let h = normalize(v + l);
    let f = fresnel_schlick(max(dot(h, v), 0.0), f0);
    let d = distribution_ggx(n, h, roughness);
    let g = geometry_smith(n, v, l, roughness);
    out.specular = d * g * f / (4.0 * n_dot_v * n_dot_l + 1e-4) * n_dot_l;
    out.diffuse = (vec3<f32>(1.0) - f) * (n_dot_l / PI);
    return out;
}

// A sphere or disc light's specular highlight, times n.l, without random sampling (the
// "representative point", Karis 2013): the BRDF toward the point of the light nearest the
// reflected ray, with the lobe widened by the light's size and its energy renormalized.
// Sampling a point on the light instead is very noisy on smooth surfaces, whose lobe is
// narrower than the light.
fn pt_area_specular(
    n : vec3<f32>,
    v : vec3<f32>,
    center : vec3<f32>,
    angular_radius : f32,
    f0 : vec3<f32>,
    roughness : f32,
) -> vec3<f32> {
    let r = reflect(-v, n);
    var l = center;
    let c = dot(r, center);
    if c >= cos(angular_radius) {
        l = r;
    } else {
        let toward = r - center * c;
        let length_toward = length(toward);
        if length_toward > 1e-5 {
            l = normalize(center * cos(angular_radius) + toward / length_toward * sin(angular_radius));
        }
    }
    let n_dot_l = dot(n, l);
    if n_dot_l <= 0.0 {
        return vec3<f32>(0.0);
    }
    let n_dot_v = max(dot(n, v), 1e-4);
    let h = normalize(v + l);
    let alpha = roughness * roughness;
    let wide = min(alpha + 0.5 * angular_radius, 1.0);
    let a2 = wide * wide;
    let n_dot_h = max(dot(n, h), 0.0);
    let denom = n_dot_h * n_dot_h * (a2 - 1.0) + 1.0;
    let d = a2 / (PI * denom * denom) * (alpha * alpha) / (wide * wide);
    let g = geometry_smith(n, v, l, roughness);
    let f = fresnel_schlick(max(dot(h, v), 0.0), f0);
    return d * g * f / (4.0 * n_dot_v * n_dot_l + 1e-4) * n_dot_l;
}

// Whether nothing lies within `distance` of `origin` along `direction` that stops light:
// opaque surfaces, and glass too while caustic photons carry the light through it.
fn pt_visible(origin : vec3<f32>, direction : vec3<f32>, distance : f32) -> bool {
    if distance <= 0.0 {
        return false;
    }
    if (pt.frame.w & PT_FLAG_CAUSTICS) == 0u {
        return rt_unoccluded(origin, direction, distance);
    }
    var query : ray_query;
    rayQueryInitialize(
        &query,
        rt_tlas,
        RayDesc(
            RAY_FLAG_TERMINATE_ON_FIRST_HIT,
            0x03u,
            0.0,
            distance,
            origin,
            direction,
        ),
    );
    loop {
        if !rayQueryProceed(&query) {
            break;
        }
        // Cut-out surfaces come up as candidates (rt_alpha.wgsl).
        if rt_candidate_blocks(rayQueryGetCandidateIntersection(&query)) {
            rayQueryConfirmIntersection(&query);
        }
    }
    return rayQueryGetCommittedIntersection(&query).kind == RAY_QUERY_INTERSECTION_NONE;
}

// What one light, picked among all in proportion to what each would add unshadowed (a
// sample at its centre), adds at a path vertex behind a shadow ray, divided by the
// chance of picking it.
fn pt_sample_one_light(
    position : vec3<f32>,
    n : vec3<f32>,
    v : vec3<f32>,
    diffuse_color : vec3<f32>,
    f0 : vec3<f32>,
    roughness : f32,
    bias : f32,
) -> vec3<f32> {
    let count = min(pt_light_count(), PT_MAX_LIGHTS);
    if count == 0u {
        return vec3<f32>(0.0);
    }
    var weights : array<f32, PT_MAX_LIGHTS>;
    var total = 0.0;
    for (var i = 0u; i < count; i = i + 1u) {
        let s = pt_light_sample(i, position, vec2<f32>(0.5));
        let brdf = pt_brdf(n, v, s.direction, f0, roughness);
        let w = pt_luminance((brdf.diffuse * diffuse_color + brdf.specular) * s.radiance);
        weights[i] = w;
        total += w;
    }
    if total <= 0.0 {
        return vec3<f32>(0.0);
    }
    var pick = pt_rand() * total;
    var chosen = count - 1u;
    for (var i = 0u; i < count; i = i + 1u) {
        pick -= weights[i];
        if pick <= 0.0 && weights[i] > 0.0 {
            chosen = i;
            break;
        }
    }
    let probability = weights[chosen] / total;
    if probability <= 0.0 {
        return vec3<f32>(0.0);
    }
    let s = pt_light_sample(chosen, position, pt_rand2());
    let brdf = pt_brdf(n, v, s.direction, f0, roughness);
    let light = (brdf.diffuse * diffuse_color + brdf.specular) * s.radiance;
    if all(light <= vec3<f32>(0.0)) {
        return vec3<f32>(0.0);
    }
    if !pt_visible(position + n * bias, s.direction, s.distance - bias) {
        return vec3<f32>(0.0);
    }
    return light / probability;
}

// ---------------------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------------------

// What a ray leaving the scene sees: the environment, plus the flat ambient light the main
// pass adds everywhere as a uniform sky (so an open surface gets what the main pass gives
// it, and an enclosed one less). Rougher paths read blurrier levels of the map, which
// stands in for the many samples a sharp one would need.
fn pt_environment(direction : vec3<f32>, roughness : f32) -> vec3<f32> {
    let hdr = environment_hdr_enabled();
    let tint = select(vec3<f32>(1.0), environment_settings.ambient_color.rgb, hdr);
    var sky = tint * environment_ambient_intensity();
    if hdr {
        let lod = environment_settings.flags_intensity.w * roughness * roughness;
        sky += sample_environment_hdr(direction, lod) * environment_hdr_intensity();
    }
    return sky;
}

// Surfaces at least this rough look about the same from anywhere, so last frame's view of
// them stands in for this ray's.
const PT_CACHE_MIN_ROUGHNESS : f32 = 0.3;

// The light leaving `position` (normal `n`, facing where the ray came from) as last
// frame's finished image showed it, if it showed that very point from the same side
// (w = 1); w = 0 otherwise. Denoised and with every bounce in it, it replaces the rest of
// a path, which takes most of the noise out of reflections and bounced light (and they
// show caustics); paths that leave the screen trace on.
fn pt_cached_radiance(position : vec3<f32>, n : vec3<f32>) -> vec4<f32> {
    if (pt.frame.w & PT_FLAG_HISTORY_VALID) == 0u {
        return vec4<f32>(0.0);
    }
    let prev = pt_prev_pixel(position);
    if prev.z < 0.5 {
        return vec4<f32>(0.0);
    }
    let q = vec2<i32>(floor(prev.xy));
    if any(q < vec2<i32>(0)) || any(q >= vec2<i32>(pt.region.zw)) {
        return vec4<f32>(0.0);
    }
    let geometry = textureLoad(pt_prev_geometry, q, 0);
    if geometry.w <= 0.0 || dot(pt_decode_normal(geometry.xy), n) < 0.8 {
        return vec4<f32>(0.0);
    }
    let depth = pt_prev_view_depth(position);
    if abs(geometry.z - depth) > 0.01 * depth + 4.0 * pt_footprint(depth) {
        return vec4<f32>(0.0);
    }
    // The cache is the scene region's size; the geometry may be half of it.
    return vec4<f32>(textureLoad(pt_radiance_cache, q * i32(pt.scale.x), 0).rgb, 1.0);
}

// Light arriving along a ray from `origin` through up to `bounces` surfaces. `width` and
// `spread` are the ray cone's width (m) and spread (radians) at the origin, for texture
// levels; `roughness` is the roughest lobe the path took so far. Returns the radiance and
// the distance to the first hit.
fn pt_trace_path(
    origin_in : vec3<f32>,
    direction_in : vec3<f32>,
    bounces : u32,
    width_in : f32,
    spread_in : f32,
    roughness_in : f32,
) -> vec4<f32> {
    var origin = origin_in;
    var direction = direction_in;
    var width = width_in;
    var spread = spread_in;
    var path_roughness = roughness_in;
    var throughput = vec3<f32>(1.0);
    var radiance = vec3<f32>(0.0);
    var first_distance = PT_MISS_DISTANCE;
    var inside = false;
    var medium = vec3<f32>(1.0);
    var vertices = 0u;
    for (var event = 0u; event < PT_MAX_EVENTS; event = event + 1u) {
        let hit = rt_closest_hit(origin, direction, PT_MAX_DISTANCE);
        if hit.kind == RAY_QUERY_INTERSECTION_NONE {
            radiance += throughput * pt_environment(direction, path_roughness);
            break;
        }
        if event == 0u {
            first_distance = min(hit.t, PT_MISS_DISTANCE);
        }
        let position = origin + direction * hit.t;
        width += spread * hit.t;
        if inside {
            throughput *= pow(max(medium, vec3<f32>(1e-4)), vec3<f32>(hit.t / PT_ABSORPTION_DISTANCE));
        }
        let material = rt_material(hit, direction, width);
        let n = material.normal;
        let v = -direction;
        let bias = RT_SHADOW_BIAS + 1e-4 * hit.t;

        // Glass: refract or reflect (by Fresnel), not counted as a bounce.
        if material.transmission > 0.0 && pt_rand() < material.transmission {
            let eta = select(1.0 / material.ior, material.ior, inside);
            let r0 = (material.ior - 1.0) / (material.ior + 1.0);
            let f0 = r0 * r0;
            let fresnel = f0 + (1.0 - f0) * pow(1.0 - clamp(dot(n, v), 0.0, 1.0), 5.0);
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
            continue;
        }
        if inside {
            // A ray inside a medium met an opaque surface (overlapping geometry).
            inside = false;
        }

        if vertices == 0u && !material.unlit && material.roughness >= PT_CACHE_MIN_ROUGHNESS {
            let cached = pt_cached_radiance(position, n);
            if cached.w > 0.5 {
                radiance += throughput * min(cached.rgb, vec3<f32>(PT_MAX_RADIANCE));
                break;
            }
        }
        radiance += throughput * material.emissive;
        if material.unlit {
            radiance += throughput * material.base_color;
            break;
        }
        vertices += 1u;
        let roughness = material.roughness;
        let f0 = mix(vec3<f32>(0.04), material.base_color, material.metallic);
        let diffuse_color = material.base_color * (1.0 - material.metallic);
        radiance += throughput
            * pt_sample_one_light(position, n, v, diffuse_color, f0, roughness, bias);
        if vertices >= bounces {
            break;
        }

        // Continue by sampling the BRDF: diffuse or specular, by their share of the light.
        let n_dot_v = max(dot(n, v), 1e-4);
        let specular_share = pt_luminance(pt_specular_albedo(f0, n_dot_v, roughness));
        let diffuse_share = pt_luminance(diffuse_color);
        let p_specular = clamp(specular_share / max(specular_share + diffuse_share, 1e-6), 0.05, 1.0);
        var next : vec3<f32>;
        if pt_rand() < p_specular {
            let alpha = roughness * roughness;
            let h = pt_sample_ggx_normal(n, v, alpha, pt_rand2());
            next = reflect(-v, h);
            let n_dot_l = dot(n, next);
            if n_dot_l <= 0.0 {
                break;
            }
            let fresnel = fresnel_schlick(max(dot(v, h), 0.0), f0);
            throughput *= fresnel * pt_smith_g1(n_dot_l, alpha) / p_specular;
            spread += alpha * 0.5;
            path_roughness = max(path_roughness, roughness);
        } else {
            next = pt_cosine_direction(n, pt_rand2());
            let fresnel = fresnel_schlick(max(dot(v, normalize(v + next)), 0.0), f0);
            throughput *= (vec3<f32>(1.0) - fresnel) * diffuse_color / (1.0 - p_specular);
            spread = max(spread, PT_DIFFUSE_SPREAD);
            path_roughness = 1.0;
        }
        if vertices >= 2u {
            let survive = clamp(max(throughput.x, max(throughput.y, throughput.z)), 0.05, 0.95);
            if pt_rand() > survive {
                break;
            }
            throughput /= survive;
        }
        direction = next;
        origin = position + n * bias;
    }
    radiance = select(radiance, vec3<f32>(0.0), radiance != radiance);
    return vec4<f32>(min(radiance, vec3<f32>(PT_MAX_RADIANCE)), first_distance);
}

// The closest surface between `t_min` and `t_max` along a ray.
fn pt_closest_hit_between(
    origin : vec3<f32>,
    direction : vec3<f32>,
    t_min : f32,
    t_max : f32,
) -> RayIntersection {
    var query : ray_query;
    rayQueryInitialize(
        &query,
        rt_tlas,
        RayDesc(RAY_FLAG_NONE, RT_MASK_SURFACES, t_min, t_max, origin, direction),
    );
    loop {
        if !rayQueryProceed(&query) {
            break;
        }
        // Cut-out surfaces come up as candidates (rt_alpha.wgsl).
        if rt_candidate_blocks(rayQueryGetCandidateIntersection(&query)) {
            rayQueryConfirmIntersection(&query);
        }
    }
    return rayQueryGetCommittedIntersection(&query);
}

// The view ray through `scene_uv` (scene texture uv), through the same jittered camera
// the G-buffer was drawn with: origin on the camera's plane (the camera itself for a
// perspective view) and direction.
fn pt_view_ray(scene_uv : vec2<f32>) -> array<vec3<f32>, 2> {
    let viewport_uv = scene_to_viewport_uv(scene_uv);
    let ndc = vec2<f32>(viewport_uv.x * 2.0 - 1.0, 1.0 - viewport_uv.y * 2.0);
    let near = post_uniform.view_proj_inv * vec4<f32>(ndc, 0.25, 1.0);
    let far = post_uniform.view_proj_inv * vec4<f32>(ndc, 0.75, 1.0);
    let a = near.xyz / near.w;
    let b = far.xyz / far.w;
    let camera = post_uniform.camera_position.xyz;
    var direction = normalize(b - a);
    if dot(direction, a - camera) < 0.0 {
        direction = -direction;
    }
    return array<vec3<f32>, 2>(a - direction * dot(a - camera, direction), direction);
}

// The surface a scene pixel shows, as the G-buffer has it, found in the acceleration
// structure for its material and a full-precision position.
struct PtPrimary {
    found : bool,
    position : vec3<f32>,
    // The view ray's direction and the distance along it.
    direction : vec3<f32>,
    t : f32,
    material : RtMaterial,
};

// The G-buffer surface at `pixel` in the acceleration structure: along the view ray, within
// the half-float position's precision of where the G-buffer has it.
fn pt_primary_hit(pixel : vec2<i32>) -> PtPrimary {
    var out : PtPrimary;
    let surface = textureLoad(pt_gbuffer_position, pixel, 0);
    if surface.w < 0.5 {
        return out;
    }
    let dims = vec2<f32>(textureDimensions(pt_gbuffer_position));
    let ray = pt_view_ray((vec2<f32>(pixel) + 0.5) / dims);
    let expected = dot(surface.xyz - ray[0], ray[1]);
    let tolerance = 0.02 + 0.01 * expected + 2e-3 * max(abs(surface.x), max(abs(surface.y), abs(surface.z)));
    let primary = pt_closest_hit_between(ray[0], ray[1], max(expected - tolerance, 0.0), expected + tolerance);
    if primary.kind == RAY_QUERY_INTERSECTION_NONE {
        return out;
    }
    out.found = true;
    out.position = ray[0] + ray[1] * primary.t;
    out.direction = ray[1];
    out.t = primary.t;
    out.material = rt_material(primary, ray[1], pt.footprint.z * primary.t);
    return out;
}

// The G-buffer's (normal mapped) normal at `pixel`, on the side of the interpolated normal
// `ng` and bent to face the viewer `v`: what shades the surface. `ng` offsets rays off it.
fn pt_shading_normal(pixel : vec2<i32>, ng : vec3<f32>, v : vec3<f32>) -> vec3<f32> {
    var n = pt_decode_normal(textureLoad(pt_gbuffer_normal, pixel, 0).xy);
    if dot(n, ng) < 0.0 {
        n = -n;
    }
    let n_dot_v_raw = dot(n, v);
    if n_dot_v_raw < 0.02 {
        n = normalize(n + v * (0.02 - n_dot_v_raw));
    }
    return n;
}

// One cosine-weighted diffuse path from the primary surface along `l`: the light it brings
// (demodulated, times what Fresnel leaves for diffuse) and its first hit's distance.
fn pt_diffuse_path(
    origin : vec3<f32>,
    l : vec3<f32>,
    ng : vec3<f32>,
    v : vec3<f32>,
    f0 : vec3<f32>,
    width : f32,
    bounces : u32,
) -> vec4<f32> {
    if dot(l, ng) <= 0.0 {
        return vec4<f32>(0.0, 0.0, 0.0, PT_MISS_DISTANCE);
    }
    let fresnel = fresnel_schlick(max(dot(v, normalize(v + l)), 0.0), f0);
    let incoming = pt_trace_path(origin, l, bounces, width, PT_DIFFUSE_SPREAD, 1.0);
    return vec4<f32>((vec3<f32>(1.0) - fresnel) * incoming.rgb, incoming.a);
}

@fragment
fn fs_pt_trace(in : VertexOutput) -> PtTraceOutput {
    var out : PtTraceOutput;
    out.emission = vec4<f32>(0.0, 0.0, 0.0, -1.0);
    let local = vec2<i32>(in.position.xy);
    let pixel = local + vec2<i32>(pt.region.xy);
    // Transmissive surfaces (marked by a negative alpha) are the refraction pass's.
    if textureLoad(pt_scene_color, pixel, 0).a < 0.0 {
        return out;
    }
    let primary = pt_primary_hit(pixel);
    if !primary.found {
        return out;
    }
    let position = primary.position;
    let pixel_angle = pt.footprint.z;
    let material = primary.material;
    if material.transmission > 0.0 {
        return out;
    }
    if material.unlit {
        out.material = vec4<f32>(0.0, 0.0, 0.0, 1.0);
        out.emission = vec4<f32>(material.base_color + material.emissive, 0.0);
        out.diffuse = vec4<f32>(0.0, 0.0, 0.0, PT_MISS_DISTANCE);
        out.specular = vec4<f32>(0.0, 0.0, 0.0, PT_MISS_DISTANCE);
        return out;
    }

    let v = -primary.direction;
    // The interpolated normal facing the ray offsets rays off the surface; the G-buffer's
    // (normal mapped) shades it, bent to face the camera.
    let ng = material.normal;
    let n = pt_shading_normal(pixel, ng, v);
    let n_dot_v = max(dot(n, v), 1e-4);
    let roughness = material.roughness;
    let alpha = roughness * roughness;
    let f0 = mix(vec3<f32>(0.04), material.base_color, material.metallic);
    let diffuse_color = material.base_color * (1.0 - material.metallic);
    let specular_albedo = max(pt_specular_albedo(f0, n_dot_v, roughness), vec3<f32>(1e-3));
    let bias = 0.005 + 1e-3 * primary.t;
    let origin = position + ng * bias;
    pt_rng_state = pt_hash(u32(pixel.x) + pt_hash(u32(pixel.y) + pt_hash(pt.frame.x)));
    // Each pixel's own shift of the R2 sequence (both dimensions from both coordinates).
    let pixel_seed = pt_hash(u32(pixel.x) + pt_hash(u32(pixel.y)));
    let offset_diffuse = vec2<u32>(pt_hash(pixel_seed ^ 0x68bc21ebu), pt_hash(pixel_seed ^ 0x02e5be93u));
    let offset_specular = vec2<u32>(pt_hash(pixel_seed ^ 0x967a889bu), pt_hash(pixel_seed ^ 0x2c1b3c6du));

    // Direct light: every light, each behind one shadow ray to a point on it.
    var diffuse = vec3<f32>(0.0);
    var specular = vec3<f32>(0.0);
    let light_count = min(pt_light_count(), PT_MAX_LIGHTS);
    for (var i = 0u; i < light_count; i = i + 1u) {
        let s = pt_light_sample(i, position, pt_rand2());
        if all(s.radiance <= vec3<f32>(0.0)) || dot(ng, s.direction) <= 0.0 {
            continue;
        }
        let brdf = pt_brdf(n, v, s.direction, f0, roughness);
        if all(brdf.diffuse + brdf.specular <= vec3<f32>(0.0)) {
            continue;
        }
        if !pt_visible(origin, s.direction, s.distance - bias) {
            continue;
        }
        diffuse += brdf.diffuse * s.radiance;
        let highlight = pt_area_specular(n, v, s.center, s.angular_radius, f0, roughness);
        specular += highlight * s.radiance / specular_albedo;
    }

    // Indirect light: one diffuse and one specular path per sample.
    let samples = max(pt.frame.z, 1u);
    let bounces = max(pt.frame.y, 1u);
    let trace_diffuse = pt_luminance(diffuse_color) > 1e-3;
    var diffuse_indirect = vec3<f32>(0.0);
    var specular_indirect = vec3<f32>(0.0);
    var diffuse_distance = 0.0;
    var specular_distance = 0.0;
    let width = pixel_angle * primary.t;
    for (var k = 0u; k < samples; k = k + 1u) {
        let index = pt.frame.x * samples + k;
        if trace_diffuse {
            let l = pt_cosine_direction(n, pt_r2(offset_diffuse, index));
            let incoming = pt_diffuse_path(origin, l, ng, v, f0, width, bounces);
            diffuse_indirect += incoming.rgb;
            diffuse_distance += incoming.a;
        }
        let h = pt_sample_ggx_normal(n, v, alpha, pt_r2(offset_specular, index));
        let l = reflect(-v, h);
        let n_dot_l = dot(n, l);
        if n_dot_l > 0.0 && dot(l, ng) > 0.0 {
            let fresnel = fresnel_schlick(max(dot(v, h), 0.0), f0);
            let incoming = pt_trace_path(origin, l, bounces, width, pixel_angle + alpha * 0.5, roughness);
            specular_indirect += fresnel * pt_smith_g1(n_dot_l, alpha) * incoming.rgb / specular_albedo;
            specular_distance += incoming.a;
        } else {
            specular_distance += PT_MISS_DISTANCE;
        }
    }
    let inv_samples = 1.0 / f32(samples);
    diffuse += diffuse_indirect * inv_samples;
    specular += specular_indirect * inv_samples;
    diffuse = clamp(select(diffuse, vec3<f32>(0.0), diffuse != diffuse), vec3<f32>(0.0), vec3<f32>(PT_MAX_RADIANCE));
    specular = clamp(select(specular, vec3<f32>(0.0), specular != specular), vec3<f32>(0.0), vec3<f32>(PT_MAX_RADIANCE));

    out.diffuse = vec4<f32>(diffuse, select(PT_MISS_DISTANCE, diffuse_distance * inv_samples, trace_diffuse));
    out.specular = vec4<f32>(specular, specular_distance * inv_samples);
    out.material = vec4<f32>(clamp(material.base_color, vec3<f32>(0.0), vec3<f32>(1.0)), max(roughness, 0.012));
    out.emission = vec4<f32>(min(material.emissive, vec3<f32>(PT_MAX_RADIANCE)), clamp(material.metallic, 0.0, 1.0));
    return out;
}

// Global illumination: the indirect diffuse light at the scene pixel a target pixel stands
// for, demodulated, with the material and geometry the denoiser needs. Nothing is written
// for specular light (the reflection pass has it) or emission (the main pass draws it).
@fragment
fn fs_gi_trace(in : VertexOutput) -> PtTraceOutput {
    var out : PtTraceOutput;
    out.emission = vec4<f32>(0.0, 0.0, 0.0, -1.0);
    let pixel = pt_scene_pixel(vec2<i32>(in.position.xy));
    if textureLoad(pt_scene_color, pixel, 0).a < 0.0 {
        return out;
    }
    let primary = pt_primary_hit(pixel);
    let material = primary.material;
    if !primary.found || material.transmission > 0.0 {
        return out;
    }
    out.material = vec4<f32>(clamp(material.base_color, vec3<f32>(0.0), vec3<f32>(1.0)), max(material.roughness, 0.012));
    out.emission = vec4<f32>(0.0, 0.0, 0.0, clamp(material.metallic, 0.0, 1.0));
    out.specular = vec4<f32>(0.0, 0.0, 0.0, PT_MISS_DISTANCE);
    out.diffuse = vec4<f32>(0.0, 0.0, 0.0, PT_MISS_DISTANCE);
    let diffuse_color = material.base_color * (1.0 - material.metallic);
    if material.unlit || pt_luminance(diffuse_color) <= 1e-3 {
        return out;
    }

    let v = -primary.direction;
    let ng = material.normal;
    let n = pt_shading_normal(pixel, ng, v);
    let f0 = mix(vec3<f32>(0.04), material.base_color, material.metallic);
    let origin = primary.position + ng * (0.005 + 1e-3 * primary.t);
    pt_rng_state = pt_hash(u32(pixel.x) + pt_hash(u32(pixel.y) + pt_hash(pt.frame.x)));
    let pixel_seed = pt_hash(u32(pixel.x) + pt_hash(u32(pixel.y)));
    let offset = vec2<u32>(pt_hash(pixel_seed ^ 0x68bc21ebu), pt_hash(pixel_seed ^ 0x02e5be93u));

    let samples = max(pt.frame.z, 1u);
    let bounces = max(pt.frame.y, 1u);
    // A target pixel covers `scale` scene pixels across.
    let width = pt.footprint.z * pt.scale.x * primary.t;
    var light = vec3<f32>(0.0);
    var distance = 0.0;
    for (var k = 0u; k < samples; k = k + 1u) {
        let l = pt_cosine_direction(n, pt_r2(offset, pt.frame.x * samples + k));
        let incoming = pt_diffuse_path(origin, l, ng, v, f0, width, bounces);
        light += incoming.rgb;
        distance += incoming.a;
    }
    let inv_samples = 1.0 / f32(samples);
    light = light * inv_samples;
    light = clamp(select(light, vec3<f32>(0.0), light != light), vec3<f32>(0.0), vec3<f32>(PT_MAX_RADIANCE));
    out.diffuse = vec4<f32>(light, distance * inv_samples);
    return out;
}

// The denoised global illumination at region position `local`, upsampled from the target
// pixels around it that show the same surface (G-buffer normal `n` and view depth `depth`,
// seen along `v`): bilinear weights from the nearest four times how well each one's normal
// and depth match, or a wider ring's matches where none of the four does. w is 0 where
// nothing matches.
fn pt_gi_upsample(local : vec2<i32>, n : vec3<f32>, depth : f32, v : vec3<f32>) -> vec4<f32> {
    let size = vec2<i32>(pt.region.zw);
    let coord = vec2<f32>(local) / pt.scale.x;
    let base = vec2<i32>(floor(coord));
    let f = coord - floor(coord);
    let c = clamp(abs(dot(n, v)), 0.1, 1.0);
    let tolerance = 0.02 * depth + 2.0 * pt_footprint(depth) * (1.0 + sqrt(1.0 - c * c) / c);
    var near = vec4<f32>(0.0);
    var wide = vec4<f32>(0.0);
    for (var y = -1; y <= 2; y = y + 1) {
        for (var x = -1; x <= 2; x = x + 1) {
            let q = base + vec2<i32>(x, y);
            if any(q < vec2<i32>(0)) || any(q >= size) {
                continue;
            }
            let geometry = textureLoad(pt_gi_geometry, q, 0);
            if geometry.w <= 0.0 {
                continue;
            }
            let w_normal = pow(max(dot(n, pt_decode_normal(geometry.xy)), 0.0), 16.0);
            let w = w_normal * exp(-abs(geometry.z - depth) / tolerance);
            if w <= 1e-4 {
                continue;
            }
            let light = vec4<f32>(textureLoad(pt_gi_light, q, 0).rgb, 1.0);
            if x >= 0 && x <= 1 && y >= 0 && y <= 1 {
                let fw = select(vec2<f32>(1.0) - f, f, vec2<i32>(x, y) == vec2<i32>(1));
                near += light * (fw.x * fw.y * w);
            }
            let d = vec2<f32>(q) - coord;
            wide += light * (w * exp(-0.5 * dot(d, d)));
        }
    }
    if near.w > 1e-3 {
        return vec4<f32>(near.rgb / near.w, 1.0);
    }
    if wide.w > 1e-6 {
        return vec4<f32>(wide.rgb / wide.w, 1.0);
    }
    return vec4<f32>(0.0);
}

// Global illumination's composite, at every scene pixel: the upsampled light times the
// surface's diffuse colour (from its material here, at full resolution), added to the
// main pass's colour. Alpha is kept by the blend state.
@fragment
fn fs_gi_composite(in : VertexOutput) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(in.position.xy);
    let local = pixel - vec2<i32>(pt.region.xy);
    if any(local < vec2<i32>(0)) || any(local >= vec2<i32>(pt.scale.yz)) {
        discard;
    }
    let primary = pt_primary_hit(pixel);
    let material = primary.material;
    if !primary.found || material.transmission > 0.0 || material.unlit {
        discard;
    }
    let diffuse_color = material.base_color * (1.0 - material.metallic);
    if pt_luminance(diffuse_color) <= 1e-3 {
        discard;
    }
    let world = textureLoad(pt_gbuffer_position, pixel, 0).xyz;
    let n = pt_decode_normal(textureLoad(pt_gbuffer_normal, pixel, 0).xy);
    let depth = max(pt_view_depth(world), 1e-3);
    let light = pt_gi_upsample(local, n, depth, -primary.direction);
    if light.w <= 0.0 {
        discard;
    }
    let color = light.rgb * diffuse_color;
    return vec4<f32>(select(color, vec3<f32>(0.0), color != color), 0.0);
}
