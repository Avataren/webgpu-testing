// Caustic resolve: the photons' per-pixel sums (pt_caustics.wgsl) gathered over a small
// neighbourhood on the same surface (a density estimate: photons land sparsely), added to
// the path tracer's demodulated diffuse light, which the denoiser then accumulates and
// filters like the rest of it.
//
// Part of the composite shader module (pt_common.wgsl, `pt` at group 0).

const CAUSTIC_RESOLVE_SCALE : f32 = 256.0;
const CAUSTIC_RESOLVE_RADIUS : i32 = 2;

@group(1) @binding(60)
var<storage, read> cr_sums : array<u32>;
@group(1) @binding(61)
var cr_gbuffer_normal : texture_2d<f32>;
@group(1) @binding(62)
var cr_gbuffer_position : texture_2d<f32>;

fn cr_sum(local : vec2<i32>) -> vec3<f32> {
    let index = u32(local.y * i32(pt.region.z) + local.x) * 3u;
    return vec3<f32>(f32(cr_sums[index]), f32(cr_sums[index + 1u]), f32(cr_sums[index + 2u]))
        / CAUSTIC_RESOLVE_SCALE;
}

// Drawn into the diffuse light target, adding.
@fragment
fn fs_pt_caustic_resolve(in : VertexOutput) -> @location(0) vec4<f32> {
    let local = vec2<i32>(in.position.xy);
    let origin = vec2<i32>(pt.region.xy);
    let size = vec2<i32>(pt.region.zw);
    let center = textureLoad(cr_gbuffer_position, local + origin, 0);
    if center.w < 0.5 {
        return vec4<f32>(0.0);
    }
    let n = pt_decode_normal(textureLoad(cr_gbuffer_normal, local + origin, 0).xy);
    let depth = pt_view_depth(center.xyz);
    let tolerance = 0.02 * depth + 2.0 * pt_footprint(depth);
    var sum = vec3<f32>(0.0);
    var weight = 0.0;
    for (var y = -CAUSTIC_RESOLVE_RADIUS; y <= CAUSTIC_RESOLVE_RADIUS; y = y + 1) {
        for (var x = -CAUSTIC_RESOLVE_RADIUS; x <= CAUSTIC_RESOLVE_RADIUS; x = x + 1) {
            let q = local + vec2<i32>(x, y);
            if any(q < vec2<i32>(0)) || any(q >= size) {
                continue;
            }
            let surface = textureLoad(cr_gbuffer_position, q + origin, 0);
            if surface.w < 0.5 {
                continue;
            }
            let qn = pt_decode_normal(textureLoad(cr_gbuffer_normal, q + origin, 0).xy);
            let plane = abs(dot(n, surface.xyz - center.xyz));
            let w = exp(-f32(x * x + y * y) / 4.5)
                * pow(max(dot(n, qn), 0.0), 8.0)
                * exp(-plane / tolerance);
            sum += cr_sum(q) * w;
            weight += w;
        }
    }
    if weight <= 0.0 {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(sum / weight, 0.0);
}
