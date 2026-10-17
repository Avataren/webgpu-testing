// Ray tracing helpers shared by the passes that shade what rays hit (reflections,
// refraction, path tracing): the scene's acceleration structure and geometry (group 3),
// hit queries, the hit surface's interpolated attributes and material, direct light at a
// hit behind shadow rays, and the environment a ray that leaves the scene sees.
//
// Needs `enable wgpu_ray_query;` (prepended by the Rust side), the main pass's constants,
// material, lighting and environment modules (the lights bind group is group 2, as in the
// main pass), rt_scene.wgsl and rt_alpha.wgsl, and one of rt_textures_bindless.wgsl /
// rt_textures_none.wgsl for `rt_sample_texture`.

// The scene's geometry, instances and materials (group 3) are in rt_scene.wgsl.

// Shadow rays start this far off the hit surface.
const RT_SHADOW_BIAS : f32 = 0.02;

fn rt_closest_hit(origin : vec3<f32>, direction : vec3<f32>, max_t : f32) -> RayIntersection {
    return rt_closest_hit_masked(origin, direction, max_t, RT_MASK_SURFACES);
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
        RayDesc(RAY_FLAG_NONE, mask, 0.0, max_t, origin, direction),
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

// Whether nothing blocks the segment from `origin` along `direction` for `distance`.
fn rt_unoccluded(origin : vec3<f32>, direction : vec3<f32>, distance : f32) -> bool {
    var query : ray_query;
    rayQueryInitialize(
        &query,
        rt_tlas,
        RayDesc(
            RAY_FLAG_TERMINATE_ON_FIRST_HIT,
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
        // Cut-out surfaces come up as candidates (rt_alpha.wgsl).
        if rt_candidate_blocks(rayQueryGetCandidateIntersection(&query)) {
            rayQueryConfirmIntersection(&query);
        }
    }
    return rayQueryGetCommittedIntersection(&query).kind == RAY_QUERY_INTERSECTION_NONE;
}

struct RtSurface {
    normal : vec3<f32>,
    uv : vec2<f32>,
    color : vec4<f32>,
    material_index : u32,
    // The instance's RT_INSTANCE_* flags.
    instance_flags : u32,
    // log2 of the triangle's uv extent per meter (half the log of the area ratio): with
    // the ray cone's width it picks the texture level.
    uv_density : f32,
    // Whether the ray hit the side the normal points away from (the normal is flipped).
    back_facing : bool,
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
    let back_facing = dot(normal, direction) > 0.0;
    if back_facing {
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
    out.instance_flags = instance.flags;
    out.uv_density = 0.5 * log2(max(uv_area, 1e-12) / max(world_area, 1e-12));
    out.back_facing = back_facing;
    return out;
}

// Direct light at a hit, each light tested with a shadow ray (the shadow maps need
// derivatives and uniform control flow, which a hit shader does not have): the
// directional lights, then the point and spot lights of the hit's world grid cell.
fn rt_direct_lighting(
    position : vec3<f32>,
    n : vec3<f32>,
    v : vec3<f32>,
    base_color : vec3<f32>,
    metallic : f32,
    roughness : f32,
) -> vec3<f32> {
    let origin = position + n * RT_SHADOW_BIAS;
    var lo = rt_directional_lighting(origin, n, v, base_color, metallic, roughness);
    let range = light_grid_cell(position);
    for (var i = 0u; i < range.count; i = i + 1u) {
        lo += rt_local_light(light_in_range(range, i), position, origin, n, v, base_color, metallic, roughness);
    }
    return lo;
}

// The directional lights' light at a hit (shadow rays start at `origin`).
fn rt_directional_lighting(
    origin : vec3<f32>,
    n : vec3<f32>,
    v : vec3<f32>,
    base_color : vec3<f32>,
    metallic : f32,
    roughness : f32,
) -> vec3<f32> {
    var lo = vec3<f32>(0.0);
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
    return lo;
}

// A point or spot light's light at a hit, behind a shadow ray from `origin`.
fn rt_local_light(
    light : LocalLight,
    position : vec3<f32>,
    origin : vec3<f32>,
    n : vec3<f32>,
    v : vec3<f32>,
    base_color : vec3<f32>,
    metallic : f32,
    roughness : f32,
) -> vec3<f32> {
    let arrival = local_light_arrival(light, position);
    if arrival.attenuation <= 0.0 || dot(n, arrival.L) <= 0.0 {
        return vec3<f32>(0.0);
    }
    if !rt_unoccluded(origin, arrival.L, arrival.distance - RT_SHADOW_BIAS) {
        return vec3<f32>(0.0);
    }
    return calculate_light_contribution(
        n, v, arrival.L, base_color, metallic, roughness,
        light.color_intensity.xyz, light.color_intensity.w * arrival.attenuation
    );
}

// Up to this many point and spot lights near a hit, rt_sampled_direct_lighting traces them
// all; with more, it traces one.
const RT_EXACT_LOCAL_LIGHTS : u32 = 4u;

// Direct light at a hit for passes that average many frames (light probes): the
// directional lights and, when few are near, every point and spot light, as
// rt_direct_lighting; with many near, one of those, picked in proportion to what each
// would add unshadowed (by `u`, uniform in 0..1), divided by the chance of picking it.
// The cost stays the same however many lights there are.
fn rt_sampled_direct_lighting(
    position : vec3<f32>,
    n : vec3<f32>,
    v : vec3<f32>,
    base_color : vec3<f32>,
    metallic : f32,
    roughness : f32,
    u : f32,
) -> vec3<f32> {
    let range = light_grid_cell(position);
    if range.count <= RT_EXACT_LOCAL_LIGHTS {
        return rt_direct_lighting(position, n, v, base_color, metallic, roughness);
    }
    let origin = position + n * RT_SHADOW_BIAS;
    var lo = rt_directional_lighting(origin, n, v, base_color, metallic, roughness);
    var total = 0.0;
    for (var i = 0u; i < range.count; i = i + 1u) {
        total += rt_unshadowed_luminance(light_in_range(range, i), position, n, v, base_color, metallic, roughness);
    }
    if total <= 0.0 {
        return lo;
    }
    var pick = u * total;
    for (var i = 0u; i < range.count; i = i + 1u) {
        let light = light_in_range(range, i);
        let w = rt_unshadowed_luminance(light, position, n, v, base_color, metallic, roughness);
        pick -= w;
        if pick <= 0.0 && w > 0.0 {
            lo += rt_local_light(light, position, origin, n, v, base_color, metallic, roughness)
                * (total / w);
            break;
        }
    }
    return lo;
}

// How bright a point or spot light's light at a hit would be without shadows.
fn rt_unshadowed_luminance(
    light : LocalLight,
    position : vec3<f32>,
    n : vec3<f32>,
    v : vec3<f32>,
    base_color : vec3<f32>,
    metallic : f32,
    roughness : f32,
) -> f32 {
    let arrival = local_light_arrival(light, position);
    if arrival.attenuation <= 0.0 || dot(n, arrival.L) <= 0.0 {
        return 0.0;
    }
    let c = calculate_light_contribution(
        n, v, arrival.L, base_color, metallic, roughness,
        light.color_intensity.xyz, light.color_intensity.w * arrival.attenuation
    );
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// A hash of `value` (PCG), and a number in 0..1 from it.
fn rt_hash(value : u32) -> u32 {
    let state = value * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

fn rt_random(seed : u32) -> f32 {
    return f32(rt_hash(seed) >> 8u) / 16777216.0;
}

// A hit surface's material, with its base colour, metallic-roughness, emissive and
// occlusion maps (normal maps are left out: the pool has no tangents). `cone_width` is how
// wide the pixel's ray cone has grown at the hit, in meters; it picks the texture level,
// as screen-space derivatives do in the main pass (ray cones, Akenine-Möller et al. 2019,
// "Texture Level of Detail Strategies for Real-Time Ray Tracing").
struct RtMaterial {
    normal : vec3<f32>,
    base_color : vec3<f32>,
    // Opacity, for blended particles.
    alpha : f32,
    emissive : vec3<f32>,
    metallic : f32,
    roughness : f32,
    occlusion : f32,
    transmission : f32,
    // Index of refraction, for transmissive surfaces.
    ior : f32,
    unlit : bool,
    // The ray hit the back of a single-sided surface (inside a closed mesh, or behind a
    // wall); double-sided surfaces have no back.
    back_facing : bool,
};

// A hit surface's material data, from the particle materials for a particle system.
// Particles are never transmissive (particle_render.wgsl draws no transmission), so rays
// stop at them instead of refracting through.
fn rt_surface_material(surface : RtSurface) -> MaterialData {
    if (surface.instance_flags & RT_INSTANCE_PARTICLES) != 0u {
        var material = rt_particle_materials[surface.material_index];
        material.transmission = 0.0;
        return material;
    }
    return rt_materials[surface.material_index];
}

fn rt_material(hit : RayIntersection, direction : vec3<f32>, cone_width : f32) -> RtMaterial {
    let surface = rt_surface(hit, direction);
    let particle = (surface.instance_flags & RT_INSTANCE_PARTICLES) != 0u;
    let material = rt_surface_material(surface);
    let flags = material.material_flags;
    let facing = max(abs(dot(surface.normal, direction)), 0.05);
    let lod_base = surface.uv_density + log2(max(cone_width, 1e-6) / facing);
    let nearest = (flags & FLAG_USE_NEAREST_FILTERING) != 0u;

    var out : RtMaterial;
    out.normal = surface.normal;
    // A particle's vertex colour is the particle's (rt_particles.wgsl).
    var base_color = material.color * surface.color;
    if (flags & FLAG_USE_BASE_COLOR_TEXTURE) != 0u {
        base_color *= rt_sample_texture(material.base_color_texture, surface.uv, lod_base, nearest);
    }
    out.base_color = base_color.rgb;
    out.alpha = base_color.a;
    out.emissive = out.base_color * material.emissive_strength;
    if (flags & FLAG_USE_EMISSIVE_TEXTURE) != 0u {
        out.emissive = rt_sample_texture(material.emissive_texture, surface.uv, lod_base, nearest).rgb
            * material.emissive_strength;
    }
    out.unlit = (flags & FLAG_UNLIT) != 0u;
    out.back_facing = surface.back_facing && (flags & FLAG_DOUBLE_SIDED) == 0u;
    out.transmission = clamp(material.transmission, 0.0, 1.0);
    out.ior = max(material.ior, 1.0);
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
    if particle {
        // As particle_render.wgsl, which draws no emission (nor transmission, above).
        out.emissive = vec3<f32>(0.0);
    }
    return out;
}

// What a ray leaving the scene sees: the HDR environment, or the background colour.
fn rt_environment(direction : vec3<f32>) -> vec3<f32> {
    if environment_hdr_enabled() {
        return sample_environment_hdr(direction, 0.0) * environment_hdr_intensity();
    }
    return environment_settings.ambient_color.rgb;
}
