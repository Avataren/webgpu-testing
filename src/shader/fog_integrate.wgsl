// src/shader/fog_integrate.wgsl
// The froxel volume's integration (src/renderer/fog.rs): one invocation per column of
// froxels walks it from the camera outward, adding up the light each slice scatters
// toward the camera (what it takes in over its length, from the slice's own scattering and
// extinction, as Hillaire's "Physically Based and Unified Volumetric Rendering in
// Frostbite" integrates it) and the share of light from behind that gets through, and
// writes after each slice the fog between the camera and that slice's far end.
//
// Needs fog_common.wgsl.

@group(0) @binding(0) var<uniform> fog: FogParams;
@group(0) @binding(1) var fog_scatter: texture_3d<f32>;
@group(0) @binding(2) var fog_integrated_out: texture_storage_3d<rgba16float, write>;

@compute @workgroup_size(8, 8, 1)
fn cs_fog_integrate(@builtin(global_invocation_id) id: vec3<u32>) {
    let dims = fog.dims.xyz;
    if (id.x >= dims.x || id.y >= dims.y) {
        return;
    }
    let uv = (vec2<f32>(id.xy) + vec2<f32>(0.5)) / vec2<f32>(dims.xy);
    // Meters along this column's view ray per meter of view depth.
    let stretch = length(fog_view_point(uv, 2.0) - fog_view_point(uv, 1.0));
    let slices = f32(dims.z);
    var light = vec3<f32>(0.0);
    var through = 1.0;
    var previous = 0.0;
    for (var z = 0u; z < dims.z; z = z + 1u) {
        let depth = fog_slice_depth(f32(z + 1u) / slices);
        let span = (depth - previous) * stretch;
        previous = depth;
        let froxel = textureLoad(fog_scatter, vec3<i32>(vec3<u32>(id.xy, z)), 0);
        let extinction = max(froxel.a, 1e-6);
        let slice_through = exp(-extinction * span);
        light += through * froxel.rgb * ((1.0 - slice_through) / extinction);
        through *= slice_through;
        textureStore(fog_integrated_out, vec3<u32>(id.xy, z), vec4<f32>(light, through));
    }
}
