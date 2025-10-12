// src/shader/environment.wgsl
// Shared environment lighting and sampling for all pipelines

// ============================================================================
// Environment Structures
// ============================================================================

struct EnvironmentSettings {
    flags_intensity: vec4<f32>,  // x: hdr_enabled, y: hdr_intensity, z: ambient_intensity, w: max_lod
    ambient_color: vec4<f32>,    // rgb: tint, a: ambient_intensity (duplicate for driver compatibility)
};

// ============================================================================
// Environment Bindings (group 2)
// ============================================================================

@group(2) @binding(8) var<uniform> environment_settings: EnvironmentSettings;
@group(2) @binding(9) var environment_map: texture_2d<f32>;
@group(2) @binding(10) var environment_sampler: sampler;

// ============================================================================
// Environment Queries
// ============================================================================

fn environment_hdr_enabled() -> bool {
    return environment_settings.flags_intensity.x > 0.5;
}

fn environment_hdr_intensity() -> f32 {
    return environment_settings.flags_intensity.y;
}

fn environment_ambient_intensity() -> f32 {
    // The ambient intensity is duplicated in both the primary flag vector and the
    // w component of the ambient color to make it accessible even on backends
    // that mishandle vec4 packing. Prefer the explicitly stored value whenever it
    // contains a meaningful value and only fall back to the flag copy if the
    // encoded channel is zero (which can happen on buggy drivers that drop the
    // final component of a vec4).
    let encoded = environment_settings.ambient_color.w;
    if (encoded > 0.0) {
        return encoded;
    }
    return environment_settings.flags_intensity.z;
}

// ============================================================================
// Environment Sampling
// ============================================================================

fn direction_to_equirect(direction: vec3<f32>) -> vec2<f32> {
    let dir = normalize(direction);
    let theta = atan2(dir.z, dir.x);
    let phi = acos(clamp(dir.y, -1.0, 1.0));
    let u = fract(0.5 - theta / TWO_PI);
    let v = clamp(phi / PI, 0.0, 1.0);
    return vec2<f32>(u, v);
}

fn environment_uv(direction: vec3<f32>) -> vec2<f32> {
    let base_uv = direction_to_equirect(direction);
    let dims_u32 = textureDimensions(environment_map, 0);
    let dims = vec2<f32>(f32(dims_u32.x), f32(dims_u32.y));
    let safe_dims = max(dims, vec2<f32>(1.0, 1.0));
    let inv_dims = vec2<f32>(1.0, 1.0) / safe_dims;
    let texel = inv_dims * 0.5;
    let shifted = base_uv + texel;
    let wrapped_u = fract(shifted.x);
    let clamped_v = clamp(shifted.y, texel.y, 1.0 - texel.y);
    return vec2<f32>(wrapped_u, clamped_v);
}

fn sample_environment_hdr(direction: vec3<f32>, lod: f32) -> vec3<f32> {
    let uv = environment_uv(direction);
    return textureSampleLevel(environment_map, environment_sampler, uv, lod).rgb;
}

// ============================================================================
// Environment Lighting Calculation
// ============================================================================

fn calculate_environment_lighting(
    N: vec3<f32>,
    V: vec3<f32>,
    base_color: vec3<f32>,
    metallic: f32,
    roughness: f32,
    occlusion: f32,
) -> vec3<f32> {
    let hdr_enabled = environment_hdr_enabled();
    
    var ambient_tint: vec3<f32>;
    if (hdr_enabled) {
        ambient_tint = environment_settings.ambient_color.rgb;
    } else {
        // When no HDR map is bound, fall back to a neutral tint so the
        // ambient intensity slider still brightens unlit surfaces even if the
        // clear color is black.
        ambient_tint = vec3<f32>(1.0);
    }
    
    let ambient_base = ambient_tint * environment_ambient_intensity();
    let fallback_ambient = ambient_base * base_color;

    if (hdr_enabled) {
        let max_lod = environment_settings.flags_intensity.w;
        
        // Sample irradiance for diffuse
        let irradiance = sample_environment_hdr(N, max_lod) * environment_hdr_intensity();
        let diffuse_color = base_color * (1.0 - metallic);
        let diffuse = irradiance * diffuse_color;

        // Sample environment for specular
        let reflected = reflect(-V, N);
        let rough_lod = max_lod * roughness * roughness;
        let spec_sample = sample_environment_hdr(reflected, rough_lod) * environment_hdr_intensity();
        let specular_color = mix(vec3<f32>(0.04), base_color, vec3<f32>(metallic));
        
        // Fresnel approximation for specular strength
        var specular_strength = pow(clamp(1.0 - roughness, 0.0, 1.0), 4.0);
        specular_strength = max(specular_strength, 0.05);
        let specular = spec_sample * specular_color * specular_strength;

        // Apply occlusion once to the entire environment contribution
        return (fallback_ambient + diffuse + specular) * occlusion;
    }

    // When no HDR is available, apply occlusion to the fallback ambient
    return fallback_ambient * occlusion;
}