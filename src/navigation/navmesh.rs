//! The runtime navmesh: tiles of convex polygons linked to their neighbours, inside a
//! tile and across tile borders, with height detail for placing agents on the floor.
//!
//! Polygons are named by [`PolyRef`]s that carry their tile's salt: rebuilding a tile
//! (an obstacle carved it) makes old references to it invalid, which is how agents
//! notice they must find a new path.

use std::collections::HashMap;

use glam::{Vec2, Vec3};

use super::recast::{PolyMesh, PolyMeshDetail};

/// Most vertices of a navmesh polygon.
pub const MAX_VERTS_PER_POLY: usize = 6;
/// A neighbour slot of an edge on the tile border: the low bits hold the side.
pub(crate) const EXT_LINK: u16 = 0x8000;

/// A polygon of a [`NavMesh`]: salt, tile slot and index packed together.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct PolyRef(pub u64);

impl PolyRef {
    pub const NONE: PolyRef = PolyRef(0);

    pub(crate) fn new(salt: u32, tile: u32, poly: u32) -> Self {
        Self(
            (u64::from(salt & 0xffff) << 48)
                | (u64::from(tile & 0xff_ffff) << 24)
                | u64::from(poly & 0xff_ffff),
        )
    }

    pub fn is_none(self) -> bool {
        self.0 == 0
    }

    pub(crate) fn salt(self) -> u32 {
        (self.0 >> 48) as u32
    }

    pub(crate) fn tile(self) -> u32 {
        ((self.0 >> 24) & 0xff_ffff) as u32
    }

    pub(crate) fn poly(self) -> u32 {
        (self.0 & 0xff_ffff) as u32
    }
}

/// How a polygon is entered: across the floor, or by an off-mesh link's jump.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PolyKind {
    #[default]
    Ground,
    OffMesh,
}

#[derive(Debug, Clone)]
pub struct NavPoly {
    pub verts: [u16; MAX_VERTS_PER_POLY],
    pub vert_count: u8,
    /// Per edge: 0 none, `1..` the neighbour polygon in this tile plus one, or
    /// `EXT_LINK | side` on the tile border.
    pub neis: [u16; MAX_VERTS_PER_POLY],
    pub area: u8,
    pub kind: PolyKind,
}

/// A way from one polygon to another across one of its edges (or the start or end of
/// an off-mesh link).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Link {
    pub target: PolyRef,
    pub edge: u8,
    /// The tile side for links across tile borders (0 -x, 1 +z, 2 +x, 3 -z), 0xff
    /// otherwise.
    pub side: u8,
    /// The part of the edge (0..1 from its first vertex) the link spans.
    pub tmin: f32,
    pub tmax: f32,
}

/// A link between two points that is not walked across the floor: a jump down, a
/// gap, a ladder.
#[derive(Debug, Clone, PartialEq)]
pub struct OffMeshConnection {
    pub start: Vec3,
    pub end: Vec3,
    /// How far from its ends the link reaches for the floor (m).
    pub radius: f32,
    pub bidirectional: bool,
    pub area: u8,
    /// Who placed it (entity bits), handed to scripts as the link's id.
    pub user_id: u64,
}

#[derive(Debug, Clone)]
pub struct NavTile {
    pub x: i32,
    pub z: i32,
    pub(crate) salt: u32,
    pub bmin: Vec3,
    pub bmax: Vec3,
    pub verts: Vec<Vec3>,
    pub polys: Vec<NavPoly>,
    pub links: Vec<Vec<Link>>,
    /// Per polygon: first detail vertex, count, first detail triangle, count.
    pub detail_meshes: Vec<[u32; 4]>,
    pub detail_verts: Vec<Vec3>,
    pub detail_tris: Vec<[u8; 3]>,
    pub poly_bounds: Vec<(Vec3, Vec3)>,
    /// Off-mesh links starting in this tile; polygon `ground_count + i` is link `i`.
    pub off_mesh: Vec<OffMeshConnection>,
    pub ground_count: usize,
}

impl NavTile {
    /// The tile's ground polygons from the generator's output (`mesh` in cells from
    /// its `bmin`, which must be the tile's own corner).
    pub(crate) fn from_poly_mesh(x: i32, z: i32, mesh: &PolyMesh, detail: &PolyMeshDetail) -> Self {
        let verts: Vec<Vec3> = mesh
            .verts
            .iter()
            .map(|v| {
                Vec3::new(
                    mesh.bmin.x + f32::from(v[0]) * mesh.cs,
                    mesh.bmin.y + f32::from(v[1]) * mesh.ch,
                    mesh.bmin.z + f32::from(v[2]) * mesh.cs,
                )
            })
            .collect();
        let nvp = mesh.nvp;
        let mut polys = Vec::with_capacity(mesh.npolys);
        for pi in 0..mesh.npolys {
            let p = mesh.poly(pi);
            let count = mesh.vert_count(pi).min(MAX_VERTS_PER_POLY);
            let mut poly = NavPoly {
                verts: [0; MAX_VERTS_PER_POLY],
                vert_count: count as u8,
                neis: [0; MAX_VERTS_PER_POLY],
                area: mesh.areas[pi],
                kind: PolyKind::Ground,
            };
            for j in 0..count {
                poly.verts[j] = p[j];
                let nei = p[nvp + j];
                poly.neis[j] = if nei == super::recast::polymesh::MESH_NULL_IDX {
                    0
                } else if nei & super::recast::polymesh::PORTAL_FLAG != 0 {
                    EXT_LINK | (nei & 0x3)
                } else {
                    nei + 1
                };
            }
            polys.push(poly);
        }
        let mut tile = Self {
            x,
            z,
            salt: 0,
            bmin: mesh.bmin,
            bmax: mesh.bmax,
            verts,
            links: vec![Vec::new(); polys.len()],
            ground_count: polys.len(),
            polys,
            detail_meshes: detail.meshes.clone(),
            detail_verts: detail.verts.clone(),
            detail_tris: detail.tris.clone(),
            poly_bounds: Vec::new(),
            off_mesh: Vec::new(),
        };
        tile.compute_bounds();
        tile
    }

    pub(crate) fn compute_bounds(&mut self) {
        self.poly_bounds = (0..self.polys.len())
            .map(|i| {
                let mut lo = Vec3::splat(f32::MAX);
                let mut hi = Vec3::splat(f32::MIN);
                for v in self.poly_points(i) {
                    lo = lo.min(v);
                    hi = hi.max(v);
                }
                if let Some(&[first, count, _, _]) = self.detail_meshes.get(i) {
                    for v in &self.detail_verts[first as usize..(first + count) as usize] {
                        lo = lo.min(*v);
                        hi = hi.max(*v);
                    }
                }
                (lo, hi)
            })
            .collect();
        let mut lo = self.bmin;
        let mut hi = self.bmax;
        for (a, b) in &self.poly_bounds {
            lo = lo.min(*a);
            hi = hi.max(*b);
        }
        self.bmin.y = lo.y;
        self.bmax.y = hi.y;
    }

    /// The polygon's vertices in order.
    pub fn poly_points(&self, index: usize) -> impl Iterator<Item = Vec3> + '_ {
        let poly = &self.polys[index];
        poly.verts[..usize::from(poly.vert_count)]
            .iter()
            .map(|&v| self.verts[usize::from(v)])
    }

    pub fn poly_center(&self, index: usize) -> Vec3 {
        let poly = &self.polys[index];
        let sum: Vec3 = self.poly_points(index).sum();
        sum / f32::from(poly.vert_count.max(1))
    }

    /// The detail triangles of a ground polygon, as world points.
    pub fn detail_triangles(&self, index: usize) -> impl Iterator<Item = [Vec3; 3]> + '_ {
        let [vbase, _, tbase, tcount] = self.detail_meshes.get(index).copied().unwrap_or([0; 4]);
        self.detail_tris[tbase as usize..(tbase + tcount) as usize]
            .iter()
            .map(move |t| {
                [
                    self.detail_verts[(vbase + u32::from(t[0])) as usize],
                    self.detail_verts[(vbase + u32::from(t[1])) as usize],
                    self.detail_verts[(vbase + u32::from(t[2])) as usize],
                ]
            })
    }

    /// The floor height under `pos` on the polygon, if its x/z lies inside.
    pub fn poly_height(&self, index: usize, pos: Vec3) -> Option<f32> {
        let poly = &self.polys[index];
        if poly.kind == PolyKind::OffMesh {
            return None;
        }
        let has_detail = self.detail_meshes.get(index).is_some_and(|m| m[3] > 0);
        if has_detail {
            for [a, b, c] in self.detail_triangles(index) {
                if let Some(h) = height_on_triangle(pos, a, b, c) {
                    return Some(h);
                }
            }
        }
        // No detail (or a point on its very edge): fan over the polygon.
        let points: Vec<Vec3> = self.poly_points(index).collect();
        for i in 1..points.len().saturating_sub(1) {
            if let Some(h) = height_on_triangle(pos, points[0], points[i], points[i + 1]) {
                return Some(h);
            }
        }
        None
    }

    /// The point of the polygon nearest `pos` and whether `pos` lies over it (x/z).
    pub fn closest_point_on_poly(&self, index: usize, pos: Vec3) -> (Vec3, bool) {
        let points: Vec<Vec3> = self.poly_points(index).collect();
        if self.polys[index].kind == PolyKind::OffMesh {
            let (a, b) = (points[0], points[1]);
            let t = closest_t_on_segment(pos, a, b);
            return (a.lerp(b, t), false);
        }
        if point_in_polygon_xz(pos, &points) {
            let h = self.poly_height(index, pos).unwrap_or(pos.y);
            return (Vec3::new(pos.x, h, pos.z), true);
        }
        let edge_point = closest_point_on_boundary_xz(pos, &points);
        let h = self.poly_height(index, edge_point).unwrap_or(edge_point.y);
        (Vec3::new(edge_point.x, h, edge_point.z), false)
    }
}

/// Grows a little, so points on shared edges count as inside either triangle.
const TRIANGLE_EPS: f32 = 1e-4;

pub(crate) fn height_on_triangle(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
    let v0 = Vec2::new(c.x - a.x, c.z - a.z);
    let v1 = Vec2::new(b.x - a.x, b.z - a.z);
    let v2 = Vec2::new(p.x - a.x, p.z - a.z);
    let denom = v0.x * v1.y - v0.y * v1.x;
    if denom.abs() < 1e-12 {
        return None;
    }
    let u = (v1.y * v2.x - v1.x * v2.y) / denom;
    let v = (v0.x * v2.y - v0.y * v2.x) / denom;
    if u >= -TRIANGLE_EPS && v >= -TRIANGLE_EPS && u + v <= 1.0 + TRIANGLE_EPS {
        Some(a.y + (c.y - a.y) * u + (b.y - a.y) * v)
    } else {
        None
    }
}

pub(crate) fn point_in_polygon_xz(p: Vec3, points: &[Vec3]) -> bool {
    let mut inside = false;
    let n = points.len();
    let mut j = n.wrapping_sub(1);
    for i in 0..n {
        let (vi, vj) = (points[i], points[j]);
        if ((vi.z > p.z) != (vj.z > p.z))
            && (p.x < (vj.x - vi.x) * (p.z - vi.z) / (vj.z - vi.z) + vi.x)
        {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Squared x/z distance from `p` to segment a..b, and the parameter of the nearest point.
pub(crate) fn distance_pt_seg_sqr_xz(p: Vec3, a: Vec3, b: Vec3) -> (f32, f32) {
    let (pqx, pqz) = (b.x - a.x, b.z - a.z);
    let (dx, dz) = (p.x - a.x, p.z - a.z);
    let d = pqx * pqx + pqz * pqz;
    let mut t = pqx * dx + pqz * dz;
    if d > 0.0 {
        t /= d;
    }
    let t = t.clamp(0.0, 1.0);
    let (ex, ez) = (a.x + t * pqx - p.x, a.z + t * pqz - p.z);
    (ex * ex + ez * ez, t)
}

fn closest_t_on_segment(p: Vec3, a: Vec3, b: Vec3) -> f32 {
    let ab = b - a;
    let d = ab.length_squared();
    if d <= 0.0 {
        0.0
    } else {
        ((p - a).dot(ab) / d).clamp(0.0, 1.0)
    }
}

pub(crate) fn closest_point_on_boundary_xz(p: Vec3, points: &[Vec3]) -> Vec3 {
    let n = points.len();
    let mut best = (f32::MAX, points[0]);
    let mut j = n - 1;
    for i in 0..n {
        let (d, t) = distance_pt_seg_sqr_xz(p, points[j], points[i]);
        if d < best.0 {
            best = (d, points[j].lerp(points[i], t));
        }
        j = i;
    }
    best.1
}

/// Tiles of polygons over a grid of `tile_size` squares from `origin`.
#[derive(Debug, Clone)]
pub struct NavMesh {
    pub origin: Vec3,
    pub tile_size: f32,
    /// How far (m) floors on either side of a tile border may differ and still link.
    pub walkable_climb: f32,
    pub(crate) tiles: Vec<Option<NavTile>>,
    salts: Vec<u32>,
    free: Vec<u32>,
    lookup: HashMap<(i32, i32), u32>,
}

/// The tile grid neighbour across `side`.
pub(crate) fn side_offset(side: u8) -> (i32, i32) {
    match side & 3 {
        0 => (-1, 0),
        1 => (0, 1),
        2 => (1, 0),
        _ => (0, -1),
    }
}

fn opposite_side(side: u8) -> u8 {
    (side + 2) & 3
}

impl NavMesh {
    pub fn new(origin: Vec3, tile_size: f32, walkable_climb: f32) -> Self {
        Self {
            origin,
            tile_size,
            walkable_climb,
            tiles: Vec::new(),
            salts: Vec::new(),
            free: Vec::new(),
            lookup: HashMap::new(),
        }
    }

    /// The tile coordinate holding world `x`, `z`.
    pub fn tile_at(&self, x: f32, z: f32) -> (i32, i32) {
        (
            ((x - self.origin.x) / self.tile_size).floor() as i32,
            ((z - self.origin.z) / self.tile_size).floor() as i32,
        )
    }

    pub fn tile_count(&self) -> usize {
        self.lookup.len()
    }

    pub fn poly_count(&self) -> usize {
        self.tiles().map(|tile| tile.ground_count).sum()
    }

    pub fn tiles(&self) -> impl Iterator<Item = &NavTile> {
        self.tiles.iter().flatten()
    }

    /// Tile slots with their tiles, for building references.
    pub(crate) fn tiles_with_slots(&self) -> impl Iterator<Item = (u32, &NavTile)> {
        self.tiles
            .iter()
            .enumerate()
            .filter_map(|(i, t)| t.as_ref().map(|t| (i as u32, t)))
    }

    pub fn tile(&self, x: i32, z: i32) -> Option<&NavTile> {
        let slot = *self.lookup.get(&(x, z))?;
        self.tiles[slot as usize].as_ref()
    }

    pub(crate) fn tile_slot(&self, x: i32, z: i32) -> Option<u32> {
        self.lookup.get(&(x, z)).copied()
    }

    /// The tile and polygon `r` names, if it is still valid.
    pub fn tile_and_poly(&self, r: PolyRef) -> Option<(&NavTile, &NavPoly)> {
        if r.is_none() {
            return None;
        }
        let tile = self.tiles.get(r.tile() as usize)?.as_ref()?;
        if tile.salt != r.salt() {
            return None;
        }
        let poly = tile.polys.get(r.poly() as usize)?;
        Some((tile, poly))
    }

    pub fn is_valid(&self, r: PolyRef) -> bool {
        self.tile_and_poly(r).is_some()
    }

    pub(crate) fn poly_ref(&self, slot: u32, poly: usize) -> PolyRef {
        let salt = self.tiles[slot as usize].as_ref().map_or(0, |t| t.salt);
        PolyRef::new(salt, slot, poly as u32)
    }

    pub fn links(&self, r: PolyRef) -> &[Link] {
        match self.tiles.get(r.tile() as usize).and_then(|t| t.as_ref()) {
            Some(tile) if tile.salt == r.salt() => tile
                .links
                .get(r.poly() as usize)
                .map_or(&[][..], Vec::as_slice),
            _ => &[],
        }
    }

    /// Add (or replace) the tile at its coordinate, linking it to its neighbours.
    /// Returns its slot.
    pub fn add_tile(&mut self, mut tile: NavTile) -> u32 {
        // A rebuilt tile keeps the off-mesh links of the one it replaces.
        let carried = match self.tile(tile.x, tile.z) {
            Some(old) if tile.off_mesh.is_empty() && !old.off_mesh.is_empty() => {
                old.off_mesh.clone()
            }
            _ => Vec::new(),
        };
        self.remove_tile(tile.x, tile.z);
        let slot = self.free.pop().unwrap_or_else(|| {
            self.tiles.push(None);
            self.salts.push(0);
            (self.tiles.len() - 1) as u32
        });
        // Salt 0 is never used, so PolyRef::NONE stays invalid.
        let salt = (self.salts[slot as usize] % 0xfffe) + 1;
        self.salts[slot as usize] = salt;
        tile.salt = salt;
        for links in tile.links.iter_mut() {
            links.clear();
        }
        let key = (tile.x, tile.z);
        self.tiles[slot as usize] = Some(tile);
        self.lookup.insert(key, slot);
        self.connect_internal_links(slot);
        for side in 0..4u8 {
            let (dx, dz) = side_offset(side);
            if let Some(neighbour) = self.tile_slot(key.0 + dx, key.1 + dz) {
                self.connect_external_links(slot, neighbour, side);
                self.connect_external_links(neighbour, slot, opposite_side(side));
            }
        }
        self.connect_off_mesh_links(slot);
        if !carried.is_empty() {
            self.set_off_mesh_links(key.0, key.1, carried);
        }
        slot
    }

    /// Remove the tile at a coordinate (and every link into it).
    pub fn remove_tile(&mut self, x: i32, z: i32) -> bool {
        let Some(slot) = self.lookup.remove(&(x, z)) else {
            return false;
        };
        let salt = self.tiles[slot as usize].as_ref().map_or(0, |t| t.salt);
        self.tiles[slot as usize] = None;
        self.free.push(slot);
        // Links from the neighbours (and from any off-mesh link landing here).
        for tile in self.tiles.iter_mut().flatten() {
            for links in tile.links.iter_mut() {
                links.retain(|link| !(link.target.tile() == slot && link.target.salt() == salt));
            }
        }
        true
    }

    fn connect_internal_links(&mut self, slot: u32) {
        let salt = self.tiles[slot as usize].as_ref().map_or(0, |t| t.salt);
        let Some(tile) = self.tiles[slot as usize].as_mut() else {
            return;
        };
        for (pi, poly) in tile.polys.iter().enumerate() {
            if poly.kind != PolyKind::Ground {
                continue;
            }
            for j in 0..usize::from(poly.vert_count) {
                let nei = poly.neis[j];
                if nei == 0 || nei & EXT_LINK != 0 {
                    continue;
                }
                tile.links[pi].push(Link {
                    target: PolyRef::new(salt, slot, u32::from(nei - 1)),
                    edge: j as u8,
                    side: 0xff,
                    tmin: 0.0,
                    tmax: 1.0,
                });
            }
        }
    }

    /// Link `from`'s border edges on `side` to the polygons of `to` they touch.
    fn connect_external_links(&mut self, from: u32, to: u32, side: u8) {
        let Some(target) = self.tiles[to as usize].as_ref() else {
            return;
        };
        let target_salt = target.salt;
        let opposite = EXT_LINK | u16::from(opposite_side(side));
        // The target's border edges facing back, as slabs.
        let mut candidates: Vec<(u32, Vec3, Vec3)> = Vec::new();
        for (pi, poly) in target.polys.iter().enumerate() {
            let n = usize::from(poly.vert_count);
            for j in 0..n {
                if poly.neis[j] == opposite {
                    candidates.push((
                        pi as u32,
                        target.verts[usize::from(poly.verts[j])],
                        target.verts[usize::from(poly.verts[(j + 1) % n])],
                    ));
                }
            }
        }
        let climb = self.walkable_climb;
        let Some(tile) = self.tiles[from as usize].as_mut() else {
            return;
        };
        let ext = EXT_LINK | u16::from(side);
        for pi in 0..tile.polys.len() {
            let poly = &tile.polys[pi];
            let n = usize::from(poly.vert_count);
            // Drop links toward this side made before (the neighbour was replaced).
            tile.links[pi].retain(|link| link.side != side);
            for j in 0..n {
                if poly.neis[j] != ext {
                    continue;
                }
                let va = tile.verts[usize::from(poly.verts[j])];
                let vb = tile.verts[usize::from(poly.verts[(j + 1) % n])];
                let (amin, amax) = slab_end_points(va, vb, side);
                let apos = slab_coord(va, side);
                for &(target_poly, vc, vd) in &candidates {
                    if (apos - slab_coord(vc, side)).abs() > 0.01 {
                        continue;
                    }
                    let (bmin, bmax) = slab_end_points(vc, vd, side);
                    if !overlap_slabs(amin, amax, bmin, bmax, 0.01, climb) {
                        continue;
                    }
                    let lo = amin.x.max(bmin.x);
                    let hi = amax.x.min(bmax.x);
                    let (ta, tb) = match side {
                        0 | 2 => (va.z, vb.z),
                        _ => (va.x, vb.x),
                    };
                    let span = tb - ta;
                    if span.abs() < 1e-6 {
                        continue;
                    }
                    let mut tmin = ((lo - ta) / span).clamp(0.0, 1.0);
                    let mut tmax = ((hi - ta) / span).clamp(0.0, 1.0);
                    if tmin > tmax {
                        std::mem::swap(&mut tmin, &mut tmax);
                    }
                    tile.links[pi].push(Link {
                        target: PolyRef::new(target_salt, to, target_poly),
                        edge: j as u8,
                        side,
                        tmin,
                        tmax,
                    });
                }
            }
        }
    }

    /// The ground polygon nearest `pos` within `half_extents`, its nearest point and
    /// whether `pos` lies over it. Polygons right under (or over) the point win over
    /// ones beside it, so an agent on a bridge stays on the bridge.
    pub fn find_nearest_poly(&self, pos: Vec3, half_extents: Vec3) -> Option<(PolyRef, Vec3)> {
        let bmin = pos - half_extents;
        let bmax = pos + half_extents;
        let mut best: Option<(f32, PolyRef, Vec3)> = None;
        self.query_polygons(bmin, bmax, |r, tile, index| {
            let (closest, over) = tile.closest_point_on_poly(index, pos);
            let d = if over {
                let dy = (pos.y - closest.y).abs() - self.walkable_climb;
                if dy > 0.0 {
                    dy * dy
                } else {
                    0.0
                }
            } else {
                (pos - closest).length_squared()
            };
            if best.is_none_or(|b| d < b.0) {
                best = Some((d, r, closest));
            }
        });
        best.map(|(_, r, p)| (r, p))
    }

    /// Call `f` with every ground polygon whose bounds overlap the box.
    pub fn query_polygons(
        &self,
        bmin: Vec3,
        bmax: Vec3,
        mut f: impl FnMut(PolyRef, &NavTile, usize),
    ) {
        let (x0, z0) = self.tile_at(bmin.x, bmin.z);
        let (x1, z1) = self.tile_at(bmax.x, bmax.z);
        for tz in z0..=z1 {
            for tx in x0..=x1 {
                let Some(slot) = self.tile_slot(tx, tz) else {
                    continue;
                };
                let Some(tile) = self.tiles[slot as usize].as_ref() else {
                    continue;
                };
                for (index, (lo, hi)) in tile.poly_bounds.iter().enumerate().take(tile.ground_count)
                {
                    if lo.x > bmax.x
                        || hi.x < bmin.x
                        || lo.y > bmax.y
                        || hi.y < bmin.y
                        || lo.z > bmax.z
                        || hi.z < bmin.z
                    {
                        continue;
                    }
                    f(PolyRef::new(tile.salt, slot, index as u32), tile, index);
                }
            }
        }
    }

    /// The floor height under `pos` on polygon `r`.
    pub fn poly_height(&self, r: PolyRef, pos: Vec3) -> Option<f32> {
        let (tile, _) = self.tile_and_poly(r)?;
        tile.poly_height(r.poly() as usize, pos)
    }

    pub fn closest_point_on_poly(&self, r: PolyRef, pos: Vec3) -> Option<(Vec3, bool)> {
        let (tile, _) = self.tile_and_poly(r)?;
        Some(tile.closest_point_on_poly(r.poly() as usize, pos))
    }

    /// The vertices of polygon `r`.
    pub fn poly_vertices(&self, r: PolyRef) -> Vec<Vec3> {
        match self.tile_and_poly(r) {
            Some((tile, _)) => tile.poly_points(r.poly() as usize).collect(),
            None => Vec::new(),
        }
    }

    pub fn poly_area(&self, r: PolyRef) -> Option<u8> {
        self.tile_and_poly(r).map(|(_, poly)| poly.area)
    }

    /// The edge `from` and `to` share, as the funnel sees it (`left`, `right`), clamped to
    /// the part a tile-border link spans; and whether `to` is an off-mesh link.
    pub fn portal_points(
        &self,
        from: PolyRef,
        to: PolyRef,
    ) -> Option<(Vec3, Vec3, PolyKind, PolyKind)> {
        let (from_tile, from_poly) = self.tile_and_poly(from)?;
        let (to_tile, to_poly) = self.tile_and_poly(to)?;
        let link = self.links(from).iter().find(|link| link.target == to)?;
        // Into or out of an off-mesh link: its end point is the portal.
        if from_poly.kind == PolyKind::OffMesh {
            let points: Vec<Vec3> = from_tile.poly_points(from.poly() as usize).collect();
            let at = if link.edge == 0 { points[0] } else { points[1] };
            return Some((at, at, from_poly.kind, to_poly.kind));
        }
        if to_poly.kind == PolyKind::OffMesh {
            let points: Vec<Vec3> = to_tile.poly_points(to.poly() as usize).collect();
            // The end of the link that lies on `from`.
            let back = self.links(to).iter().find(|l| l.target == from);
            let at = match back {
                Some(l) if l.edge == 1 => points[1],
                _ => points[0],
            };
            return Some((at, at, from_poly.kind, to_poly.kind));
        }
        let n = usize::from(from_poly.vert_count);
        let edge = usize::from(link.edge);
        let v0 = from_tile.verts[usize::from(from_poly.verts[edge])];
        let v1 = from_tile.verts[usize::from(from_poly.verts[(edge + 1) % n])];
        let (left, right) = if link.side != 0xff && (link.tmin > 0.0 || link.tmax < 1.0) {
            (v0.lerp(v1, link.tmin), v0.lerp(v1, link.tmax))
        } else {
            (v0, v1)
        };
        Some((left, right, from_poly.kind, to_poly.kind))
    }

    /// The midpoint of the portal between two polygons.
    pub fn edge_mid_point(&self, from: PolyRef, to: PolyRef) -> Option<Vec3> {
        let (left, right, _, _) = self.portal_points(from, to)?;
        Some((left + right) * 0.5)
    }

    /// Off-mesh links whose start lies in this tile become polygons linked to the
    /// floor at both ends.
    fn connect_off_mesh_links(&mut self, slot: u32) {
        // Links of this tile, and links of other tiles landing in it.
        let mut landing_here: Vec<(u32, usize)> = Vec::new();
        for (other_slot, tile) in self.tiles_with_slots() {
            for (i, link) in tile.off_mesh.iter().enumerate() {
                if other_slot == slot || self.tile_at(link.end.x, link.end.z) == self.coord_of(slot)
                {
                    landing_here.push((other_slot, i));
                }
            }
        }
        for (owner, i) in landing_here {
            self.link_off_mesh(owner, i);
        }
    }

    fn coord_of(&self, slot: u32) -> (i32, i32) {
        self.tiles[slot as usize]
            .as_ref()
            .map_or((i32::MIN, i32::MIN), |t| (t.x, t.z))
    }

    /// (Re)link off-mesh connection `i` of tile `owner` to the floor at both ends.
    fn link_off_mesh(&mut self, owner: u32, i: usize) {
        let Some(tile) = self.tiles[owner as usize].as_ref() else {
            return;
        };
        let connection = tile.off_mesh[i].clone();
        let link_poly = tile.ground_count + i;
        let link_ref = PolyRef::new(tile.salt, owner, link_poly as u32);
        let extents = Vec3::new(
            connection.radius,
            self.walkable_climb.max(connection.radius),
            connection.radius,
        );
        let start = self
            .find_nearest_poly(connection.start, extents)
            .map(|(r, _)| r);
        let end = self
            .find_nearest_poly(connection.end, extents)
            .map(|(r, _)| r);
        // Clear what this link had and what pointed at it.
        if let Some(tile) = self.tiles[owner as usize].as_mut() {
            tile.links[link_poly].clear();
        }
        for tile in self.tiles.iter_mut().flatten() {
            for links in tile.links.iter_mut() {
                links.retain(|link| link.target != link_ref);
            }
        }
        let add = |mesh: &mut NavMesh, from: PolyRef, link: Link| {
            if let Some(tile) = mesh.tiles[from.tile() as usize].as_mut() {
                if tile.salt == from.salt() {
                    if let Some(links) = tile.links.get_mut(from.poly() as usize) {
                        links.push(link);
                    }
                }
            }
        };
        let link = |target: PolyRef, edge: u8| Link {
            target,
            edge,
            side: 0xff,
            tmin: 0.0,
            tmax: 1.0,
        };
        if let (Some(start), Some(end)) = (start, end) {
            // The link polygon reaches both ends (edge 0 its start, edge 1 its end);
            // the floor enters it from the start, and from the end both ways.
            add(self, link_ref, link(start, 0));
            add(self, link_ref, link(end, 1));
            add(self, start, link(link_ref, 0xff));
            if connection.bidirectional {
                add(self, end, link(link_ref, 0xff));
            }
        }
    }

    /// Give tile `(x, z)` the off-mesh links that start in it (replacing its old ones).
    pub fn set_off_mesh_links(&mut self, x: i32, z: i32, connections: Vec<OffMeshConnection>) {
        let Some(slot) = self.tile_slot(x, z) else {
            return;
        };
        let old_salt = self.tiles[slot as usize].as_ref().map_or(0, |t| t.salt);
        let Some(tile) = self.tiles[slot as usize].as_mut() else {
            return;
        };
        let ground = tile.ground_count;
        let old_links: Vec<PolyRef> = (ground..tile.polys.len())
            .map(|i| PolyRef::new(old_salt, slot, i as u32))
            .collect();
        tile.polys.truncate(ground);
        tile.links.truncate(ground);
        tile.poly_bounds.truncate(ground);
        tile.off_mesh = connections;
        for (i, connection) in tile.off_mesh.iter().enumerate() {
            let base = tile.verts.len() as u16;
            tile.verts.push(connection.start);
            tile.verts.push(connection.end);
            let mut poly = NavPoly {
                verts: [0; MAX_VERTS_PER_POLY],
                vert_count: 2,
                neis: [0; MAX_VERTS_PER_POLY],
                area: connection.area,
                kind: PolyKind::OffMesh,
            };
            poly.verts[0] = base;
            poly.verts[1] = base + 1;
            tile.polys.push(poly);
            tile.links.push(Vec::new());
            tile.poly_bounds.push((
                connection.start.min(connection.end),
                connection.start.max(connection.end),
            ));
            let _ = i;
        }
        for tile in self.tiles.iter_mut().flatten() {
            for links in tile.links.iter_mut() {
                links.retain(|link| !old_links.contains(&link.target));
            }
        }
        for i in 0..self.tiles[slot as usize]
            .as_ref()
            .map_or(0, |t| t.off_mesh.len())
        {
            self.link_off_mesh(slot, i);
        }
    }

    /// The off-mesh connection polygon `r` stands for.
    pub fn off_mesh_connection(&self, r: PolyRef) -> Option<&OffMeshConnection> {
        let (tile, poly) = self.tile_and_poly(r)?;
        if poly.kind != PolyKind::OffMesh {
            return None;
        }
        tile.off_mesh.get(r.poly() as usize - tile.ground_count)
    }
}

fn slab_coord(v: Vec3, side: u8) -> f32 {
    match side {
        0 | 2 => v.x,
        _ => v.z,
    }
}

/// An edge on a tile border as (position along the border, height) end points.
fn slab_end_points(va: Vec3, vb: Vec3, side: u8) -> (Vec2, Vec2) {
    let (a, b) = match side {
        0 | 2 => (Vec2::new(va.z, va.y), Vec2::new(vb.z, vb.y)),
        _ => (Vec2::new(va.x, va.y), Vec2::new(vb.x, vb.y)),
    };
    if a.x < b.x {
        (a, b)
    } else {
        (b, a)
    }
}

/// Whether two border edges overlap along the border and lie within `py` of each other
/// in height (or cross).
fn overlap_slabs(amin: Vec2, amax: Vec2, bmin: Vec2, bmax: Vec2, px: f32, py: f32) -> bool {
    let minx = (amin.x + px).max(bmin.x + px);
    let maxx = (amax.x - px).min(bmax.x - px);
    if minx > maxx {
        return false;
    }
    let slope = |lo: Vec2, hi: Vec2| {
        let dx = hi.x - lo.x;
        if dx.abs() < 1e-9 {
            0.0
        } else {
            (hi.y - lo.y) / dx
        }
    };
    let ad = slope(amin, amax);
    let ak = amin.y - ad * amin.x;
    let bd = slope(bmin, bmax);
    let bk = bmin.y - bd * bmin.x;
    let dmin = (bd * minx + bk) - (ad * minx + ak);
    let dmax = (bd * maxx + bk) - (ad * maxx + ak);
    if dmin * dmax < 0.0 {
        return true;
    }
    let thr = (py * 2.0) * (py * 2.0);
    dmin * dmin <= thr || dmax * dmax <= thr
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poly_refs_pack_and_unpack() {
        let r = PolyRef::new(7, 1234, 56);
        assert_eq!((r.salt(), r.tile(), r.poly()), (7, 1234, 56));
        assert!(!r.is_none());
        assert!(PolyRef::NONE.is_none());
    }

    #[test]
    fn heights_interpolate_on_triangles() {
        let a = Vec3::new(0.0, 0.0, 0.0);
        let b = Vec3::new(2.0, 2.0, 0.0);
        let c = Vec3::new(0.0, 0.0, 2.0);
        let h = height_on_triangle(Vec3::new(1.0, 9.0, 0.5), a, b, c).unwrap();
        assert!((h - 1.0).abs() < 1e-5);
        assert!(height_on_triangle(Vec3::new(3.0, 0.0, 3.0), a, b, c).is_none());
    }

    #[test]
    fn slabs_overlap_within_climb() {
        let a = slab_end_points(Vec3::new(0.0, 1.0, 0.0), Vec3::new(0.0, 1.0, 4.0), 0);
        let b = slab_end_points(Vec3::new(0.0, 1.2, 2.0), Vec3::new(0.0, 1.2, 6.0), 0);
        assert!(overlap_slabs(a.0, a.1, b.0, b.1, 0.01, 0.3));
        let high = slab_end_points(Vec3::new(0.0, 4.0, 2.0), Vec3::new(0.0, 4.0, 6.0), 0);
        assert!(!overlap_slabs(a.0, a.1, high.0, high.1, 0.01, 0.3));
    }
}
