// src/shader/lighting_with_shadows.wgsl
// Complete scene lighting calculation with shadow support
// Requires: lighting_common.wgsl and shadows.wgsl

// ============================================================================
// Scene Lighting with Shadows
// ============================================================================

// Calculate lighting from all scene lights WITH shadows
// This is the complete lighting solution for main geometry
fn calculate_scene_lighting(
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
        let shadow = sample_directional_shadow(i, world_pos);
        Lo += shadow * calculate_light_contribution(
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
        
        // ALWAYS sample shadow in uniform control flow
        let shadow = sample_point_shadow(
            i, 
            world_pos, 
            light.position_range.xyz, 
            light.position_range.w
        );
        
        // Then conditionally use the result
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
            Lo += shadow * calculate_light_contribution(
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
        
        // ALWAYS sample shadow in uniform control flow
        let shadow = sample_spot_shadow_advanced(
            i, 
            world_pos, 
            N,
            light.position_range.xyz,
            light.direction.xyz
        );
        
        // Then conditionally use the result
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
                Lo += shadow * calculate_light_contribution(
                    N, V, L, base_color, metallic, roughness,
                    light_color, light_intensity
                );
            }
        }
    }

    return Lo;
}