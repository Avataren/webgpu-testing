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
    return calculate_scene_lighting_at(
        world_pos, N, V, base_color, metallic, roughness, vec2<f32>(-1.0)
    );
}

// The same for the opaque surface at framebuffer `pixel`, whose directional shadows may
// come from the ray-traced shadow mask instead of the shadow maps (pixel.x < 0: never;
// the pixel also turns the shadow maps' soft shadow filter).
fn calculate_scene_lighting_at(
    world_pos: vec3<f32>,
    N: vec3<f32>,
    V: vec3<f32>,
    base_color: vec3<f32>,
    metallic: f32,
    roughness: f32,
    pixel: vec2<f32>,
) -> vec3<f32> {
    var Lo = vec3<f32>(0.0);
    // A light that cannot reach the surface (facing away, out of range, outside the
    // cone, or fully shadowed) is skipped before its shadow is sampled: its contribution
    // would be zero anyway, and the shadow functions sample at an explicit level, so
    // they need no uniform control flow.

    // Directional lights
    let dir_count = min(lights.counts.x, MAX_DIRECTIONAL_LIGHTS);
    for (var i = 0u; i < dir_count; i = i + 1u) {
        let light = lights.directionals[i];
        let light_dir = normalize(-light.direction.xyz);
        if (dot(N, light_dir) <= 0.0) {
            continue;
        }
        var shadow = rt_shadow_bit(i, pixel);
        if (shadow < 0.0) {
            shadow = sample_directional_shadow(i, world_pos, N, light_dir, pixel);
        }
        if (shadow <= 0.0) {
            continue;
        }
        let light_color = light.color_intensity.xyz;
        let light_intensity = light.color_intensity.w;
        Lo += shadow * calculate_light_contribution(
            N, V, light_dir, base_color, metallic, roughness,
            light_color, light_intensity
        );
    }

    // Point and spot lights of the surface's cluster
    let range = light_cluster(world_pos);
    for (var i = 0u; i < range.count; i = i + 1u) {
        let light = light_in_range(range, i);
        let arrival = local_light_arrival(light, world_pos);
        if (arrival.attenuation <= 0.0 || dot(N, arrival.L) <= 0.0) {
            continue;
        }
        var shadow = -1.0;
        let rt_bit = i32(light.params.z);
        if (rt_bit >= 0) {
            shadow = rt_shadow_bit(u32(rt_bit), pixel);
        }
        if (shadow < 0.0) {
            shadow = sample_local_shadow(light, world_pos, N);
        }
        if (shadow <= 0.0) {
            continue;
        }
        let light_color = light.color_intensity.xyz;
        let light_intensity = light.color_intensity.w * arrival.attenuation;
        Lo += shadow * calculate_light_contribution(
            N, V, arrival.L, base_color, metallic, roughness,
            light_color, light_intensity
        );
    }

    return Lo;
}
