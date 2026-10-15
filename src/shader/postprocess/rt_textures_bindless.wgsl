// Material textures at ray hits: the main pass's bindless texture array (same layout as
// bindings_bindless_*.wgsl), bound as group 4 of the ray-traced reflection pass.

@group(4) @binding(0) var rt_textures : binding_array<texture_2d<f32>, 256>;
@group(4) @binding(1) var rt_sampler_linear : sampler;
@group(4) @binding(2) var rt_sampler_nearest : sampler;

// `lod_base` is the hit's mip level for a 1x1 texture (see rt_shade); the
// texture's size moves it to the right level.
fn rt_sample_texture(index : u32, uv : vec2<f32>, lod_base : f32, nearest : bool) -> vec4<f32> {
    // A slot past the array (more textures loaded than it holds): leave the factors as
    // they are.
    if index >= 256u {
        return vec4<f32>(1.0);
    }
    let size = vec2<f32>(textureDimensions(rt_textures[index]));
    let lod = max(lod_base + 0.5 * log2(size.x * size.y), 0.0);
    if nearest {
        return textureSampleLevel(rt_textures[index], rt_sampler_nearest, uv, lod);
    }
    return textureSampleLevel(rt_textures[index], rt_sampler_linear, uv, lod);
}
