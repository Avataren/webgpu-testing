// Screen-space editor shapes (renderer/gizmo_shapes.rs): anti-aliased lines,
// discs, rings, squares, arrow heads and quads anchored at world positions but
// sized in pixels, drawn on the finished image without depth.

struct ShapeUniform {
    viewport_size : vec2<f32>,
    _padding : vec2<f32>,
};
@group(1) @binding(0) var<uniform> shape_uniform : ShapeUniform;

const KIND_LINE : u32 = 0u;
const KIND_DISC : u32 = 1u;
const KIND_SQUARE : u32 = 2u;
const KIND_ARROW : u32 = 3u;
const KIND_QUAD : u32 = 4u;

// Clip-space w below which a point counts as behind the camera.
const NEAR_W : f32 = 1e-4;

struct ShapeInput {
    @location(0) p0 : vec4<f32>,
    @location(1) p1 : vec4<f32>,
    @location(2) p2 : vec4<f32>,
    @location(3) p3 : vec4<f32>,
    @location(4) color : vec4<f32>,
    @location(5) outline : vec4<f32>,
    // x: grow the shape by this many pixels, y: 1 for lines with square ends.
    @location(6) extra : vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position : vec4<f32>,
    // Pixel position inside the viewport (+y up).
    @location(0) pixel : vec2<f32>,
    @location(1) @interpolate(flat) kind : u32,
    // Shape points in pixels: lines a/b (and in b.x whether the ends are square),
    // arrows tip/base, quads corners 0-3.
    @location(2) @interpolate(flat) points_a : vec4<f32>,
    @location(3) @interpolate(flat) points_b : vec4<f32>,
    // x, y: size parameters, z: outline width, w: growth.
    @location(4) @interpolate(flat) params : vec4<f32>,
    @location(5) @interpolate(flat) color : vec4<f32>,
    @location(6) @interpolate(flat) outline : vec4<f32>,
};

fn to_pixels(clip : vec4<f32>) -> vec2<f32> {
    let ndc = clip.xy / clip.w;
    return (ndc * 0.5 + 0.5) * shape_uniform.viewport_size;
}

fn project(p : vec3<f32>) -> vec4<f32> {
    return globals.view_proj * vec4<f32>(p, 1.0);
}

fn perpendicular(v : vec2<f32>) -> vec2<f32> {
    return vec2<f32>(-v.y, v.x);
}

fn culled() -> VertexOutput {
    var out : VertexOutput;
    out.position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    out.pixel = vec2<f32>(0.0);
    out.kind = 0u;
    out.points_a = vec4<f32>(0.0);
    out.points_b = vec4<f32>(0.0);
    out.params = vec4<f32>(0.0);
    out.color = vec4<f32>(0.0);
    out.outline = vec4<f32>(0.0);
    return out;
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index : u32, shape : ShapeInput) -> VertexOutput {
    // Two triangles over the shape's bounding rectangle, as (0|1, 0|1) corners.
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0),
    );
    let corner = corners[vertex_index];
    let kind = u32(shape.p0.w + 0.5);
    let outline_width = shape.p3.w;
    // Room for growth and anti-aliasing beyond the shape's edge.
    let margin = 1.5 + max(shape.extra.x, 0.0);

    var out : VertexOutput;
    out.kind = kind;
    out.color = shape.color;
    out.outline = shape.outline;
    out.params = vec4<f32>(shape.p1.w, shape.p2.w, outline_width, shape.extra.x);
    out.points_a = vec4<f32>(0.0);
    out.points_b = vec4<f32>(0.0);

    // A rectangle as origin + corner.x * axis_u + corner.y * axis_v.
    var origin = vec2<f32>(0.0);
    var axis_u = vec2<f32>(0.0);
    var axis_v = vec2<f32>(0.0);

    if (kind == KIND_LINE) {
        var a = project(shape.p0.xyz);
        var b = project(shape.p1.xyz);
        if (a.w < NEAR_W && b.w < NEAR_W) {
            return culled();
        }
        // Clip the part behind the camera.
        if (a.w < NEAR_W) {
            a = mix(a, b, (NEAR_W - a.w) / (b.w - a.w));
        } else if (b.w < NEAR_W) {
            b = mix(b, a, (NEAR_W - b.w) / (a.w - b.w));
        }
        let pa = to_pixels(a);
        let pb = to_pixels(b);
        out.points_a = vec4<f32>(pa, pb);
        out.points_b = vec4<f32>(shape.extra.y, 0.0, 0.0, 0.0);
        let delta = pb - pa;
        let length_px = length(delta);
        let dir = select(vec2<f32>(1.0, 0.0), delta / length_px, length_px > 1e-4);
        let half_extent = 0.5 * shape.p1.w + margin;
        origin = pa - dir * half_extent - perpendicular(dir) * half_extent;
        axis_u = dir * (length_px + 2.0 * half_extent);
        axis_v = perpendicular(dir) * (2.0 * half_extent);
    } else if (kind == KIND_DISC || kind == KIND_SQUARE) {
        let center = project(shape.p0.xyz);
        if (center.w < NEAR_W) {
            return culled();
        }
        let pc = to_pixels(center);
        out.points_a = vec4<f32>(pc, 0.0, 0.0);
        let extent = shape.p1.w + 0.5 * shape.p2.w + margin;
        origin = pc - vec2<f32>(extent);
        axis_u = vec2<f32>(2.0 * extent, 0.0);
        axis_v = vec2<f32>(0.0, 2.0 * extent);
    } else if (kind == KIND_ARROW) {
        let base = project(shape.p0.xyz);
        let tip = project(shape.p1.xyz);
        if (base.w < NEAR_W || tip.w < NEAR_W) {
            return culled();
        }
        let pt = to_pixels(tip);
        let delta = pt - to_pixels(base);
        if (dot(delta, delta) < 1e-6) {
            return culled();
        }
        let dir = normalize(delta);
        let pb = pt - dir * shape.p1.w;
        out.points_a = vec4<f32>(pt, pb);
        out.points_b = vec4<f32>(perpendicular(dir) * shape.p2.w, 0.0, 0.0);
        origin = pb - dir * margin - perpendicular(dir) * (shape.p2.w + margin);
        axis_u = dir * (shape.p1.w + 2.0 * margin);
        axis_v = perpendicular(dir) * (2.0 * (shape.p2.w + margin));
    } else {
        let c0 = project(shape.p0.xyz);
        let c1 = project(shape.p1.xyz);
        let c2 = project(shape.p2.xyz);
        let c3 = project(shape.p3.xyz);
        if (min(min(c0.w, c1.w), min(c2.w, c3.w)) < NEAR_W) {
            return culled();
        }
        let q0 = to_pixels(c0);
        let q1 = to_pixels(c1);
        let q2 = to_pixels(c2);
        let q3 = to_pixels(c3);
        out.points_a = vec4<f32>(q0, q1);
        out.points_b = vec4<f32>(q2, q3);
        let lo = min(min(q0, q1), min(q2, q3)) - vec2<f32>(margin);
        let hi = max(max(q0, q1), max(q2, q3)) + vec2<f32>(margin);
        origin = lo;
        axis_u = vec2<f32>(hi.x - lo.x, 0.0);
        axis_v = vec2<f32>(0.0, hi.y - lo.y);
    }

    let pixel = origin + corner.x * axis_u + corner.y * axis_v;
    out.pixel = pixel;
    let ndc = pixel / shape_uniform.viewport_size * 2.0 - 1.0;
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    return out;
}

fn sd_segment(p : vec2<f32>, a : vec2<f32>, b : vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-6), 0.0, 1.0);
    return length(pa - ba * h);
}

fn cross2(a : vec2<f32>, b : vec2<f32>) -> f32 {
    return a.x * b.y - a.y * b.x;
}

// Signed distance to a convex or concave polygon (negative inside), after
// Inigo Quilez's sdPolygon.
fn sd_quad(p : vec2<f32>, v0 : vec2<f32>, v1 : vec2<f32>, v2 : vec2<f32>, v3 : vec2<f32>) -> f32 {
    var v = array<vec2<f32>, 4>(v0, v1, v2, v3);
    var d = dot(p - v[0], p - v[0]);
    var s = 1.0;
    var j = 3;
    for (var i = 0; i < 4; i = i + 1) {
        let e = v[j] - v[i];
        let w = p - v[i];
        let b = w - e * clamp(dot(w, e) / max(dot(e, e), 1e-6), 0.0, 1.0);
        d = min(d, dot(b, b));
        let c = vec3<bool>((p.y >= v[i].y), (p.y < v[j].y), (e.x * w.y > e.y * w.x));
        if (all(c) || !any(c)) {
            s = -s;
        }
        j = i;
    }
    return s * sqrt(d);
}

fn sd_triangle(p : vec2<f32>, p0 : vec2<f32>, p1 : vec2<f32>, p2 : vec2<f32>) -> f32 {
    let e0 = p1 - p0;
    let e1 = p2 - p1;
    let e2 = p0 - p2;
    let v0 = p - p0;
    let v1 = p - p1;
    let v2 = p - p2;
    let pq0 = v0 - e0 * clamp(dot(v0, e0) / dot(e0, e0), 0.0, 1.0);
    let pq1 = v1 - e1 * clamp(dot(v1, e1) / dot(e1, e1), 0.0, 1.0);
    let pq2 = v2 - e2 * clamp(dot(v2, e2) / dot(e2, e2), 0.0, 1.0);
    let s = sign(cross2(e0, e2));
    let d = min(
        min(
            vec2<f32>(dot(pq0, pq0), s * cross2(v0, e0)),
            vec2<f32>(dot(pq1, pq1), s * cross2(v1, e1)),
        ),
        vec2<f32>(dot(pq2, pq2), s * cross2(v2, e2)),
    );
    return -sqrt(d.x) * sign(d.y);
}

fn sd_rounded_box(p : vec2<f32>, half_size : f32, radius : f32) -> f32 {
    let r = min(radius, half_size);
    let q = abs(p) - vec2<f32>(half_size - r);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

@fragment
fn fs_main(in : VertexOutput) -> @location(0) vec4<f32> {
    let p = in.pixel;
    // Signed distance in pixels, negative inside the shape.
    var d = 0.0;
    if (in.kind == KIND_LINE) {
        if (in.points_b.x > 0.5) {
            // Square ends, cut without anti-aliasing over [start, end): a pixel on
            // the joint of two polyline segments belongs to exactly one of them, so
            // translucent polylines show neither overlaps nor seams. Growing only
            // widens the line.
            let a = in.points_a.xy;
            let ba = in.points_a.zw - a;
            let length_px = max(length(ba), 1e-4);
            let dir = ba / length_px;
            let local = p - a;
            let along = dot(local, dir);
            if (along < 0.0 || along >= length_px) {
                discard;
            }
            d = abs(dot(local, perpendicular(dir))) - 0.5 * in.params.x;
        } else {
            d = sd_segment(p, in.points_a.xy, in.points_a.zw) - 0.5 * in.params.x;
        }
    } else if (in.kind == KIND_DISC) {
        let r = length(p - in.points_a.xy) - in.params.x;
        // A ring when it has a thickness, else a filled disc.
        d = select(r, abs(r) - 0.5 * in.params.y, in.params.y > 0.0);
    } else if (in.kind == KIND_SQUARE) {
        d = sd_rounded_box(p - in.points_a.xy, in.params.x, in.params.y);
    } else if (in.kind == KIND_ARROW) {
        let tip = in.points_a.xy;
        let base = in.points_a.zw;
        let side = in.points_b.xy;
        d = sd_triangle(p, tip, base + side, base - side);
    } else {
        d = sd_quad(p, in.points_a.xy, in.points_a.zw, in.points_b.xy, in.points_b.zw);
    }

    d = d - in.params.w;
    let coverage = clamp(0.5 - d, 0.0, 1.0);
    if (coverage <= 0.0) {
        discard;
    }
    // The outline is a band of the given width just inside the edge.
    let outline_width = in.params.z;
    let band = select(0.0, coverage * clamp(0.5 + d + outline_width, 0.0, 1.0), outline_width > 0.0);
    let fill = in.color * (coverage - band);
    let rim = in.outline * band;
    // Premultiplied alpha.
    let rgb = fill.rgb * fill.a + rim.rgb * rim.a;
    let alpha = fill.a + rim.a;
    return vec4<f32>(rgb, alpha);
}
