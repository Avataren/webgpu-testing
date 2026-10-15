// Helpers shared by the screen-space (ssr.wgsl) and ray-traced (rt_reflections.wgsl)
// reflection traces. No bindings: either trace includes this.

// Rougher surfaces than this reflect too little to be worth a trace.
const SSR_MIN_SMOOTHNESS : f32 = 0.05;
// Brightest reflected value, so a tiny HDR highlight does not turn into a blurred blotch.
const SSR_MAX_RADIANCE : f32 = 32.0;

// Interleaved gradient noise: well spread over every small neighbourhood, so the blur
// averages the ray offsets out, and the same every frame, so nothing flickers.
fn ssr_jitter(pixel : vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(pixel, vec2<f32>(0.06711056, 0.00583715))));
}

// Radiance squeezed into 0..1 for blurring (a lone bright sample cannot dominate), and back.
fn ssr_compress(color : vec3<f32>) -> vec3<f32> {
    return color / (1.0 + luminance_rec709(color));
}

fn ssr_expand(color : vec3<f32>) -> vec3<f32> {
    return color / max(1.0 - luminance_rec709(color), 1e-3);
}

// The main pass stores 1 + smoothness (1 - roughness) in the position's w; below 0.5
// there is no surface.
fn ssr_smoothness(position : vec4<f32>) -> f32 {
    return clamp(position.w - 1.0, 0.0, 1.0);
}

fn ssr_view_depth(world : vec3<f32>) -> f32 {
    return -(post_uniform.view * vec4<f32>(world, 1.0)).z;
}

// Where `world` lands in the viewport (uv 0-1, origin top left); z is 0 behind the camera.
fn ssr_viewport_uv(world : vec3<f32>) -> vec3<f32> {
    let clip = post_uniform.view_proj * vec4<f32>(world, 1.0);
    if clip.w <= 1e-5 {
        return vec3<f32>(-1.0, -1.0, 0.0);
    }
    let ndc = clip.xy / clip.w;
    return vec3<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5, 1.0);
}

fn ssr_on_viewport(uv : vec2<f32>) -> bool {
    return all(uv >= vec2<f32>(0.0)) && all(uv <= vec2<f32>(1.0));
}
