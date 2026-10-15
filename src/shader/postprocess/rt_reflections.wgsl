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
// Needs `enable wgpu_ray_query;` (prepended by the Rust side), post-processing's common
// and reflection_common modules, the main pass's constants, material, lighting and
// environment modules (the lights bind group is group 2, as in the main pass), and one
// of rt_textures_bindless.wgsl / rt_textures_none.wgsl for `rt_sample_texture`.

@group(1) @binding(170)
var rt_scene_color : texture_2d<f32>;
@group(1) @binding(171)
var rt_gbuffer_normal : texture_2d<f32>;
@group(1) @binding(172)
var rt_gbuffer_position : texture_2d<f32>;
@group(1) @binding(173)
var rt_scene_sampler : sampler;

// The engine's `Vertex` (src/renderer/vertex.rs), as the ray tracing pool stores it;
// scalars keep the vec3s at their packed offsets. `RtInstance` mirrors
// src/renderer/raytracing/scene.rs.
struct RtVertex {
    px : f32,
    py : f32,
    pz : f32,
    nx : f32,
    ny : f32,
    nz : f32,
    u : f32,
    v : f32,
    tangent : vec4<f32>,
    color : vec4<f32>,
};

fn rt_vertex_position(vertex : RtVertex) -> vec3<f32> {
    return vec3<f32>(vertex.px, vertex.py, vertex.pz);
}

fn rt_vertex_normal(vertex : RtVertex) -> vec3<f32> {
    return vec3<f32>(vertex.nx, vertex.ny, vertex.nz);
}

struct RtInstance {
    vertex_base : u32,
    index_base : u32,
    material_index : u32,
    _padding : u32,
};

@group(3) @binding(0)
var rt_tlas : acceleration_structure;
@group(3) @binding(1)
var<storage, read> rt_vertices : array<RtVertex>;
@group(3) @binding(2)
var<storage, read> rt_indices : array<u32>;
@group(3) @binding(3)
var<storage, read> rt_instances : array<RtInstance>;
@group(3) @binding(4)
var<storage, read> rt_materials : array<MaterialData>;

// Shadow rays start this far off the hit surface.
const RT_SHADOW_BIAS : f32 = 0.02;
// Hit surfaces at least this smooth get a second reflection ray (a mirror seen in a
// mirror). It is not blurred by the surface's roughness, so rougher surfaces would show a
// reflection far sharper than their own.
const RT_BOUNCE_MIN_SMOOTHNESS : f32 = 0.7;

fn rt_closest_hit(origin : vec3<f32>, direction : vec3<f32>, max_t : f32) -> RayIntersection {
    return rt_closest_hit_masked(origin, direction, max_t, 0xffu);
}

// `mask` 0x01 sees opaque geometry only (MASK_OPAQUE in raytracing/scene.rs).
fn rt_closest_hit_masked(
    origin : vec3<f32>,
    direction : vec3<f32>,
    max_t : f32,
    mask : u32,
) -> RayIntersection {
    var query : ray_query;
    rayQueryInitialize(
        &query,
        rt_tlas,
        RayDesc(RAY_FLAG_FORCE_OPAQUE, mask, 0.0, max_t, origin, direction),
    );
    loop {
        if !rayQueryProceed(&query) {
            break;
        }
    }
    return rayQueryGetCommittedIntersection(&query);
}

// Whether nothing blocks the segment from `origin` along `direction` for `distance`.
fn rt_unoccluded(origin : vec3<f32>, direction : vec3<f32>, distance : f32) -> bool {
    var query : ray_query;
    rayQueryInitialize(
        &query,
        rt_tlas,
        RayDesc(
            RAY_FLAG_FORCE_OPAQUE | RAY_FLAG_TERMINATE_ON_FIRST_HIT,
            // Opaque geometry only (MASK_OPAQUE in raytracing/scene.rs): light passes glass.
            0x01u,
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
    }
    return rayQueryGetCommittedIntersection(&query).kind == RAY_QUERY_INTERSECTION_NONE;
}

struct RtSurface {
    normal : vec3<f32>,
    uv : vec2<f32>,
    color : vec4<f32>,
    material_index : u32,
    // log2 of the triangle's uv extent per meter (half the log of the area ratio): with
    // the ray cone's width it picks the texture level.
    uv_density : f32,
};

// The hit triangle's interpolated attributes, with the normal in world space and facing
// the ray.
fn rt_surface(hit : RayIntersection, direction : vec3<f32>) -> RtSurface {
    let instance = rt_instances[hit.instance_custom_data];
    let first = instance.index_base + hit.primitive_index * 3u;
    let a = rt_vertices[instance.vertex_base + rt_indices[first]];
    let b = rt_vertices[instance.vertex_base + rt_indices[first + 1u]];
    let c = rt_vertices[instance.vertex_base + rt_indices[first + 2u]];
    let w = vec3<f32>(1.0 - hit.barycentrics.x - hit.barycentrics.y, hit.barycentrics);

    let object_normal = rt_vertex_normal(a) * w.x + rt_vertex_normal(b) * w.y + rt_vertex_normal(c) * w.z;
    // Normals transform by the inverse transpose: the transpose of world_to_object.
    let m = hit.world_to_object;
    var normal = vec3<f32>(dot(m[0], object_normal), dot(m[1], object_normal), dot(m[2], object_normal));
    let length_sq = dot(normal, normal);
    if length_sq < 1e-12 {
        normal = -direction;
    } else {
        normal = normal * inverseSqrt(length_sq);
    }
    if dot(normal, direction) > 0.0 {
        normal = -normal;
    }

    let uv_a = vec2<f32>(a.u, a.v);
    let uv_b = vec2<f32>(b.u, b.v);
    let uv_c = vec2<f32>(c.u, c.v);
    let uv_edge_1 = uv_b - uv_a;
    let uv_edge_2 = uv_c - uv_a;
    let uv_area = abs(uv_edge_1.x * uv_edge_2.y - uv_edge_2.x * uv_edge_1.y);
    let world_area = length(cross(
        hit.object_to_world * vec4<f32>(rt_vertex_position(b) - rt_vertex_position(a), 0.0),
        hit.object_to_world * vec4<f32>(rt_vertex_position(c) - rt_vertex_position(a), 0.0),
    ));

    var out : RtSurface;
    out.normal = normal;
    out.uv = uv_a * w.x + uv_b * w.y + uv_c * w.z;
    out.color = a.color * w.x + b.color * w.y + c.color * w.z;
    out.material_index = instance.material_index;
    out.uv_density = 0.5 * log2(max(uv_area, 1e-12) / max(world_area, 1e-12));
    return out;
}

// Direct light at a hit, each light tested with a shadow ray (the shadow maps need
// derivatives and uniform control flow, which a hit shader does not have).
fn rt_direct_lighting(
    position : vec3<f32>,
    n : vec3<f32>,
    v : vec3<f32>,
    base_color : vec3<f32>,
    metallic : f32,
    roughness : f32,
) -> vec3<f32> {
    var lo = vec3<f32>(0.0);
    let origin = position + n * RT_SHADOW_BIAS;

    let dir_count = min(lights.counts.x, MAX_DIRECTIONAL_LIGHTS);
    for (var i = 0u; i < dir_count; i = i + 1u) {
        let light = lights.directionals[i];
        let l = normalize(-light.direction.xyz);
        if dot(n, l) <= 0.0 || !rt_unoccluded(origin, l, 1.0e4) {
            continue;
        }
        lo += calculate_light_contribution(
            n, v, l, base_color, metallic, roughness,
            light.color_intensity.xyz, light.color_intensity.w
        );
    }

    let point_count = min(lights.counts.y, MAX_POINT_LIGHTS);
    for (var i = 0u; i < point_count; i = i + 1u) {
        let light = lights.points[i];
        let to_light = light.position_range.xyz - position;
        let distance = length(to_light);
        let range = light.position_range.w;
        if distance < 1e-4 || (range > 0.0 && distance > range) {
            continue;
        }
        let l = to_light / distance;
        if dot(n, l) <= 0.0 || !rt_unoccluded(origin, l, distance - RT_SHADOW_BIAS) {
            continue;
        }
        var attenuation = 1.0 / max(distance * distance, 1e-4);
        if range > 0.0 {
            let range_factor = clamp(1.0 - distance / range, 0.0, 1.0);
            attenuation = attenuation * range_factor * range_factor;
        }
        lo += calculate_light_contribution(
            n, v, l, base_color, metallic, roughness,
            light.color_intensity.xyz, light.color_intensity.w * attenuation
        );
    }

    let spot_count = min(lights.counts.z, MAX_SPOT_LIGHTS);
    for (var i = 0u; i < spot_count; i = i + 1u) {
        let light = lights.spots[i];
        let to_light = light.position_range.xyz - position;
        let distance = length(to_light);
        let range = light.position_range.w;
        if distance < 1e-4 || (range > 0.0 && distance > range) {
            continue;
        }
        let l = to_light / distance;
        let cos_theta = dot(normalize(light.direction.xyz), -l);
        let cos_inner = light.cone_params.x;
        let cos_outer = light.cone_params.y;
        if cos_theta < cos_outer || dot(n, l) <= 0.0 {
            continue;
        }
        if !rt_unoccluded(origin, l, distance - RT_SHADOW_BIAS) {
            continue;
        }
        var spot = clamp((cos_theta - cos_outer) / max(cos_inner - cos_outer, 1e-4), 0.0, 1.0);
        spot = spot * spot;
        var attenuation = 1.0 / max(distance * distance, 1e-4);
        if range > 0.0 {
            let range_factor = clamp(1.0 - distance / range, 0.0, 1.0);
            attenuation = attenuation * range_factor * range_factor;
        }
        lo += calculate_light_contribution(
            n, v, l, base_color, metallic, roughness,
            light.color_intensity.xyz, light.color_intensity.w * attenuation * spot
        );
    }
    return lo;
}

// A hit surface's material, with its base colour, metallic-roughness, emissive and
// occlusion maps (normal maps are left out: the pool has no tangents). `cone_width` is how
// wide the pixel's ray cone has grown at the hit, in meters; it picks the texture level,
// as screen-space derivatives do in the main pass (ray cones, Akenine-Möller et al. 2019,
// "Texture Level of Detail Strategies for Real-Time Ray Tracing").
struct RtMaterial {
    normal : vec3<f32>,
    base_color : vec3<f32>,
    emissive : vec3<f32>,
    metallic : f32,
    roughness : f32,
    occlusion : f32,
    transmission : f32,
    unlit : bool,
};

fn rt_material(hit : RayIntersection, direction : vec3<f32>, cone_width : f32) -> RtMaterial {
    let surface = rt_surface(hit, direction);
    let material = rt_materials[surface.material_index];
    let flags = material.material_flags;
    let facing = max(abs(dot(surface.normal, direction)), 0.05);
    let lod_base = surface.uv_density + log2(max(cone_width, 1e-6) / facing);
    let nearest = (flags & FLAG_USE_NEAREST_FILTERING) != 0u;

    var out : RtMaterial;
    out.normal = surface.normal;
    out.base_color = material.color.rgb * surface.color.rgb;
    if (flags & FLAG_USE_BASE_COLOR_TEXTURE) != 0u {
        out.base_color *= rt_sample_texture(material.base_color_texture, surface.uv, lod_base, nearest).rgb;
    }
    out.emissive = out.base_color * material.emissive_strength;
    if (flags & FLAG_USE_EMISSIVE_TEXTURE) != 0u {
        out.emissive = rt_sample_texture(material.emissive_texture, surface.uv, lod_base, nearest).rgb
            * material.emissive_strength;
    }
    out.unlit = (flags & FLAG_UNLIT) != 0u;
    out.transmission = clamp(material.transmission, 0.0, 1.0);
    out.metallic = material.metallic_factor;
    out.roughness = material.roughness_factor;
    if (flags & FLAG_USE_METALLIC_ROUGHNESS_TEXTURE) != 0u {
        let mr = rt_sample_texture(material.metallic_roughness_texture, surface.uv, lod_base, nearest);
        out.metallic *= mr.b;
        out.roughness *= mr.g;
    }
    out.roughness = max(out.roughness, 0.01);
    out.occlusion = 1.0;
    if (flags & FLAG_USE_OCCLUSION_TEXTURE) != 0u {
        out.occlusion = rt_sample_texture(material.occlusion_texture, surface.uv, lod_base, nearest).r;
    }
    return out;
}

// What the main pass would have drawn at a hit: lights (each behind a shadow ray),
// environment and emission, without the surface's own reflection.
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
    let ambient = calculate_environment_lighting(
        material.normal, v, lit_color, material.metallic, material.roughness,
        material.occlusion
    );
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
    let data = rt_materials[surface.material_index];
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
    let v = normalize(origin - post_uniform.camera_position.xyz);
    let r = reflect(v, n);
    if dot(r, n) <= 0.0 {
        return vec4<f32>(0.0);
    }

    let max_distance = post_uniform.ssr.y;
    // Start a little off the surface so it does not hit itself.
    let start = origin + n * (0.01 + 0.001 * ssr_view_depth(origin));
    let hit = rt_closest_hit(start, r, max_distance);
    if hit.kind == RAY_QUERY_INTERSECTION_NONE {
        // Misses reflect the environment, which the main pass's lighting already adds.
        return vec4<f32>(0.0);
    }

    let hit_position = start + r * hit.t;
    var color : vec3<f32>;
    let screen = rt_screen_radiance(hit_position);
    if screen.w > 0.5 {
        color = screen.rgb;
    } else {
        // Angle one trace texel covers, from the projection and the viewport's height
        // (not the target's, which the editor draws into part of), in output pixels so
        // reflected textures keep the output's detail while upscaling.
        let output_height = max(post_uniform.resolution.y, 1.0) / render_scale();
        let pixel_angle = 2.0 * ssr_texel_scale() / (post_uniform.proj[1][1] * output_height);
        let travelled = distance(post_uniform.camera_position.xyz, origin) + hit.t;
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

// What a ray leaving the scene sees: the HDR environment, or the background colour.
fn rt_environment(direction : vec3<f32>) -> vec3<f32> {
    if environment_hdr_enabled() {
        return sample_environment_hdr(direction, 0.0) * environment_hdr_intensity();
    }
    return environment_settings.ambient_color.rgb;
}

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
        let material = rt_materials[surface.material_index];
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
    let camera = post_uniform.camera_position.xyz;
    let primary = rt_closest_hit(camera, v, 1.0e3);
    if primary.kind == RAY_QUERY_INTERSECTION_NONE {
        return vec3<f32>(0.0);
    }
    let entry_surface = rt_surface(primary, v);
    let entry_material = rt_materials[entry_surface.material_index];
    if entry_material.transmission <= 0.0 {
        return vec3<f32>(0.0);
    }
    let ior = max(entry_material.ior, 1.0);
    let tint = entry_material.color.rgb * entry_surface.color.rgb;
    let n = entry_surface.normal;
    let f0 = pow((ior - 1.0) / (ior + 1.0), 2.0);
    let fresnel = f0 + (1.0 - f0) * pow(1.0 - clamp(-dot(v, n), 0.0, 1.0), 5.0);
    let weight = entry_material.transmission * (1.0 - fresnel);

    let entry = camera + v * primary.t;
    let bias = 0.002 + 0.0005 * primary.t;
    var color = vec3<f32>(0.0);
    if entry_material.dispersion <= 0.0 {
        color = rt_refraction_path(entry, n, v, primary.t, ior, tint, 0.0, bias, pixel_angle);
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
                * rt_refraction_path(entry, n, v, primary.t, path_ior, tint, position, bias, pixel_angle);
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
