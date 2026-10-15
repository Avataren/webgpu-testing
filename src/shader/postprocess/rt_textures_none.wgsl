// Material textures at ray hits when they cannot be bound (no bindless textures, or too
// few bind groups): hits are shaded from the material factors alone.

fn rt_sample_texture(index : u32, uv : vec2<f32>, lod_base : f32, nearest : bool) -> vec4<f32> {
    return vec4<f32>(1.0);
}
