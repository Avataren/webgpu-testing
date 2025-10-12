// src/shader/lighting_common.wgsl
// Shared lighting structures, bindings, and functions for all pipelines

// ============================================================================
// Light Structures
// ============================================================================


struct DirectionalLight {
    direction: vec4<f32>,
    color_intensity: vec4<f32>,
};

struct PointLight {
    position_range: vec4<f32>,
    color_intensity: vec4<f32>,
};

struct SpotLight {
    position_range: vec4<f32>,
    direction: vec4<f32>,
    color_intensity: vec4<f32>,
    cone_params: vec4<f32>,  // x: cos_inner, y: cos_outer
};

struct Lights {
    counts: vec4<u32>,  // x: directional, y: point, z: spot, w: unused
    directionals: array<DirectionalLight, MAX_DIRECTIONAL_LIGHTS>,
    points: array<PointLight, MAX_POINT_LIGHTS>,
    spots: array<SpotLight, MAX_SPOT_LIGHTS>,
};

// ============================================================================
// Light Bindings (group 2)
// ============================================================================

@group(2) @binding(0) var<storage, read> lights: Lights;

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

    // Point lights
    let point_count = min(lights.counts.y, MAX_POINT_LIGHTS);
    for (var i = 0u; i < point_count; i = i + 1u) {
        let light = lights.points[i];
        let to_light = light.position_range.xyz - world_pos;
        let distance = length(to_light);
        
        if (distance > 0.0001) {
            let L = to_light / distance;
            var attenuation = 1.0 / max(distance * distance, 0.0001);
            let range = light.position_range.w;
            if (range > 0.0) {
                let range_factor = clamp(1.0 - distance / range, 0.0, 1.0);
                attenuation = attenuation * range_factor * range_factor;
            }
            let light_color = light.color_intensity.xyz;
            let light_intensity = light.color_intensity.w * attenuation;
            Lo += calculate_light_contribution(
                N, V, L, base_color, metallic, roughness,
                light_color, light_intensity
            );
        }
    }

    // Spot lights
    let spot_count = min(lights.counts.z, MAX_SPOT_LIGHTS);
    for (var i = 0u; i < spot_count; i = i + 1u) {
        let light = lights.spots[i];
        let to_light = light.position_range.xyz - world_pos;
        let distance = length(to_light);
        
        if (distance > 0.0001) {
            let L = to_light / distance;
            var attenuation = 1.0 / max(distance * distance, 0.0001);
            let range = light.position_range.w;
            if (range > 0.0) {
                let range_factor = clamp(1.0 - distance / range, 0.0, 1.0);
                attenuation = attenuation * range_factor * range_factor;
            }

            let light_dir = normalize(light.direction.xyz);
            let cos_theta = dot(light_dir, -L);
            let cos_inner = light.cone_params.x;
            let cos_outer = light.cone_params.y;
            var spot_effect = 0.0;
            if (cos_theta >= cos_outer) {
                let denom = max(cos_inner - cos_outer, 0.0001);
                spot_effect = clamp((cos_theta - cos_outer) / denom, 0.0, 1.0);
                spot_effect = spot_effect * spot_effect;
            }

            if (spot_effect > 0.0) {
                let light_color = light.color_intensity.xyz;
                let light_intensity = light.color_intensity.w * attenuation * spot_effect;
                Lo += calculate_light_contribution(
                    N, V, L, base_color, metallic, roughness,
                    light_color, light_intensity
                );
            }
        }
    }

    return Lo;
}