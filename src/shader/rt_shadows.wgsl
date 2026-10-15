// Ray-traced shadow mask: for every pixel of the depth prepass, whether each directional
// light (the sun) reaches the surface, one channel per light. Runs between the depth
// prepass and the main pass, which reads the mask instead of the shadow maps for the
// lights whose bit is set in lights.counts.w (shadows.wgsl, rt_directional_shadow).
//
// The surface is rebuilt from depth with the same (jittered) camera the prepass drew
// with; its geometric normal comes from neighbouring depths. A light with an apparent
// size casts soft shadows: each pixel and frame aims its ray at a different point of the
// light's disc, and TAA averages them.
//
// Needs `enable wgpu_ray_query;` (prepended by the Rust side).

struct Globals {
    view_proj : mat4x4<f32>,
    inverse_view_proj : mat4x4<f32>,
    camera_pos : vec3<f32>,
    _padding0 : f32,
    camera_forward : vec3<f32>,
    _padding1 : f32,
    camera_up : vec3<f32>,
    _padding2 : f32,
    unjittered_view_proj : mat4x4<f32>,
    prev_view_proj : mat4x4<f32>,
};
@group(0) @binding(0) var<uniform> globals : Globals;

// Mirrors `RtShadowUniform` in src/renderer/raytracing/shadows.rs.
struct RtShadowParams {
    // xyz: direction toward light i, w: 1 when its channel is traced.
    lights : array<vec4<f32>, 4>,
    // x: tan of the light's angular radius, y: frame index.
    params : vec4<f32>,
    // The region the prepass drew into, in pixels: x, y, width, height.
    region : vec4<f32>,
};
@group(1) @binding(0) var<uniform> rt_shadow : RtShadowParams;
@group(1) @binding(1) var rt_depth : texture_depth_2d;

@group(2) @binding(0) var rt_tlas : acceleration_structure;

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

fn rt_unoccluded(origin : vec3<f32>, direction : vec3<f32>) -> bool {
    var query : ray_query;
    rayQueryInitialize(
        &query,
        rt_tlas,
        RayDesc(
            RAY_FLAG_FORCE_OPAQUE | RAY_FLAG_TERMINATE_ON_FIRST_HIT,
            0xffu,
            0.0,
            1.0e4,
            origin,
            direction,
        ),
    );
    loop {
        if !rayQueryProceed(&query) {
            break;
        }
    }
    return rayQueryGetCommittedIntersection(&query).kind == RAY_QUERY_INTERSECTION_NONE;
}

// Interleaved gradient noise, shifted every frame so TAA averages the soft-shadow rays.
fn rt_noise(pixel : vec2<f32>, frame : f32, salt : f32) -> f32 {
    let p = pixel + vec2<f32>(5.588238, 3.412) * (frame + salt * 17.0);
    return fract(52.9829189 * fract(dot(p, vec2<f32>(0.06711056, 0.00583715))));
}

@fragment
fn fs_rt_shadows(in : VertexOutput) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(in.position.xy);
    let center = rt_surface_at(pixel);
    if center.w < 0.5 {
        return vec4<f32>(1.0);
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

    // A random point on the light's disc, the same for every light this pixel and frame.
    let angle = 6.2831853 * rt_noise(in.position.xy, rt_shadow.params.y, 0.0);
    let radius = sqrt(rt_noise(in.position.xy, rt_shadow.params.y, 1.0)) * rt_shadow.params.x;
    let disc = vec2<f32>(cos(angle), sin(angle)) * radius;

    var visibility = vec4<f32>(1.0);
    for (var i = 0; i < 4; i = i + 1) {
        let light = rt_shadow.lights[i];
        if light.w < 0.5 {
            continue;
        }
        let l = light.xyz;
        if dot(n, l) <= 0.0 {
            visibility[i] = 0.0;
            continue;
        }
        let side = normalize(cross(l, select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(l.y) > 0.9)));
        let up = cross(side, l);
        let direction = normalize(l + side * disc.x + up * disc.y);
        visibility[i] = select(0.0, 1.0, rt_unoccluded(origin, direction));
    }
    return visibility;
}
