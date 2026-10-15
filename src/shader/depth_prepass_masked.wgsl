// Cut-out surfaces (glTF MASK) in the depth prepass: depth_prepass.wgsl's depth, minus
// what the alpha test removes, so the ray-traced shadow mask (traced from this depth) and
// the main pass see the surface itself, not what shows through its holes. Built after
// depth_prepass.wgsl with the material texture bindings (group 3).

struct MaskedVsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) instance: u32,
};

@vertex
fn vs_masked(in: VsIn) -> MaskedVsOut {
    var out: MaskedVsOut;
    out.position = prepass_position(in);
    out.uv = in.uv;
    out.instance = in.instance;
    return out;
}

@fragment
fn fs_masked(in: MaskedVsOut) {
    let material = materials[objects[in.instance].material_index];
    let base_color_sample = sample_base_color_texture(material.base_color_texture, in.uv);
    var alpha = material.color.a;
    if (material.material_flags & FLAG_USE_BASE_COLOR_TEXTURE) != 0u {
        alpha = alpha * base_color_sample.a;
    }
    if alpha < material_alpha_cutoff(material.material_flags) {
        discard;
    }
}
