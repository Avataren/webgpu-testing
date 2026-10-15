// Ray-traced reflections: the hardware counterpart of fs_ssr_trace (ssr.wgsl). It writes
// the same reflection texture (half or full resolution), so SSR's temporal, blur and
// apply passes finish it.
//
// Each smooth pixel's reflected ray is traced against the scene's acceleration structure,
// so it finds what it hits even when that is off screen or hidden. Where the hit point is
// what the screen shows at that spot, the reflection takes the lit colour from the screen
// (full lighting, shadows, textures, transparency); elsewhere the hit is shaded here from
// its material, the environment and the lights, each light behind a shadow ray.
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

fn rt_closest_hit(origin : vec3<f32>, direction : vec3<f32>, max_t : f32) -> RayIntersection {
    var query : ray_query;
    rayQueryInitialize(
        &query,
        rt_tlas,
        RayDesc(RAY_FLAG_FORCE_OPAQUE, 0xffu, 0.0, max_t, origin, direction),
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
            0xffu,
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

// What the main pass would have drawn at the hit: the material with its base colour,
// metallic-roughness, emissive and occlusion maps (normal maps are left out: the pool has
// no tangents). `cone_width` is how wide the pixel's ray cone has grown at the hit, in
// meters; it picks the texture level, as screen-space derivatives do in the main pass
// (ray cones, Akenine-Möller et al. 2019, "Texture Level of Detail Strategies for
// Real-Time Ray Tracing").
fn rt_shade(
    hit : RayIntersection,
    position : vec3<f32>,
    direction : vec3<f32>,
    cone_width : f32,
) -> vec3<f32> {
    let surface = rt_surface(hit, direction);
    let material = rt_materials[surface.material_index];
    let flags = material.material_flags;
    let facing = max(abs(dot(surface.normal, direction)), 0.05);
    let lod_base = surface.uv_density + log2(max(cone_width, 1e-6) / facing);
    let nearest = (flags & FLAG_USE_NEAREST_FILTERING) != 0u;

    var base_color = material.color.rgb * surface.color.rgb;
    if (flags & FLAG_USE_BASE_COLOR_TEXTURE) != 0u {
        base_color *= rt_sample_texture(material.base_color_texture, surface.uv, lod_base, nearest).rgb;
    }
    var emissive = base_color * material.emissive_strength;
    if (flags & FLAG_USE_EMISSIVE_TEXTURE) != 0u {
        emissive = rt_sample_texture(material.emissive_texture, surface.uv, lod_base, nearest).rgb
            * material.emissive_strength;
    }
    if (flags & FLAG_UNLIT) != 0u {
        return base_color + emissive;
    }
    var metallic = material.metallic_factor;
    var roughness = material.roughness_factor;
    if (flags & FLAG_USE_METALLIC_ROUGHNESS_TEXTURE) != 0u {
        let mr = rt_sample_texture(material.metallic_roughness_texture, surface.uv, lod_base, nearest);
        metallic *= mr.b;
        roughness *= mr.g;
    }
    roughness = max(roughness, 0.01);
    var occlusion = 1.0;
    if (flags & FLAG_USE_OCCLUSION_TEXTURE) != 0u {
        occlusion = rt_sample_texture(material.occlusion_texture, surface.uv, lod_base, nearest).r;
    }

    let v = -direction;
    let direct = rt_direct_lighting(position, surface.normal, v, base_color, metallic, roughness);
    let ambient = calculate_environment_lighting(
        surface.normal, v, base_color, metallic, roughness, occlusion
    );
    return ambient + direct + emissive;
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
    // A pixel covers more of the world further away; allow for that and for TAA jitter.
    let tolerance = 0.03 + 0.01 * ssr_view_depth(position);
    if distance(surface.xyz, position) > tolerance {
        return vec4<f32>(0.0);
    }
    let color = textureSampleLevel(rt_scene_color, rt_scene_sampler, scene_uv, 0.0).rgb;
    return vec4<f32>(color, 1.0);
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
        // Angle one trace texel covers, from the projection.
        let pixel_angle = 2.0 * ssr_texel_scale() / (post_uniform.proj[1][1] * f32(dims.y));
        let travelled = distance(post_uniform.camera_position.xyz, origin) + hit.t;
        color = rt_shade(hit, hit_position, r, pixel_angle * travelled);
    }
    color = select(color, vec3<f32>(0.0), color != color);
    color = clamp(color, vec3<f32>(0.0), vec3<f32>(SSR_MAX_RADIANCE));

    let weight = 1.0 - smoothstep(0.8, 1.0, hit.t / max_distance);
    return vec4<f32>(ssr_compress(color) * weight, weight);
}
