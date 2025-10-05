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

@group(2) @binding(0) var textures: binding_array<texture_2d<f32>, 256>;
@group(2) @binding(1) var tex_sampler: sampler;

// Material flags
const FLAG_USE_BASE_COLOR_TEXTURE: u32 = 1u;
const FLAG_USE_METALLIC_ROUGHNESS_TEXTURE: u32 = 2u;
const FLAG_USE_NORMAL_TEXTURE: u32 = 4u;
const FLAG_USE_EMISSIVE_TEXTURE: u32 = 8u;
const FLAG_USE_OCCLUSION_TEXTURE: u32 = 16u;
const FLAG_ALPHA_BLEND: u32 = 32u;

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

// TODO: Replace this with dynamic lights from ECS
// This is a temporary hardcoded three-point lighting setup for PBR testing
fn calculate_scene_lighting(
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

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let obj = objects[in.instance_id];
    
    // Sample base color
    var base_color: vec4<f32>;
    if ((obj.material_flags & FLAG_USE_BASE_COLOR_TEXTURE) != 0u) {
        base_color = textureSample(textures[obj.base_color_texture], tex_sampler, in.uv);
        base_color = base_color * obj.color;
    } else {
        base_color = obj.color;
    }
    
    // Sample metallic and roughness
    var metallic: f32;
    var roughness: f32;
    if ((obj.material_flags & FLAG_USE_METALLIC_ROUGHNESS_TEXTURE) != 0u) {
        let mr = textureSample(textures[obj.metallic_roughness_texture], tex_sampler, in.uv);
        metallic = mr.b * obj.metallic_factor;
        roughness = mr.g * obj.roughness_factor;
    } else {
        metallic = obj.metallic_factor;
        roughness = obj.roughness_factor;
    }
    roughness = max(roughness, 0.01); // Prevent division by zero
    
    // Sample normal map and transform to world space
    var N: vec3<f32>;
    if ((obj.material_flags & FLAG_USE_NORMAL_TEXTURE) != 0u) {
        // Sample normal map (in tangent space, range [0,1])
        let normal_sample = textureSample(textures[obj.normal_texture], tex_sampler, in.uv).xyz;
        
        // Convert from [0,1] to [-1,1]
        let tangent_normal = normal_sample * 2.0 - 1.0;
        
        // Build TBN matrix to transform from tangent space to world space
        let T = normalize(in.tangent);
        let B = normalize(in.bitangent);
        let N_base = normalize(in.normal);
        let TBN = mat3x3<f32>(T, B, N_base);
        
        // Transform normal to world space
        N = normalize(TBN * tangent_normal);
    } else {
        N = normalize(in.normal);
    }
    
    // Sample occlusion
    var occlusion = 1.0;
    if ((obj.material_flags & FLAG_USE_OCCLUSION_TEXTURE) != 0u) {
        occlusion = textureSample(textures[obj.occlusion_texture], tex_sampler, in.uv).r;
    }
    
    // Sample emissive
    var emissive = vec3<f32>(0.0);
    if ((obj.material_flags & FLAG_USE_EMISSIVE_TEXTURE) != 0u) {
        emissive = textureSample(textures[obj.emissive_texture], tex_sampler, in.uv).rgb * obj.emissive_strength;
    }
    
    // View direction (from surface to camera)
    let V = normalize(globals.camera_pos - in.world_pos);
    
    // Calculate lighting (currently hardcoded, will be replaced with ECS light entities)
    let Lo = calculate_scene_lighting(N, V, base_color.rgb, metallic, roughness);
    
    // Ambient (very simple)
    let ambient = vec3<f32>(0.01) * base_color.rgb * occlusion;
    
    var color = ambient + Lo + emissive;
    
    // Tone mapping (simple Reinhard)
    color = color / (color + vec3<f32>(1.0));
    
    return vec4<f32>(color, base_color.a);
}