// Ray-traced ambient occlusion: the hardware counterpart of fs_ssao (ssao.wgsl). It writes
// the same occlusion texture, which SSAO's blur passes smooth and the composite applies.
//
// Each pixel casts RT_AO_RAYS cosine-weighted rays over its hemisphere (from the G-buffer
// position and normal) and counts those that hit something within the SSAO radius, so
// occluders off screen or behind other surfaces count too. The pattern of rays is fixed
// per pixel (rotated by interleaved gradient noise), not per frame: the pass runs after
// TAA, so changing it every frame would flicker; the blur averages neighbouring patterns.
//
// Needs `enable wgpu_ray_query;` (prepended by the Rust side) and post-processing's
// common module.

@group(1) @binding(180)
var rt_ao_normal : texture_2d<f32>;
@group(1) @binding(181)
var rt_ao_position : texture_2d<f32>;

@group(2) @binding(0)
var rt_ao_tlas : acceleration_structure;

const RT_AO_RAYS : u32 = 8u;

fn rt_ao_blocked(origin : vec3<f32>, direction : vec3<f32>, distance : f32) -> bool {
    var query : ray_query;
    rayQueryInitialize(
        &query,
        rt_ao_tlas,
        RayDesc(
            RAY_FLAG_FORCE_OPAQUE | RAY_FLAG_TERMINATE_ON_FIRST_HIT,
            0xffu,
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
    }
    return rayQueryGetCommittedIntersection(&query).kind != RAY_QUERY_INTERSECTION_NONE;
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
    let n = decode_normal_oct(textureLoad(rt_ao_normal, pixel, 0).xy);
    let distance_to_camera = distance(post_uniform.camera_position.xyz, position.xyz);
    let origin = position.xyz + n * (0.005 + 0.001 * distance_to_camera);
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
