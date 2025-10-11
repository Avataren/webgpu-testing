// src/shader/pbr_lighting.wgsl
// Shared PBR lighting functions used by both main renderer and GPU particles

// ============================================================================
// Constants
// ============================================================================

const PI: f32 = 3.14159265359;

// ============================================================================
// PBR Helper Functions
// ============================================================================

// Normal Distribution Function (GGX/Trowbridge-Reitz)
fn distribution_ggx(n_dot_h: f32, roughness: f32) -> f32 {
    let a = roughness * roughness;
    let a2 = a * a;
    let denom = n_dot_h * n_dot_h * (a2 - 1.0) + 1.0;
    return a2 / (PI * denom * denom);
}

// Geometry Function (Smith's Schlick-GGX)
fn geometry_schlick_ggx(n_dot_v: f32, roughness: f32) -> f32 {
    let r = roughness + 1.0;
    let k = (r * r) / 8.0;
    return n_dot_v / (n_dot_v * (1.0 - k) + k);
}

fn geometry_smith(n_dot_v: f32, n_dot_l: f32, roughness: f32) -> f32 {
    let ggx2 = geometry_schlick_ggx(n_dot_v, roughness);
    let ggx1 = geometry_schlick_ggx(n_dot_l, roughness);
    return ggx1 * ggx2;
}

// Fresnel Function (Schlick approximation)
fn fresnel_schlick(cos_theta: f32, f0: vec3<f32>) -> vec3<f32> {
    return f0 + (1.0 - f0) * pow(clamp(1.0 - cos_theta, 0.0, 1.0), 5.0);
}

// ============================================================================
// PBR Lighting Evaluation
// ============================================================================

struct PbrSurface {
    position: vec3<f32>,
    normal: vec3<f32>,
    view_dir: vec3<f32>,
    albedo: vec3<f32>,
    metallic: f32,
    roughness: f32,
    ao: f32,
}

// Calculate direct lighting contribution from a single light
fn calculate_direct_lighting(
    surface: PbrSurface,
    light_dir: vec3<f32>,
    light_color: vec3<f32>,
    light_intensity: f32,
) -> vec3<f32> {
    let h = normalize(surface.view_dir + light_dir);
    
    let n_dot_v = max(dot(surface.normal, surface.view_dir), 0.0);
    let n_dot_l = max(dot(surface.normal, light_dir), 0.0);
    let n_dot_h = max(dot(surface.normal, h), 0.0);
    let h_dot_v = max(dot(h, surface.view_dir), 0.0);
    
    // Calculate F0 (surface reflection at zero incidence)
    var f0 = vec3<f32>(0.04);
    f0 = mix(f0, surface.albedo, surface.metallic);
    
    // Cook-Torrance BRDF
    let ndf = distribution_ggx(n_dot_h, surface.roughness);
    let g = geometry_smith(n_dot_v, n_dot_l, surface.roughness);
    let f = fresnel_schlick(h_dot_v, f0);
    
    let numerator = ndf * g * f;
    let denominator = 4.0 * n_dot_v * n_dot_l + 0.0001;
    let specular = numerator / denominator;
    
    // Energy conservation
    let k_d = (vec3<f32>(1.0) - f) * (1.0 - surface.metallic);
    
    let diffuse = k_d * surface.albedo / PI;
    
    return (diffuse + specular) * light_color * light_intensity * n_dot_l;
}

// Calculate ambient lighting using simple ambient term
fn calculate_ambient_lighting(surface: PbrSurface, ambient_color: vec3<f32>, ambient_intensity: f32) -> vec3<f32> {
    return ambient_color * surface.albedo * ambient_intensity * surface.ao;
}