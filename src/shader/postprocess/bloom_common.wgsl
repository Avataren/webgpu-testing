fn bloom_gaussian_weight(offset : vec2<f32>, sigma : f32) -> f32 {
    return exp(-(dot(offset, offset)) / (2.0 * sigma * sigma));
}

// The scene may fill only part of the post-processing targets (the editor's game view).
// Bloom steps scale with that part so a glow covers the same share of the view at any
// viewport size, and samples stay inside it so nothing outside bleeds in.
fn bloom_texel(tex_size : vec2<f32>) -> vec2<f32> {
    return post_uniform.viewport_scale / tex_size;
}

fn bloom_clamp_uv(uv : vec2<f32>) -> vec2<f32> {
    let lo = post_uniform.viewport_offset;
    return clamp(uv, lo, lo + post_uniform.viewport_scale);
}
