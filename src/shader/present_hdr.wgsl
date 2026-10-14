// HDR presentation: composite the finished scene (linear, 1.0 = paper white) with the
// egui layer (sRGB-encoded, premultiplied alpha) and encode for the display.

struct PresentUniform {
    // 0 simulated on an SDR surface, 1 scRGB, 2 HDR10 (PQ)
    mode : f32,
    // white in nits
    paper_white : f32,
    // brightest highlight relative to paper white
    peak : f32,
    // 1.0 when an SDR surface needs sRGB encoding in the shader
    encode_srgb : f32,
};

@group(0) @binding(0)
var<uniform> present : PresentUniform;
@group(0) @binding(1)
var present_scene : texture_2d<f32>;
@group(0) @binding(2)
var present_ui : texture_2d<f32>;

struct PresentVertex {
    @builtin(position) position : vec4<f32>,
};

@vertex
fn vs_present(@builtin(vertex_index) vertex_index : u32) -> PresentVertex {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(3.0, 1.0),
    );
    var out : PresentVertex;
    out.position = vec4<f32>(positions[vertex_index], 0.0, 1.0);
    return out;
}

fn present_srgb_to_linear(c : vec3<f32>) -> vec3<f32> {
    let low = c / 12.92;
    let high = pow((max(c, vec3<f32>(0.0)) + 0.055) / 1.055, vec3<f32>(2.4));
    return select(high, low, c <= vec3<f32>(0.04045));
}

fn present_linear_to_srgb(c : vec3<f32>) -> vec3<f32> {
    let x = max(c, vec3<f32>(0.0));
    let low = x * 12.92;
    let high = 1.055 * pow(x, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(high, low, x <= vec3<f32>(0.0031308));
}

// BT.709 to BT.2020 primaries (both linear, D65).
const BT709_TO_BT2020 = mat3x3<f32>(
    vec3<f32>(0.6274040, 0.0690970, 0.0163916),
    vec3<f32>(0.3292820, 0.9195400, 0.0880132),
    vec3<f32>(0.0433136, 0.0113612, 0.8955950),
);

// SMPTE ST 2084 (PQ): absolute luminance (nits) to a 0-1 signal.
fn pq_encode(nits : vec3<f32>) -> vec3<f32> {
    let m1 = 0.1593017578125;
    let m2 = 78.84375;
    let c1 = 0.8359375;
    let c2 = 18.8515625;
    let c3 = 18.6875;
    let y = pow(clamp(nits / 10000.0, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(m1));
    return pow((c1 + c2 * y) / (1.0 + c3 * y), vec3<f32>(m2));
}

// For the SDR simulation: compress what lies above white into the last part of the range.
fn fit_to_sdr(c : vec3<f32>) -> vec3<f32> {
    let knee = 0.8;
    let over = max(c - vec3<f32>(knee), vec3<f32>(0.0));
    let room = 1.0 - knee;
    let range = max(present.peak - knee, 1e-3);
    let compressed = vec3<f32>(knee) + room * (1.0 - exp(-over * (3.0 / range)));
    return select(c, compressed, c > vec3<f32>(knee));
}

@fragment
fn fs_present(in : PresentVertex) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(in.position.xy);
    var color = max(textureLoad(present_scene, pixel, 0).rgb, vec3<f32>(0.0));
    color = select(color, vec3<f32>(0.0), color != color);

    // UI on top at paper white, blended in linear light.
    let ui = textureLoad(present_ui, pixel, 0);
    if ui.a > 0.0 {
        let ui_linear = present_srgb_to_linear(ui.rgb / ui.a);
        color = color * (1.0 - ui.a) + ui_linear * ui.a;
    }

    let mode = u32(present.mode + 0.5);
    if mode == 1u {
        // scRGB: linear BT.709 where 1.0 is 80 nits.
        return vec4<f32>(color * (present.paper_white / 80.0), 1.0);
    }
    if mode == 2u {
        return vec4<f32>(pq_encode(BT709_TO_BT2020 * color * present.paper_white), 1.0);
    }
    let sdr = clamp(fit_to_sdr(color), vec3<f32>(0.0), vec3<f32>(1.0));
    if present.encode_srgb > 0.5 {
        return vec4<f32>(present_linear_to_srgb(sdr), 1.0);
    }
    return vec4<f32>(sdr, 1.0);
}
