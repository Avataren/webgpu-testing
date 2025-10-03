struct Globals {
    view_proj: mat4x4<f32>,
};
@group(0) @binding(0) var<uniform> globals: Globals;

struct Object {
    model: mat4x4<f32>,
};
@group(1) @binding(0) var<storage, read> objects: array<Object>;

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @builtin(instance_index) instance: u32,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    let M = objects[in.instance].model;
    let world_pos = M * vec4(in.pos, 1.0);

    // For correctness with non-uniform scales you'd use inverse-transpose
    let n = normalize((M * vec4(in.normal, 0.0)).xyz);

    var out: VsOut;
    out.pos = globals.view_proj * world_pos;
    out.normal = n;
    out.uv = in.uv;           // use mesh UVs across all faces
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Simple lambert-ish: light from +Z
    let L = normalize(vec3<f32>(0.5, 0.3, 1.0));
    let ndotl = max(dot(normalize(in.normal), L), 0.2); // min ambient

    // ---------- Checker (UV-based) ----------
    let tiles = 6.0;                          // number of squares per 0..1 UV range
    let line_frac = 0.06;                     // grid line thickness as fraction of a tile
    let colorA = vec3<f32>(0.08, 0.09, 0.11); // charcoal
    let colorB = vec3<f32>(0.93, 0.93, 0.96); // soft ivory
    let inlay  = vec3<f32>(0.92, 0.78, 0.36); // subtle gold

    // Tile-space UV
    let c = in.uv * tiles;

    // Parity-based checker
    let parity = (i32(floor(c.x) + floor(c.y)) & 1) == 0;
    var base = select(colorA, colorB, parity);

    // Analytic AA for crisp edges (use footprint in tile space)
    let fw = max(fwidth(c.x), fwidth(c.y));
    let aa = fw * 0.75;

    // Distance to nearest gridline within current tile
    let fx = abs(fract(c.x) - 0.5);
    let fy = abs(fract(c.y) - 0.5);
    let edge_proximity = 0.5 - min(fx, fy);   // higher near grid lines

    // Decorative inlay lines
    let line = smoothstep(line_frac + aa, line_frac - aa, edge_proximity);
    base = mix(base, inlay, line * 0.7);

    // Soft rim for a classy pop
    let rim = pow(1.0 - max(dot(normalize(in.normal), normalize(-L)), 0.0), 3.0);
    base = base + rim * 0.06;

    return vec4(base * ndotl, 1.0);
}
