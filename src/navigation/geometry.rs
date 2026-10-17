//! The triangles a navmesh is built from, gathered from the scene's static meshes
//! (and optionally fixed colliders) in world space, with an index by tile so each
//! tile only rasterizes what overlaps it.

use std::collections::HashMap;

use glam::{Mat4, Vec2, Vec3};
use hecs::{Entity, World};

use super::components::{
    NavAgent, NavCollect, NavGeometrySource, NavMeshSurface, NavModifier, NavObstacle,
};
use super::recast::{mark_walkable_triangles, FORBIDDEN_AREA, WALKABLE_AREA};
use crate::asset::Assets;
use crate::physics::{BodyKind, RigidBody};
use crate::scene::components::{Children, MeshComponent, Parent, Visible};
use crate::scene::skeletal::SkinnedMesh;

/// A triangle soup in world space, each triangle with its area.
#[derive(Debug, Clone, Default)]
pub struct NavGeometry {
    pub verts: Vec<Vec3>,
    pub tris: Vec<[u32; 3]>,
    /// Per triangle: [`WALKABLE_AREA`], a custom area, [`NULL_AREA`](super::recast::NULL_AREA)
    /// (too steep) or [`FORBIDDEN_AREA`] (solid, never walked on).
    pub areas: Vec<u8>,
    pub bmin: Vec3,
    pub bmax: Vec3,
    index_origin: Vec2,
    index_cell: f32,
    index: HashMap<(i32, i32), Vec<u32>>,
}

/// How one entity's geometry goes into the navmesh.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Contribution {
    Skip,
    Area(u8),
}

impl NavGeometry {
    pub fn triangle_count(&self) -> usize {
        self.tris.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tris.is_empty()
    }

    /// Add triangles (`points` already in world space).
    pub fn add_triangles(&mut self, points: &[Vec3], tris: &[[u32; 3]], area: u8) {
        let base = self.verts.len() as u32;
        self.verts.extend_from_slice(points);
        for tri in tris {
            if tri.iter().any(|&i| i as usize >= points.len()) {
                continue;
            }
            self.tris
                .push([base + tri[0], base + tri[1], base + tri[2]]);
            self.areas.push(area);
        }
    }

    /// Mark triangles too steep for `slope_degrees` unwalkable, clip to `bounds`
    /// (a world box, if given), compute the bounds and index the triangles in squares
    /// of `cell` metres.
    pub(crate) fn finish(&mut self, slope_degrees: f32, bounds: Option<(Vec3, Vec3)>, cell: f32) {
        mark_walkable_triangles(&self.verts, &self.tris, slope_degrees, &mut self.areas);
        if let Some((lo, hi)) = bounds {
            let mut kept_tris = Vec::with_capacity(self.tris.len());
            let mut kept_areas = Vec::with_capacity(self.areas.len());
            for (tri, area) in self.tris.iter().zip(&self.areas) {
                let (a, b, c) = (
                    self.verts[tri[0] as usize],
                    self.verts[tri[1] as usize],
                    self.verts[tri[2] as usize],
                );
                let tmin = a.min(b).min(c);
                let tmax = a.max(b).max(c);
                if tmax.x < lo.x
                    || tmin.x > hi.x
                    || tmax.y < lo.y
                    || tmin.y > hi.y
                    || tmax.z < lo.z
                    || tmin.z > hi.z
                {
                    continue;
                }
                kept_tris.push(*tri);
                kept_areas.push(*area);
            }
            self.tris = kept_tris;
            self.areas = kept_areas;
        }
        let mut lo = Vec3::splat(f32::MAX);
        let mut hi = Vec3::splat(f32::MIN);
        for tri in &self.tris {
            for &i in tri {
                lo = lo.min(self.verts[i as usize]);
                hi = hi.max(self.verts[i as usize]);
            }
        }
        if let Some((blo, bhi)) = bounds {
            lo = lo.max(blo);
            hi = hi.min(bhi);
        }
        if self.tris.is_empty() {
            lo = Vec3::ZERO;
            hi = Vec3::ZERO;
        }
        self.bmin = lo;
        self.bmax = hi;
        self.build_index(cell);
    }

    fn build_index(&mut self, cell: f32) {
        self.index.clear();
        self.index_cell = cell.max(0.5);
        self.index_origin = Vec2::new(self.bmin.x, self.bmin.z);
        for (t, tri) in self.tris.iter().enumerate() {
            let (a, b, c) = (
                self.verts[tri[0] as usize],
                self.verts[tri[1] as usize],
                self.verts[tri[2] as usize],
            );
            let lo = a.min(b).min(c);
            let hi = a.max(b).max(c);
            let (x0, z0) = self.cell_of(lo.x, lo.z);
            let (x1, z1) = self.cell_of(hi.x, hi.z);
            for z in z0..=z1 {
                for x in x0..=x1 {
                    self.index.entry((x, z)).or_default().push(t as u32);
                }
            }
        }
    }

    fn cell_of(&self, x: f32, z: f32) -> (i32, i32) {
        (
            ((x - self.index_origin.x) / self.index_cell).floor() as i32,
            ((z - self.index_origin.y) / self.index_cell).floor() as i32,
        )
    }

    /// The triangles overlapping a world x/z rectangle, as their own vertex list.
    pub(crate) fn triangles_in(&self, lo: Vec2, hi: Vec2) -> (Vec<Vec3>, Vec<[u32; 3]>, Vec<u8>) {
        let (x0, z0) = self.cell_of(lo.x, lo.y);
        let (x1, z1) = self.cell_of(hi.x, hi.y);
        let mut picked: Vec<u32> = Vec::new();
        for z in z0..=z1 {
            for x in x0..=x1 {
                if let Some(list) = self.index.get(&(x, z)) {
                    picked.extend_from_slice(list);
                }
            }
        }
        picked.sort_unstable();
        picked.dedup();
        let mut remap: HashMap<u32, u32> = HashMap::new();
        let mut verts = Vec::new();
        let mut tris = Vec::with_capacity(picked.len());
        let mut areas = Vec::with_capacity(picked.len());
        for t in picked {
            let tri = self.tris[t as usize];
            let (a, b, c) = (
                self.verts[tri[0] as usize],
                self.verts[tri[1] as usize],
                self.verts[tri[2] as usize],
            );
            let tmin = a.min(b).min(c);
            let tmax = a.max(b).max(c);
            if tmax.x < lo.x || tmin.x > hi.x || tmax.z < lo.y || tmin.z > hi.y {
                continue;
            }
            let mut local = [0u32; 3];
            for (k, &v) in tri.iter().enumerate() {
                local[k] = *remap.entry(v).or_insert_with(|| {
                    verts.push(self.verts[v as usize]);
                    (verts.len() - 1) as u32
                });
            }
            tris.push(local);
            areas.push(self.areas[t as usize]);
        }
        (verts, tris, areas)
    }
}

fn ancestors(world: &World, entity: Entity) -> impl Iterator<Item = Entity> + '_ {
    let mut current = Some(entity);
    let mut guard = 0;
    std::iter::from_fn(move || {
        let e = current?;
        guard += 1;
        if guard > 256 {
            return None;
        }
        current = world.get::<&Parent>(e).ok().map(|p| p.0);
        Some(e)
    })
}

/// How `entity`'s own geometry contributes: skipped (hidden, moving, an agent or an
/// obstacle, or ignored by a modifier) or with an area.
fn contribution(world: &World, entity: Entity) -> Contribution {
    let mut area = WALKABLE_AREA;
    let mut modifier_found = false;
    for (depth, e) in ancestors(world, entity).enumerate() {
        if world.get::<&Visible>(e).is_ok_and(|v| !v.0) {
            return Contribution::Skip;
        }
        if world
            .satisfies::<hecs::Or<&NavAgent, &NavObstacle>>(e)
            .unwrap_or(false)
        {
            return Contribution::Skip;
        }
        if world
            .get::<&RigidBody>(e)
            .is_ok_and(|body| body.kind != BodyKind::Fixed)
        {
            return Contribution::Skip;
        }
        if !modifier_found {
            if let Ok(modifier) = world.get::<&NavModifier>(e) {
                if depth == 0 || modifier.include_children {
                    modifier_found = true;
                    if modifier.ignore {
                        return Contribution::Skip;
                    }
                    area = if modifier.walkable {
                        modifier.area.clamp(1, WALKABLE_AREA)
                    } else {
                        FORBIDDEN_AREA
                    };
                }
            }
        }
    }
    Contribution::Area(area)
}

fn descendants(world: &World, root: Entity) -> Vec<Entity> {
    let mut out = vec![root];
    let mut stack = vec![root];
    while let Some(e) = stack.pop() {
        if let Ok(children) = world.get::<&Children>(e) {
            for &child in &children.0 {
                out.push(child);
                stack.push(child);
            }
        }
    }
    out
}

/// Gather the geometry `surface` (on entity `owner`) is built from.
pub fn collect_geometry(
    world: &World,
    assets: &Assets,
    owner: Entity,
    surface: &NavMeshSurface,
) -> NavGeometry {
    let mut geometry = NavGeometry::default();
    let allowed: Option<std::collections::HashSet<Entity>> = match surface.collect {
        NavCollect::All => None,
        NavCollect::Children => Some(descendants(world, owner).into_iter().collect()),
    };
    let in_scope = |e: Entity| allowed.as_ref().is_none_or(|set| set.contains(&e));

    if matches!(
        surface.geometry,
        NavGeometrySource::RenderMeshes | NavGeometrySource::Both
    ) {
        let mut query = world.query::<&MeshComponent>();
        for (entity, mesh) in query.iter() {
            if !in_scope(entity) || world.satisfies::<&SkinnedMesh>(entity).unwrap_or(false) {
                continue;
            }
            let Contribution::Area(area) = contribution(world, entity) else {
                continue;
            };
            let Some(mesh) = assets.meshes.get(mesh.0) else {
                continue;
            };
            let data = mesh.data();
            let matrix: Mat4 = crate::physics::world_transform(world, entity).matrix();
            let points: Vec<Vec3> = data
                .vertices
                .iter()
                .map(|v| matrix.transform_point3(Vec3::from(v.pos)))
                .collect();
            let tris: Vec<[u32; 3]> = data.indices.as_chunks::<3>().0.to_vec();
            geometry.add_triangles(&points, &tris, area);
        }
    }
    if matches!(
        surface.geometry,
        NavGeometrySource::Colliders | NavGeometrySource::Both
    ) {
        let parts = crate::physics::fixed_collider_triangles(world, assets, |e| {
            in_scope(e) && contribution(world, e) != Contribution::Skip
        });
        for (entity, points, tris) in parts {
            if let Contribution::Area(area) = contribution(world, entity) {
                geometry.add_triangles(&points, &tris, area);
            }
        }
    }

    let bounds = surface_bounds(world, owner, surface);
    let cfg = surface.recast_config();
    geometry.finish(cfg.walkable_slope_angle, bounds, cfg.tile_world_size());
    geometry
}

/// The world box `surface.bounds` asks for, if any.
pub fn surface_bounds(
    world: &World,
    owner: Entity,
    surface: &NavMeshSurface,
) -> Option<(Vec3, Vec3)> {
    let size = Vec3::from(surface.bounds);
    if size.x <= 0.0 || size.y <= 0.0 || size.z <= 0.0 {
        return None;
    }
    let center = crate::physics::world_transform(world, owner).translation;
    Some((center - size * 0.5, center + size * 0.5))
}

#[cfg(test)]
mod tests {
    use super::super::recast::NULL_AREA;
    use super::*;

    #[test]
    fn index_finds_overlapping_triangles() {
        let mut g = NavGeometry::default();
        let quad = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(10.0, 0.0, 0.0),
            Vec3::new(10.0, 0.0, 10.0),
            Vec3::new(0.0, 0.0, 10.0),
        ];
        g.add_triangles(&quad, &[[0, 2, 1], [0, 3, 2]], WALKABLE_AREA);
        let far = [
            Vec3::new(50.0, 0.0, 50.0),
            Vec3::new(51.0, 0.0, 50.0),
            Vec3::new(50.0, 0.0, 51.0),
        ];
        g.add_triangles(&far, &[[0, 2, 1]], WALKABLE_AREA);
        g.finish(45.0, None, 4.0);
        let (verts, tris, areas) = g.triangles_in(Vec2::new(1.0, 1.0), Vec2::new(3.0, 3.0));
        assert_eq!(tris.len(), 2);
        assert_eq!(verts.len(), 4);
        assert!(areas.iter().all(|&a| a == WALKABLE_AREA));
        let (_, tris, _) = g.triangles_in(Vec2::new(49.0, 49.0), Vec2::new(52.0, 52.0));
        assert_eq!(tris.len(), 1);
    }

    #[test]
    fn steep_triangles_become_unwalkable() {
        let mut g = NavGeometry::default();
        let wall = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        ];
        g.add_triangles(&wall, &[[0, 1, 2]], WALKABLE_AREA);
        g.finish(45.0, None, 4.0);
        assert_eq!(g.areas, vec![NULL_AREA]);
    }
}
