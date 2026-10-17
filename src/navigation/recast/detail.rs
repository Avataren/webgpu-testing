//! Height detail: each polygon gets a small triangle mesh that follows the floor under
//! it (polygon vertices only sit on its outline, so a hill or a flight of stairs inside a
//! polygon would otherwise be flattened). Edges are tessellated the same way from both
//! sides, so neighbouring polygons' detail meets without cracks.

use glam::Vec3;

use super::compact::CompactHeightfield;
use super::polymesh::PolyMesh;
use super::NULL_AREA;

const MAX_VERTS: usize = 127;
const MAX_EDGE_SAMPLES: usize = 32;

/// The detail meshes of every polygon of a [`PolyMesh`].
#[derive(Debug, Clone, Default)]
pub(crate) struct PolyMeshDetail {
    /// Per polygon: first vertex, vertex count, first triangle, triangle count. The
    /// first vertices of each are the polygon's own, in order.
    pub meshes: Vec<[u32; 4]>,
    /// World space.
    pub verts: Vec<Vec3>,
    /// Indices into the polygon's own vertex range.
    pub tris: Vec<[u8; 3]>,
}

struct Sampler<'a> {
    chf: &'a CompactHeightfield,
    /// How far (m) a floor may be from the expected height and still count.
    tolerance: f32,
}

impl Sampler<'_> {
    /// The floor height (m) at world `x`, `z` nearest `expected`, preferring spans of
    /// region `reg`.
    fn height(&self, x: f32, z: f32, expected: f32, reg: u16) -> Option<f32> {
        let chf = self.chf;
        let ix = ((x - chf.bmin.x) / chf.cs).floor() as i32;
        let iz = ((z - chf.bmin.z) / chf.cs).floor() as i32;
        let mut best: Option<(f32, f32, bool)> = None;
        // The cell itself first; its neighbours when it holds nothing suitable (samples
        // on an outline fall between cells).
        for ring in 0..2 {
            for dz in -ring..=ring {
                for dx in -ring..=ring {
                    if ring == 1 && dx == 0 && dz == 0 {
                        continue;
                    }
                    let (cx, cz) = (ix + dx, iz + dz);
                    if cx < 0 || cz < 0 || cx >= chf.width || cz >= chf.height {
                        continue;
                    }
                    let cell = chf.cell(cx, cz);
                    for i in cell.index..cell.index + cell.count {
                        let i = i as usize;
                        if chf.areas[i] == NULL_AREA {
                            continue;
                        }
                        let span = chf.spans[i];
                        let y = span.top;
                        let error = (y - expected).abs();
                        if error > self.tolerance {
                            continue;
                        }
                        let same_region = reg != 0 && span.reg == reg;
                        let better = match best {
                            None => true,
                            Some((_, best_error, best_same)) => {
                                (same_region && !best_same)
                                    || (same_region == best_same && error < best_error)
                            }
                        };
                        if better {
                            best = Some((y, error, same_region));
                        }
                    }
                }
            }
            if best.is_some_and(|(_, _, same)| same) {
                break;
            }
        }
        best.map(|(y, _, _)| y)
    }
}

fn dist2_xz(a: Vec3, b: Vec3) -> f32 {
    let dx = a.x - b.x;
    let dz = a.z - b.z;
    dx * dx + dz * dz
}

/// Twice the signed area of a, b, c on the x/z plane.
fn cross_xz(a: Vec3, b: Vec3, c: Vec3) -> f32 {
    (b.x - a.x) * (c.z - a.z) - (c.x - a.x) * (b.z - a.z)
}

/// The height at `p` on triangle a, b, c and the smallest barycentric weight, if p's
/// x/z lies inside (with `eps` slack).
fn height_on_triangle(p: Vec3, a: Vec3, b: Vec3, c: Vec3, eps: f32) -> Option<(f32, f32)> {
    let area = cross_xz(a, b, c);
    if area.abs() < 1e-9 {
        return None;
    }
    let wa = cross_xz(b, c, p) / area;
    let wb = cross_xz(c, a, p) / area;
    let wc = 1.0 - wa - wb;
    if wa < -eps || wb < -eps || wc < -eps {
        return None;
    }
    Some((a.y * wa + b.y * wb + c.y * wc, wa.min(wb).min(wc)))
}

/// Whether `d` is inside the circumcircle of a, b, c (x/z), whatever their winding.
fn in_circumcircle(a: Vec3, b: Vec3, c: Vec3, d: Vec3) -> bool {
    let (ax, az) = (a.x - d.x, a.z - d.z);
    let (bx, bz) = (b.x - d.x, b.z - d.z);
    let (cx, cz) = (c.x - d.x, c.z - d.z);
    let det = (ax * ax + az * az) * (bx * cz - cx * bz) - (bx * bx + bz * bz) * (ax * cz - cx * az)
        + (cx * cx + cz * cz) * (ax * bz - bx * az);
    let orient = cross_xz(a, b, c);
    det * orient.signum() > 1e-9
}

/// Distance from `p` to the polygon's outline, negative inside (x/z).
fn signed_distance_to_poly(p: Vec3, poly: &[Vec3]) -> f32 {
    let mut inside = false;
    let mut dmin = f32::MAX;
    let n = poly.len();
    let mut j = n - 1;
    for i in 0..n {
        let (vi, vj) = (poly[i], poly[j]);
        if ((vi.z > p.z) != (vj.z > p.z))
            && (p.x < (vj.x - vi.x) * (p.z - vi.z) / (vj.z - vi.z) + vi.x)
        {
            inside = !inside;
        }
        dmin = dmin.min(distance_pt_seg_xz(p, vj, vi));
        j = i;
    }
    if inside {
        -dmin
    } else {
        dmin
    }
}

fn distance_pt_seg_xz(p: Vec3, a: Vec3, b: Vec3) -> f32 {
    let (abx, abz) = (b.x - a.x, b.z - a.z);
    let (apx, apz) = (p.x - a.x, p.z - a.z);
    let d = abx * abx + abz * abz;
    let t = if d > 0.0 {
        ((abx * apx + abz * apz) / d).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (dx, dz) = (a.x + abx * t - p.x, a.z + abz * t - p.z);
    (dx * dx + dz * dz).sqrt()
}

/// Triangulate a convex outline whose first `original` indices are corners (the rest
/// lie on its edges), zig-zagging from the ear with the shortest perimeter.
fn triangulate_hull(verts: &[Vec3], hull: &[usize], original: usize) -> Vec<[usize; 3]> {
    let n = hull.len();
    let mut tris = Vec::with_capacity(n.saturating_sub(2));
    if n < 3 {
        return tris;
    }
    let next = |i: usize| (i + 1) % n;
    let prev = |i: usize| (i + n - 1) % n;
    let (mut start, mut left, mut right) = (0, 1, n - 1);
    let mut dmin = f32::MAX;
    for i in 0..n {
        if hull[i] >= original {
            continue;
        }
        let (pi, ni) = (prev(i), next(i));
        let (pv, cv, nv) = (verts[hull[pi]], verts[hull[i]], verts[hull[ni]]);
        let d = dist2_xz(pv, cv) + dist2_xz(cv, nv) + dist2_xz(nv, pv);
        if d < dmin {
            start = i;
            left = ni;
            right = pi;
            dmin = d;
        }
    }
    tris.push([hull[start], hull[left], hull[right]]);
    while next(left) != right {
        let nleft = next(left);
        let nright = prev(right);
        let (cvl, nvl, cvr, nvr) = (
            verts[hull[left]],
            verts[hull[nleft]],
            verts[hull[right]],
            verts[hull[nright]],
        );
        let dleft = dist2_xz(cvl, nvl) + dist2_xz(nvl, cvr);
        let dright = dist2_xz(cvr, nvr) + dist2_xz(cvl, nvr);
        if dleft < dright {
            tris.push([hull[left], hull[nleft], hull[right]]);
            left = nleft;
        } else {
            tris.push([hull[left], hull[nright], hull[right]]);
            right = nright;
        }
    }
    tris
}

/// Insert `p` (index `pi`) into the triangle `t` it lies in, then restore the Delaunay
/// property by flipping edges (never the outline's: they have no neighbour).
fn insert_point(verts: &[Vec3], tris: &mut Vec<[usize; 3]>, t: usize, pi: usize) {
    let [a, b, c] = tris[t];
    tris[t] = [a, b, pi];
    tris.push([b, c, pi]);
    tris.push([c, a, pi]);
    let mut stack = vec![(a, b), (b, c), (c, a)];
    let mut guard = 0;
    while let Some((u, v)) = stack.pop() {
        guard += 1;
        if guard > 4096 {
            break;
        }
        let Some(ti) = tris
            .iter()
            .position(|t| has_directed_edge(t, u, v) && t.contains(&pi))
        else {
            continue;
        };
        let Some(tj) = tris.iter().position(|t| has_directed_edge(t, v, u)) else {
            continue;
        };
        let q = third_vertex(&tris[tj], v, u);
        if in_circumcircle(verts[u], verts[v], verts[pi], verts[q]) {
            tris[ti] = [u, q, pi];
            tris[tj] = [q, v, pi];
            stack.push((u, q));
            stack.push((q, v));
        }
    }
}

fn has_directed_edge(t: &[usize; 3], u: usize, v: usize) -> bool {
    (t[0] == u && t[1] == v) || (t[1] == u && t[2] == v) || (t[2] == u && t[0] == v)
}

fn third_vertex(t: &[usize; 3], u: usize, v: usize) -> usize {
    *t.iter().find(|&&x| x != u && x != v).unwrap_or(&t[0])
}

impl PolyMeshDetail {
    /// Build the detail of every polygon of `mesh` from the floors in `chf`.
    /// `sample_dist` (m) spaces the height samples (0: no detail beyond the polygons),
    /// `max_error` (m) is how far the detail may stray from the floor.
    pub fn build(
        mesh: &PolyMesh,
        chf: &CompactHeightfield,
        sample_dist: f32,
        max_error: f32,
    ) -> Self {
        let mut detail = PolyMeshDetail::default();
        let sampler = Sampler {
            chf,
            tolerance: (chf.walkable_climb.max(1) as f32 * chf.ch * 2.0).max(max_error * 4.0),
        };
        let nvp = mesh.nvp;
        for pi in 0..mesh.npolys {
            let p = mesh.poly(pi);
            let nv = mesh.vert_count(pi);
            let reg = mesh.regs[pi];
            let poly: Vec<Vec3> = p[..nv]
                .iter()
                .map(|&v| {
                    let v = mesh.verts[usize::from(v)];
                    Vec3::new(
                        mesh.bmin.x + f32::from(v[0]) * mesh.cs,
                        mesh.bmin.y + f32::from(v[1]) * mesh.ch,
                        mesh.bmin.z + f32::from(v[2]) * mesh.cs,
                    )
                })
                .collect();
            let _ = nvp;
            let (verts, tris) = build_poly_detail(&poly, reg, &sampler, sample_dist, max_error);
            detail.meshes.push([
                detail.verts.len() as u32,
                verts.len() as u32,
                detail.tris.len() as u32,
                tris.len() as u32,
            ]);
            detail.verts.extend(verts);
            detail.tris.extend(tris);
        }
        detail
    }
}

fn build_poly_detail(
    poly: &[Vec3],
    reg: u16,
    sampler: &Sampler,
    sample_dist: f32,
    max_error: f32,
) -> (Vec<Vec3>, Vec<[u8; 3]>) {
    let nin = poly.len();
    // The polygon's corners at the floor's exact height (the same for every polygon
    // sharing a corner: the region is not considered).
    let poly: Vec<Vec3> = poly
        .iter()
        .map(|v| Vec3::new(v.x, sampler.height(v.x, v.z, v.y, 0).unwrap_or(v.y), v.z))
        .collect();
    let poly = poly.as_slice();
    let mut verts: Vec<Vec3> = poly.to_vec();
    let mut hull: Vec<usize> = Vec::with_capacity(nin * 2);

    for i in 0..nin {
        let j = (i + 1) % nin;
        hull.push(i);
        if sample_dist <= 0.0 {
            continue;
        }
        let (mut a, mut b) = (poly[i], poly[j]);
        // Sample in a fixed direction, so both polygons sharing the edge agree.
        let swapped = (b.x - a.x).abs() < 1e-6 && b.z < a.z || (b.x - a.x) < -1e-6;
        if swapped {
            std::mem::swap(&mut a, &mut b);
        }
        let length = dist2_xz(a, b).sqrt();
        let nn = ((length / sample_dist).floor() as usize + 1).min(MAX_EDGE_SAMPLES - 1);
        if nn < 2 || verts.len() + nn >= MAX_VERTS {
            continue;
        }
        let samples: Vec<Vec3> = (0..=nn)
            .map(|k| {
                let t = k as f32 / nn as f32;
                let mut s = a.lerp(b, t);
                if let Some(y) = sampler.height(s.x, s.z, s.y, reg) {
                    s.y = y;
                }
                s
            })
            .collect();
        // Keep the samples that stray furthest until the rest are within the error.
        let mut keep = vec![0usize, nn];
        let mut k = 0;
        while k + 1 < keep.len() {
            let (k0, k1) = (keep[k], keep[k + 1]);
            let (s0, s1) = (samples[k0], samples[k1]);
            let mut worst = None;
            let mut worst_error = max_error;
            for (m, s) in samples.iter().enumerate().take(k1).skip(k0 + 1) {
                let t = (m - k0) as f32 / (k1 - k0) as f32;
                let error = (s.y - (s0.y + (s1.y - s0.y) * t)).abs();
                if error > worst_error {
                    worst_error = error;
                    worst = Some(m);
                }
            }
            match worst {
                Some(m) if verts.len() + keep.len() < MAX_VERTS => keep.insert(k + 1, m),
                _ => k += 1,
            }
        }
        let mut inner: Vec<Vec3> = keep[1..keep.len() - 1]
            .iter()
            .map(|&m| samples[m])
            .collect();
        if swapped {
            inner.reverse();
        }
        for s in inner {
            hull.push(verts.len());
            verts.push(s);
        }
    }

    let mut tris = triangulate_hull(&verts, &hull, nin);

    if sample_dist > 0.0 && !tris.is_empty() {
        let (mut lo, mut hi) = (poly[0], poly[0]);
        for v in poly {
            lo = lo.min(*v);
            hi = hi.max(*v);
        }
        let x0 = (lo.x / sample_dist).floor() as i32;
        let x1 = (hi.x / sample_dist).ceil() as i32;
        let z0 = (lo.z / sample_dist).floor() as i32;
        let z1 = (hi.z / sample_dist).ceil() as i32;
        let mut samples: Vec<(Vec3, bool)> = Vec::new();
        for z in z0..z1 {
            for x in x0..x1 {
                let mut p = Vec3::new(x as f32 * sample_dist, 0.0, z as f32 * sample_dist);
                if signed_distance_to_poly(p, poly) > -sample_dist * 0.5 {
                    continue;
                }
                // The polygon's own height there, as the expected floor.
                let expected = tris.iter().find_map(|t| {
                    height_on_triangle(p, verts[t[0]], verts[t[1]], verts[t[2]], 1e-4).map(|h| h.0)
                });
                let Some(expected) = expected else {
                    continue;
                };
                if let Some(y) = sampler.height(p.x, p.z, expected, reg) {
                    p.y = y;
                    samples.push((p, false));
                }
            }
        }
        while verts.len() < MAX_VERTS {
            let mut best: Option<(usize, usize, f32)> = None;
            for (si, (s, used)) in samples.iter().enumerate() {
                if *used {
                    continue;
                }
                let hit = tris.iter().enumerate().find_map(|(ti, t)| {
                    height_on_triangle(*s, verts[t[0]], verts[t[1]], verts[t[2]], 1e-4)
                        .map(|(h, weight)| (ti, h, weight))
                });
                let Some((ti, h, weight)) = hit else {
                    continue;
                };
                let error = (s.y - h).abs();
                if weight > 1e-3 && error > max_error && best.is_none_or(|b| error > b.2) {
                    best = Some((si, ti, error));
                }
            }
            let Some((si, ti, _)) = best else {
                break;
            };
            samples[si].1 = true;
            let pi = verts.len();
            verts.push(samples[si].0);
            insert_point(&verts, &mut tris, ti, pi);
        }
    }

    let tris = tris
        .into_iter()
        .filter(|t| cross_xz(verts[t[0]], verts[t[1]], verts[t[2]]).abs() > 1e-9)
        .map(|t| [t[0] as u8, t[1] as u8, t[2] as u8])
        .collect();
    (verts, tris)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hull_triangulation_covers_a_tessellated_square() {
        let verts = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 2.0),
            Vec3::new(2.0, 0.0, 2.0),
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(1.0, 0.0, 2.0),
        ];
        let hull = vec![0, 4, 1, 5, 2, 3];
        let tris = triangulate_hull(&verts, &hull, 4);
        assert_eq!(tris.len(), 4);
        let area: f32 = tris
            .iter()
            .map(|t| cross_xz(verts[t[0]], verts[t[1]], verts[t[2]]).abs() * 0.5)
            .sum();
        assert!((area - 4.0).abs() < 1e-5);
    }

    #[test]
    fn inserted_points_keep_the_area_and_stay_delaunay() {
        let mut verts = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 4.0),
            Vec3::new(4.0, 0.0, 4.0),
            Vec3::new(4.0, 0.0, 0.0),
        ];
        let mut tris = triangulate_hull(&verts, &[0, 1, 2, 3], 4);
        for p in [
            Vec3::new(1.0, 1.0, 1.0),
            Vec3::new(3.0, 0.5, 2.5),
            Vec3::new(2.0, 2.0, 2.2),
        ] {
            let t = tris
                .iter()
                .position(|t| {
                    height_on_triangle(p, verts[t[0]], verts[t[1]], verts[t[2]], 0.0).is_some()
                })
                .unwrap();
            let pi = verts.len();
            verts.push(p);
            insert_point(&verts, &mut tris, t, pi);
        }
        let area: f32 = tris
            .iter()
            .map(|t| cross_xz(verts[t[0]], verts[t[1]], verts[t[2]]).abs() * 0.5)
            .sum();
        assert!((area - 16.0).abs() < 1e-4);
        assert_eq!(tris.len(), 2 + 2 * 3);
    }
}
