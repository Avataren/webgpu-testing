//! Convex polygons from the region outlines: each contour is triangulated, the
//! triangles merged into polygons of up to `nvp` vertices, vertices on the tile border
//! that only exist to cut it removed, and neighbours found across shared edges.

use std::collections::HashMap;

use glam::Vec3;

use super::contour::{ContourSet, BORDER_VERTEX};

/// No vertex / no neighbour.
pub(crate) const MESH_NULL_IDX: u16 = 0xffff;
/// A neighbour slot holding this bit marks an edge on the tile border; the low bits
/// say which side (0: -x, 1: +z, 2: +x, 3: -z).
pub(crate) const PORTAL_FLAG: u16 = 0x8000;
const MULTIPLE_REGS: u16 = 0;

pub(crate) struct PolyMesh {
    /// Vertices in cells (x, y, z) from `bmin`.
    pub verts: Vec<[u16; 3]>,
    /// Per polygon: `nvp` vertex indices then `nvp` neighbour slots.
    pub polys: Vec<u16>,
    pub regs: Vec<u16>,
    pub areas: Vec<u8>,
    pub npolys: usize,
    pub nvp: usize,
    pub bmin: Vec3,
    pub bmax: Vec3,
    pub cs: f32,
    pub ch: f32,
    pub border_size: i32,
}

impl PolyMesh {
    pub fn poly(&self, index: usize) -> &[u16] {
        &self.polys[index * self.nvp * 2..(index + 1) * self.nvp * 2]
    }

    pub fn vert_count(&self, index: usize) -> usize {
        let p = self.poly(index);
        p[..self.nvp]
            .iter()
            .position(|&v| v == MESH_NULL_IDX)
            .unwrap_or(self.nvp)
    }

    pub fn build(cset: &ContourSet, nvp: usize) -> Self {
        let mut mesh = PolyMesh {
            verts: Vec::new(),
            polys: Vec::new(),
            regs: Vec::new(),
            areas: Vec::new(),
            npolys: 0,
            nvp,
            bmin: cset.bmin,
            bmax: cset.bmax,
            cs: cset.cs,
            ch: cset.ch,
            border_size: cset.border_size,
        };
        let max_tris: usize = cset
            .conts
            .iter()
            .filter(|c| c.verts.len() >= 3)
            .map(|c| c.verts.len() - 2)
            .sum();
        let mut vflags: Vec<bool> = Vec::new();
        let mut vertex_lookup: HashMap<(u16, u16), Vec<u16>> = HashMap::new();

        for contour in &cset.conts {
            let n = contour.verts.len();
            if n < 3 {
                continue;
            }
            let points: Vec<[i32; 3]> = contour.verts.iter().map(|v| [v[0], v[1], v[2]]).collect();
            let mut indices: Vec<u32> = (0..n as u32).collect();
            let mut tris = Vec::with_capacity(n * 3);
            let ntris = triangulate(&points, &mut indices, &mut tris);
            if ntris <= 0 {
                log::debug!("navmesh: bad triangulation of a contour");
            }
            let mut vidx = vec![0u16; n];
            for (j, v) in contour.verts.iter().enumerate() {
                let index = add_vertex(
                    &mut mesh.verts,
                    &mut vertex_lookup,
                    v[0] as u16,
                    v[1].clamp(0, 0xffff) as u16,
                    v[2] as u16,
                );
                vidx[j] = index;
                if usize::from(index) >= vflags.len() {
                    vflags.resize(usize::from(index) + 1, false);
                }
                if v[3] & BORDER_VERTEX != 0 {
                    vflags[usize::from(index)] = true;
                }
            }

            let mut polys: Vec<Vec<u16>> = Vec::new();
            for t in tris.as_chunks::<3>().0 {
                if t[0] != t[1] && t[0] != t[2] && t[1] != t[2] {
                    let mut p = vec![MESH_NULL_IDX; nvp];
                    p[0] = vidx[t[0] as usize];
                    p[1] = vidx[t[1] as usize];
                    p[2] = vidx[t[2] as usize];
                    polys.push(p);
                }
            }
            if polys.is_empty() {
                continue;
            }
            if nvp > 3 {
                merge_polys(&mut polys, &mesh.verts, nvp, None);
            }
            for p in polys {
                mesh.polys.extend_from_slice(&p);
                mesh.polys.extend(std::iter::repeat_n(MESH_NULL_IDX, nvp));
                mesh.regs.push(contour.reg);
                mesh.areas.push(contour.area);
                mesh.npolys += 1;
            }
        }

        // Remove the vertices that only cut the tile border.
        let mut i = 0;
        while i < mesh.verts.len() {
            if vflags.get(i).copied().unwrap_or(false) {
                if !can_remove_vertex(&mesh, i as u16) {
                    i += 1;
                    continue;
                }
                remove_vertex(&mut mesh, i as u16, max_tris);
                vflags.remove(i);
            } else {
                i += 1;
            }
        }

        build_mesh_adjacency(&mut mesh);

        // Mark edges on the tile border with the side they face.
        if mesh.border_size > 0 {
            let w = cset.width as u16;
            let h = cset.height as u16;
            for pi in 0..mesh.npolys {
                let base = pi * nvp * 2;
                for j in 0..nvp {
                    let va = mesh.polys[base + j];
                    if va == MESH_NULL_IDX {
                        break;
                    }
                    if mesh.polys[base + nvp + j] != MESH_NULL_IDX {
                        continue;
                    }
                    let mut nj = j + 1;
                    if nj >= nvp || mesh.polys[base + nj] == MESH_NULL_IDX {
                        nj = 0;
                    }
                    let a = mesh.verts[usize::from(va)];
                    let b = mesh.verts[usize::from(mesh.polys[base + nj])];
                    let side = if a[0] == 0 && b[0] == 0 {
                        Some(0)
                    } else if a[2] == h && b[2] == h {
                        Some(1)
                    } else if a[0] == w && b[0] == w {
                        Some(2)
                    } else if a[2] == 0 && b[2] == 0 {
                        Some(3)
                    } else {
                        None
                    };
                    if let Some(side) = side {
                        mesh.polys[base + nvp + j] = PORTAL_FLAG | side;
                    }
                }
            }
        }
        mesh
    }
}

fn add_vertex(
    verts: &mut Vec<[u16; 3]>,
    lookup: &mut HashMap<(u16, u16), Vec<u16>>,
    x: u16,
    y: u16,
    z: u16,
) -> u16 {
    let bucket = lookup.entry((x, z)).or_default();
    for &index in bucket.iter() {
        let v = verts[usize::from(index)];
        if (i32::from(v[1]) - i32::from(y)).abs() <= 2 {
            return index;
        }
    }
    let index = verts.len() as u16;
    verts.push([x, y, z]);
    bucket.push(index);
    index
}

// Geometry helpers on (x, _, z) integer points.

fn area2(a: [i32; 3], b: [i32; 3], c: [i32; 3]) -> i64 {
    i64::from(b[0] - a[0]) * i64::from(c[2] - a[2])
        - i64::from(c[0] - a[0]) * i64::from(b[2] - a[2])
}

fn left(a: [i32; 3], b: [i32; 3], c: [i32; 3]) -> bool {
    area2(a, b, c) < 0
}

fn left_on(a: [i32; 3], b: [i32; 3], c: [i32; 3]) -> bool {
    area2(a, b, c) <= 0
}

fn collinear(a: [i32; 3], b: [i32; 3], c: [i32; 3]) -> bool {
    area2(a, b, c) == 0
}

fn intersect_prop(a: [i32; 3], b: [i32; 3], c: [i32; 3], d: [i32; 3]) -> bool {
    if collinear(a, b, c) || collinear(a, b, d) || collinear(c, d, a) || collinear(c, d, b) {
        return false;
    }
    (left(a, b, c) ^ left(a, b, d)) && (left(c, d, a) ^ left(c, d, b))
}

fn between(a: [i32; 3], b: [i32; 3], c: [i32; 3]) -> bool {
    if !collinear(a, b, c) {
        return false;
    }
    if a[0] != b[0] {
        (a[0] <= c[0] && c[0] <= b[0]) || (a[0] >= c[0] && c[0] >= b[0])
    } else {
        (a[2] <= c[2] && c[2] <= b[2]) || (a[2] >= c[2] && c[2] >= b[2])
    }
}

fn intersect(a: [i32; 3], b: [i32; 3], c: [i32; 3], d: [i32; 3]) -> bool {
    intersect_prop(a, b, c, d)
        || between(a, b, c)
        || between(a, b, d)
        || between(c, d, a)
        || between(c, d, b)
}

fn vequal(a: [i32; 3], b: [i32; 3]) -> bool {
    a[0] == b[0] && a[2] == b[2]
}

const INDEX_MASK: u32 = 0x0fff_ffff;
const REMOVABLE: u32 = 0x8000_0000;

/// Ear-clipping triangulation of a contour (`verts` in cells, any extra fields
/// ignored); returns the triangle count, negative when the outline was broken and only
/// part could be triangulated.
fn triangulate(verts: &[[i32; 3]], indices: &mut Vec<u32>, tris: &mut Vec<u32>) -> i32 {
    let point = |index: u32| -> [i32; 3] { verts[(index & INDEX_MASK) as usize] };
    let mut n = indices.len();
    let next = |i: usize, n: usize| if i + 1 >= n { 0 } else { i + 1 };
    let prev = |i: usize, n: usize| if i == 0 { n - 1 } else { i - 1 };
    let mut ntris = 0;

    for i in 0..n {
        let i1 = next(i, n);
        let i2 = next(i1, n);
        if diagonal(i, i2, n, indices, &point) {
            indices[i1] |= REMOVABLE;
        }
    }
    while n > 3 {
        let mut min_len = -1i64;
        let mut mini = None;
        for i in 0..n {
            let i1 = next(i, n);
            if indices[i1] & REMOVABLE != 0 {
                let p0 = point(indices[i]);
                let p2 = point(indices[next(i1, n)]);
                let dx = i64::from(p2[0] - p0[0]);
                let dz = i64::from(p2[2] - p0[2]);
                let len = dx * dx + dz * dz;
                if min_len < 0 || len < min_len {
                    min_len = len;
                    mini = Some(i);
                }
            }
        }
        if mini.is_none() {
            // Overlapping segments: loosen the test so a diagonal can still be found.
            min_len = -1;
            for i in 0..n {
                let i1 = next(i, n);
                let i2 = next(i1, n);
                if diagonal_loose(i, i2, n, indices, &point) {
                    let p0 = point(indices[i]);
                    let p2 = point(indices[next(i2, n)]);
                    let dx = i64::from(p2[0] - p0[0]);
                    let dz = i64::from(p2[2] - p0[2]);
                    let len = dx * dx + dz * dz;
                    if min_len < 0 || len < min_len {
                        min_len = len;
                        mini = Some(i);
                    }
                }
            }
        }
        let Some(i) = mini else {
            return -ntris;
        };
        let mut i1 = next(i, n);
        let i2 = next(i1, n);
        tris.push(indices[i] & INDEX_MASK);
        tris.push(indices[i1] & INDEX_MASK);
        tris.push(indices[i2] & INDEX_MASK);
        ntris += 1;
        indices.remove(i1);
        n -= 1;
        if i1 >= n {
            i1 = 0;
        }
        let i = prev(i1, n);
        if diagonal(prev(i, n), i1, n, indices, &point) {
            indices[i] |= REMOVABLE;
        } else {
            indices[i] &= INDEX_MASK;
        }
        if diagonal(i, next(i1, n), n, indices, &point) {
            indices[i1] |= REMOVABLE;
        } else {
            indices[i1] &= INDEX_MASK;
        }
    }
    tris.push(indices[0] & INDEX_MASK);
    tris.push(indices[1] & INDEX_MASK);
    tris.push(indices[2] & INDEX_MASK);
    ntris + 1
}

fn in_cone(
    i: usize,
    j: usize,
    n: usize,
    indices: &[u32],
    point: &impl Fn(u32) -> [i32; 3],
) -> bool {
    let pi = point(indices[i]);
    let pj = point(indices[j]);
    let pi1 = point(indices[if i + 1 >= n { 0 } else { i + 1 }]);
    let pin1 = point(indices[if i == 0 { n - 1 } else { i - 1 }]);
    if left_on(pin1, pi, pi1) {
        return left(pi, pj, pin1) && left(pj, pi, pi1);
    }
    !(left_on(pi, pj, pi1) && left_on(pj, pi, pin1))
}

fn in_cone_loose(
    i: usize,
    j: usize,
    n: usize,
    indices: &[u32],
    point: &impl Fn(u32) -> [i32; 3],
) -> bool {
    let pi = point(indices[i]);
    let pj = point(indices[j]);
    let pi1 = point(indices[if i + 1 >= n { 0 } else { i + 1 }]);
    let pin1 = point(indices[if i == 0 { n - 1 } else { i - 1 }]);
    if left_on(pin1, pi, pi1) {
        return left_on(pi, pj, pin1) && left_on(pj, pi, pi1);
    }
    !(left_on(pi, pj, pi1) && left_on(pj, pi, pin1))
}

fn diagonalie(
    i: usize,
    j: usize,
    n: usize,
    indices: &[u32],
    point: &impl Fn(u32) -> [i32; 3],
    loose: bool,
) -> bool {
    let d0 = point(indices[i]);
    let d1 = point(indices[j]);
    for k in 0..n {
        let k1 = if k + 1 >= n { 0 } else { k + 1 };
        if k == i || k1 == i || k == j || k1 == j {
            continue;
        }
        let p0 = point(indices[k]);
        let p1 = point(indices[k1]);
        if vequal(d0, p0) || vequal(d1, p0) || vequal(d0, p1) || vequal(d1, p1) {
            continue;
        }
        let hit = if loose {
            intersect_prop(d0, d1, p0, p1)
        } else {
            intersect(d0, d1, p0, p1)
        };
        if hit {
            return false;
        }
    }
    true
}

fn diagonal(
    i: usize,
    j: usize,
    n: usize,
    indices: &[u32],
    point: &impl Fn(u32) -> [i32; 3],
) -> bool {
    in_cone(i, j, n, indices, point) && diagonalie(i, j, n, indices, point, false)
}

fn diagonal_loose(
    i: usize,
    j: usize,
    n: usize,
    indices: &[u32],
    point: &impl Fn(u32) -> [i32; 3],
) -> bool {
    in_cone_loose(i, j, n, indices, point) && diagonalie(i, j, n, indices, point, true)
}

fn count_poly_verts(p: &[u16], nvp: usize) -> usize {
    p[..nvp]
        .iter()
        .position(|&v| v == MESH_NULL_IDX)
        .unwrap_or(nvp)
}

fn uleft(a: [u16; 3], b: [u16; 3], c: [u16; 3]) -> bool {
    (i32::from(b[0]) - i32::from(a[0])) * (i32::from(c[2]) - i32::from(a[2]))
        - (i32::from(c[0]) - i32::from(a[0])) * (i32::from(b[2]) - i32::from(a[2]))
        < 0
}

/// How good merging `pa` and `pb` is (the shared edge's squared length, so long edges
/// go first), or `None` when they share no edge or would not stay convex.
fn poly_merge_value(
    pa: &[u16],
    pb: &[u16],
    verts: &[[u16; 3]],
    nvp: usize,
) -> Option<(i32, usize, usize)> {
    let na = count_poly_verts(pa, nvp);
    let nb = count_poly_verts(pb, nvp);
    if na + nb - 2 > nvp {
        return None;
    }
    let mut shared = None;
    'outer: for i in 0..na {
        let (mut va0, mut va1) = (pa[i], pa[(i + 1) % na]);
        if va0 > va1 {
            std::mem::swap(&mut va0, &mut va1);
        }
        for j in 0..nb {
            let (mut vb0, mut vb1) = (pb[j], pb[(j + 1) % nb]);
            if vb0 > vb1 {
                std::mem::swap(&mut vb0, &mut vb1);
            }
            if va0 == vb0 && va1 == vb1 {
                shared = Some((i, j));
                break 'outer;
            }
        }
    }
    let (ea, eb) = shared?;
    let v = |i: u16| verts[usize::from(i)];
    if !uleft(v(pa[(ea + na - 1) % na]), v(pa[ea]), v(pb[(eb + 2) % nb])) {
        return None;
    }
    if !uleft(v(pb[(eb + nb - 1) % nb]), v(pb[eb]), v(pa[(ea + 2) % na])) {
        return None;
    }
    let a = v(pa[ea]);
    let b = v(pa[(ea + 1) % na]);
    let dx = i32::from(a[0]) - i32::from(b[0]);
    let dz = i32::from(a[2]) - i32::from(b[2]);
    Some((dx * dx + dz * dz, ea, eb))
}

fn merge_poly_verts(pa: &[u16], pb: &[u16], ea: usize, eb: usize, nvp: usize) -> Vec<u16> {
    let na = count_poly_verts(pa, nvp);
    let nb = count_poly_verts(pb, nvp);
    let mut merged = vec![MESH_NULL_IDX; nvp];
    let mut n = 0;
    for i in 0..na - 1 {
        merged[n] = pa[(ea + 1 + i) % na];
        n += 1;
    }
    for i in 0..nb - 1 {
        merged[n] = pb[(eb + 1 + i) % nb];
        n += 1;
    }
    merged
}

/// Greedily merge the polygon pair with the longest shared edge until none can merge.
/// `regs` (one per polygon) follows the merges when given.
fn merge_polys(
    polys: &mut Vec<Vec<u16>>,
    verts: &[[u16; 3]],
    nvp: usize,
    mut regs: Option<(&mut Vec<u16>, &mut Vec<u8>)>,
) {
    loop {
        let mut best: Option<(i32, usize, usize, usize, usize)> = None;
        for j in 0..polys.len().saturating_sub(1) {
            for k in j + 1..polys.len() {
                if let Some((value, ea, eb)) = poly_merge_value(&polys[j], &polys[k], verts, nvp) {
                    if best.is_none_or(|b| value > b.0) {
                        best = Some((value, j, k, ea, eb));
                    }
                }
            }
        }
        let Some((_, pa, pb, ea, eb)) = best else {
            break;
        };
        polys[pa] = merge_poly_verts(&polys[pa], &polys[pb], ea, eb, nvp);
        if let Some((regs, areas)) = regs.as_mut() {
            if regs[pa] != regs[pb] {
                regs[pa] = MULTIPLE_REGS;
            }
            regs.swap_remove(pb);
            areas.swap_remove(pb);
        }
        polys.swap_remove(pb);
    }
}

fn can_remove_vertex(mesh: &PolyMesh, rem: u16) -> bool {
    let nvp = mesh.nvp;
    let mut touched_verts = 0;
    let mut remaining_edges = 0;
    for pi in 0..mesh.npolys {
        let p = mesh.poly(pi);
        let nv = count_poly_verts(p, nvp);
        let removed = p[..nv].iter().filter(|&&v| v == rem).count();
        touched_verts += removed;
        if removed > 0 {
            remaining_edges += nv as i32 - (removed as i32 + 1);
        }
    }
    if remaining_edges <= 2 {
        return false;
    }
    // Edges through the vertex, and how many polygons use each.
    let mut edges: Vec<(u16, u16, i32)> = Vec::with_capacity(touched_verts * 2);
    for pi in 0..mesh.npolys {
        let p = mesh.poly(pi);
        let nv = count_poly_verts(p, nvp);
        let mut k = nv - 1;
        for j in 0..nv {
            if p[j] == rem || p[k] == rem {
                let (mut a, mut b) = (p[j], p[k]);
                if b == rem {
                    std::mem::swap(&mut a, &mut b);
                }
                match edges.iter_mut().find(|e| e.1 == b) {
                    Some(e) => e.2 += 1,
                    None => edges.push((a, b, 1)),
                }
            }
            k = j;
        }
    }
    // Two polygons that only share the vertex: removing it would join them wrongly.
    edges.iter().filter(|e| e.2 < 2).count() <= 2
}

fn remove_vertex(mesh: &mut PolyMesh, rem: u16, max_tris: usize) {
    let nvp = mesh.nvp;
    // The edges of the polygons around the vertex that do not touch it, then those
    // polygons go.
    let mut edges: Vec<[u16; 4]> = Vec::new();
    let mut pi = 0;
    while pi < mesh.npolys {
        let base = pi * nvp * 2;
        let nv = count_poly_verts(&mesh.polys[base..base + nvp], nvp);
        let has_rem = mesh.polys[base..base + nv].contains(&rem);
        if has_rem {
            let mut k = nv - 1;
            for j in 0..nv {
                let (pj, pk) = (mesh.polys[base + j], mesh.polys[base + k]);
                if pj != rem && pk != rem {
                    edges.push([pk, pj, mesh.regs[pi], u16::from(mesh.areas[pi])]);
                }
                k = j;
            }
            let last = mesh.npolys - 1;
            if pi != last {
                let (head, tail) = mesh.polys.split_at_mut(last * nvp * 2);
                head[base..base + nvp * 2].copy_from_slice(&tail[..nvp * 2]);
            }
            mesh.polys.truncate(last * nvp * 2);
            mesh.regs.swap_remove(pi);
            mesh.areas.swap_remove(pi);
            mesh.npolys -= 1;
        } else {
            pi += 1;
        }
    }

    mesh.verts.remove(usize::from(rem));
    for pi in 0..mesh.npolys {
        let base = pi * nvp * 2;
        for j in 0..nvp {
            let v = &mut mesh.polys[base + j];
            if *v != MESH_NULL_IDX && *v > rem {
                *v -= 1;
            }
        }
    }
    for e in edges.iter_mut() {
        if e[0] > rem {
            e[0] -= 1;
        }
        if e[1] > rem {
            e[1] -= 1;
        }
    }
    if edges.is_empty() {
        return;
    }

    // Chain the edges into the outline of the hole.
    let mut hole: std::collections::VecDeque<u16> = std::collections::VecDeque::new();
    let mut hreg: std::collections::VecDeque<u16> = std::collections::VecDeque::new();
    let mut harea: std::collections::VecDeque<u16> = std::collections::VecDeque::new();
    hole.push_back(edges[0][0]);
    hreg.push_back(edges[0][2]);
    harea.push_back(edges[0][3]);
    while !edges.is_empty() {
        let mut matched = false;
        let mut i = 0;
        while i < edges.len() {
            let [ea, eb, r, a] = edges[i];
            let mut add = false;
            if hole.front() == Some(&eb) {
                hole.push_front(ea);
                hreg.push_front(r);
                harea.push_front(a);
                add = true;
            } else if hole.back() == Some(&ea) {
                hole.push_back(eb);
                hreg.push_back(r);
                harea.push_back(a);
                add = true;
            }
            if add {
                edges.swap_remove(i);
                matched = true;
            } else {
                i += 1;
            }
        }
        if !matched {
            break;
        }
    }
    let hole: Vec<u16> = hole.into();
    let hreg: Vec<u16> = hreg.into();
    let harea: Vec<u16> = harea.into();
    let tverts: Vec<[i32; 3]> = hole
        .iter()
        .map(|&v| {
            let p = mesh.verts[usize::from(v)];
            [i32::from(p[0]), i32::from(p[1]), i32::from(p[2])]
        })
        .collect();
    let mut thole: Vec<u32> = (0..hole.len() as u32).collect();
    let mut tris = Vec::new();
    triangulate(&tverts, &mut thole, &mut tris);

    let mut polys: Vec<Vec<u16>> = Vec::new();
    let mut pregs: Vec<u16> = Vec::new();
    let mut pareas: Vec<u8> = Vec::new();
    for t in tris.as_chunks::<3>().0 {
        let (t0, t1, t2) = (t[0] as usize, t[1] as usize, t[2] as usize);
        if t0 != t1 && t0 != t2 && t1 != t2 {
            let mut p = vec![MESH_NULL_IDX; nvp];
            p[0] = hole[t0];
            p[1] = hole[t1];
            p[2] = hole[t2];
            polys.push(p);
            pregs.push(if hreg[t0] != hreg[t1] || hreg[t1] != hreg[t2] {
                MULTIPLE_REGS
            } else {
                hreg[t0]
            });
            pareas.push(harea[t0] as u8);
        }
    }
    if polys.is_empty() {
        return;
    }
    if nvp > 3 {
        merge_polys(
            &mut polys,
            &mesh.verts,
            nvp,
            Some((&mut pregs, &mut pareas)),
        );
    }
    for ((p, reg), area) in polys.into_iter().zip(pregs).zip(pareas) {
        if mesh.npolys >= max_tris {
            break;
        }
        mesh.polys.extend_from_slice(&p);
        mesh.polys.extend(std::iter::repeat_n(MESH_NULL_IDX, nvp));
        mesh.regs.push(reg);
        mesh.areas.push(area);
        mesh.npolys += 1;
    }
}

fn build_mesh_adjacency(mesh: &mut PolyMesh) {
    let nvp = mesh.nvp;
    struct Edge {
        vert: [u16; 2],
        poly_edge: [usize; 2],
        poly: [usize; 2],
    }
    let mut edges: Vec<Edge> = Vec::new();
    let mut first_edge: Vec<Option<usize>> = vec![None; mesh.verts.len()];
    let mut next_edge: Vec<Option<usize>> = Vec::new();
    let edge_end = |p: &[u16], j: usize| -> u16 {
        if j + 1 >= nvp || p[j + 1] == MESH_NULL_IDX {
            p[0]
        } else {
            p[j + 1]
        }
    };
    for pi in 0..mesh.npolys {
        let p = mesh.poly(pi);
        for j in 0..nvp {
            if p[j] == MESH_NULL_IDX {
                break;
            }
            let v0 = p[j];
            let v1 = edge_end(p, j);
            if v0 < v1 {
                edges.push(Edge {
                    vert: [v0, v1],
                    poly_edge: [j, 0],
                    poly: [pi, pi],
                });
                next_edge.push(first_edge[usize::from(v0)]);
                first_edge[usize::from(v0)] = Some(edges.len() - 1);
            }
        }
    }
    for pi in 0..mesh.npolys {
        let p = mesh.poly(pi).to_vec();
        for j in 0..nvp {
            if p[j] == MESH_NULL_IDX {
                break;
            }
            let v0 = p[j];
            let v1 = edge_end(&p, j);
            if v0 > v1 {
                let mut e = first_edge[usize::from(v1)];
                while let Some(ei) = e {
                    let edge = &mut edges[ei];
                    if edge.vert[1] == v0 && edge.poly[0] == edge.poly[1] {
                        edge.poly[1] = pi;
                        edge.poly_edge[1] = j;
                        break;
                    }
                    e = next_edge[ei];
                }
            }
        }
    }
    for edge in &edges {
        if edge.poly[0] != edge.poly[1] {
            let (p0, p1) = (edge.poly[0], edge.poly[1]);
            mesh.polys[p0 * nvp * 2 + nvp + edge.poly_edge[0]] = p1 as u16;
            mesh.polys[p1 * nvp * 2 + nvp + edge.poly_edge[1]] = p0 as u16;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triangulates_a_concave_outline() {
        // An L shape (cells), wound like Recast's contours.
        let verts: Vec<[i32; 3]> = vec![
            [0, 0, 0],
            [0, 0, 4],
            [4, 0, 4],
            [4, 0, 2],
            [2, 0, 2],
            [2, 0, 0],
        ];
        let mut indices: Vec<u32> = (0..verts.len() as u32).collect();
        let mut tris = Vec::new();
        let n = triangulate(&verts, &mut indices, &mut tris);
        assert_eq!(n, 4);
        // Twice the triangles' area equals the L's 12 cells.
        let doubled: i64 = tris
            .as_chunks::<3>()
            .0
            .iter()
            .map(|t| {
                area2(
                    verts[t[0] as usize],
                    verts[t[1] as usize],
                    verts[t[2] as usize],
                )
                .abs()
            })
            .sum();
        assert_eq!(doubled, 24);
    }
}
