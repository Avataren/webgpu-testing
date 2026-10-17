//! Region outlines: traced around each region's cells, then simplified to the few
//! vertices that keep them within the allowed error, holes merged into their outline.

use glam::Vec3;

use super::compact::CompactHeightfield;
use super::{dir_offset_x, dir_offset_z, BORDER_REG};

/// Neighbour region bits of a contour vertex's flags.
pub(crate) const CONTOUR_REG_MASK: i32 = 0xffff;
/// The vertex lies on the tile border (to be removed from the polygons).
pub(crate) const BORDER_VERTEX: i32 = 0x10000;
/// The edge after the vertex separates two areas.
pub(crate) const AREA_BORDER: i32 = 0x20000;

#[derive(Debug, Clone, Default)]
pub(crate) struct Contour {
    /// Simplified vertices: x, y, z (cells, tile border removed) and flags.
    pub verts: Vec<[i32; 4]>,
    pub reg: u16,
    pub area: u8,
}

pub(crate) struct ContourSet {
    pub conts: Vec<Contour>,
    pub bmin: Vec3,
    pub bmax: Vec3,
    pub cs: f32,
    pub ch: f32,
    pub width: i32,
    pub height: i32,
    pub border_size: i32,
}

impl ContourSet {
    /// Trace and simplify every region's outline. `max_error` is the furthest (cells) a
    /// simplified wall may stray from the traced one, `max_edge_len` the longest wall
    /// edge (cells, 0 for no limit).
    pub fn build(chf: &CompactHeightfield, max_error: f32, max_edge_len: i32) -> Self {
        let w = chf.width;
        let h = chf.height;
        let border_size = chf.border_size;
        let mut bmin = chf.bmin;
        let mut bmax = chf.bmax;
        if border_size > 0 {
            let pad = border_size as f32 * chf.cs;
            bmin.x += pad;
            bmin.z += pad;
            bmax.x -= pad;
            bmax.z -= pad;
        }
        let mut set = Self {
            conts: Vec::new(),
            bmin,
            bmax,
            cs: chf.cs,
            ch: chf.ch,
            width: w - border_size * 2,
            height: h - border_size * 2,
            border_size,
        };

        // Which edges of each span leave its region.
        let mut flags = vec![0u8; chf.span_count()];
        for z in 0..h {
            for x in 0..w {
                let cell = chf.cell(x, z);
                for i in cell.index..cell.index + cell.count {
                    let i = i as usize;
                    let span = chf.spans[i];
                    if span.reg == 0 || span.reg & BORDER_REG != 0 {
                        flags[i] = 0;
                        continue;
                    }
                    let mut res = 0u8;
                    for dir in 0..4 {
                        let mut r = 0;
                        if span.connected(dir) {
                            r = chf.spans[chf.neighbor_index(x, z, &span, dir)].reg;
                        }
                        if r == span.reg {
                            res |= 1 << dir;
                        }
                    }
                    flags[i] = res ^ 0xf;
                }
            }
        }

        let mut verts = Vec::new();
        let mut simplified = Vec::new();
        for z in 0..h {
            for x in 0..w {
                let cell = chf.cell(x, z);
                for i in cell.index..cell.index + cell.count {
                    let i = i as usize;
                    if flags[i] == 0 || flags[i] == 0xf {
                        flags[i] = 0;
                        continue;
                    }
                    let reg = chf.spans[i].reg;
                    if reg == 0 || reg & BORDER_REG != 0 {
                        continue;
                    }
                    let area = chf.areas[i];
                    verts.clear();
                    simplified.clear();
                    walk_contour(chf, x, z, i, &mut flags, &mut verts);
                    simplify_contour(&verts, &mut simplified, max_error, max_edge_len);
                    remove_degenerate_segments(&mut simplified);
                    if simplified.len() >= 3 {
                        let mut contour = Contour {
                            verts: simplified.clone(),
                            reg,
                            area,
                        };
                        if border_size > 0 {
                            for v in &mut contour.verts {
                                v[0] -= border_size;
                                v[2] -= border_size;
                            }
                        }
                        set.conts.push(contour);
                    }
                }
            }
        }

        merge_holes(&mut set, usize::from(chf.max_regions) + 1);
        set
    }
}

fn merge_holes(set: &mut ContourSet, region_count: usize) {
    if set.conts.is_empty() {
        return;
    }
    let winding: Vec<i32> = set
        .conts
        .iter()
        .map(|c| {
            if area_of_polygon_2d(&c.verts) < 0 {
                -1
            } else {
                1
            }
        })
        .collect();
    if !winding.contains(&-1) {
        return;
    }
    let mut outlines: Vec<Option<usize>> = vec![None; region_count];
    let mut holes: Vec<Vec<usize>> = vec![Vec::new(); region_count];
    for (i, contour) in set.conts.iter().enumerate() {
        let reg = usize::from(contour.reg);
        if reg >= region_count {
            continue;
        }
        if winding[i] > 0 {
            if outlines[reg].is_some() {
                log::warn!("navmesh: multiple outlines for region {reg}");
            }
            outlines[reg] = Some(i);
        } else {
            holes[reg].push(i);
        }
    }
    for reg in 0..region_count {
        if holes[reg].is_empty() {
            continue;
        }
        let Some(outline) = outlines[reg] else {
            log::warn!("navmesh: missing outline for region {reg}");
            continue;
        };
        merge_region_holes(set, outline, &holes[reg]);
    }
    set.conts.retain(|c| c.verts.len() >= 3);
}

/// Index of the lowest-x (then lowest-z) vertex, and that vertex.
fn left_most_vertex(verts: &[[i32; 4]]) -> (i32, i32, usize) {
    let mut best = (verts[0][0], verts[0][2], 0);
    for (i, v) in verts.iter().enumerate().skip(1) {
        if v[0] < best.0 || (v[0] == best.0 && v[2] < best.1) {
            best = (v[0], v[2], i);
        }
    }
    best
}

fn merge_region_holes(set: &mut ContourSet, outline: usize, hole_indices: &[usize]) {
    let mut holes: Vec<(usize, i32, i32, usize)> = hole_indices
        .iter()
        .map(|&h| {
            let (min_x, min_z, leftmost) = left_most_vertex(&set.conts[h].verts);
            (h, min_x, min_z, leftmost)
        })
        .collect();
    holes.sort_by_key(|h| (h.1, h.2));

    for hi in 0..holes.len() {
        let (hole, _, _, leftmost) = holes[hi];
        let hole_len = set.conts[hole].verts.len();
        let mut index = None;
        let mut best_vertex = leftmost;
        for _ in 0..hole_len {
            let corner = set.conts[hole].verts[best_vertex];
            let outline_verts = &set.conts[outline].verts;
            let mut diags: Vec<(usize, i32)> = (0..outline_verts.len())
                .filter(|&j| in_cone_contour(j, outline_verts, corner))
                .map(|j| {
                    let dx = outline_verts[j][0] - corner[0];
                    let dz = outline_verts[j][2] - corner[2];
                    (j, dx * dx + dz * dz)
                })
                .collect();
            diags.sort_by_key(|d| d.1);
            for &(vert, _) in &diags {
                let pt = outline_verts[vert];
                let mut intersect = intersect_seg_contour(pt, corner, Some(vert), outline_verts);
                for &(other, _, _, _) in &holes[hi..] {
                    if intersect {
                        break;
                    }
                    intersect |= intersect_seg_contour(pt, corner, None, &set.conts[other].verts);
                }
                if !intersect {
                    index = Some(vert);
                    break;
                }
            }
            if index.is_some() {
                break;
            }
            best_vertex = (best_vertex + 1) % hole_len;
        }
        let Some(ia) = index else {
            log::warn!("navmesh: could not merge a hole into its outline");
            continue;
        };
        let hole_verts = std::mem::take(&mut set.conts[hole].verts);
        let outline_verts = &mut set.conts[outline].verts;
        let mut merged = Vec::with_capacity(outline_verts.len() + hole_verts.len() + 2);
        for k in 0..=outline_verts.len() {
            merged.push(outline_verts[(ia + k) % outline_verts.len()]);
        }
        for k in 0..=hole_verts.len() {
            merged.push(hole_verts[(best_vertex + k) % hole_verts.len()]);
        }
        *outline_verts = merged;
    }
}

fn area_of_polygon_2d(verts: &[[i32; 4]]) -> i32 {
    let n = verts.len();
    let mut area = 0;
    let mut j = n - 1;
    for i in 0..n {
        let vi = verts[i];
        let vj = verts[j];
        area += vi[0] * vj[2] - vj[0] * vi[2];
        j = i;
    }
    (area + 1) / 2
}

fn area2(a: [i32; 4], b: [i32; 4], c: [i32; 4]) -> i64 {
    i64::from(b[0] - a[0]) * i64::from(c[2] - a[2])
        - i64::from(c[0] - a[0]) * i64::from(b[2] - a[2])
}

fn left(a: [i32; 4], b: [i32; 4], c: [i32; 4]) -> bool {
    area2(a, b, c) < 0
}

fn left_on(a: [i32; 4], b: [i32; 4], c: [i32; 4]) -> bool {
    area2(a, b, c) <= 0
}

fn collinear(a: [i32; 4], b: [i32; 4], c: [i32; 4]) -> bool {
    area2(a, b, c) == 0
}

fn intersect_prop(a: [i32; 4], b: [i32; 4], c: [i32; 4], d: [i32; 4]) -> bool {
    if collinear(a, b, c) || collinear(a, b, d) || collinear(c, d, a) || collinear(c, d, b) {
        return false;
    }
    (left(a, b, c) ^ left(a, b, d)) && (left(c, d, a) ^ left(c, d, b))
}

fn between(a: [i32; 4], b: [i32; 4], c: [i32; 4]) -> bool {
    if !collinear(a, b, c) {
        return false;
    }
    if a[0] != b[0] {
        (a[0] <= c[0] && c[0] <= b[0]) || (a[0] >= c[0] && c[0] >= b[0])
    } else {
        (a[2] <= c[2] && c[2] <= b[2]) || (a[2] >= c[2] && c[2] >= b[2])
    }
}

fn intersect(a: [i32; 4], b: [i32; 4], c: [i32; 4], d: [i32; 4]) -> bool {
    intersect_prop(a, b, c, d)
        || between(a, b, c)
        || between(a, b, d)
        || between(c, d, a)
        || between(c, d, b)
}

fn vequal(a: [i32; 4], b: [i32; 4]) -> bool {
    a[0] == b[0] && a[2] == b[2]
}

fn intersect_seg_contour(
    d0: [i32; 4],
    d1: [i32; 4],
    skip: Option<usize>,
    verts: &[[i32; 4]],
) -> bool {
    let n = verts.len();
    for k in 0..n {
        let k1 = (k + 1) % n;
        if skip == Some(k) || skip == Some(k1) {
            continue;
        }
        let p0 = verts[k];
        let p1 = verts[k1];
        if vequal(d0, p0) || vequal(d1, p0) || vequal(d0, p1) || vequal(d1, p1) {
            continue;
        }
        if intersect(d0, d1, p0, p1) {
            return true;
        }
    }
    false
}

fn in_cone_contour(i: usize, verts: &[[i32; 4]], pj: [i32; 4]) -> bool {
    let n = verts.len();
    let pi = verts[i];
    let pi1 = verts[(i + 1) % n];
    let pin1 = verts[(i + n - 1) % n];
    if left_on(pin1, pi, pi1) {
        return left(pi, pj, pin1) && left(pj, pi, pi1);
    }
    !(left_on(pi, pj, pi1) && left_on(pj, pi, pin1))
}

/// The height of a span corner: the highest floor around it, and whether it lies on
/// the tile border between two border cells (such vertices are removed later).
fn corner_height(chf: &CompactHeightfield, x: i32, z: i32, i: usize, dir: usize) -> (i32, bool) {
    let span = chf.spans[i];
    let mut ch = i32::from(span.y);
    let dirp = (dir + 1) & 3;
    let mut regs = [0u32; 4];
    regs[0] = u32::from(span.reg) | (u32::from(chf.areas[i]) << 16);
    if span.connected(dir) {
        let ax = x + dir_offset_x(dir);
        let az = z + dir_offset_z(dir);
        let ai = chf.neighbor_index(x, z, &span, dir);
        let a = chf.spans[ai];
        ch = ch.max(i32::from(a.y));
        regs[1] = u32::from(a.reg) | (u32::from(chf.areas[ai]) << 16);
        if a.connected(dirp) {
            let ai2 = chf.neighbor_index(ax, az, &a, dirp);
            let a2 = chf.spans[ai2];
            ch = ch.max(i32::from(a2.y));
            regs[2] = u32::from(a2.reg) | (u32::from(chf.areas[ai2]) << 16);
        }
    }
    if span.connected(dirp) {
        let ax = x + dir_offset_x(dirp);
        let az = z + dir_offset_z(dirp);
        let ai = chf.neighbor_index(x, z, &span, dirp);
        let a = chf.spans[ai];
        ch = ch.max(i32::from(a.y));
        regs[3] = u32::from(a.reg) | (u32::from(chf.areas[ai]) << 16);
        if a.connected(dir) {
            let ai2 = chf.neighbor_index(ax, az, &a, dir);
            let a2 = chf.spans[ai2];
            ch = ch.max(i32::from(a2.y));
            regs[2] = u32::from(a2.reg) | (u32::from(chf.areas[ai2]) << 16);
        }
    }
    let border = u32::from(BORDER_REG);
    let mut is_border_vertex = false;
    for j in 0..4 {
        let a = j;
        let b = (j + 1) & 3;
        let c = (j + 2) & 3;
        let d = (j + 3) & 3;
        let two_same_exts = (regs[a] & regs[b] & border) != 0 && regs[a] == regs[b];
        let two_ints = ((regs[c] | regs[d]) & border) == 0;
        let ints_same_area = (regs[c] >> 16) == (regs[d] >> 16);
        let no_zeros = regs[a] != 0 && regs[b] != 0 && regs[c] != 0 && regs[d] != 0;
        if two_same_exts && two_ints && ints_same_area && no_zeros {
            is_border_vertex = true;
            break;
        }
    }
    (ch, is_border_vertex)
}

fn walk_contour(
    chf: &CompactHeightfield,
    mut x: i32,
    mut z: i32,
    mut i: usize,
    flags: &mut [u8],
    points: &mut Vec<[i32; 4]>,
) {
    let mut dir = 0;
    while flags[i] & (1 << dir) == 0 {
        dir += 1;
    }
    let start_dir = dir;
    let start_i = i;
    let area = chf.areas[i];
    let mut iter = 0;
    loop {
        iter += 1;
        if iter >= 40000 {
            break;
        }
        if flags[i] & (1 << dir) != 0 {
            let (py, is_border_vertex) = corner_height(chf, x, z, i, dir);
            let mut px = x;
            let mut pz = z;
            match dir {
                0 => pz += 1,
                1 => {
                    px += 1;
                    pz += 1;
                }
                2 => px += 1,
                _ => {}
            }
            let span = chf.spans[i];
            let mut r = 0i32;
            let mut is_area_border = false;
            if span.connected(dir) {
                let ai = chf.neighbor_index(x, z, &span, dir);
                r = i32::from(chf.spans[ai].reg);
                if area != chf.areas[ai] {
                    is_area_border = true;
                }
            }
            if is_border_vertex {
                r |= BORDER_VERTEX;
            }
            if is_area_border {
                r |= AREA_BORDER;
            }
            points.push([px, py, pz, r]);
            flags[i] &= !(1 << dir);
            dir = (dir + 1) & 3;
        } else {
            let span = chf.spans[i];
            if !span.connected(dir) {
                return;
            }
            let ni = chf.neighbor_index(x, z, &span, dir);
            x += dir_offset_x(dir);
            z += dir_offset_z(dir);
            i = ni;
            dir = (dir + 3) & 3;
        }
        if start_i == i && start_dir == dir {
            break;
        }
    }
}

/// Squared distance of (x, z) from the segment p..q.
fn distance_pt_seg(x: i32, z: i32, px: i32, pz: i32, qx: i32, qz: i32) -> f32 {
    let pqx = (qx - px) as f32;
    let pqz = (qz - pz) as f32;
    let mut dx = (x - px) as f32;
    let mut dz = (z - pz) as f32;
    let d = pqx * pqx + pqz * pqz;
    let mut t = pqx * dx + pqz * dz;
    if d > 0.0 {
        t /= d;
    }
    t = t.clamp(0.0, 1.0);
    dx = px as f32 + t * pqx - x as f32;
    dz = pz as f32 + t * pqz - z as f32;
    dx * dx + dz * dz
}

fn simplify_contour(
    points: &[[i32; 4]],
    simplified: &mut Vec<[i32; 4]>,
    max_error: f32,
    max_edge_len: i32,
) {
    let pn = points.len();
    if pn == 0 {
        return;
    }
    let has_connections = points.iter().any(|p| p[3] & CONTOUR_REG_MASK != 0);
    if has_connections {
        // A vertex wherever the neighbouring region (or area) changes.
        for i in 0..pn {
            let ii = (i + 1) % pn;
            let different_regs =
                (points[i][3] & CONTOUR_REG_MASK) != (points[ii][3] & CONTOUR_REG_MASK);
            let area_borders = (points[i][3] & AREA_BORDER) != (points[ii][3] & AREA_BORDER);
            if different_regs || area_borders {
                simplified.push([points[i][0], points[i][1], points[i][2], i as i32]);
            }
        }
    }
    if simplified.is_empty() {
        // No neighbours at all: start from the lower-left and upper-right vertices.
        let mut ll = (points[0], 0usize);
        let mut ur = (points[0], 0usize);
        for (i, p) in points.iter().enumerate() {
            if p[0] < ll.0[0] || (p[0] == ll.0[0] && p[2] < ll.0[2]) {
                ll = (*p, i);
            }
            if p[0] > ur.0[0] || (p[0] == ur.0[0] && p[2] > ur.0[2]) {
                ur = (*p, i);
            }
        }
        simplified.push([ll.0[0], ll.0[1], ll.0[2], ll.1 as i32]);
        simplified.push([ur.0[0], ur.0[1], ur.0[2], ur.1 as i32]);
    }

    // Add the furthest raw points until every one is within the error.
    let mut i = 0;
    while i < simplified.len() {
        let ii = (i + 1) % simplified.len();
        let (mut ax, mut az, ai) = (
            simplified[i][0],
            simplified[i][2],
            simplified[i][3] as usize,
        );
        let (mut bx, mut bz, bi) = (
            simplified[ii][0],
            simplified[ii][2],
            simplified[ii][3] as usize,
        );
        let mut maxd = 0.0f32;
        let mut maxi: Option<usize> = None;
        // Walk the raw points in lexicographic order, so opposite edges simplify alike.
        let (cinc, mut ci, endi) = if bx > ax || (bx == ax && bz > az) {
            (1, (ai + 1) % pn, bi)
        } else {
            std::mem::swap(&mut ax, &mut bx);
            std::mem::swap(&mut az, &mut bz);
            (pn - 1, (bi + pn - 1) % pn, ai)
        };
        // Only walls and area borders are tessellated.
        if (points[ci][3] & CONTOUR_REG_MASK) == 0 || (points[ci][3] & AREA_BORDER) != 0 {
            while ci != endi {
                let d = distance_pt_seg(points[ci][0], points[ci][2], ax, az, bx, bz);
                if d > maxd {
                    maxd = d;
                    maxi = Some(ci);
                }
                ci = (ci + cinc) % pn;
            }
        }
        match maxi {
            Some(m) if maxd > max_error * max_error => {
                simplified.insert(i + 1, [points[m][0], points[m][1], points[m][2], m as i32]);
            }
            _ => i += 1,
        }
    }

    // Split wall edges longer than the limit.
    if max_edge_len > 0 {
        let mut i = 0;
        while i < simplified.len() {
            let ii = (i + 1) % simplified.len();
            let (ax, az, ai) = (
                simplified[i][0],
                simplified[i][2],
                simplified[i][3] as usize,
            );
            let (bx, bz, bi) = (
                simplified[ii][0],
                simplified[ii][2],
                simplified[ii][3] as usize,
            );
            let ci = (ai + 1) % pn;
            let mut maxi = None;
            if points[ci][3] & CONTOUR_REG_MASK == 0 {
                let dx = bx - ax;
                let dz = bz - az;
                if dx * dx + dz * dz > max_edge_len * max_edge_len {
                    let n = if bi < ai { bi + pn - ai } else { bi - ai };
                    if n > 1 {
                        maxi = Some(if bx > ax || (bx == ax && bz > az) {
                            (ai + n / 2) % pn
                        } else {
                            (ai + n.div_ceil(2)) % pn
                        });
                    }
                }
            }
            match maxi {
                Some(m) => {
                    simplified.insert(i + 1, [points[m][0], points[m][1], points[m][2], m as i32]);
                }
                None => i += 1,
            }
        }
    }

    // Each vertex takes its border flag from its own raw point and its neighbour region
    // from the next one (the edge that starts at it).
    for v in simplified.iter_mut() {
        let ai = (v[3] as usize + 1) % pn;
        let bi = v[3] as usize;
        v[3] = (points[ai][3] & (CONTOUR_REG_MASK | AREA_BORDER)) | (points[bi][3] & BORDER_VERTEX);
    }
}

fn remove_degenerate_segments(simplified: &mut Vec<[i32; 4]>) {
    let mut npts = simplified.len();
    let mut i = 0;
    while i < npts {
        let ni = (i + 1) % npts;
        if vequal(simplified[i], simplified[ni]) {
            simplified.remove(i);
            npts -= 1;
        }
        i += 1;
    }
}
