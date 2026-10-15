// Path tracing composite: the denoised (or raw) light, remodulated by the surface's
// colours, written over the scene where the path tracer shaded it; elsewhere (sky,
// transmissive surfaces, geometry it does not see) the main pass's colour stays. The
// colour's alpha is kept (it marks transmissive surfaces for the refraction pass).
//
// Needs post-processing's fullscreen module and pt_common.wgsl.

@group(0) @binding(0)
var<uniform> pt : PtUniform;
@group(1) @binding(50)
var pc_diffuse : texture_2d<f32>;
@group(1) @binding(51)
var pc_specular : texture_2d<f32>;
@group(1) @binding(52)
var pc_material : texture_2d<f32>;
@group(1) @binding(53)
var pc_emission : texture_2d<f32>;
@group(1) @binding(54)
var pc_gbuffer_normal : texture_2d<f32>;
@group(1) @binding(55)
var pc_gbuffer_position : texture_2d<f32>;

@fragment
fn fs_pt_composite(in : VertexOutput) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(in.position.xy);
    let local = pixel - vec2<i32>(pt.region.xy);
    if any(local < vec2<i32>(0)) || any(local >= vec2<i32>(pt.region.zw)) {
        discard;
    }
    let emission = textureLoad(pc_emission, local, 0);
    if emission.a < 0.0 {
        discard;
    }
    let material = textureLoad(pc_material, local, 0);
    let base_color = material.rgb;
    let roughness = material.a;
    let metallic = emission.a;

    // As the trace has it: the G-buffer normal, facing the camera.
    let world = textureLoad(pc_gbuffer_position, pixel, 0).xyz;
    var v : vec3<f32>;
    if pt.camera_position.w > 0.5 {
        v = normalize(pt.camera_position.xyz - world);
    } else {
        v = normalize(vec3<f32>(pt.view[0].z, pt.view[1].z, pt.view[2].z));
    }
    var n = pt_decode_normal(textureLoad(pc_gbuffer_normal, pixel, 0).xy);
    n = n * select(1.0, -1.0, dot(n, v) < 0.0);
    let n_dot_v_raw = dot(n, v);
    if n_dot_v_raw < 0.02 {
        n = normalize(n + v * (0.02 - n_dot_v_raw));
    }
    let n_dot_v = max(dot(n, v), 1e-4);

    let f0 = mix(vec3<f32>(0.04), base_color, metallic);
    let specular_albedo = max(pt_specular_albedo(f0, n_dot_v, roughness), vec3<f32>(1e-3));
    let diffuse_color = base_color * (1.0 - metallic);
    let diffuse = textureLoad(pc_diffuse, local, 0).rgb;
    let specular = textureLoad(pc_specular, local, 0).rgb;
    var color = emission.rgb + diffuse * diffuse_color + specular * specular_albedo;
    color = select(color, vec3<f32>(0.0), color != color);
    return vec4<f32>(max(color, vec3<f32>(0.0)), 0.0);
}
