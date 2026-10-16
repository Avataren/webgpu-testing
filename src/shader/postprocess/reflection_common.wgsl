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

// The main pass packs smoothness (1 - roughness) and the specular colour into the bits
// of the position's half-float w (encode_surface_w in common.wgsl); below 0.5 there is
// no surface. The f32 read back has the half's power of two and its 10 mantissa bits at
// the top of its own.
fn ssr_surface_code(position : vec4<f32>) -> u32 {
    let bits = bitcast<u32>(position.w);
    let power = i32((bits >> 23u) & 0xffu) - 127;
    if power < 0 {
        return 0u;
    }
    return min((u32(power) << 10u) | ((bits >> 13u) & 1023u), 0x3fffu);
}

fn ssr_smoothness(position : vec4<f32>) -> f32 {
    return f32(ssr_surface_code(position) >> 9u) / 31.0;
}

// F0: how much of a reflection the surface shows head-on, per channel (0.04 for
// dielectrics, the base colour for metals).
fn ssr_specular_color(position : vec4<f32>) -> vec3<f32> {
    let code = ssr_surface_code(position);
    let encoded = vec3<f32>(
        f32((code >> 6u) & 7u),
        f32((code >> 3u) & 7u),
        f32(code & 7u),
    ) / 7.0;
    return vec3<f32>(0.04) + 0.96 * encoded * encoded;
}

// How metallic a specular colour is: 0 at the dielectric 0.04, 1 from 0.54 up.
fn ssr_metalness(f0 : vec3<f32>) -> f32 {
    return clamp((max(f0.r, max(f0.g, f0.b)) - 0.04) / 0.5, 0.0, 1.0);
}

// How much of a reflection fs_ssr_apply adds to a surface: smoother surfaces and grazing
// views reflect more (Schlick's Fresnel from the surface's specular colour, so metals
// reflect fully and in their own colour; dielectrics get a floor so a surface seen head-on
// still shows a sheen).
fn ssr_apply_weight(position : vec4<f32>, n_dot_v : f32) -> vec3<f32> {
    let smoothness = ssr_smoothness(position);
    let f0 = ssr_specular_color(position);
    let fresnel = f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - n_dot_v, 5.0);
    let sheen = 0.2 * (1.0 - ssr_metalness(f0));
    return post_uniform.ssr.x * smoothness * smoothness * (sheen + (1.0 - sheen) * fresnel);
}

// Full-resolution pixels per reflection texel: 2 for the default half-resolution trace,
// 1 with ssr.full_resolution.
fn ssr_texel_scale() -> f32 {
    return post_uniform.reflection.x;
}

// The full-resolution G-buffer pixel a reflection texel at fragment position `texel`
// traces from.
fn ssr_source_pixel(texel : vec2<f32>, dims : vec2<i32>) -> vec2<i32> {
    return min(vec2<i32>(texel * ssr_texel_scale()), dims - vec2<i32>(1));
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
