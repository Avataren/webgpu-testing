// Ray-traced shadow mask: for every pixel of the depth prepass, whether each shadowed
// light reaches the surface, one bit per light (bits 0-3 directional lights, 4-31 the
// point and spot lights given a bit, LocalLight.params.z). Runs between the depth
// prepass and the main pass, which reads the mask instead of the shadow maps for the
// lights whose bit is set in lights.counts.w (shadows.wgsl, rt_shadow_bit). A pixel
// traces only the lights that reach it.
//
// The surface is rebuilt from depth with the same (jittered) camera the prepass drew
// with; its geometric normal comes from neighbouring depths. Lights with a size cast soft
// shadows: each pixel and frame aims its ray at a different point of the light (a disc
// across the sun, a sphere around a point or spot light), and TAA averages the results.
//
// Needs `enable wgpu_ray_query;`, the material module and the scene modules
// (raytracing::scene_shader_modules), composed by raytracing/shadows.rs.

// The camera (`globals`, group 0) comes from camera.wgsl.

// A point or spot light with mask bit 4 + its index: position and range, and a spot
// light's axis and the cosine of its outer angle (below -1.5: a point light; range 0:
// not traced).
struct RtShadowLight {
    position_range : vec4<f32>,
    direction : vec4<f32>,
};

// Mirrors `RtShadowUniform` in src/renderer/raytracing/shadows.rs. w = 1 marks a light
// that is traced.
struct RtShadowParams {
    // xyz: direction toward directional light i.
    directionals : array<vec4<f32>, 4>,
    locals : array<RtShadowLight, RT_SHADOW_LOCAL_LIGHTS>,
    // x: tan of the directional lights' angular radius, y: frame index, z: point and spot
    // light radius in meters.
    params : vec4<f32>,
    // The region the prepass drew into, in pixels: x, y, width, height.
    region : vec4<f32>,
};
@group(1) @binding(0) var<uniform> rt_shadow : RtShadowParams;
@group(1) @binding(1) var rt_depth : texture_depth_2d;

// Point and spot lights the mask has bits for (RT_SHADOW_LOCAL_LIGHTS in shadows.rs).
const RT_SHADOW_LOCAL_LIGHTS : u32 = 28u;

// The ray tracing scene is group 2 (rt_scene.wgsl), the material textures group 3.

struct VertexOutput {
    @builtin(position) position : vec4<f32>,
};

@vertex
fn vs_fullscreen(@builtin(vertex_index) index : u32) -> VertexOutput {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var out : VertexOutput;
    out.position = vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
    return out;
}

// World position of the surface at `pixel` (w = 0 where nothing was drawn).
fn rt_surface_at(pixel : vec2<i32>) -> vec4<f32> {
    let dims = vec2<i32>(textureDimensions(rt_depth));
    let p = clamp(pixel, vec2<i32>(0), dims - vec2<i32>(1));
    let depth = textureLoad(rt_depth, p, 0);
    if depth >= 1.0 {
        return vec4<f32>(0.0);
    }
    let uv = (vec2<f32>(p) + 0.5 - rt_shadow.region.xy) / rt_shadow.region.zw;
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let world = globals.inverse_view_proj * vec4<f32>(ndc, depth, 1.0);
    return vec4<f32>(world.xyz / world.w, 1.0);
}

// The neighbour step along one axis that stays on the same surface: the side whose depth
// continues the centre's (at a silhouette, the other side belongs to something else).
fn rt_surface_step(center : vec3<f32>, before : vec4<f32>, after : vec4<f32>) -> vec3<f32> {
    let forward = after.xyz - center;
    let backward = center - before.xyz;
    if after.w < 0.5 {
        return backward;
    }
    if before.w < 0.5 {
        return forward;
    }
    return select(backward, forward, dot(forward, forward) < dot(backward, backward));
}

fn rt_unoccluded(origin : vec3<f32>, direction : vec3<f32>, distance : f32) -> bool {
    var query : ray_query;
    rayQueryInitialize(
        &query,
        rt_tlas,
        RayDesc(
            RAY_FLAG_TERMINATE_ON_FIRST_HIT,
            // Opaque geometry only (MASK_OPAQUE in raytracing/scene.rs): light passes glass.
            0x01u,
            0.0,
            distance,
            origin,
            direction,
        ),
    );
    loop {
        if !rayQueryProceed(&query) {
            break;
        }
        // Cut-out surfaces come up as candidates (rt_alpha.wgsl).
        if rt_candidate_blocks(rayQueryGetCandidateIntersection(&query)) {
            rayQueryConfirmIntersection(&query);
        }
    }
    return rayQueryGetCommittedIntersection(&query).kind == RAY_QUERY_INTERSECTION_NONE;
}

// Interleaved gradient noise, shifted every frame so TAA averages the soft-shadow rays.
fn rt_noise(pixel : vec2<f32>, frame : f32, salt : f32) -> f32 {
    let p = pixel + vec2<f32>(5.588238, 3.412) * (frame + salt * 17.0);
    return fract(52.9829189 * fract(dot(p, vec2<f32>(0.06711056, 0.00583715))));
}

// Two directions across `axis`, for offsetting a ray within a light's extent.
fn rt_basis(axis : vec3<f32>) -> mat2x3<f32> {
    let helper = select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(axis.y) > 0.9);
    let side = normalize(cross(axis, helper));
    return mat2x3<f32>(side, cross(side, axis));
}

// Whether a light at `light_position` (radius `radius`) reaches `origin`, whose surface
// faces `n`; `disc` picks the point on the light (unit disc).
fn rt_local_light_visible(
    origin : vec3<f32>,
    n : vec3<f32>,
    light_position : vec3<f32>,
    radius : f32,
    disc : vec2<f32>,
) -> bool {
    let to_light = light_position - origin;
    let distance = length(to_light);
    if distance < 1e-4 || dot(n, to_light) <= 0.0 {
        return false;
    }
    let basis = rt_basis(to_light / distance);
    let aim = light_position + (basis[0] * disc.x + basis[1] * disc.y) * radius;
    let to_aim = aim - origin;
    let length_to_aim = length(to_aim);
    // Stop short of the light, which may sit inside geometry it lights (a lamp shade).
    return rt_unoccluded(origin, to_aim / length_to_aim, max(length_to_aim - radius - 0.01, 0.0));
}

@fragment
fn fs_rt_shadows(in : VertexOutput) -> @location(0) u32 {
    let pixel = vec2<i32>(in.position.xy);
    let center = rt_surface_at(pixel);
    if center.w < 0.5 {
        return 0xffffffffu;
    }
    let dx = rt_surface_step(
        center.xyz,
        rt_surface_at(pixel - vec2<i32>(1, 0)),
        rt_surface_at(pixel + vec2<i32>(1, 0)),
    );
    let dy = rt_surface_step(
        center.xyz,
        rt_surface_at(pixel - vec2<i32>(0, 1)),
        rt_surface_at(pixel + vec2<i32>(0, 1)),
    );
    let to_camera = globals.camera_pos - center.xyz;
    var n = cross(dx, dy);
    let length_sq = dot(n, n);
    n = select(normalize(to_camera), n * inverseSqrt(length_sq), length_sq > 1e-20);
    if dot(n, to_camera) < 0.0 {
        n = -n;
    }
    // Off the surface by more further away, where depth (and so the position) is coarser.
    let origin = center.xyz + n * (0.005 + 0.0015 * length(to_camera));

    // A random point on a light's disc, the same for every light this pixel and frame.
    let angle = 6.2831853 * rt_noise(in.position.xy, rt_shadow.params.y, 0.0);
    let disc = vec2<f32>(cos(angle), sin(angle)) * sqrt(rt_noise(in.position.xy, rt_shadow.params.y, 1.0));

    var lit = 0xffffffffu;
    for (var i = 0u; i < 4u; i = i + 1u) {
        let light = rt_shadow.directionals[i];
        if light.w > 0.5 {
            let l = light.xyz;
            var visible = false;
            if dot(n, l) > 0.0 {
                let basis = rt_basis(l);
                let offset = (basis[0] * disc.x + basis[1] * disc.y) * rt_shadow.params.x;
                visible = rt_unoccluded(origin, normalize(l + offset), 1.0e4);
            }
            if !visible {
                lit &= ~(1u << i);
            }
        }
    }
    // A point or spot light that does not reach the surface keeps its bit: the main pass
    // gives it no light there anyway.
    for (var i = 0u; i < RT_SHADOW_LOCAL_LIGHTS; i = i + 1u) {
        let light = rt_shadow.locals[i];
        let range = light.position_range.w;
        let to_surface = center.xyz - light.position_range.xyz;
        if range <= 0.0 || dot(to_surface, to_surface) > range * range {
            continue;
        }
        if light.direction.w > -1.5
            && dot(normalize(light.direction.xyz), normalize(to_surface)) < light.direction.w {
            continue;
        }
        if !rt_local_light_visible(origin, n, light.position_range.xyz, rt_shadow.params.z, disc) {
            lit &= ~(1u << (4u + i));
        }
    }
    return lit;
}
