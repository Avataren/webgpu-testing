fn bloom_gaussian_weight(offset : vec2<f32>, sigma : f32) -> f32 {
    return exp(-(dot(offset, offset)) / (2.0 * sigma * sigma));
}
