// src/shader/lighting_common.wgsl
// Shared lighting structures, bindings, and functions for all pipelines

// ============================================================================
// Light Structures
// ============================================================================


struct DirectionalLight {
    direction: vec4<f32>,
    color_intensity: vec4<f32>,
};

// A point or spot light (LocalLightRaw in src/renderer/lights.rs).
struct LocalLight {
    // xyz: position, w: range (always > 0: where the light has faded out)
    position_range: vec4<f32>,
    color_intensity: vec4<f32>,
    // xyz: a spot light's axis, w: the cosine of its outer angle (below -1.5: a point
    // light, which has no cone)
    direction: vec4<f32>,
    // x: the cosine of a spot light's inner angle, y: its slot in the shadows uniform,
    // z: its bit in the ray-traced shadow mask (-1: none)
    params: vec4<f32>,
};

// The lights buffer (LightsHeader, then the local lights; src/renderer/lights.rs).
struct Lights {
    // x: directional lights, y: local lights, z: 1 when the world grid was built this
    // frame, w: which bits of the ray-traced shadow mask the frame traced
    counts: vec4<u32>,
    // How a surface finds its cluster (light_cluster): the camera's view projection
    // without TAA jitter, the view depth of a point (dot(xyz, p) + w), the clusters along
    // x, y and depth (and their count), and the depth slice: floor(log(depth) * x + y),
    // or floor(depth * x + y) when w = 1 (orthographic).
    cluster_view_proj: mat4x4<f32>,
    cluster_depth_axis: vec4<f32>,
    cluster_dims: vec4<u32>,
    cluster_slices: vec4<f32>,
    // The world grid (light_grid_cell): its lowest corner and cells per meter, and its
    // cells along x, y and z and where their entries start in light_lists.
    grid_origin: vec4<f32>,
    grid_dims: vec4<u32>,
    directionals: array<DirectionalLight, MAX_DIRECTIONAL_LIGHTS>,
    _padding: array<vec4<f32>, 14>,
    locals: array<LocalLight>,
};

// ============================================================================
// Light Bindings (group 2)
// ============================================================================

@group(2) @binding(0) var<storage, read> lights: Lights;
// Which local lights reach each cluster of the view and each cell of the world grid
// (src/renderer/light_culling.rs): (first, count) entries into a pool of light indices.
@group(2) @binding(15) var<storage, read> light_lists: array<u32>;

// The local lights that may reach a point: light_lists[first .. first + count] index
// lights.locals.
struct LightRange {
    first: u32,
    count: u32,
};

// Where the cluster entries start in light_lists (FIRST_CLUSTER_ENTRY in
// light_culling.rs).
const FIRST_CLUSTER_ENTRY: u32 = 4u;

fn light_range_at(entry: u32) -> LightRange {
    return LightRange(light_lists[entry], light_lists[entry + 1u]);
}

// The point and spot lights whose reach (and cone) touches the cluster of the camera's
// view that holds `world_pos`.
fn light_cluster(world_pos: vec3<f32>) -> LightRange {
    if (lights.counts.y == 0u) {
        return LightRange(0u, 0u);
    }
    let dims = lights.cluster_dims;
    let clip = lights.cluster_view_proj * vec4<f32>(world_pos, 1.0);
    let ndc = clip.xy / max(clip.w, 1e-6);
    let tile = clamp(
        floor(vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5) * vec2<f32>(dims.xy)),
        vec2<f32>(0.0),
        vec2<f32>(dims.xy) - 1.0,
    );
    let depth = dot(lights.cluster_depth_axis.xyz, world_pos) + lights.cluster_depth_axis.w;
    let slices = lights.cluster_slices;
    let slice_f = select(
        log(max(depth, 1e-4)) * slices.x + slices.y,
        depth * slices.x + slices.y,
        slices.w > 0.5,
    );
    let slice = u32(clamp(floor(slice_f), 0.0, f32(dims.z) - 1.0));
    let cluster = (slice * dims.y + u32(tile.y)) * dims.x + u32(tile.x);
    return light_range_at(FIRST_CLUSTER_ENTRY + 2u * cluster);
}

// The point and spot lights whose reach touches the world grid cell holding `world_pos`,
// for surfaces the camera's clusters do not describe (ray hits). Empty outside the grid,
// which covers every light's reach near the camera.
fn light_grid_cell(world_pos: vec3<f32>) -> LightRange {
    let dims = lights.grid_dims;
    if (lights.counts.y == 0u || lights.counts.z == 0u || dims.x == 0u) {
        return LightRange(0u, 0u);
    }
    let cell_f = floor((world_pos - lights.grid_origin.xyz) * lights.grid_origin.w);
    if (any(cell_f < vec3<f32>(0.0)) || any(cell_f >= vec3<f32>(dims.xyz))) {
        return LightRange(0u, 0u);
    }
    let cell = vec3<u32>(cell_f);
    return light_range_at(dims.w + 2u * ((cell.z * dims.y + cell.y) * dims.x + cell.x));
}

// Light `index` of `range`.
fn light_in_range(range: LightRange, index: u32) -> LocalLight {
    return lights.locals[light_lists[range.first + index]];
}

// How a local light arrives at a point: the direction toward it, its distance, and the
// share of its intensity that gets there (inverse square falloff faded out toward its
// range, times a spot light's cone; 0 where it does not reach).
struct LocalLightArrival {
    L: vec3<f32>,
    distance: f32,
    attenuation: f32,
};

fn local_light_arrival(light: LocalLight, world_pos: vec3<f32>) -> LocalLightArrival {
    var out: LocalLightArrival;
    let to_light = light.position_range.xyz - world_pos;
    out.distance = length(to_light);
    if (out.distance <= 0.0001) {
        return out;
    }
    out.L = to_light / out.distance;
    let range_factor = clamp(1.0 - out.distance / light.position_range.w, 0.0, 1.0);
    var attenuation = range_factor * range_factor / max(out.distance * out.distance, 0.0001);
    if (light.direction.w > -1.5) {
        let cos_theta = dot(normalize(light.direction.xyz), -out.L);
        let cos_outer = light.direction.w;
        let cos_inner = light.params.x;
        var spot_effect = 0.0;
        if (cos_theta >= cos_outer) {
            let denom = max(cos_inner - cos_outer, 0.0001);
            spot_effect = clamp((cos_theta - cos_outer) / denom, 0.0, 1.0);
            spot_effect = spot_effect * spot_effect;
        }
        attenuation = attenuation * spot_effect;
    }
    out.attenuation = attenuation;
    return out;
}

// ============================================================================
// PBR Helper Functions
// ============================================================================

// Normal Distribution Function (GGX/Trowbridge-Reitz)
fn distribution_ggx(N: vec3<f32>, H: vec3<f32>, roughness: f32) -> f32 {
    let a = roughness * roughness;
    let a2 = a * a;
    let NdotH = max(dot(N, H), 0.0);
    let NdotH2 = NdotH * NdotH;
    
    let nom = a2;
    let denom = (NdotH2 * (a2 - 1.0) + 1.0);
    let denom2 = PI * denom * denom;
    
    return nom / denom2;
}

// Geometry Function (Smith's Schlick-GGX)
fn geometry_schlick_ggx(NdotV: f32, roughness: f32) -> f32 {
    let r = (roughness + 1.0);
    let k = (r * r) / 8.0;
    
    let nom = NdotV;
    let denom = NdotV * (1.0 - k) + k;
    
    return nom / denom;
}

fn geometry_smith(N: vec3<f32>, V: vec3<f32>, L: vec3<f32>, roughness: f32) -> f32 {
    let NdotV = max(dot(N, V), 0.0);
    let NdotL = max(dot(N, L), 0.0);
    let ggx2 = geometry_schlick_ggx(NdotV, roughness);
    let ggx1 = geometry_schlick_ggx(NdotL, roughness);
    return ggx1 * ggx2;
}

// Fresnel Function (Schlick approximation)
fn fresnel_schlick(cosTheta: f32, F0: vec3<f32>) -> vec3<f32> {
    return F0 + (1.0 - F0) * pow(clamp(1.0 - cosTheta, 0.0, 1.0), 5.0);
}

fn fresnel_schlick_roughness(cosTheta: f32, F0: vec3<f32>, roughness: f32) -> vec3<f32> {
    return F0 + (max(vec3(1.0 - roughness), F0) - F0) * pow(clamp(1.0 - cosTheta, 0.0, 1.0), 5.0);
}

// ============================================================================
// Light Contribution Calculation
// ============================================================================

// Calculate PBR lighting contribution from a single light source
// Returns the color contribution (diffuse + specular) * radiance * NdotL
fn calculate_light_contribution(
    N: vec3<f32>,           // Surface normal
    V: vec3<f32>,           // View direction
    L: vec3<f32>,           // Light direction
    base_color: vec3<f32>,  // Surface albedo
    metallic: f32,          // Metallic factor
    roughness: f32,         // Roughness factor
    light_color: vec3<f32>, // Light color
    light_intensity: f32    // Light intensity
) -> vec3<f32> {
    let NdotL = max(dot(N, L), 0.0);
    
    // Early out if light doesn't hit surface
    if (NdotL <= 0.0) {
        return vec3<f32>(0.0);
    }
    
    let H = normalize(V + L);
    
    // F0 for dielectrics is 0.04, for metals use albedo
    let F0 = mix(vec3<f32>(0.04), base_color, metallic);
    
    // Cook-Torrance BRDF
    let NDF = distribution_ggx(N, H, roughness);
    let G = geometry_smith(N, V, L, roughness);
    let F = fresnel_schlick(max(dot(H, V), 0.0), F0);
    
    let numerator = NDF * G * F;
    let NdotV = max(dot(N, V), 0.0);
    let denominator = 4.0 * NdotV * NdotL + 0.0001;
    let specular = numerator / denominator;
    
    // Energy conservation
    let kS = F;
    var kD = vec3<f32>(1.0) - kS;
    kD = kD * (1.0 - metallic);
    
    // Diffuse
    let diffuse = kD * base_color / PI;
    
    // Combine
    let radiance = light_color * light_intensity;
    return (diffuse + specular) * radiance * NdotL;
}

// ============================================================================
// Scene Lighting (without shadows)
// ============================================================================

// Calculate lighting from all scene lights (no shadows)
// Use this for particles or when shadows aren't needed
fn calculate_scene_lighting_no_shadows(
    world_pos: vec3<f32>,
    N: vec3<f32>,
    V: vec3<f32>,
    base_color: vec3<f32>,
    metallic: f32,
    roughness: f32
) -> vec3<f32> {
    var Lo = vec3<f32>(0.0);

    // Directional lights
    let dir_count = min(lights.counts.x, MAX_DIRECTIONAL_LIGHTS);
    for (var i = 0u; i < dir_count; i = i + 1u) {
        let light = lights.directionals[i];
        let light_dir = normalize(-light.direction.xyz);
        let light_color = light.color_intensity.xyz;
        let light_intensity = light.color_intensity.w;
        Lo += calculate_light_contribution(
            N, V, light_dir, base_color, metallic, roughness,
            light_color, light_intensity
        );
    }

    // Point and spot lights of the surface's cluster
    let range = light_cluster(world_pos);
    for (var i = 0u; i < range.count; i = i + 1u) {
        let light = light_in_range(range, i);
        let arrival = local_light_arrival(light, world_pos);
        if (arrival.attenuation <= 0.0) {
            continue;
        }
        Lo += calculate_light_contribution(
            N, V, arrival.L, base_color, metallic, roughness,
            light.color_intensity.xyz, light.color_intensity.w * arrival.attenuation
        );
    }

    return Lo;
}

// ============================================================================
// Lights for shader materials
// ============================================================================

// The lights that may reach a point, for custom lighting in shader materials:
//
//     let here = scene_lights_at(in.world_pos);
//     for (var i = 0u; i < scene_light_count(here); i = i + 1u) {
//         let light = scene_light(here, i, in.world_pos);
//         lighting += my_brdf(N, V, light.direction) * light.radiance;
//     }
//
// The directional lights come first, then the point and spot lights of the point's
// cluster (lights out of reach are not listed). Shadows are not included; with
// `// @include_shadows`, calculate_scene_lighting gives PBR lighting with them.
struct SceneLights {
    directional: u32,
    locals: LightRange,
};

struct SceneLight {
    // Toward the light (unit length).
    direction: vec3<f32>,
    // Colour times intensity that arrives at the point (falloff and cone included).
    radiance: vec3<f32>,
};

fn scene_lights_at(world_pos: vec3<f32>) -> SceneLights {
    return SceneLights(min(lights.counts.x, MAX_DIRECTIONAL_LIGHTS), light_cluster(world_pos));
}

fn scene_light_count(here: SceneLights) -> u32 {
    return here.directional + here.locals.count;
}

fn scene_light(here: SceneLights, index: u32, world_pos: vec3<f32>) -> SceneLight {
    if (index < here.directional) {
        let light = lights.directionals[index];
        return SceneLight(
            normalize(-light.direction.xyz),
            light.color_intensity.xyz * light.color_intensity.w,
        );
    }
    let light = light_in_range(here.locals, index - here.directional);
    let arrival = local_light_arrival(light, world_pos);
    return SceneLight(
        arrival.L,
        light.color_intensity.xyz * light.color_intensity.w * arrival.attenuation,
    );
}
