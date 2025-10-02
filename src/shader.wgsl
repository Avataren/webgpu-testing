struct Camera {
    view_proj : mat4x4<f32>,
};
@group(0) @binding(0) var<uniform> camera : Camera;

struct VsIn {
    @location(0) pos    : vec3<f32>,
    @location(1) normal : vec3<f32>,
    @location(2) uv     : vec2<f32>,
};

struct VsOut {
    @builtin(position) pos_cs : vec4<f32>,
    @location(0) normal_ws    : vec3<f32>,
    @location(1) uv           : vec2<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    let model = mat4x4<f32>(
        vec4<f32>(1.0, 0.0, 0.0, 0.0),
        vec4<f32>(0.0, 1.0, 0.0, 0.0),
        vec4<f32>(0.0, 0.0, 1.0, 0.0),
        vec4<f32>(0.0, 0.0, 0.0, 1.0)
    );
    let pos_ws = model * vec4<f32>(in.pos, 1.0);
    out.pos_cs = camera.view_proj * pos_ws;
    // No non-uniform scaling, so normal stays the same
    out.normal_ws = in.normal;
    out.uv = in.uv;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Simple directional light
    let light_dir = normalize(vec3<f32>(0.6, 1.0, 0.8));
    let n = normalize(in.normal_ws);
    let lambert = max(dot(n, light_dir), 0.0);

    // Subtle base tint + UV-based variation
    let base = vec3<f32>(0.15, 0.55, 0.85);
    let tint = base * (0.4 + 0.6 * lambert);
    let checker = step(0.5, fract((in.uv.x + in.uv.y) * 4.0));
    let color = mix(tint * 0.9, tint * 1.1, checker);

    return vec4<f32>(color, 1.0);
}
