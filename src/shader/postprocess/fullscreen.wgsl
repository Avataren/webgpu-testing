struct VertexOutput {
    @builtin(position) position : vec4<f32>,
    @location(0) uv : vec2<f32>,
};

@vertex
fn vs_fullscreen(@builtin(vertex_index) vertex_index : u32) -> VertexOutput {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(3.0, 1.0),
    );
    let pos = positions[vertex_index];
    var out : VertexOutput;
    out.position = vec4<f32>(pos, 0.0, 1.0);
    // Flip Y coordinate: top of screen (clip Y=1) maps to UV.y=0
    out.uv = vec2<f32>(0.5 * (pos.x + 1.0), 0.5 * (1.0 - pos.y));
    return out;
}
