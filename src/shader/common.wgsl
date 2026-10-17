// src/shader/common.wgsl
// Main PBR shader - now using modular lighting system

// ============================================================================
// Camera and Global Bindings
// ============================================================================

// The camera (`globals`, group 0) is camera.wgsl.

fn material_mip_bias() -> f32 {
    return globals.mip_bias;
}

// ============================================================================
// Material System
// ============================================================================

// `objects` and `materials` (group 1) are objects.wgsl.
// Last frame's model matrices, indexed like `objects` (motion vectors).
@group(1) @binding(2) var<storage, read> prev_models: array<mat4x4<f32>>;

// Last frame's model matrix for an instance; an all-zero entry means no history.
fn previous_model(instance: u32, current: mat4x4<f32>) -> mat4x4<f32> {
    let prev = prev_models[instance];
    if prev[3][3] == 0.0 {
        return current;
    }
    return prev;
}


// Octahedral normal encoding: a unit vector in two numbers, leaving room for velocity in
// the same G-buffer texel.
fn encode_normal_oct(n: vec3<f32>) -> vec2<f32> {
    let p = n.xy / max(abs(n.x) + abs(n.y) + abs(n.z), 1e-6);
    if n.z >= 0.0 {
        return p;
    }
    let signs = select(vec2<f32>(-1.0), vec2<f32>(1.0), p >= vec2<f32>(0.0));
    return (1.0 - abs(p.yx)) * signs;
}

// G-buffer normal texel: encoded normal in xy, motion vector in zw.
fn pack_normal_velocity(n: vec3<f32>, velocity: vec2<f32>) -> vec4<f32> {
    return vec4<f32>(encode_normal_oct(normalize(n)), velocity);
}

// Screen motion since last frame in UV units (+y down), jitter excluded.
fn motion_vector(curr_clip: vec4<f32>, prev_clip: vec4<f32>) -> vec2<f32> {
    if curr_clip.w <= 0.0 || prev_clip.w <= 0.0 {
        return vec2<f32>(0.0);
    }
    let delta = curr_clip.xy / curr_clip.w - prev_clip.xy / prev_clip.w;
    return delta * vec2<f32>(0.5, -0.5);
}


// Note: Lighting, shadows, and environment are imported from modules below
// Group 2 bindings:
//   @binding(0) - lights (from lighting_common.wgsl)
//   @binding(1-7) - shadows (from shadows.wgsl)
//   @binding(8-10) - environment (from environment.wgsl)

// ============================================================================
// Texture Bindings (Bindless or Traditional)
// ============================================================================

// Include one of these based on your feature flag:
// #include "bindings_bindless.wgsl"
// or
// #include "bindings_traditional.wgsl"


// ============================================================================
// Import Lighting Modules
// ============================================================================

// Include in this order:
// 1. lighting_common.wgsl - Provides lights binding, PBR functions, light contribution
// 2. shadows.wgsl - Provides shadow bindings and sampling
// 3. environment.wgsl - Provides environment bindings and lighting
// 4. lighting_with_shadows.wgsl - Provides complete scene lighting function

// In your build system, you'd concatenate these files or use a proper include mechanism
// For demonstration, I'm showing the structure with comments

// ============================================================================
// Vertex Shader
// ============================================================================

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) tangent: vec4<f32>,  // xyz = tangent, w = handedness
    @location(4) color: vec4<f32>,    // linear, multiplies the material colour
    // Last frame's position (a skinned instance's previous pose; `pos` otherwise).
    @location(5) prev_pos: vec3<f32>,
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
    // The material colour times the vertex colour, so it varies across a face.
    @location(6) material_color: vec4<f32>,
    @location(7) @interpolate(flat) material_texture_indices0: vec4<u32>,
    // The occlusion texture, and interior mapping's parameters (MaterialData.interior).
    @location(8) @interpolate(flat) material_texture_indices1: vec2<u32>,
    @location(9) @interpolate(flat) material_flags: u32,
    // metallic, roughness, emissive strength, transmission
    @location(10) @interpolate(flat) material_factors: vec4<f32>,
    @location(11) curr_clip: vec4<f32>,
    @location(12) prev_clip: vec4<f32>,
};

struct ShadedFragment {
    color: vec4<f32>,
    normal: vec4<f32>,
    world_pos: vec4<f32>,
    // Share of light the surface lets through (glass); 0 for opaque surfaces.
    transmission: f32,
};

// The colour target's alpha in the G-buffer pass: transmissive surfaces store
// -transmission, which marks them for the ray-traced refraction pass (opaque surfaces
// store their alpha, never negative). See rt_reflections.wgsl, fs_rt_refraction_trace.
fn gbuffer_color(shaded: ShadedFragment) -> vec4<f32> {
    return vec4<f32>(
        shaded.color.rgb,
        select(shaded.color.a, -shaded.transmission, shaded.transmission > 0.0),
    );
}

struct GBufferOut {
    @location(0) color: vec4<f32>,
    @location(1) normal: vec4<f32>,
    @location(2) world_pos: vec4<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    let obj = objects[in.instance];
    let M = obj.model;
    let world_pos = M * vec4(in.pos, 1.0);
    let material = materials[obj.material_index];

    // Transform normal and tangent to world space
    let n = normalize(normal_matrix(M) * in.normal);
    let t = normalize((M * vec4(in.tangent.xyz, 0.0)).xyz);
    
    // Calculate bitangent using the handedness from the tangent w component
    let b = cross(n, t) * in.tangent.w;

    var out: VsOut;
    out.pos = globals.view_proj * world_pos;
    out.curr_clip = globals.unjittered_view_proj * world_pos;
    out.prev_clip = globals.prev_view_proj
        * (previous_model(in.instance, M) * vec4(in.prev_pos, 1.0));
    out.world_pos = world_pos.xyz;
    out.normal = n;
    out.uv = in.uv;
    out.instance_id = in.instance;
    out.tangent = t;
    out.bitangent = b;
    out.material_color = material.color * in.color;
    out.material_texture_indices0 = vec4<u32>(
        material.base_color_texture,
        material.metallic_roughness_texture,
        material.normal_texture,
        material.emissive_texture,
    );
    out.material_texture_indices1 = vec2<u32>(material.occlusion_texture, material.interior);
    out.material_flags = material.material_flags;
    out.material_factors = vec4<f32>(
        material.metallic_factor,
        material.roughness_factor,
        material.emissive_strength,
        material.transmission,
    );
    return out;
}

// ============================================================================
// Fragment Shader
// ============================================================================

// A cut-out surface (glTF MASK) is discarded where `alpha` is below its cutoff and opaque
// elsewhere; other surfaces keep their alpha.
fn alpha_tested(alpha: f32, material_flags: u32) -> f32 {
    let cutoff = material_alpha_cutoff(material_flags);
    if cutoff <= 0.0 || (material_flags & FLAG_ALPHA_BLEND) != 0u {
        return alpha;
    }
    if alpha < cutoff {
        discard;
    }
    return 1.0;
}

// `front_facing` is false only on the back faces of double-sided materials (the others
// are culled); they are lit from the side the camera sees.
fn shade_fragment(in: VsOut, front_facing: bool) -> ShadedFragment {
    // ALWAYS sample all textures (uniform control flow)
    let material_flags = in.material_flags;
    let base_color_sample = sample_base_color_texture(
        in.material_texture_indices0.x, in.uv
    );
    let mr_sample = sample_metallic_roughness_texture(
        in.material_texture_indices0.y, in.uv
    );
    let normal_sample = sample_normal_texture(
        in.material_texture_indices0.z, in.uv
    );
    let emissive_sample = sample_emissive_texture(
        in.material_texture_indices0.w, in.uv
    );
    let occlusion_sample = sample_occlusion_texture(
        in.material_texture_indices1.x, in.uv
    );
    // Interior mapping's uv frame and texture footprint, also only in uniform control
    // flow, so every fragment takes them.
    let dpos_dx = dpdx(in.world_pos);
    let dpos_dy = dpdy(in.world_pos);
    let duv_dx = dpdx(in.uv);
    let duv_dy = dpdy(in.uv);

    // Then conditionally USE the samples
    var base_color: vec4<f32>;
    if ((material_flags & FLAG_USE_BASE_COLOR_TEXTURE) != 0u) {
        base_color = base_color_sample * in.material_color;
    } else {
        base_color = in.material_color;
    }

    var metallic: f32;
    var roughness: f32;
    if ((material_flags & FLAG_USE_METALLIC_ROUGHNESS_TEXTURE) != 0u) {
        metallic = mr_sample.b * in.material_factors.x;
        roughness = mr_sample.g * in.material_factors.y;
    } else {
        metallic = in.material_factors.x;
        roughness = in.material_factors.y;
    }
    roughness = max(roughness, 0.01);

    var N: vec3<f32>;
    if ((material_flags & FLAG_USE_NORMAL_TEXTURE) != 0u) {
        let tangent_normal = normal_sample * 2.0 - 1.0;
        let T = normalize(in.tangent);
        let B = normalize(in.bitangent);
        let N_base = normalize(in.normal);
        let TBN = mat3x3<f32>(T, B, N_base);
        N = normalize(TBN * tangent_normal);
    } else {
        N = normalize(in.normal);
    }
    var N_geometric = normalize(in.normal);
    if (!front_facing) {
        N = -N;
        N_geometric = -N_geometric;
    }

    var occlusion = 1.0;
    if ((material_flags & FLAG_USE_OCCLUSION_TEXTURE) != 0u) {
        occlusion = occlusion_sample;
    }

    var emissive = vec3<f32>(0.0);
    if ((material_flags & FLAG_USE_EMISSIVE_TEXTURE) != 0u) {
        emissive = emissive_sample * in.material_factors.z;
    } else {
        // Without an emissive map the surface glows in its own color. The strength is
        // not limited to 1, so bright values can push it past the bloom threshold.
        emissive = base_color.rgb * in.material_factors.z;
    }

    // Calculate lighting using imported functions
    let V = normalize(globals.camera_pos - in.world_pos);
    
    // Light a transmissive surface lets through is not scattered back diffusely: its
    // diffuse share goes (the refraction pass adds what shows through instead).
    let transmission = clamp(in.material_factors.w, 0.0, 1.0);
    var diffuse_share = 1.0 - transmission;
    let interior = (material_flags & FLAG_INTERIOR) != 0u;
    if (interior) {
        // An interior-mapped window: the frame (base colour texture alpha) is lit as
        // usual; the glass reflects, and lets the room's light through, tinted by the
        // base colour (interior.wgsl).
        let frame = select(0.0, base_color_sample.a, (material_flags & FLAG_USE_BASE_COLOR_TEXTURE) != 0u);
        let room = interior_window(
            in, N, N_geometric, V, interior_uv_frame(dpos_dx, dpos_dy, duv_dx, duv_dy), duv_dx, duv_dy
        );
        let glass = (1.0 - frame) * (1.0 - fresnel_schlick(max(dot(N, V), 0.0), vec3<f32>(0.04)));
        emissive = room * in.material_color.rgb * glass;
        diffuse_share = frame;
    }

    // This function is from lighting_with_shadows.wgsl. Opaque surfaces are the ones the
    // ray-traced shadow mask was traced for (from the depth prepass).
    let opaque = (material_flags & FLAG_ALPHA_BLEND) == 0u;
    let lit_color = base_color.rgb * diffuse_share;
    let Lo = calculate_scene_lighting_at(
        in.world_pos, N, V, lit_color, metallic, roughness,
        select(vec2<f32>(-1.0), in.pos.xy, opaque)
    );
    
    // This function is from environment.wgsl. While ray-traced global illumination lights
    // opaque surfaces, their diffuse share of the environment comes from it instead
    // (billboards turn to the camera and are not in the acceleration structure; glass is
    // the refraction pass's).
    var environment_light: vec3<f32>;
    let traced = opaque && transmission <= 0.0 && (material_flags & FLAG_BILLBOARDED) == 0u;
    if (traced && environment_diffuse_traced()) {
        environment_light = calculate_environment_specular(
            N, V, lit_color, metallic, roughness, occlusion
        );
    } else {
        environment_light = calculate_environment_lighting(
            N, V, lit_color, metallic, roughness, occlusion
        );
        // Inside a light probe grid (light_probes.wgsl) the probes bring it instead: the
        // sky and flat ambient only where they get in, plus the light bounced off the
        // scene.
        let probed = light_probe_environment_lighting(
            in.world_pos, N, V, lit_color, metallic, roughness, occlusion
        );
        environment_light = mix(environment_light, probed.rgb, probed.w);
    }
    
    // Conditionally use lighting based on material flags
    var color: vec3<f32>;
    if ((material_flags & FLAG_UNLIT) != 0u) {
        color = base_color.rgb + emissive;
    } else {
        color = environment_light + Lo + emissive;
    }
    
    // Linear HDR: post-processing applies exposure and tonemapping.

    // Reflections need the surface's smoothness and specular colour (none when unlit).
    let lit = (material_flags & FLAG_UNLIT) == 0u;
    let smoothness = select(0.0, clamp(1.0 - roughness, 0.0, 1.0), lit);
    let specular_color = mix(vec3<f32>(0.04), base_color.rgb, metallic);
    // Cut-out surfaces end here, after every texture is sampled (uniform control flow).
    // Interior-mapped windows are opaque: their alpha marks the frame.
    let alpha = alpha_tested(select(base_color.a, 1.0, interior), material_flags);
    // Drawn see-through (the transparent pass, without ray-traced refraction), a
    // transmissive surface shows what is behind it in proportion, keeping a little opacity
    // so its highlights and edges still show.
    return ShadedFragment(
        vec4<f32>(color, alpha * (1.0 - 0.85 * transmission)),
        vec4<f32>(normalize(N), 1.0),
        vec4<f32>(in.world_pos, encode_surface_w(smoothness, specular_color)),
        transmission,
    );
}

// What an interior-mapped window shows of its room (interior.wgsl), before the glass's
// tint and reflection. `n` is the shading normal (a normal map ripples old glass and
// bends the view through it), `n_geometric` the surface's.
fn interior_window(
    in: VsOut,
    n: vec3<f32>,
    n_geometric: vec3<f32>,
    v: vec3<f32>,
    frame: InteriorFrame,
    duv_dx: vec2<f32>,
    duv_dy: vec2<f32>,
) -> vec3<f32> {
    let params = in.material_texture_indices1.y;
    let seed = interior_object_seed(objects[in.instance_id].model[3].xyz);
    let ray = -v - (n - n_geometric) * 0.3;
    let hit = interior_trace(in.uv, ray, frame, n_geometric, params, seed);
    var room = interior_plain_room(hit);
    if ((in.material_flags & FLAG_USE_EMISSIVE_TEXTURE) != 0u) {
        room = sample_emissive_texture_grad(
            in.material_texture_indices0.w,
            hit.atlas_uv,
            duv_dx * hit.atlas_scale,
            duv_dy * hit.atlas_scale,
        );
    }
    return interior_radiance(hit, room, interior_daylight(in.world_pos, n_geometric, v), in.material_factors.z);
}

// The daylight on an interior-mapped window that lights its room: the sky's and the flat
// ambient's (the light probes' inside a volume), a share of it.
fn interior_daylight(world_pos: vec3<f32>, n: vec3<f32>, v: vec3<f32>) -> vec3<f32> {
    let hdr = environment_hdr_enabled();
    var light = select(vec3<f32>(1.0), environment_settings.ambient_color.rgb, hdr)
        * environment_ambient_intensity();
    if (hdr) {
        light += sample_environment_hdr(n, environment_settings.flags_intensity.w)
            * environment_hdr_intensity();
    }
    let probed = light_probe_irradiance_at(world_pos, n, v);
    return mix(light, probed.rgb, probed.w) * INTERIOR_DAYLIGHT;
}

// The G-buffer position's w (a half float): below 0.5 there is no surface. A surface
// stores a 14-bit code in the bits of a half float of at least 1 (1.0 itself is code 0:
// rough, dielectric): the top 4 bits pick the power of two, the low 10 the mantissa, so
// the value is exact in the half-float target. The code holds the smoothness reflections
// use (5 bits, highest) over the specular colour F0 (3 bits a channel, 0 = the dielectric
// 0.04, square-root spaced up to 1). Decoded by ssr_smoothness / ssr_specular_color in
// postprocess/reflection_common.wgsl.
fn encode_surface_w(smoothness: f32, specular_color: vec3<f32>) -> f32 {
    let s = u32(round(clamp(smoothness, 0.0, 1.0) * 31.0));
    let above_dielectric = clamp((specular_color - vec3<f32>(0.04)) / 0.96, vec3<f32>(0.0), vec3<f32>(1.0));
    let f0 = vec3<u32>(round(sqrt(above_dielectric) * 7.0));
    let code = (s << 9u) | (f0.r << 6u) | (f0.g << 3u) | f0.b;
    return exp2(f32(code >> 10u)) * (1.0 + f32(code & 1023u) / 1024.0);
}

@fragment
fn fs_main(in: VsOut, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    return shade_fragment(in, front_facing).color;
}

// The transparent pass: drawn over the fogged opaque scene, a surface takes the fog
// between it and the camera itself (fog.wgsl).
@fragment
fn fs_transparent(in: VsOut, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    let color = shade_fragment(in, front_facing).color;
    return vec4<f32>(fog_apply(color.rgb, in.world_pos), color.a);
}

@fragment
fn fs_main_gbuffer(in: VsOut, @builtin(front_facing) front_facing: bool) -> GBufferOut {
    let shaded = shade_fragment(in, front_facing);
    return GBufferOut(
        gbuffer_color(shaded),
        pack_normal_velocity(shaded.normal.xyz, motion_vector(in.curr_clip, in.prev_clip)),
        shaded.world_pos,
    );
}

// The pick pass: the entity's pick id where the surface is visible to a click. Fully
// transparent parts of blended surfaces let clicks through to what is behind.
@fragment
fn fs_pick(in: VsOut) -> @location(0) vec2<u32> {
    let base_color_sample = sample_base_color_texture(in.material_texture_indices0.x, in.uv);
    var alpha = in.material_color.a;
    if (in.material_flags & FLAG_USE_BASE_COLOR_TEXTURE) != 0u {
        alpha = alpha * base_color_sample.a;
    }
    if (in.material_flags & FLAG_ALPHA_BLEND) != 0u && alpha <= 0.0 {
        discard;
    }
    alpha_tested(alpha, in.material_flags);
    let obj = objects[in.instance_id];
    return vec2<u32>(obj.pick_id[0u], obj.pick_id[1u]);
}
