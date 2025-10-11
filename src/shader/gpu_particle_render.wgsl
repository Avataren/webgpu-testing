// GPU-driven particle rendering - reads particle state directly

struct Globals {
    view_proj: mat4x4<f32>,
    inverse_view_proj: mat4x4<f32>,
    camera_pos: vec3<f32>,
    _padding: f32,
};
@group(0) @binding(0) var<uniform> globals: Globals;

struct ParticleState {
    position: vec3<f32>,
    speed: f32,
    rotation: vec4<f32>,
    angular_axis: vec3<f32>,
    angular_speed: f32,
    scale: f32,
    seed: u32,
    _padding: vec2<u32>,
};

@group(1) @binding(0)
var<storage, read> particles: array<ParticleState>;

struct MaterialData {
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
    _padding2: vec2<u32>,
};

@group(1) @binding(1)
var<uniform> particle_material: MaterialData;

// Material flags
const FLAG_USE_BASE_COLOR_TEXTURE: u32 = 1u;
const FLAG_USE_METALLIC_ROUGHNESS_TEXTURE: u32 = 2u;
const FLAG_USE_NORMAL_TEXTURE: u32 = 4u;
const FLAG_USE_EMISSIVE_TEXTURE: u32 = 8u;
const FLAG_USE_OCCLUSION_TEXTURE: u32 = 16u;
const FLAG_ALPHA_BLEND: u32 = 32u;
const FLAG_UNLIT: u32 = 128u;
const FLAG_USE_NEAREST_SAMPLER: u32 = 256u;

// Lighting (same as main shader)
const MAX_DIRECTIONAL_LIGHTS: u32 = 4u;
const MAX_POINT_LIGHTS: u32 = 4u;
const MAX_SPOT_LIGHTS: u32 = 4u;

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

// Textures (bindless)
@group(3) @binding(0) var textures: binding_array<texture_2d<f32>, 256>;
@group(3) @binding(1) var tex_sampler_linear: sampler;
@group(3) @binding(2) var tex_sampler_nearest: sampler;

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) tangent: vec4<f32>,
    @builtin(instance_index) instance: u32,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) tangent: vec3<f32>,
    @location(4) bitangent: vec3<f32>,
};

fn rotation_matrix(q: vec4<f32>) -> mat3x3<f32> {
    let x = q.x;
    let y = q.y;
    let z = q.z;
    let w = q.w;

    let xx = x + x;
    let yy = y + y;
    let zz = z + z;
    let xy = x * yy;
    let xz = x * zz;
    let yz = y * zz;
    let wx = w * xx;
    let wy = w * yy;
    let wz = w * zz;

    let c0 = vec3<f32>(1.0 - (y * yy + z * zz), xy + wz, xz - wy);
    let c1 = vec3<f32>(xy - wz, 1.0 - (x * xx + z * zz), yz + wx);
    let c2 = vec3<f32>(xz + wy, yz - wx, 1.0 - (x * xx + y * yy));
    return mat3x3<f32>(c0, c1, c2);
}

@vertex
fn vs_main(in: VsIn) -> VsOut {
    // Read particle state directly
    let particle = particles[in.instance];
    
    // Build model matrix on-the-fly
    let rot_mat = rotation_matrix(particle.rotation);
    let scaled_rot = rot_mat * particle.scale;
    
    // Transform vertex position
    let local_pos = scaled_rot * in.pos;
    let world_pos = local_pos + particle.position;
    
    // Transform normal and tangent
    let world_normal = normalize(rot_mat * in.normal);
    let world_tangent = normalize(rot_mat * in.tangent.xyz);
    let bitangent = cross(world_normal, world_tangent) * in.tangent.w;
    
    var out: VsOut;
    out.pos = globals.view_proj * vec4<f32>(world_pos, 1.0);
    out.world_pos = world_pos;
    out.normal = world_normal;
    out.uv = in.uv;
    out.tangent = world_tangent;
    out.bitangent = bitangent;
    
    return out;
}

// Simplified fragment shader (PBR lighting)
const PI: f32 = 3.14159265359;

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

fn calculate_light_contribution(
    N: vec3<f32>,
    V: vec3<f32>,
    L: vec3<f32>,
    base_color: vec3<f32>,
    metallic: f32,
    roughness: f32,
    light_color: vec3<f32>,
    light_intensity: f32
) -> vec3<f32> {
    let NdotL = max(dot(N, L), 0.0);
    if (NdotL <= 0.0) {
        return vec3<f32>(0.0);
    }
    
    let H = normalize(V + L);
    let F0 = mix(vec3<f32>(0.04), base_color, metallic);
    
    let NDF = distribution_ggx(N, H, roughness);
    let G = geometry_smith(N, V, L, roughness);
    let F = fresnel_schlick(max(dot(H, V), 0.0), F0);
    
    let numerator = NDF * G * F;
    let NdotV = max(dot(N, V), 0.0);
    let denominator = 4.0 * NdotV * NdotL + 0.0001;
    let specular = numerator / denominator;
    
    let kS = F;
    var kD = vec3<f32>(1.0) - kS;
    kD = kD * (1.0 - metallic);
    
    let diffuse = kD * base_color / PI;
    let radiance = light_color * light_intensity;
    return (diffuse + specular) * radiance * NdotL;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Sample base color texture if enabled
    var base_color = particle_material.color;
    
    if ((particle_material.material_flags & FLAG_USE_BASE_COLOR_TEXTURE) != 0u) {
        let tex_idx = particle_material.base_color_texture;
        let use_nearest = (particle_material.material_flags & FLAG_USE_NEAREST_SAMPLER) != 0u;
        
        // Sample with appropriate sampler
        var tex_color: vec4<f32>;
        if (use_nearest) {
            tex_color = textureSample(textures[tex_idx], tex_sampler_nearest, in.uv);
        } else {
            tex_color = textureSample(textures[tex_idx], tex_sampler_linear, in.uv);
        }
        
        base_color = base_color * tex_color;
    }
    
    let metallic = particle_material.metallic_factor;
    let roughness = max(particle_material.roughness_factor, 0.01);
    
    let N = normalize(in.normal);
    let V = normalize(globals.camera_pos - in.world_pos);
    
    var Lo = vec3<f32>(0.0);
    
    // Directional lights
    let dir_count = min(lights.counts.x, MAX_DIRECTIONAL_LIGHTS);
    for (var i = 0u; i < dir_count; i = i + 1u) {
        let light = lights.directionals[i];
        let light_dir = normalize(-light.direction.xyz);
        Lo += calculate_light_contribution(
            N, V, light_dir, base_color.rgb, metallic, roughness,
            light.color_intensity.xyz, light.color_intensity.w
        );
    }
    
    // Point lights
    let point_count = min(lights.counts.y, MAX_POINT_LIGHTS);
    for (var i = 0u; i < point_count; i = i + 1u) {
        let light = lights.points[i];
        let to_light = light.position_range.xyz - in.world_pos;
        let distance = length(to_light);
        if (distance > 0.0001) {
            let L = to_light / distance;
            var attenuation = 1.0 / max(distance * distance, 0.0001);
            let range = light.position_range.w;
            if (range > 0.0) {
                let range_factor = clamp(1.0 - distance / range, 0.0, 1.0);
                attenuation = attenuation * range_factor * range_factor;
            }
            Lo += calculate_light_contribution(
                N, V, L, base_color.rgb, metallic, roughness,
                light.color_intensity.xyz, light.color_intensity.w * attenuation
            );
        }
    }
    
    let ambient = vec3<f32>(0.03) * base_color.rgb;
    var color = ambient + Lo;
    
    // Tone mapping
    color = color / (color + vec3<f32>(1.0));
    
    return vec4<f32>(color, base_color.a);
}