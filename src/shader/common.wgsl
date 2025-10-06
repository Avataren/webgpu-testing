// PBR Shader with Normal Mapping and Modular Lighting

struct Globals {
    view_proj: mat4x4<f32>,
    camera_pos: vec3<f32>,
    _padding: f32,
};
@group(0) @binding(0) var<uniform> globals: Globals;

struct Object {
    model: mat4x4<f32>,
    color: vec4<f32>,
    base_color_texture: u32,
    metallic_roughness_texture: u32,
    normal_texture: u32,
    emissive_texture: u32,
    occlusion_texture: u32,
    material_flags: u32,
    metallic_factor: f32,
    roughness_factor: f32,
    emissive_strength: f32,
    _padding: u32,
};
@group(1) @binding(0) var<storage, read> objects: array<Object>;

// Material flags
const FLAG_USE_BASE_COLOR_TEXTURE: u32 = 1u;
const FLAG_USE_METALLIC_ROUGHNESS_TEXTURE: u32 = 2u;
const FLAG_USE_NORMAL_TEXTURE: u32 = 4u;
const FLAG_USE_EMISSIVE_TEXTURE: u32 = 8u;
const FLAG_USE_OCCLUSION_TEXTURE: u32 = 16u;
const FLAG_ALPHA_BLEND: u32 = 32u;

const MAX_DIRECTIONAL_LIGHTS: u32 = 4u;
const MAX_POINT_LIGHTS: u32 = 16u;
const MAX_SPOT_LIGHTS: u32 = 8u;
const POINT_SHADOW_FACE_COUNT: u32 = 6u;

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
    cone_params: vec4<f32>,
};

struct Lights {
    counts: vec4<u32>,
    directionals: array<DirectionalLight, MAX_DIRECTIONAL_LIGHTS>,
    points: array<PointLight, MAX_POINT_LIGHTS>,
    spots: array<SpotLight, MAX_SPOT_LIGHTS>,
};

@group(2) @binding(0) var<storage, read> lights: Lights;

struct DirectionalShadow {
    view_proj: mat4x4<f32>,
    params: vec4<f32>,
    _padding: vec4<f32>,
};

struct PointShadow {
    view_proj: array<mat4x4<f32>, POINT_SHADOW_FACE_COUNT>,
    params: vec4<f32>,
};

struct SpotShadow {
    view_proj: mat4x4<f32>,
    params: vec4<f32>,
};

struct Shadows {
    counts: vec4<u32>,
    directionals: array<DirectionalShadow, MAX_DIRECTIONAL_LIGHTS>,
    points: array<PointShadow, MAX_POINT_LIGHTS>,
    spots: array<SpotShadow, MAX_SPOT_LIGHTS>,
};

@group(2) @binding(1) var<uniform> shadow_info: Shadows;

@group(2) @binding(2) var directional_shadow_maps: texture_depth_2d_array;
@group(2) @binding(3) var directional_shadow_sampler: sampler_comparison;
@group(2) @binding(4) var spot_shadow_maps: texture_depth_2d_array;
@group(2) @binding(5) var spot_shadow_sampler: sampler_comparison;
@group(2) @binding(6) var point_shadow_maps: texture_depth_2d_array;
@group(2) @binding(7) var point_shadow_sampler: sampler_comparison;

const PI: f32 = 3.14159265359;

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) tangent: vec4<f32>,  // xyz = tangent, w = handedness
    @builtin(instance_index) instance: u32,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) @interpolate(flat) instance_id: u32,
    @location(4) tangent: vec3<f32>,
    @location(5) bitangent: vec3<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    let obj = objects[in.instance];
    let M = obj.model;
    let world_pos = M * vec4(in.pos, 1.0);
    
    // Transform normal and tangent to world space
    // For non-uniform scaling, we should use inverse transpose of the model matrix
    // But for now, this works for uniform scaling
    let n = normalize((M * vec4(in.normal, 0.0)).xyz);
    let t = normalize((M * vec4(in.tangent.xyz, 0.0)).xyz);
    
    // Calculate bitangent using the handedness from the tangent w component
    // B = (N × T) * handedness
    let b = cross(n, t) * in.tangent.w;

    var out: VsOut;
    out.pos = globals.view_proj * world_pos;
    out.world_pos = world_pos.xyz;
    out.normal = n;
    out.uv = in.uv;
    out.instance_id = in.instance;
    out.tangent = t;
    out.bitangent = b;
    return out;
}

// PBR Functions

fn distribution_ggx(N: vec3<f32>, H: vec3<f32>, roughness: f32) -> f32 {
    let a = roughness * roughness;
    let a2 = a * a;
    let NdotH = max(dot(N, H), 0.0);
    let NdotH2 = NdotH * NdotH;
    
    let denom = (NdotH2 * (a2 - 1.0) + 1.0);
    return a2 / (PI * denom * denom);
}

fn geometry_schlick_ggx(NdotV: f32, roughness: f32) -> f32 {
    let r = (roughness + 1.0);
    let k = (r * r) / 8.0;
    return NdotV / (NdotV * (1.0 - k) + k);
}

fn geometry_smith(N: vec3<f32>, V: vec3<f32>, L: vec3<f32>, roughness: f32) -> f32 {
    let NdotV = max(dot(N, V), 0.0);
    let NdotL = max(dot(N, L), 0.0);
    let ggx2 = geometry_schlick_ggx(NdotV, roughness);
    let ggx1 = geometry_schlick_ggx(NdotL, roughness);
    return ggx1 * ggx2;
}

fn fresnel_schlick(cos_theta: f32, F0: vec3<f32>) -> vec3<f32> {
    return F0 + (1.0 - F0) * pow(clamp(1.0 - cos_theta, 0.0, 1.0), 5.0);
}

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

// Retained hardcoded lighting for testing and fallback scenarios
fn calculate_test_lighting(
    _world_pos: vec3<f32>,
    N: vec3<f32>,
    V: vec3<f32>,
    base_color: vec3<f32>,
    metallic: f32,
    roughness: f32
) -> vec3<f32> {
    var Lo = vec3<f32>(0.0);
    
    // Key light (main directional light from above-right)
    {
        let light_dir = normalize(vec3<f32>(0.5, 0.8, 0.3));
        let light_color = vec3<f32>(1.0, 1.0, 1.0);
        let light_intensity = 2.5;
        Lo += calculate_light_contribution(N, V, light_dir, base_color, metallic, roughness, light_color, light_intensity);
    }
    
    // Fill light (from camera direction, softer - ensures specular highlights are visible)
    {
        let light_dir = normalize(V + vec3<f32>(0.0, 0.3, 0.0));
        let light_color = vec3<f32>(0.9, 0.95, 1.0); // Slightly cool tint
        let light_intensity = 1.5;
        Lo += calculate_light_contribution(N, V, light_dir, base_color, metallic, roughness, light_color, light_intensity);
    }
    
    // Rim light (from behind, for edge definition)
    {
        let light_dir = normalize(vec3<f32>(-0.3, 0.2, -0.5));
        let light_color = vec3<f32>(1.0, 0.95, 0.9); // Slightly warm tint
        let light_intensity = 1.0;
        Lo += calculate_light_contribution(N, V, light_dir, base_color, metallic, roughness, light_color, light_intensity);
    }
    
    return Lo;
}


// fn project_shadow(matrix: mat4x4<f32>, world_pos: vec3<f32>) -> vec3<f32> {
//     let clip = matrix * vec4<f32>(world_pos, 1.0);
//     if (clip.w <= 0.0) {
//         return vec3<f32>(-1.0, -1.0, -1.0);
//     }
//     let ndc = clip.xyz / clip.w;
//     return vec3<f32>(ndc.xy * 0.5 + 0.5, ndc.z);  // Use ndc.z directly
// }

fn project_shadow(matrix: mat4x4<f32>, world_pos: vec3<f32>) -> vec3<f32> {
    let clip = matrix * vec4<f32>(world_pos, 1.0);
    if (clip.w <= 0.0) {
        return vec3<f32>(-1.0, -1.0, -1.0);
    }
    let ndc = clip.xyz / clip.w;
    // Our projection matrices already map the clip space depth into
    // wgpu's [0, 1] range, so we only need to remap the XY coordinates
    // from [-1, 1] into [0, 1]. The Y axis of texture coordinates is
    // flipped compared to clip space however (texture V=0 is at the top
    // of the image), so we mirror the Y coordinate while remapping. Re-
    // mapping Z again would shrink the usable depth range and skew the
    // shadow comparison.
    return vec3<f32>(
        ndc.x * 0.5 + 0.5,
        -ndc.y * 0.5 + 0.5,
        ndc.z,
    );
}
// fn project_shadow(matrix: mat4x4<f32>, world_pos: vec3<f32>) -> vec3<f32> {
//     let clip = matrix * vec4<f32>(world_pos, 1.0);
//     if (clip.w <= 0.0) {
//         return vec3<f32>(-1.0, -1.0, -1.0);
//     }
//     let ndc = clip.xyz / clip.w;
//     //return vec3<f32>(ndc.xy * 0.5 + 0.5, ndc.z * 0.5 + 0.5);
//     return vec3<f32>(ndc.xy * 0.5 + 0.5, ndc.z);
// }

fn sample_directional_shadow(index: u32, world_pos: vec3<f32>) -> f32 {
    let info = shadow_info.directionals[index];
    if (info.params.x == 0.0) {
        return 1.0;  // No shadow data - fully lit
    }

    let proj = project_shadow(info.view_proj, world_pos);
    if (proj.z < 0.0 || proj.z > 1.0) {
        return 1.0;  // Outside depth range - fully lit
    }
    if (proj.x < 0.0 || proj.x > 1.0 || proj.y < 0.0 || proj.y > 1.0) {
        return 1.0;  // Outside shadow map bounds - fully lit
    }

    let depth = clamp(proj.z - info.params.y, 0.0, 1.0);
    return textureSampleCompare(
        directional_shadow_maps,
        directional_shadow_sampler,
        proj.xy,
        i32(index),
        depth,
    );
}

fn sample_spot_shadow(index: u32, world_pos: vec3<f32>) -> f32 {
    let info = shadow_info.spots[index];
    if (info.params.x == 0.0) {
        return 1.0;
    }

    let proj = project_shadow(info.view_proj, world_pos);
    if (proj.z < 0.0 || proj.z > 1.0) {
        return 1.0;
    }
    if (proj.x < 0.0 || proj.x > 1.0 || proj.y < 0.0 || proj.y > 1.0) {
        return 1.0;
    }

    let depth = clamp(proj.z - info.params.y, 0.0, 1.0);
    return textureSampleCompare(
        spot_shadow_maps,
        spot_shadow_sampler,
        proj.xy,
        i32(index),
        depth,
    );
}

fn select_point_face(direction: vec3<f32>) -> u32 {
    let abs_dir = abs(direction);
    if (abs_dir.x >= abs_dir.y && abs_dir.x >= abs_dir.z) {
        if (direction.x > 0.0) {
            return 0u;
        } else {
            return 1u;
        }
    } else if (abs_dir.y >= abs_dir.z) {
        if (direction.y > 0.0) {
            return 2u;
        } else {
            return 3u;
        }
    } else {
        if (direction.z > 0.0) {
            return 4u;
        } else {
            return 5u;
        }
    }
}

fn sample_point_shadow(index: u32, world_pos: vec3<f32>) -> f32 {
    let info = shadow_info.points[index];
    if (info.params.x == 0.0) {
        return 1.0;
    }

    let light = lights.points[index];
    let light_pos = light.position_range.xyz;
    let to_fragment = world_pos - light_pos;
    let distance = length(to_fragment);
    if (distance <= 0.0001) {
        return 1.0;
    }

    let range = light.position_range.w;
    if (range > 0.0 && distance > range) {
        return 1.0;
    }

    let dir = normalize(to_fragment);
    let face = select_point_face(dir);
    let matrix = info.view_proj[face];
    let proj = project_shadow(matrix, world_pos);
    if (proj.z < 0.0 || proj.z > 1.0) {
        return 1.0;
    }
    if (proj.x < 0.0 || proj.x > 1.0 || proj.y < 0.0 || proj.y > 1.0) {
        return 1.0;
    }

    let layer = i32(index * POINT_SHADOW_FACE_COUNT + face);
    let depth = clamp(proj.z - info.params.y, 0.0, 1.0);
    return textureSampleCompare(
        point_shadow_maps,
        point_shadow_sampler,
        proj.xy,
        layer,
        depth,
    );
}

fn calculate_scene_lighting(
    world_pos: vec3<f32>,
    N: vec3<f32>,
    V: vec3<f32>,
    base_color: vec3<f32>,
    metallic: f32,
    roughness: f32
) -> vec3<f32> {
    // if (lights.counts.x == 0u && lights.counts.y == 0u && lights.counts.z == 0u) {
    //     return calculate_test_lighting(world_pos, N, V, base_color, metallic, roughness);
    // }

    var Lo = vec3<f32>(0.0);

    let dir_count = min(lights.counts.x, MAX_DIRECTIONAL_LIGHTS);
    for (var i = 0u; i < dir_count; i = i + 1u) {
        let light = lights.directionals[i];
        let light_dir = normalize(-light.direction.xyz);
        let light_color = light.color_intensity.xyz;
        let light_intensity = light.color_intensity.w;
        let shadow = sample_directional_shadow(i, world_pos);
        Lo += shadow * calculate_light_contribution(
            N,
            V,
            light_dir,
            base_color,
            metallic,
            roughness,
            light_color,
            light_intensity,
        );
    }

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
            let shadow = sample_point_shadow(i, world_pos);
            Lo += shadow * calculate_light_contribution(
                N,
                V,
                L,
                base_color,
                metallic,
                roughness,
                light_color,
                light_intensity,
            );
        }
    }

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
                let shadow = sample_spot_shadow(i, world_pos);
                Lo += shadow * calculate_light_contribution(
                    N,
                    V,
                    L,
                    base_color,
                    metallic,
                    roughness,
                    light_color,
                    light_intensity,
                );
            }
        }
    }

    return Lo;
}

// @fragment
// fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
//     let obj = objects[in.instance_id];
    
//     // DEBUG: Check if world position is in shadow map bounds
//     let info = shadow_info.directionals[0u];
//     if (info.params.x != 0.0) {
//         let proj = project_shadow(info.view_proj, in.world_pos);
        
//         // Visualize shadow map coordinates
//         if (proj.x >= 0.0 && proj.x <= 1.0 && proj.y >= 0.0 && proj.y <= 1.0) {
//             // In shadow map bounds - show depth as color
//             return vec4<f32>(proj.z, proj.z, proj.z, 1.0);
//         } else {
//             // Outside shadow map - show red
//             return vec4<f32>(1.0, 0.0, 0.0, 1.0);
//         }
//     }
    
//     // If no shadow info, show blue
//     return vec4<f32>(0.0, 0.0, 1.0, 1.0);
// }

// @fragment
// fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
//     let shadow = sample_directional_shadow(0u, in.world_pos);
//     return vec4<f32>(shadow, shadow, shadow, 1.0);
// }

// @fragment
// fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
//     let shadow_mat = shadow_info.directionals[0u].view_proj;
//     let test_pos = vec3<f32>(0.0, 0.1, 0.0);
    
//     // Shadow projection - manual multiply to see intermediate values
//     let shadow_clip = shadow_mat * vec4<f32>(test_pos, 1.0);
//     let shadow_w = shadow_clip.w;
//     let shadow_z = shadow_clip.z;
    
//     // Show raw values (might be outside [0,1])
//     // Red = clip.z, Green = clip.w (for perspective divide)
//     return vec4<f32>(
//         shadow_z * 0.1 + 0.5,  // Scale to make visible
//         shadow_w * 0.1 + 0.5,
//         0.0,
//         1.0
//     );
// }

// @fragment
// fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
//     let spot_count = min(lights.counts.z, MAX_SPOT_LIGHTS);
//     if (spot_count == 0u) {
//         return vec4<f32>(1.0, 0.0, 0.0, 1.0);  // Red = no lights
//     }
    
//     // Test first spot light
//     let light = lights.spots[0u];
//     let to_light = light.position_range.xyz - in.world_pos;
//     let distance = length(to_light);
//     let L = to_light / distance;
//     let light_dir = normalize(light.direction.xyz);
//     let cos_theta = dot(light_dir, -L);
    
//     // Visualize the angle check
//     return vec4<f32>(
//         cos_theta,  // Red channel shows the angle
//         light.cone_params.y,  // Green = cos_outer threshold
//         0.0,
//         1.0
//     );
// }


@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let obj = objects[in.instance_id];

    // shadow debug    
    // let shadow = sample_directional_shadow(0u, in.world_pos);
    // if (shadow < 0.99) {
    //     return vec4<f32>(1.0, 0.0, 0.0, 1.0);  // Red = in shadow
    // }

    // ALWAYS sample all textures (uniform control flow)
    let base_color_sample = sample_base_color_texture(obj.base_color_texture, in.uv);
    let mr_sample = sample_metallic_roughness_texture(obj.metallic_roughness_texture, in.uv);
    let normal_sample = sample_normal_texture(obj.normal_texture, in.uv);
    let emissive_sample = sample_emissive_texture(obj.emissive_texture, in.uv);
    let occlusion_sample = sample_occlusion_texture(obj.occlusion_texture, in.uv);
    
    // Then conditionally USE the samples (non-uniform control flow is OK here)
    var base_color: vec4<f32>;
    if ((obj.material_flags & FLAG_USE_BASE_COLOR_TEXTURE) != 0u) {
        base_color = base_color_sample * obj.color;
    } else {
        base_color = obj.color;
    }
    
    var metallic: f32;
    var roughness: f32;
    if ((obj.material_flags & FLAG_USE_METALLIC_ROUGHNESS_TEXTURE) != 0u) {
        metallic = mr_sample.b * obj.metallic_factor;
        roughness = mr_sample.g * obj.roughness_factor;
    } else {
        metallic = obj.metallic_factor;
        roughness = obj.roughness_factor;
    }
    roughness = max(roughness, 0.01);
    
    var N: vec3<f32>;
    if ((obj.material_flags & FLAG_USE_NORMAL_TEXTURE) != 0u) {
        let tangent_normal = normal_sample * 2.0 - 1.0;
        let T = normalize(in.tangent);
        let B = normalize(in.bitangent);
        let N_base = normalize(in.normal);
        let TBN = mat3x3<f32>(T, B, N_base);
        N = normalize(TBN * tangent_normal);
    } else {
        N = normalize(in.normal);
    }
    
    var occlusion = 1.0;
    if ((obj.material_flags & FLAG_USE_OCCLUSION_TEXTURE) != 0u) {
        occlusion = occlusion_sample;
    }
    
    var emissive = vec3<f32>(0.0);
    if ((obj.material_flags & FLAG_USE_EMISSIVE_TEXTURE) != 0u) {
        emissive = emissive_sample * obj.emissive_strength;
    }
    
    let V = normalize(globals.camera_pos - in.world_pos);
    let Lo =
        calculate_scene_lighting(in.world_pos, N, V, base_color.rgb, metallic, roughness);
    let ambient = vec3<f32>(0.03) * base_color.rgb * occlusion;
    
    var color = ambient + Lo + emissive;
    color = color / (color + vec3<f32>(1.0));
    color = pow(color, vec3<f32>(1.0 / 2.2));
    return vec4<f32>(color, base_color.a);
}