//! Profile-based meshes for turned objects, polygon extrusions and assembled models.
//! All outputs have outward CCW triangles, UV seams and tangent-space material support.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use glam::{EulerRot, Mat3, Quat, Vec2, Vec3};
use serde::{Deserialize, Serialize};

use super::GeneratedMesh;
use crate::asset::MeshData;
use crate::renderer::{primitives::PrimitiveMeshDescriptor, vertex::v};

pub const MAX_VERTICES: usize = 250_000;
pub const MAX_COMPOSE_PARTS: usize = 128;
const EPS: f32 = 1e-6;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LatheOptions {
    /// [radius, height], ordered bottom to top. Equal heights form ledges.
    pub profile: Vec<[f32; 2]>,
    pub segments: u32,
    pub caps: bool,
    /// Adjacent profile edges exceeding this angle get separate normals.
    pub crease_angle: f32,
    /// Trim distance along profile edges at each corner, clamped to fit short edges.
    pub bevel: f32,
    /// One subdivision gives a chamfer; more subdivisions round the profile corner.
    pub bevel_segments: u32,
}

impl Default for LatheOptions {
    fn default() -> Self {
        Self {
            profile: Vec::new(),
            segments: 64,
            caps: true,
            crease_angle: 45.0,
            bevel: 0.0,
            bevel_segments: 3,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ExtrudeOptions {
    /// Simple XY outline, either winding. Extrudes along Z, centered at Z = 0.
    pub polygon: Vec<[f32; 2]>,
    pub depth: f32,
    pub caps: bool,
    /// Inset distance at both cap rims; zero keeps sharp rims.
    pub bevel: f32,
    pub bevel_segments: u32,
    /// Subdivisions along the extrusion, useful for taper and twist.
    pub steps: u32,
    pub twist: f32,
    /// Scale at the positive Z end relative to the negative Z end.
    pub taper: f32,
}

impl Default for ExtrudeOptions {
    fn default() -> Self {
        Self {
            polygon: Vec::new(),
            depth: 1.0,
            caps: true,
            bevel: 0.0,
            bevel_segments: 1,
            steps: 1,
            twist: 0.0,
            taper: 1.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum Shape {
    Lathe(LatheOptions),
    Extrude(ExtrudeOptions),
    Primitive { primitive: String },
    Sweep(SweepOptions),
    Heightfield(HeightfieldOptions),
}

fn one() -> [f32; 3] {
    [1.0; 3]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshPart {
    #[serde(flatten)]
    pub shape: Shape,
    #[serde(default)]
    pub translation: [f32; 3],
    /// Euler degrees, yaw then pitch then roll, matching remote entity transforms.
    #[serde(default)]
    pub rotation: [f32; 3],
    #[serde(default = "one")]
    pub scale: [f32; 3],
}

fn empty() -> MeshData {
    MeshData {
        vertices: Vec::new(),
        indices: Vec::new(),
    }
}

/// Sweep a circular or simple polygon cross-section along an open 3D polyline.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SweepOptions {
    pub path: Vec<[f32; 3]>,
    /// CCW XY cross-section. Empty selects a circle of radius `radius`.
    pub profile: Vec<[f32; 2]>,
    pub radius: f32,
    pub segments: u32,
    pub caps: bool,
}
impl Default for SweepOptions {
    fn default() -> Self {
        Self {
            path: Vec::new(),
            profile: Vec::new(),
            radius: 0.1,
            segments: 24,
            caps: true,
        }
    }
}

/// XZ grid with explicit row-major heights or seeded fractal noise.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HeightfieldOptions {
    pub resolution: [u32; 2],
    pub size: [f32; 2],
    pub heights: Vec<f32>,
    pub height_scale: f32,
    pub seed: u64,
    pub noise_scale: f32,
    pub octaves: u32,
}
impl Default for HeightfieldOptions {
    fn default() -> Self {
        Self {
            resolution: [64, 64],
            size: [20.0, 20.0],
            heights: Vec::new(),
            height_scale: 3.0,
            seed: 0,
            noise_scale: 4.0,
            octaves: 4,
        }
    }
}

/// Build a tube, cable or polygonal rail. Frames parallel-transport along the path.
/// Sharp bends may intersect the cross-section; this generator does not perform CSG.
///
/// ```
/// use wgpu_cube::procgen::geometry::{sweep, SweepOptions};
/// let mesh = sweep(&SweepOptions {
///     path: vec![[0.0, 0.0, 0.0], [0.0, 0.0, 2.0]],
///     ..Default::default()
/// }).unwrap();
/// assert!(!mesh.indices.is_empty());
/// ```
pub fn sweep(options: &SweepOptions) -> Result<MeshData, String> {
    if !(2..=4096).contains(&options.path.len())
        || !(3..=512).contains(&options.segments)
        || !finite(&[options.radius])
        || options.radius <= 0.0
        || options.path.iter().any(|p| !finite(p))
    {
        return Err(
            "sweep needs 2..=4096 finite path points, radius > 0 and 3..=512 segments".into(),
        );
    }
    let path = options
        .path
        .iter()
        .copied()
        .map(Vec3::from)
        .collect::<Vec<_>>();
    if path.windows(2).any(|p| p[0].distance(p[1]) < EPS) {
        return Err("sweep path contains a zero-length edge".into());
    }
    let mut profile = if options.profile.is_empty() {
        (0..options.segments)
            .map(|i| {
                let a = i as f32 / options.segments as f32 * std::f32::consts::TAU;
                Vec2::new(a.cos(), a.sin()) * options.radius
            })
            .collect::<Vec<_>>()
    } else {
        if !(3..=512).contains(&options.profile.len()) || options.profile.iter().any(|p| !finite(p))
        {
            return Err("profile needs 3..=512 finite points".into());
        }
        options.profile.iter().copied().map(Vec2::from).collect()
    };
    validate_polygon(&profile)?;
    if area(&profile) < 0.0 {
        profile.reverse();
    }
    let n = profile.len();
    if path.len() * (n + 1) + 2 * n > MAX_VERTICES {
        return Err("sweep exceeds vertex budget".into());
    }
    let caps = triangulate(&profile)?;
    let mut data = empty();
    let mut previous_tangent = (path[1] - path[0]).normalize();
    let reference = if previous_tangent.y.abs() < 0.9 {
        Vec3::Y
    } else {
        Vec3::X
    };
    let mut right = reference.cross(previous_tangent).normalize();
    let mut distance = 0.0;
    let mut frames = Vec::new();
    for i in 0..path.len() {
        let tangent = if i == 0 {
            previous_tangent
        } else if i + 1 == path.len() {
            (path[i] - path[i - 1]).normalize()
        } else {
            ((path[i] - path[i - 1]).normalize() + (path[i + 1] - path[i]).normalize())
                .try_normalize()
                .ok_or("sweep cannot reverse direction at a path point")?
        };
        if previous_tangent.dot(tangent) < -0.999 {
            return Err("sweep has a reversing bend".into());
        }
        right = Quat::from_rotation_arc(previous_tangent, tangent) * right;
        right = (right - tangent * right.dot(tangent)).normalize();
        let up = tangent.cross(right).normalize();
        if i > 0 {
            distance += path[i].distance(path[i - 1]);
        }
        frames.push((right, up, tangent));
        for j in 0..=n {
            let point = profile[j % n];
            let before = (point - profile[(j + n - 1) % n]).normalize();
            let after = (profile[(j + 1) % n] - point).normalize();
            let edge = (before + after)
                .try_normalize()
                .ok_or("profile has a reversing corner")?;
            let normal = right * edge.y - up * edge.x;
            let along = right * edge.x + up * edge.y;
            let position = path[i] + right * point.x + up * point.y;
            data.vertices.push(v(
                position.to_array(),
                normal.to_array(),
                [j as f32 / n as f32, distance],
                [along.x, along.y, along.z, 1.0],
            ));
        }
        previous_tangent = tangent;
    }
    for i in 0..path.len() - 1 {
        for j in 0..n {
            let a = (i * (n + 1) + j) as u32;
            let b = a + 1;
            let c = a + (n + 1) as u32;
            let d = c + 1;
            triangle(&mut data, a, b, c);
            triangle(&mut data, b, d, c);
        }
    }
    if options.caps {
        for (i, sign) in [(0, -1.0), (path.len() - 1, 1.0)] {
            let (right, up, tangent) = frames[i];
            let normal = tangent * sign;
            let base = data.vertices.len() as u32;
            for point in &profile {
                let p = path[i] + right * point.x + up * point.y;
                data.vertices.push(v(
                    p.to_array(),
                    normal.to_array(),
                    point.to_array(),
                    [right.x, right.y, right.z, sign],
                ));
            }
            for indices in &caps {
                let [a, b, c] = indices.map(|index| base + index as u32);
                if sign > 0.0 {
                    triangle(&mut data, a, b, c);
                } else {
                    triangle(&mut data, a, c, b);
                }
            }
        }
    }
    check_budget(&data)?;
    Ok(data)
}

/// Build an upward-facing grid with slope normals and tangent-space UVs.
///
/// ```
/// use wgpu_cube::procgen::geometry::{heightfield, HeightfieldOptions};
/// let mesh = heightfield(&HeightfieldOptions {
///     resolution: [2, 2], heights: vec![0.0; 4], ..Default::default()
/// }).unwrap();
/// assert_eq!(mesh.indices.len(), 6);
/// ```
pub fn heightfield(options: &HeightfieldOptions) -> Result<MeshData, String> {
    let [w, h] = options.resolution.map(|n| n as usize);
    if w < 2
        || h < 2
        || w > 512
        || h > 512
        || w * h > MAX_VERTICES
        || !finite(&options.size)
        || options.size.iter().any(|x| *x <= 0.0)
        || !finite(&[options.height_scale, options.noise_scale])
        || options.noise_scale <= 0.0
        || !(1..=12).contains(&options.octaves)
    {
        return Err("heightfield needs resolution 2..=512 within vertex budget, positive size/noise_scale and octaves 1..=12".into());
    }
    if !options.heights.is_empty() && (options.heights.len() != w * h || !finite(&options.heights))
    {
        return Err(
            "heights must contain resolution.x * resolution.y finite values, x fastest".into(),
        );
    }
    let seed_offset = [
        (options.seed & 65535) as f64 * 0.013,
        (options.seed >> 16 & 65535) as f64 * 0.017,
        (options.seed >> 32) as f64 * 0.019,
    ];
    let heights = if options.heights.is_empty() {
        (0..w * h)
            .map(|index| {
                let x = (index % w) as f64 / (w - 1) as f64 * options.noise_scale as f64;
                let z = (index / w) as f64 / (h - 1) as f64 * options.noise_scale as f64;
                super::noise::fbm3(
                    x + seed_offset[0],
                    seed_offset[1] + 0.37,
                    z + seed_offset[2],
                    options.octaves,
                ) as f32
                    * options.height_scale
            })
            .collect::<Vec<_>>()
    } else {
        options
            .heights
            .iter()
            .map(|v| v * options.height_scale)
            .collect()
    };
    let dx = options.size[0] / (w - 1) as f32;
    let dz = options.size[1] / (h - 1) as f32;
    let mut data = empty();
    for z in 0..h {
        for x in 0..w {
            let xl = x.saturating_sub(1);
            let xr = (x + 1).min(w - 1);
            let zl = z.saturating_sub(1);
            let zr = (z + 1).min(h - 1);
            let sx = (heights[z * w + xr] - heights[z * w + xl]) / ((xr - xl) as f32 * dx);
            let sz = (heights[zr * w + x] - heights[zl * w + x]) / ((zr - zl) as f32 * dz);
            let normal = Vec3::new(-sx, 1.0, -sz).normalize();
            let tangent = Vec3::new(1.0, sx, 0.0).normalize();
            data.vertices.push(v(
                [
                    x as f32 * dx - options.size[0] * 0.5,
                    heights[z * w + x],
                    z as f32 * dz - options.size[1] * 0.5,
                ],
                normal.to_array(),
                [x as f32 / (w - 1) as f32, z as f32 / (h - 1) as f32],
                [tangent.x, tangent.y, tangent.z, -1.0],
            ));
        }
    }
    for z in 0..h - 1 {
        for x in 0..w - 1 {
            let a = (z * w + x) as u32;
            let b = a + 1;
            let c = a + w as u32;
            let d = c + 1;
            triangle(&mut data, a, c, b);
            triangle(&mut data, b, c, d);
        }
    }
    check_budget(&data)?;
    Ok(data)
}

fn finite(values: &[f32]) -> bool {
    values.iter().all(|x| x.is_finite() && x.abs() <= 1e5)
}

fn check_budget(data: &MeshData) -> Result<(), String> {
    if data.vertices.len() > MAX_VERTICES || data.indices.len() > MAX_VERTICES * 6 {
        return Err(format!(
            "mesh exceeds {MAX_VERTICES} vertices or {} indices",
            MAX_VERTICES * 6
        ));
    }
    if data
        .vertices
        .iter()
        .any(|v| !finite(&v.pos) || !v.normal.iter().all(|x| x.is_finite()))
    {
        return Err("geometry or transform produced nonfinite or out-of-range vertices".into());
    }
    Ok(())
}

/// Hash the actual output with a generator-specific namespace, sharing identical GPU meshes.
pub fn generated(data: MeshData) -> Arc<GeneratedMesh> {
    let mut hash = DefaultHasher::new();
    "profile_geometry_v1".hash(&mut hash);
    bytemuck::cast_slice::<_, u8>(&data.vertices).hash(&mut hash);
    data.indices.hash(&mut hash);
    GeneratedMesh::new(hash.finish(), data)
}

fn triangle(data: &mut MeshData, a: u32, b: u32, c: u32) {
    let pa = Vec3::from(data.vertices[a as usize].pos);
    let pb = Vec3::from(data.vertices[b as usize].pos);
    let pc = Vec3::from(data.vertices[c as usize].pos);
    if (pb - pa).cross(pc - pa).length_squared() > 1e-20 {
        data.indices.extend([a, b, c]);
    }
}

pub fn lathe(options: &LatheOptions) -> Result<MeshData, String> {
    if !(2..=512).contains(&options.profile.len()) || !(3..=512).contains(&options.segments) {
        return Err("lathe needs 2..=512 profile points and 3..=512 segments".into());
    }
    if !finite(&[options.bevel])
        || options.bevel < 0.0
        || !(1..=8).contains(&options.bevel_segments)
    {
        return Err("bevel must be finite and nonnegative; bevel_segments must be 1..=8".into());
    }
    if !options.crease_angle.is_finite() || !(0.0..=180.0).contains(&options.crease_angle) {
        return Err("crease_angle must be 0..=180 degrees".into());
    }
    if options
        .profile
        .iter()
        .any(|point| !finite(point) || point[0] < 0.0)
    {
        return Err("profile needs finite nonnegative radii and finite heights".into());
    }
    // Validate the original outline before rounding; a fillet must not repair invalid input.
    for pair in options.profile.windows(2) {
        let delta = Vec2::from(pair[1]) - Vec2::from(pair[0]);
        if delta.length() <= EPS || delta.y < 0.0 || (pair[0][0] == 0.0 && pair[1][0] == 0.0) {
            return Err(
                "profile must rise bottom to top, with distinct points and no axis-only edge"
                    .into(),
            );
        }
    }
    let profile = rounded_profile(options);
    let p = &profile;
    let mut edge_normals = Vec::new();
    let mut distance = vec![0.0];
    for pair in p.windows(2) {
        let delta = Vec2::from(pair[1]) - Vec2::from(pair[0]);
        if delta.length() <= EPS || delta.y < 0.0 || (pair[0][0] == 0.0 && pair[1][0] == 0.0) {
            return Err(
                "profile must rise bottom to top, with distinct points and no axis-only edge"
                    .into(),
            );
        }
        edge_normals.push(Vec2::new(delta.y, -delta.x).normalize());
        distance.push(distance.last().unwrap() + delta.length());
    }
    if p.last().unwrap()[1] - p[0][1] <= EPS {
        return Err("lathe profile must have positive height".into());
    }
    let n = options.segments as usize;
    let cos_crease = options.crease_angle.to_radians().cos();
    let smooth = |edge: usize, point: usize| {
        let own = edge_normals[edge];
        let neighbour = if point == edge {
            edge.checked_sub(1)
        } else {
            (edge + 1 < edge_normals.len()).then_some(edge + 1)
        };
        neighbour
            .map(|other| {
                let normal = edge_normals[other];
                if own.dot(normal) >= cos_crease {
                    (own + normal).normalize_or_zero()
                } else {
                    own
                }
            })
            .unwrap_or(own)
    };
    let mut data = empty();
    let mut previous_upper: Option<(Vec2, u32)> = None;
    for edge in 0..p.len() - 1 {
        let mut rows = [0; 2];
        for (row, point) in [edge, edge + 1].into_iter().enumerate() {
            let normal = smooth(edge, point);
            // Smooth neighbouring edges have the same UV and frame at their join.
            // Reuse that ring; split rings remain necessary at actual creases.
            if row == 0 {
                if let Some((previous_normal, start)) = previous_upper {
                    if previous_normal == normal {
                        rows[row] = start;
                        continue;
                    }
                }
            }
            if data.vertices.len() + n + 1 > MAX_VERTICES {
                return Err("lathe resolution exceeds vertex limit".into());
            }
            rows[row] = data.vertices.len() as u32;
            for segment in 0..=n {
                // Exact seam position and normal prevent tiny cracks after export.
                let theta = std::f32::consts::TAU * (segment % n) as f32 / n as f32;
                let (s, c) = theta.sin_cos();
                data.vertices.push(v(
                    [p[point][0] * c, p[point][1], p[point][0] * s],
                    [normal.x * c, normal.y, normal.x * s],
                    [
                        segment as f32 / n as f32,
                        distance[point] / distance.last().unwrap(),
                    ],
                    [-s, 0.0, c, -1.0],
                ));
            }
            if row == 1 {
                previous_upper = Some((normal, rows[row]));
            }
        }
        for j in 0..n as u32 {
            let a = rows[0] + j;
            let b = rows[1] + j;
            triangle(&mut data, a, b, a + 1);
            triangle(&mut data, a + 1, b, b + 1);
        }
    }
    if options.caps {
        for (point, sign) in [(p[0], -1.0), (*p.last().unwrap(), 1.0)] {
            if point[0] <= EPS {
                continue;
            }
            let base = data.vertices.len() as u32;
            data.vertices.push(v(
                [0.0, point[1], 0.0],
                [0.0, sign, 0.0],
                [0.5, 0.5],
                [1.0, 0.0, 0.0, -sign],
            ));
            for j in 0..=n {
                let theta = std::f32::consts::TAU * (j % n) as f32 / n as f32;
                let (s, c) = theta.sin_cos();
                data.vertices.push(v(
                    [point[0] * c, point[1], point[0] * s],
                    [0.0, sign, 0.0],
                    [0.5 + c * 0.5, 0.5 + s * 0.5],
                    [1.0, 0.0, 0.0, -sign],
                ));
            }
            for j in 0..n as u32 {
                if sign > 0.0 {
                    triangle(&mut data, base, base + j + 2, base + j + 1);
                } else {
                    triangle(&mut data, base, base + j + 1, base + j + 2);
                }
            }
        }
    }
    check_budget(&data)?;
    Ok(data)
}

fn rounded_profile(options: &LatheOptions) -> Vec<[f32; 2]> {
    if options.bevel == 0.0 {
        return options.profile.clone();
    }
    let mut profile = vec![options.profile[0]];
    for i in 1..options.profile.len() - 1 {
        let previous = Vec2::from(options.profile[i - 1]);
        let corner = Vec2::from(options.profile[i]);
        let next = Vec2::from(options.profile[i + 1]);
        let incoming = corner - previous;
        let outgoing = next - corner;
        // Leave poles and already straight edges alone. Adjacent trims cannot overlap.
        if corner.x <= EPS || incoming.normalize().dot(outgoing.normalize()) > 0.9999 {
            profile.push(corner.to_array());
            continue;
        }
        let trim = options
            .bevel
            .min(incoming.length() * 0.49)
            .min(outgoing.length() * 0.49);
        if trim <= EPS * 2.0 {
            profile.push(corner.to_array());
            continue;
        }
        let a = corner - incoming.normalize() * trim;
        let b = corner + outgoing.normalize() * trim;
        for segment in 0..=options.bevel_segments {
            let t = segment as f32 / options.bevel_segments as f32;
            let point = a.lerp(corner, t).lerp(corner.lerp(b, t), t);
            profile.push(point.to_array());
        }
    }
    profile.push(*options.profile.last().unwrap());
    profile
}

fn cross(a: Vec2, b: Vec2, c: Vec2) -> f32 {
    (b - a).perp_dot(c - a)
}

fn area(p: &[Vec2]) -> f32 {
    (0..p.len())
        .map(|i| p[i].perp_dot(p[(i + 1) % p.len()]))
        .sum::<f32>()
        * 0.5
}

fn intersects(a: Vec2, b: Vec2, c: Vec2, d: Vec2) -> bool {
    let on = |a: Vec2, b: Vec2, p: Vec2| {
        cross(a, b, p).abs() <= EPS
            && p.cmpge(a.min(b) - Vec2::splat(EPS)).all()
            && p.cmple(a.max(b) + Vec2::splat(EPS)).all()
    };
    (cross(a, b, c) * cross(a, b, d) < 0.0 && cross(c, d, a) * cross(c, d, b) < 0.0)
        || on(a, b, c)
        || on(a, b, d)
        || on(c, d, a)
        || on(c, d, b)
}

fn validate_polygon(p: &[Vec2]) -> Result<(), String> {
    if area(p).abs() <= EPS {
        return Err("polygon has zero area".into());
    }
    for i in 0..p.len() {
        let next = (i + 1) % p.len();
        if p[i].distance(p[next]) <= EPS {
            return Err("polygon has a duplicate or zero-length edge".into());
        }
        for j in i + 1..p.len() {
            let other = (j + 1) % p.len();
            if next == j || other == i {
                continue;
            }
            if intersects(p[i], p[next], p[j], p[other]) {
                return Err("polygon intersects itself (holes are not supported)".into());
            }
        }
    }
    Ok(())
}

/// Ear clipping supports concave silhouettes, unlike triangle fans.
fn triangulate(p: &[Vec2]) -> Result<Vec<[usize; 3]>, String> {
    let mut remaining: Vec<usize> = (0..p.len()).collect();
    let mut tris = Vec::new();
    while remaining.len() > 3 {
        let mut ear = None;
        for i in 0..remaining.len() {
            let a = remaining[(i + remaining.len() - 1) % remaining.len()];
            let b = remaining[i];
            let c = remaining[(i + 1) % remaining.len()];
            if cross(p[a], p[b], p[c]) <= EPS {
                continue;
            }
            if remaining.iter().any(|&q| {
                q != a
                    && q != b
                    && q != c
                    && cross(p[a], p[b], p[q]) >= -EPS
                    && cross(p[b], p[c], p[q]) >= -EPS
                    && cross(p[c], p[a], p[q]) >= -EPS
            }) {
                continue;
            }
            ear = Some((i, [a, b, c]));
            break;
        }
        let (index, tri) =
            ear.ok_or("cannot triangulate polygon; remove collinear points or very small edges")?;
        tris.push(tri);
        remaining.remove(index);
    }
    tris.push([remaining[0], remaining[1], remaining[2]]);
    Ok(tris)
}

fn inset_polygon(p: &[Vec2], amount: f32) -> Result<Vec<Vec2>, String> {
    let mut inset = Vec::new();
    for i in 0..p.len() {
        let prev = (p[i] - p[(i + p.len() - 1) % p.len()]).normalize();
        let next = (p[(i + 1) % p.len()] - p[i]).normalize();
        let a = Vec2::new(-prev.y, prev.x);
        let b = Vec2::new(-next.y, next.x);
        let denom = 1.0 + a.dot(b);
        if denom < EPS {
            return Err("polygon has a reversing edge".into());
        }
        inset.push(p[i] + (a + b) * (amount / denom));
    }
    validate_polygon(&inset).map_err(|e| format!("bevel too large: {e}"))?;
    if area(&inset) <= EPS || area(&inset) >= area(p) {
        return Err("bevel too large for polygon".into());
    }
    for i in 0..p.len() {
        let j = (i + 1) % p.len();
        if (inset[j] - inset[i]).dot(p[j] - p[i]) <= 0.0 {
            return Err("bevel collapses a polygon edge; reduce bevel".into());
        }
    }
    Ok(inset)
}

pub fn extrude(options: &ExtrudeOptions) -> Result<MeshData, String> {
    if !(3..=256).contains(&options.polygon.len()) || options.polygon.iter().any(|p| !finite(p)) {
        return Err("extrusion needs 3..=256 finite XY polygon points".into());
    }
    if !finite(&[options.depth, options.bevel, options.twist, options.taper])
        || options.depth <= EPS
        || options.bevel < 0.0
        || options.bevel * 2.0 >= options.depth
        || options.taper <= EPS
        || options.taper > 100.0
        || !(1..=256).contains(&options.steps)
        || !(1..=8).contains(&options.bevel_segments)
    {
        return Err("depth/taper must be positive, bevel < depth/2, steps 1..=256, bevel_segments 1..=8; use finite values".into());
    }
    let mut p: Vec<Vec2> = options.polygon.iter().copied().map(Vec2::from).collect();
    // A repeated closing point is conventional and harmless.
    if p[0].distance(*p.last().unwrap()) <= EPS {
        p.pop();
    }
    if p.len() < 3 {
        return Err("polygon needs three distinct points".into());
    }
    validate_polygon(&p)?;
    if area(&p) < 0.0 {
        p.reverse();
    }
    // Remove straight intermediate points so valid silhouettes triangulate reliably.
    loop {
        let redundant = (0..p.len()).find(|&i| {
            cross(p[(i + p.len() - 1) % p.len()], p[i], p[(i + 1) % p.len()]).abs() <= EPS
        });
        match redundant {
            Some(i) if p.len() > 3 => {
                p.remove(i);
            }
            _ => break,
        }
    }
    let inset = if options.bevel > 0.0 {
        inset_polygon(&p, options.bevel)?
    } else {
        p.clone()
    };
    let mut rings = Vec::new();
    let half = options.depth * 0.5;
    if options.bevel > 0.0 {
        for i in 0..options.bevel_segments {
            let t = i as f32 / options.bevel_segments as f32;
            let angle = t * std::f32::consts::FRAC_PI_2;
            rings.push((
                -half + options.bevel * (1.0 - angle.cos()),
                1.0 - angle.sin(),
            ));
        }
    }
    for i in 0..=options.steps {
        let t = i as f32 / options.steps as f32;
        rings.push((
            -half + options.bevel + (options.depth - 2.0 * options.bevel) * t,
            0.0,
        ));
    }
    if options.bevel > 0.0 {
        for i in 1..=options.bevel_segments {
            let t = i as f32 / options.bevel_segments as f32;
            let angle = t * std::f32::consts::FRAC_PI_2;
            // Exact endpoint inset keeps side/cap positions coincident.
            let inset = if i == options.bevel_segments {
                1.0
            } else {
                1.0 - angle.cos()
            };
            rings.push((half - options.bevel + options.bevel * angle.sin(), inset));
        }
    }
    if (rings.len() - 1) * p.len() * 6 + p.len() * 2 > MAX_VERTICES {
        return Err("extrusion resolution exceeds vertex limit".into());
    }
    let positions: Vec<Vec<Vec3>> = rings
        .iter()
        .map(|&(z, amount)| {
            let t = (z + half) / options.depth;
            let theta = options.twist.to_radians() * t;
            let (s, c) = theta.sin_cos();
            let scale = 1.0 + (options.taper - 1.0) * t;
            p.iter()
                .zip(&inset)
                .map(|(&a, &b)| {
                    let q = a.lerp(b, amount) * scale;
                    Vec3::new(q.x * c - q.y * s, q.x * s + q.y * c, z)
                })
                .collect()
        })
        .collect();
    let mut perimeter = vec![0.0];
    for i in 0..p.len() {
        perimeter.push(perimeter[i] + p[i].distance(p[(i + 1) % p.len()]));
    }
    let mut data = empty();
    for ring in 0..positions.len() - 1 {
        for i in 0..p.len() {
            let j = (i + 1) % p.len();
            let corners = [
                positions[ring][i],
                positions[ring][j],
                positions[ring + 1][j],
                positions[ring + 1][i],
            ];
            // Each triangle has its own frame: twist can make side quads nonplanar.
            let u0 = perimeter[i] / perimeter.last().unwrap();
            let u1 = perimeter[i + 1] / perimeter.last().unwrap();
            let v0 = (rings[ring].0 + half) / options.depth;
            let v1 = (rings[ring + 1].0 + half) / options.depth;
            let uv = [[u0, v0], [u1, v0], [u1, v1], [u0, v1]];
            for tri in [[0, 1, 2], [0, 2, 3]] {
                let a = corners[tri[0]];
                let b = corners[tri[1]];
                let c = corners[tri[2]];
                let n = (b - a).cross(c - a).normalize_or_zero();
                let duv1 = Vec2::from(uv[tri[1]]) - Vec2::from(uv[tri[0]]);
                let duv2 = Vec2::from(uv[tri[2]]) - Vec2::from(uv[tri[0]]);
                let tangent = ((b - a) * duv2.y - (c - a) * duv1.y).normalize_or_zero();
                let base = data.vertices.len() as u32;
                for q in tri {
                    data.vertices.push(v(
                        corners[q].to_array(),
                        n.to_array(),
                        uv[q],
                        [tangent.x, tangent.y, tangent.z, 1.0],
                    ));
                }
                triangle(&mut data, base, base + 1, base + 2);
            }
        }
    }
    if options.caps {
        let tris = triangulate(&inset)?;
        let min = inset.iter().copied().reduce(Vec2::min).unwrap();
        let max = inset.iter().copied().reduce(Vec2::max).unwrap();
        for (ring, sign) in [(0, -1.0), (positions.len() - 1, 1.0)] {
            let theta = if sign > 0.0 {
                options.twist.to_radians()
            } else {
                0.0
            };
            let base = data.vertices.len() as u32;
            for (i, q) in inset.iter().enumerate() {
                data.vertices.push(v(
                    positions[ring][i].to_array(),
                    [0.0, 0.0, sign],
                    ((*q - min) / (max - min)).to_array(),
                    [theta.cos(), theta.sin(), 0.0, sign],
                ));
            }
            for &[a, b, c] in &tris {
                if sign > 0.0 {
                    triangle(&mut data, base + a as u32, base + b as u32, base + c as u32);
                } else {
                    triangle(&mut data, base + a as u32, base + c as u32, base + b as u32);
                }
            }
        }
    }
    check_budget(&data)?;
    Ok(data)
}

/// Assemble transformed parts into a single mesh. This joins surfaces without a boolean union.
pub fn compose(parts: &[MeshPart]) -> Result<MeshData, String> {
    if !(1..=MAX_COMPOSE_PARTS).contains(&parts.len()) {
        return Err(format!("compose needs 1..={MAX_COMPOSE_PARTS} parts"));
    }
    let mut combined = empty();
    for (index, part) in parts.iter().enumerate() {
        let mut data = match &part.shape {
            Shape::Lathe(options) => lathe(options),
            Shape::Extrude(options) => extrude(options),
            Shape::Sweep(options) => sweep(options),
            Shape::Heightfield(options) => heightfield(options),
            Shape::Primitive { primitive } => {
                let descriptor = match primitive.as_str() {
                    "cube" => PrimitiveMeshDescriptor::Cube,
                    "sphere" => PrimitiveMeshDescriptor::Sphere,
                    "plane" => PrimitiveMeshDescriptor::Plane,
                    "cylinder" => PrimitiveMeshDescriptor::Cylinder,
                    "cone" => PrimitiveMeshDescriptor::Cone,
                    "torus" => PrimitiveMeshDescriptor::Torus,
                    _ => return Err(format!("unknown primitive {primitive:?} in part {index}")),
                };
                Ok(descriptor.mesh_data())
            }
        }
        .map_err(|err| format!("part {index}: {err}"))?;
        if !finite(&part.translation)
            || !finite(&part.rotation)
            || !finite(&part.scale)
            || part.scale.iter().any(|s| s.abs() <= EPS)
        {
            return Err(format!(
                "part {index}: transform must be finite with nonzero scale"
            ));
        }
        if combined.vertices.len() + data.vertices.len() > MAX_VERTICES
            || combined.indices.len() + data.indices.len() > MAX_VERTICES * 6
        {
            return Err("composed mesh exceeds vertex/index limit".into());
        }
        let rotation = Quat::from_euler(
            EulerRot::YXZ,
            part.rotation[1].to_radians(),
            part.rotation[0].to_radians(),
            part.rotation[2].to_radians(),
        );
        let matrix = Mat3::from_quat(rotation) * Mat3::from_diagonal(Vec3::from(part.scale));
        let normals = matrix.inverse().transpose();
        let mirrored = part.scale.iter().product::<f32>() < 0.0;
        for vertex in &mut data.vertices {
            vertex.pos =
                (matrix * Vec3::from(vertex.pos) + Vec3::from(part.translation)).to_array();
            let normal = (normals * Vec3::from(vertex.normal)).normalize_or_zero();
            let tangent =
                matrix * Vec3::new(vertex.tangent[0], vertex.tangent[1], vertex.tangent[2]);
            let tangent = (tangent - normal * normal.dot(tangent)).normalize_or_zero();
            vertex.normal = normal.to_array();
            vertex.tangent = [
                tangent.x,
                tangent.y,
                tangent.z,
                vertex.tangent[3] * if mirrored { -1.0 } else { 1.0 },
            ];
        }
        let offset = combined.vertices.len() as u32;
        if mirrored {
            for tri in data.indices.as_chunks_mut::<3>().0 {
                tri.swap(1, 2);
            }
        }
        combined
            .indices
            .extend(data.indices.iter().map(|i| i + offset));
        combined.vertices.extend(data.vertices);
    }
    check_budget(&combined)?;
    Ok(combined)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sweeps_keep_outward_winding_and_reject_reversing_paths() {
        let options = SweepOptions {
            path: vec![[0., 0., 0.], [0., 0., 2.], [0.5, 0., 3.]],
            ..Default::default()
        };
        assert_frames_and_winding(&sweep(&options).unwrap());
        assert!(sweep(&SweepOptions {
            path: vec![[0., 0., 0.], [0., 0., 1.], [0., 0., 0.]],
            ..Default::default()
        })
        .is_err());
    }

    #[test]
    fn heightfields_reproduce_seeds_and_match_sloped_normals() {
        let options = HeightfieldOptions {
            resolution: [3, 3],
            size: [2., 2.],
            heights: vec![0., 1., 2., 0., 1., 2., 0., 1., 2.],
            height_scale: 1.,
            ..Default::default()
        };
        let mesh = heightfield(&options).unwrap();
        assert_frames_and_winding(&mesh);
        assert!(Vec3::from(mesh.vertices[4].normal)
            .abs_diff_eq(Vec3::new(-1., 1., 0.).normalize(), 1e-6));
        let options = HeightfieldOptions {
            seed: 12,
            ..Default::default()
        };
        assert_eq!(
            generated(heightfield(&options).unwrap()).key,
            generated(heightfield(&options).unwrap()).key
        );
        assert_ne!(
            generated(heightfield(&options).unwrap()).key,
            generated(
                heightfield(&HeightfieldOptions {
                    seed: 13,
                    ..options
                })
                .unwrap()
            )
            .key
        );
    }

    fn assert_frames_and_winding(mesh: &MeshData) {
        assert!(!mesh.indices.is_empty());
        for vertex in &mesh.vertices {
            let n = Vec3::from(vertex.normal);
            let t = Vec3::new(vertex.tangent[0], vertex.tangent[1], vertex.tangent[2]);
            assert!((n.length() - 1.0).abs() < 1e-4, "normal {n:?}");
            assert!((t.length() - 1.0).abs() < 1e-4, "tangent {t:?}");
            assert!(n.dot(t).abs() < 1e-4);
            assert!(vertex.uv.iter().all(|x| x.is_finite()));
        }
        for tri in mesh.indices.as_chunks::<3>().0 {
            let a = &mesh.vertices[tri[0] as usize];
            let b = &mesh.vertices[tri[1] as usize];
            let c = &mesh.vertices[tri[2] as usize];
            let face = (Vec3::from(b.pos) - Vec3::from(a.pos))
                .cross(Vec3::from(c.pos) - Vec3::from(a.pos));
            assert!(face.length_squared() > 1e-20);
            assert!(
                face.dot(Vec3::from(a.normal) + Vec3::from(b.normal) + Vec3::from(c.normal)) > 0.0
            );
        }
    }

    #[test]
    fn lathe_handles_caps_poles_ledges_and_uv_seams() {
        let mesh = lathe(&LatheOptions {
            profile: vec![[0.8, 0.0], [0.8, 0.2], [0.4, 0.2], [0.4, 1.0], [0.0, 1.5]],
            segments: 32,
            ..Default::default()
        })
        .unwrap();
        assert_frames_and_winding(&mesh);
        assert_eq!(mesh.vertices[0].pos, mesh.vertices[32].pos);
        assert_eq!(mesh.vertices[0].normal, mesh.vertices[32].normal);
        assert_eq!(mesh.vertices[0].uv[0], 0.0);
        assert_eq!(mesh.vertices[32].uv[0], 1.0);
        let volume: f32 = mesh
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|t| {
                Vec3::from(mesh.vertices[t[0] as usize].pos).dot(
                    Vec3::from(mesh.vertices[t[1] as usize].pos)
                        .cross(Vec3::from(mesh.vertices[t[2] as usize].pos)),
                ) / 6.0
            })
            .sum();
        assert!(volume > 0.5);
    }

    #[test]
    fn concave_extrusion_cap_area_matches_outline_in_both_windings() {
        let polygon = vec![[0., 0.], [3., 0.], [3., 1.], [1., 1.], [1., 3.], [0., 3.]];
        for reverse in [false, true] {
            let mut polygon = polygon.clone();
            if reverse {
                polygon.reverse();
            }
            let mesh = extrude(&ExtrudeOptions {
                polygon,
                depth: 2.0,
                ..Default::default()
            })
            .unwrap();
            assert_frames_and_winding(&mesh);
            let cap_area: f32 = mesh
                .indices
                .as_chunks::<3>()
                .0
                .iter()
                .filter_map(|t| {
                    let a = &mesh.vertices[t[0] as usize];
                    if a.normal[2] != 1.0 {
                        return None;
                    }
                    Some(
                        (Vec3::from(mesh.vertices[t[1] as usize].pos) - Vec3::from(a.pos))
                            .cross(Vec3::from(mesh.vertices[t[2] as usize].pos) - Vec3::from(a.pos))
                            .length()
                            * 0.5,
                    )
                })
                .sum();
            assert!((cap_area - 5.0).abs() < 1e-5);
        }
    }

    #[test]
    fn bevel_twist_taper_and_mirrored_nonuniform_transforms_keep_frames() {
        let options = ExtrudeOptions {
            polygon: vec![[-1., -1.], [1., -1.], [1., 1.], [-1., 1.]],
            depth: 2.,
            bevel: 0.1,
            bevel_segments: 3,
            steps: 12,
            twist: 40.,
            taper: 0.6,
            ..Default::default()
        };
        assert_frames_and_winding(&extrude(&options).unwrap());
        let part = MeshPart {
            shape: Shape::Extrude(options),
            translation: [1., 2., 3.],
            rotation: [12., 25., 30.],
            scale: [-2., 0.5, 1.],
        };
        let mesh = compose(&[part]).unwrap();
        assert_frames_and_winding(&mesh);
        assert_eq!(generated(mesh.clone()).key, generated(mesh).key);
    }

    #[test]
    fn lathe_bevels_round_corners_without_changing_endpoints_or_creating_gaps() {
        let original = vec![[0.4, 0.0], [0.4, 0.2], [0.2, 0.2], [0.2, 1.0], [0.0, 1.2]];
        let options = LatheOptions {
            profile: original.clone(),
            bevel: 0.04,
            bevel_segments: 4,
            ..Default::default()
        };
        let profile = rounded_profile(&options);
        assert_eq!(profile[0], original[0]);
        assert_eq!(profile.last(), original.last());
        assert!(!profile.contains(&original[1]));
        assert!(!profile.contains(&original[2]));
        assert!(profile.windows(2).all(|pair| pair[1][1] >= pair[0][1]));
        assert_frames_and_winding(&lathe(&options).unwrap());
        // Oversized bevels clamp to each feature instead of overlapping neighbouring trims.
        assert_frames_and_winding(
            &lathe(&LatheOptions {
                bevel: 10.0,
                ..options
            })
            .unwrap(),
        );
    }

    #[test]
    fn rejects_invalid_profiles_outlines_bevels_and_allocations() {
        for profile in [
            vec![[1., 1.], [1., 0.]],
            vec![[1., 0.], [1., 0.]],
            vec![[-1., 0.], [1., 1.]],
            vec![[0., 0.], [0., 1.]],
        ] {
            assert!(lathe(&LatheOptions {
                profile,
                ..Default::default()
            })
            .is_err());
        }
        assert!(lathe(&LatheOptions {
            profile: (0..512).map(|i| [1., i as f32]).collect(),
            segments: 512,
            ..Default::default()
        })
        .is_err());
        assert!(extrude(&ExtrudeOptions {
            polygon: vec![[0., 0.], [2., 2.], [0., 2.], [2., 0.]],
            ..Default::default()
        })
        .is_err());
        assert!(extrude(&ExtrudeOptions {
            polygon: vec![[0., 0.], [1., 0.], [1., 1.], [0., 1.]],
            depth: 4.,
            bevel: 0.6,
            ..Default::default()
        })
        .is_err());
        assert!(compose(&[]).is_err());
    }
}
