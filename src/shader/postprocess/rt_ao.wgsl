// Ray-traced ambient occlusion: the hardware counterpart of fs_ssao (ssao.wgsl). It writes
// the same occlusion texture, which SSAO's blur passes smooth and the composite applies.
//
// Each pixel casts RT_AO_RAYS cosine-weighted rays over its hemisphere (from the surface
// the depth buffer places, around the G-buffer normal) and counts those that hit something
// within the SSAO radius, so occluders off screen or behind other surfaces count too. The
// pattern of rays is fixed per pixel (rotated by interleaved gradient noise), not per
// frame: the pass runs after TAA, so changing it every frame would flicker; the blur
// averages neighbouring patterns.
//
// Needs `enable wgpu_ray_query;`, post-processing's common module, the material module
// and the scene modules (raytracing::scene_shader_modules), composed by rt_ao.rs.

@group(1) @binding(180)
var rt_ao_normal : texture_2d<f32>;
@group(1) @binding(181)
var rt_ao_position : texture_2d<f32>;
// The scene's depth (single-sample), the same size as the G-buffer.
@group(1) @binding(182)
var rt_ao_depth : texture_depth_2d;

// The ray tracing scene is group 2 (rt_scene.wgsl), the material textures group 3.

const RT_AO_RAYS : u32 = 8u;

fn rt_ao_blocked(origin : vec3<f32>, direction : vec3<f32>, distance : f32) -> bool {
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
    return rayQueryGetCommittedIntersection(&query).kind != RAY_QUERY_INTERSECTION_NONE;
}

// The surface at G-buffer `pixel` at full precision: rebuilt from the depth buffer through
// the jittered camera the G-buffer was drawn with. The G-buffer's half-float position is
// rounded to steps of 4-8 mm between 4 and 16 m from the origin, enough to put it behind
// the surface, where every ray hits the surface itself (stripes of full occlusion across
// characters away from the origin). Where depth shows another surface than the G-buffer
// (further than that rounding allows), the G-buffer's `rounded` position stands.
fn rt_ao_surface(pixel : vec2<i32>, dims : vec2<i32>, rounded : vec3<f32>) -> vec3<f32> {
    let depth = textureLoad(rt_ao_depth, pixel, 0);
    if depth >= 1.0 {
        return rounded;
    }
    let viewport_uv = scene_to_viewport_uv((vec2<f32>(pixel) + 0.5) / vec2<f32>(dims));
    let ndc = vec2<f32>(viewport_uv.x * 2.0 - 1.0, 1.0 - viewport_uv.y * 2.0);
    let world = post_uniform.view_proj_inv * vec4<f32>(ndc, depth, 1.0);
    let position = world.xyz / world.w;
    let largest = max(abs(rounded.x), max(abs(rounded.y), abs(rounded.z)));
    if distance(position, rounded) > 0.01 + 2e-3 * largest {
        return rounded;
    }
    return position;
}

@fragment
fn fs_rt_ao(in : VertexOutput) -> @location(0) vec4<f32> {
    if post_uniform.effects.x < 0.5 {
        return vec4<f32>(1.0);
    }
    let dims = vec2<i32>(textureDimensions(rt_ao_position));
    let pixel = min(vec2<i32>(in.position.xy), dims - vec2<i32>(1));
    let position = textureLoad(rt_ao_position, pixel, 0);
    if position.w < 0.5 {
        return vec4<f32>(1.0);
    }
    let surface = rt_ao_surface(pixel, dims, position.xyz);
    let n = decode_normal_oct(textureLoad(rt_ao_normal, pixel, 0).xy);
    let distance_to_camera = distance(post_uniform.camera_position.xyz, surface);
    let origin = surface + n * (0.005 + 0.001 * distance_to_camera);
    let radius = max(post_uniform.radius_bias.x, 0.001);

    let helper = select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(n.y) > 0.9);
    let tangent = normalize(cross(n, helper));
    let bitangent = cross(n, tangent);
    // Interleaved gradient noise: neighbouring pixels rotate their rays differently, so
    // the blur averages more directions.
    let rotation = fract(52.9829189 * fract(dot(in.position.xy, vec2<f32>(0.06711056, 0.00583715))));

    var blocked = 0.0;
    for (var i = 0u; i < RT_AO_RAYS; i = i + 1u) {
        // Stratified over the disc, then lifted onto the hemisphere (cosine-weighted).
        let u = (f32(i) + 0.5) / f32(RT_AO_RAYS);
        let angle = 6.2831853 * fract(f32(i) * 0.618034 + rotation);
        let r = sqrt(u);
        let direction = tangent * (r * cos(angle))
            + bitangent * (r * sin(angle))
            + n * sqrt(max(1.0 - u, 0.0));
        if rt_ao_blocked(origin, direction, radius) {
            blocked = blocked + 1.0;
        }
    }

    // As fs_ssao: occlusion shaped by the power, blended in by the intensity.
    let ao = 1.0 - blocked / f32(RT_AO_RAYS);
    let ao_pow = pow(ao, max(post_uniform.intensity_power.y, 0.01));
    let strength = clamp(post_uniform.intensity_power.x, 0.0, 1.0);
    let result = mix(1.0, ao_pow, strength);
    return vec4<f32>(result, result, result, 1.0);
}
