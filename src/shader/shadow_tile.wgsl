// src/shader/shadow_tile.wgsl
// Whole tiles of the local shadow atlas (src/renderer/internal/shadows.rs), drawn as a
// triangle over the tile's viewport: clearing a tile to the far plane, and copying a
// tile's static casters from the static atlas, which has the same layout (WebGPU copies
// only whole depth textures, never part of one).

@group(0) @binding(0) var static_atlas: texture_depth_2d;

@vertex
fn vs_tile(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 1.0, 1.0);
}

@fragment
fn fs_copy(@builtin(position) position: vec4<f32>) -> @builtin(frag_depth) f32 {
    return textureLoad(static_atlas, vec2<i32>(position.xy), 0);
}
