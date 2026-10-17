// src/shader/fog_volume.wgsl
// The froxel volume's light (src/renderer/fog.rs): one invocation per froxel finds the
// fog's density at a point inside it and the light the fog scatters toward the camera
// there: the directional lights through their cascaded shadow maps, the point and spot
// lights of the point's cluster through their shadow maps, and the ambient light (the
// light probes' inside a probe volume, else the environment's), all scattered by the
// albedo, plus the fog's glow. The point moves inside its froxel from frame to frame and
// each froxel keeps most of what it held last frame where it was (reprojected), so shadow
// edges and small lights come out smooth rather than in steps.
//
// Needs fog_common.wgsl, fog.wgsl, lighting_common.wgsl, shadows.wgsl, environment.wgsl
// and light_probes.wgsl (the lights group).

@group(0) @binding(0) var fog_history: texture_3d<f32>;
@group(0) @binding(1) var fog_scatter_out: texture_storage_3d<rgba16float, write>;
@group(0) @binding(2) var fog_history_sampler: sampler;

// Near a point or spot light its inverse-square falloff grows without bound, but a froxel
// spans a volume: the falloff is held at this distance's (m).
const FOG_LIGHT_MIN_DISTANCE: f32 = 0.2;

fn fog_pcg3d(seed: vec3<u32>) -> vec3<u32> {
    var v = seed * 1664525u + vec3<u32>(1013904223u);
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    v = v ^ (v >> vec3<u32>(16u));
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    return v;
}

// Where in froxel `id` this frame's point lies (0-1 on each axis): the R3 sequence over
// frames, started at a different place for every froxel.
fn fog_jitter(id: vec3<u32>) -> vec3<f32> {
    let start = vec3<f32>(fog_pcg3d(id) >> vec3<u32>(8u)) / 16777216.0;
    let step = vec3<f32>(0.8191725134, 0.6710436067, 0.5497004779);
    return fract(start + step * fog.flags.z);
}

// How much of directional light `index` reaches world point `p` past its cascades: the
// nearest cascade whose map holds the point, one filtered compare.
fn fog_directional_shadow(index: u32, p: vec3<f32>) -> f32 {
    let params = shadow_info.directionals[index].params;
    let count = min(u32(params.x), MAX_SHADOW_CASCADES);
    if (count == 0u) {
        return 1.0;
    }
    let view_depth = shadow_info.directionals[index].view_depth;
    let fade = shadow_info.directionals[index].fade;
    let faded = clamp((dot(view_depth.xyz, p) + view_depth.w - fade.x) * fade.y, 0.0, 1.0);
    if (faded >= 1.0) {
        return 1.0;
    }
    let point = (shadow_info.directionals[index].light_view * vec4<f32>(p, 1.0)).xyz;
    let margin = 1.0 / max(params.z, 1.0);
    var cascade = count;
    for (var c = 0u; c < count; c = c + 1u) {
        if (cascade_edge(index, c, point, margin) > 0.0) {
            cascade = c;
            break;
        }
    }
    if (cascade == count) {
        return 1.0;
    }
    let info = shadow_info.directionals[index].cascades[cascade];
    let depth = point.z * info.depth_transform.x + info.depth_transform.y;
    if (depth <= 0.0) {
        return 1.0;
    }
    let uv = point.xy * info.uv_transform.xy + info.uv_transform.zw;
    let lit = textureSampleCompareLevel(
        directional_shadow_maps, shadow_sampler, uv, i32(params.y) + i32(cascade), min(depth, 1.0)
    );
    return mix(lit, 1.0, faded);
}

// How much of a point or spot light reaches world point `p` past its shadow map: one
// filtered compare in the face's tile (1 for a light without one).
fn fog_local_shadow(light: LocalLight, p: vec3<f32>) -> f32 {
    let slot = i32(light.params.y);
    if (slot < 0) {
        return 1.0;
    }
    let info = shadow_info.locals[slot];
    let to_point = p - light.position_range.xyz;
    let distance = length(to_point);
    if (distance <= 0.0001 || distance > light.position_range.w) {
        return 1.0;
    }
    var face = 0u;
    if (info.params.x > 1.5) {
        face = select_point_face(to_point / distance);
    }
    let proj = project_shadow(info.view_proj[face], p);
    if (!shadow_projection_valid(proj)) {
        return 1.0;
    }
    let rect = info.rects[face];
    let texel = rect.zw / max(info.params.y, 1.0);
    let uv = clamp(rect.xy + proj.xy * rect.zw, rect.xy + texel * 0.5, rect.xy + rect.zw - texel * 0.5);
    return textureSampleCompareLevel(local_shadow_atlas, shadow_sampler, uv, proj.z);
}

// The ambient light at world point `p`, alike in every direction: the light probes' (the
// mean of their light from above and below) where a probe volume covers the point, the
// environment's elsewhere.
fn fog_ambient_at(p: vec3<f32>) -> vec3<f32> {
    let ambient = fog_environment_ambient();
    let probed = light_probe_light_at(
        p, vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(0.0), vec3<f32>(0.0, -1.0, 0.0)
    );
    if (probed.coverage <= 0.0) {
        return ambient;
    }
    return mix(ambient, (probed.diffuse + probed.other) * 0.5, probed.coverage);
}

// The light arriving at world point `p` that the fog scatters toward the eye along
// `view_dir` (before the albedo).
fn fog_froxel_light(p: vec3<f32>, view_dir: vec3<f32>) -> vec3<f32> {
    var light = vec3<f32>(0.0);
    let count = min(lights.counts.x, MAX_DIRECTIONAL_LIGHTS);
    for (var i = 0u; i < count; i = i + 1u) {
        let directional = lights.directionals[i];
        let radiance = directional.color_intensity.xyz * directional.color_intensity.w;
        if (all(radiance <= vec3<f32>(0.0))) {
            continue;
        }
        let to_light = normalize(-directional.direction.xyz);
        let shadow = fog_directional_shadow(i, p);
        if (shadow <= 0.0) {
            continue;
        }
        light += radiance * (shadow * fog_phase(to_light, view_dir));
    }

    let range = light_cluster(p);
    for (var i = 0u; i < range.count; i = i + 1u) {
        let local = light_in_range(range, i);
        let arrival = local_light_arrival(local, p);
        if (arrival.attenuation <= 0.0) {
            continue;
        }
        let d2 = arrival.distance * arrival.distance;
        let held = d2 / max(d2, FOG_LIGHT_MIN_DISTANCE * FOG_LIGHT_MIN_DISTANCE);
        let shadow = fog_local_shadow(local, p);
        if (shadow <= 0.0) {
            continue;
        }
        light += local.color_intensity.xyz * local.color_intensity.w
            * (arrival.attenuation * held * shadow * fog_phase(arrival.L, view_dir));
    }
    return light * fog.volume.z + fog_ambient_at(p) * fog.emission.w;
}

// Where world point `p` lay in last frame's volume (texture coordinates; outside 0-1:
// it was not in it).
fn fog_previous_cell(p: vec3<f32>) -> vec3<f32> {
    let clip = fog.prev_view_proj * vec4<f32>(p, 1.0);
    if (clip.w <= 1e-6) {
        return vec3<f32>(-1.0);
    }
    let ndc = clip.xy / clip.w;
    let depth = dot(p - fog.prev_camera_position.xyz, fog.prev_camera_forward.xyz);
    if (depth < 0.0 || depth > fog.volume.x) {
        return vec3<f32>(-1.0);
    }
    return vec3<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5, sqrt(depth / fog.volume.x));
}

@compute @workgroup_size(4, 4, 4)
fn cs_fog_scatter(@builtin(global_invocation_id) id: vec3<u32>) {
    let dims = fog.dims.xyz;
    if (any(id >= dims)) {
        return;
    }
    let size = vec3<f32>(dims);
    let cell = (vec3<f32>(id) + fog_jitter(id)) / size;
    let p = fog_view_point(cell.xy, fog_slice_depth(cell.z));
    let to_point = p - fog.camera_position.xyz;
    var density = fog_density_at(p);
    if (length(to_point) > fog.density.w) {
        density = 0.0;
    }
    var value = vec4<f32>(0.0, 0.0, 0.0, density);
    if (density > 0.0) {
        // The ray's direction, not normalize(to_point): in the first slice the point can
        // lie on the camera itself.
        let light = fog_froxel_light(p, fog_view_ray(cell.xy));
        value = vec4<f32>((fog.albedo.rgb * light + fog.emission.rgb) * density, density);
    }

    let keep = fog.flags.w;
    if (keep > 0.0) {
        let center = (vec3<f32>(id) + vec3<f32>(0.5)) / size;
        let previous = fog_previous_cell(fog_view_point(center.xy, fog_slice_depth(center.z)));
        if (all(previous >= vec3<f32>(0.0)) && all(previous <= vec3<f32>(1.0))) {
            let history = textureSampleLevel(fog_history, fog_history_sampler, previous, 0.0);
            value = mix(value, history, keep);
        }
    }
    textureStore(fog_scatter_out, id, value);
}
