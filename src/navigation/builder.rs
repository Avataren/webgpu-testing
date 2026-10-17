//! Building navmeshes tile by tile, and the tile cache that rebuilds the tiles
//! obstacles carve while the game runs.

use std::collections::HashMap;

use glam::{Vec2, Vec3};
use rayon::prelude::*;

use super::geometry::NavGeometry;
use super::navmesh::{NavMesh, NavTile};
use super::recast::{self, AreaMark, CompactHeightfield, MarkShape, RecastConfig};

/// The tile grid a navmesh is built on.
#[derive(Debug, Clone, PartialEq)]
pub struct TileGrid {
    /// The corner of tile (0, 0); `y` is the floor of the build's height range.
    pub origin: Vec3,
    /// The top of the height range.
    pub max_y: f32,
    pub tiles_x: i32,
    pub tiles_z: i32,
    pub tile_size: f32,
}

impl TileGrid {
    pub(crate) fn new(geometry: &NavGeometry, cfg: &RecastConfig) -> Self {
        let tile_size = cfg.tile_world_size();
        let width = (geometry.bmax.x - geometry.bmin.x).max(0.0);
        let depth = (geometry.bmax.z - geometry.bmin.z).max(0.0);
        Self {
            origin: geometry.bmin,
            max_y: geometry.bmax.y + cfg.ch,
            tiles_x: ((width / tile_size).ceil() as i32).max(1),
            tiles_z: ((depth / tile_size).ceil() as i32).max(1),
            tile_size,
        }
    }

    /// The tile's own corner and opposite corner (x/z), without the border.
    pub fn tile_rect(&self, x: i32, z: i32) -> (Vec2, Vec2) {
        let lo = Vec2::new(
            self.origin.x + x as f32 * self.tile_size,
            self.origin.z + z as f32 * self.tile_size,
        );
        (lo, lo + Vec2::splat(self.tile_size))
    }

    /// The tiles a world x/z rectangle touches (clamped to the grid).
    pub fn tiles_touching(&self, lo: Vec2, hi: Vec2) -> impl Iterator<Item = (i32, i32)> {
        let x0 = (((lo.x - self.origin.x) / self.tile_size).floor() as i32).max(0);
        let z0 = (((lo.y - self.origin.z) / self.tile_size).floor() as i32).max(0);
        let x1 = (((hi.x - self.origin.x) / self.tile_size).floor() as i32).min(self.tiles_x - 1);
        let z1 = (((hi.y - self.origin.z) / self.tile_size).floor() as i32).min(self.tiles_z - 1);
        (z0..=z1).flat_map(move |z| (x0..=x1).map(move |x| (x, z)))
    }
}

/// Rasterize tile (x, z): its compact heightfield before carving and erosion.
pub(crate) fn rasterize(
    geometry: &NavGeometry,
    cfg: &RecastConfig,
    grid: &TileGrid,
    x: i32,
    z: i32,
) -> Option<CompactHeightfield> {
    let (lo, hi) = grid.tile_rect(x, z);
    let pad = cfg.border_size as f32 * cfg.cs;
    let hf_lo = lo - Vec2::splat(pad);
    let hf_hi = hi + Vec2::splat(pad);
    let (verts, tris, areas) = geometry.triangles_in(hf_lo, hf_hi);
    if tris.is_empty() {
        return None;
    }
    recast::rasterize_tile(
        cfg,
        Vec3::new(hf_lo.x, grid.origin.y, hf_lo.y),
        Vec3::new(hf_hi.x, grid.max_y, hf_hi.y),
        &verts,
        &tris,
        &areas,
    )
}

/// Finish a rasterized tile into navmesh polygons.
pub(crate) fn finish(
    cfg: &RecastConfig,
    x: i32,
    z: i32,
    chf: CompactHeightfield,
    carves: &[MarkShape],
    marks: &[AreaMark],
) -> Option<NavTile> {
    let (mesh, detail) = recast::build_tile_polys(cfg, chf, carves, marks)?;
    Some(NavTile::from_poly_mesh(x, z, &mesh, &detail))
}

/// How long a build took and what it made.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BuildStats {
    pub tiles: usize,
    pub polys: usize,
    pub triangles: usize,
    pub millis: f64,
}

/// A finished build: the navmesh and, when asked for, the tile cache to rebuild tiles
/// with.
pub struct NavMeshBuild {
    pub mesh: NavMesh,
    pub grid: TileGrid,
    pub cache: TileCache,
    pub stats: BuildStats,
}

/// One tile's build: its grid coordinates, its cached voxels and its polygons.
type TileOutput = ((i32, i32), Option<CompactHeightfield>, Option<NavTile>);

/// Build every tile of the geometry (in parallel where threads exist).
/// `progress` (when given) counts the tiles finished.
pub(crate) fn build_navmesh(
    geometry: &NavGeometry,
    cfg: &RecastConfig,
    walkable_climb: f32,
    keep_cache: bool,
    progress: Option<&std::sync::atomic::AtomicUsize>,
) -> NavMeshBuild {
    let started = instant::Instant::now();
    let grid = TileGrid::new(geometry, cfg);
    let coords: Vec<(i32, i32)> = (0..grid.tiles_z)
        .flat_map(|z| (0..grid.tiles_x).map(move |x| (x, z)))
        .collect();
    let results: Vec<TileOutput> = if geometry.is_empty() {
        Vec::new()
    } else {
        coords
            .par_iter()
            .map(|&(x, z)| {
                let chf = rasterize(geometry, cfg, &grid, x, z);
                let tile = chf.clone().and_then(|chf| finish(cfg, x, z, chf, &[], &[]));
                if let Some(progress) = progress {
                    progress.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                ((x, z), if keep_cache { chf } else { None }, tile)
            })
            .collect()
    };
    let mut mesh = NavMesh::new(grid.origin, grid.tile_size, walkable_climb);
    let mut cache = TileCache::new(cfg.clone(), grid.clone());
    for (coord, chf, tile) in results {
        if keep_cache {
            cache.tiles.insert(coord, chf);
        }
        if let Some(tile) = tile {
            mesh.add_tile(tile);
        }
    }
    let stats = BuildStats {
        tiles: mesh.tile_count(),
        polys: mesh.poly_count(),
        triangles: geometry.triangle_count(),
        millis: started.elapsed().as_secs_f64() * 1000.0,
    };
    NavMeshBuild {
        mesh,
        grid,
        cache,
        stats,
    }
}

/// Rasterized tiles kept to rebuild them quickly when obstacles carve: the costly
/// part of a build (rasterizing and filtering) is done once per tile, lazily.
pub struct TileCache {
    pub(crate) cfg: RecastConfig,
    pub grid: TileGrid,
    /// Per tile: its compact heightfield, or `None` when nothing there is walkable.
    tiles: HashMap<(i32, i32), Option<CompactHeightfield>>,
}

impl TileCache {
    pub(crate) fn new(cfg: RecastConfig, grid: TileGrid) -> Self {
        Self {
            cfg,
            grid,
            tiles: HashMap::new(),
        }
    }

    pub fn cached_tiles(&self) -> usize {
        self.tiles.len()
    }

    /// Rebuild tile (x, z) with these carves (rasterizing it first if needed).
    pub(crate) fn rebuild(
        &mut self,
        geometry: &NavGeometry,
        x: i32,
        z: i32,
        carves: &[MarkShape],
    ) -> Option<NavTile> {
        if x < 0 || z < 0 || x >= self.grid.tiles_x || z >= self.grid.tiles_z {
            return None;
        }
        let cfg = &self.cfg;
        let grid = &self.grid;
        let chf = self
            .tiles
            .entry((x, z))
            .or_insert_with(|| rasterize(geometry, cfg, grid, x, z))
            .clone()?;
        // Only the carves that reach this tile (with its border).
        let (lo, hi) = self.grid.tile_rect(x, z);
        let pad = (self.cfg.border_size as f32 + 1.0) * self.cfg.cs;
        let relevant: Vec<MarkShape> = carves
            .iter()
            .filter(|shape| {
                let (slo, shi) = shape.bounds_xz();
                shi.x >= lo.x - pad
                    && slo.x <= hi.x + pad
                    && shi.y >= lo.y - pad
                    && slo.y <= hi.y + pad
            })
            .cloned()
            .collect();
        finish(&self.cfg, x, z, chf, &relevant, &[])
    }
}

#[cfg(test)]
mod tests {
    use super::super::components::NavMeshSurface;
    use super::super::query::{find_path_points, QueryFilter};
    use super::super::recast::WALKABLE_AREA;
    use super::*;

    /// A box's six faces as triangles (outward winding).
    fn add_box(g: &mut NavGeometry, lo: Vec3, hi: Vec3) {
        let p = [
            Vec3::new(lo.x, lo.y, lo.z),
            Vec3::new(hi.x, lo.y, lo.z),
            Vec3::new(hi.x, lo.y, hi.z),
            Vec3::new(lo.x, lo.y, hi.z),
            Vec3::new(lo.x, hi.y, lo.z),
            Vec3::new(hi.x, hi.y, lo.z),
            Vec3::new(hi.x, hi.y, hi.z),
            Vec3::new(lo.x, hi.y, hi.z),
        ];
        let faces = [
            [4, 6, 5],
            [4, 7, 6],
            [0, 1, 2],
            [0, 2, 3],
            [0, 4, 5],
            [0, 5, 1],
            [2, 6, 7],
            [2, 7, 3],
            [1, 5, 6],
            [1, 6, 2],
            [0, 3, 7],
            [0, 7, 4],
        ];
        g.add_triangles(&p, &faces, WALKABLE_AREA);
    }

    fn surface() -> NavMeshSurface {
        NavMeshSurface {
            tile_size: 32,
            ..Default::default()
        }
    }

    fn build(g: &mut NavGeometry, surface: &NavMeshSurface) -> NavMeshBuild {
        let cfg = surface.recast_config();
        g.finish(cfg.walkable_slope_angle, None, cfg.tile_world_size());
        build_navmesh(g, &cfg, surface.agent_max_climb, true, None)
    }

    #[test]
    fn a_floor_builds_linked_tiles_and_a_straight_path() {
        let mut g = NavGeometry::default();
        add_box(
            &mut g,
            Vec3::new(0.0, -0.5, 0.0),
            Vec3::new(20.0, 0.0, 20.0),
        );
        let s = surface();
        let built = build(&mut g, &s);
        assert!(
            built.mesh.tile_count() >= 4,
            "tiles {}",
            built.mesh.tile_count()
        );
        assert!(built.mesh.poly_count() > 0);
        let filter = QueryFilter::default();
        let (corners, path) = find_path_points(
            &built.mesh,
            Vec3::new(2.0, 0.0, 2.0),
            Vec3::new(18.0, 0.0, 17.0),
            Vec3::new(1.0, 2.0, 1.0),
            &filter,
        )
        .expect("a path");
        assert!(path.complete, "path crosses tile borders");
        // An open floor needs no corners between start and end.
        assert_eq!(corners.len(), 2, "{corners:?}");
        assert!((corners[1].pos - Vec3::new(18.0, 0.0, 17.0)).length() < 0.3);
        assert!(corners.iter().all(|c| c.pos.y.abs() < 0.2));
    }

    #[test]
    fn walls_bend_the_path_and_agents_keep_their_radius() {
        let mut g = NavGeometry::default();
        add_box(
            &mut g,
            Vec3::new(0.0, -0.5, 0.0),
            Vec3::new(20.0, 0.0, 20.0),
        );
        // A wall across the middle with a gap at the +x end.
        add_box(&mut g, Vec3::new(0.0, 0.0, 9.5), Vec3::new(16.0, 3.0, 10.5));
        let s = surface();
        let built = build(&mut g, &s);
        let filter = QueryFilter::default();
        let (corners, path) = find_path_points(
            &built.mesh,
            Vec3::new(4.0, 0.0, 4.0),
            Vec3::new(4.0, 0.0, 16.0),
            Vec3::new(1.0, 2.0, 1.0),
            &filter,
        )
        .expect("a path");
        assert!(path.complete);
        assert!(corners.len() >= 3, "goes round the wall: {corners:?}");
        // The turn is near the wall's end, a radius away from it.
        let turn = corners[1].pos;
        assert!(turn.x > 16.0 && turn.x < 17.5, "{turn:?}");
        let length: f32 = corners
            .windows(2)
            .map(|w| w[0].pos.distance(w[1].pos))
            .sum();
        assert!(length > 25.0 && length < 30.0, "length {length}");
    }

    #[test]
    fn a_bridge_and_the_path_under_it_are_separate_floors() {
        let mut g = NavGeometry::default();
        // Ground, and a deck 3 m up across it on two piers.
        add_box(
            &mut g,
            Vec3::new(0.0, -0.5, 0.0),
            Vec3::new(20.0, 0.0, 20.0),
        );
        add_box(&mut g, Vec3::new(8.0, 3.0, 0.0), Vec3::new(12.0, 3.4, 20.0));
        let s = surface();
        let built = build(&mut g, &s);
        let mesh = &built.mesh;
        let extents = Vec3::new(0.5, 0.6, 0.5);
        let (under, under_pos) = mesh
            .find_nearest_poly(Vec3::new(10.0, 0.0, 10.0), extents)
            .expect("ground under the bridge");
        let (over, over_pos) = mesh
            .find_nearest_poly(Vec3::new(10.0, 3.4, 10.0), extents)
            .expect("the bridge deck");
        assert_ne!(under, over);
        assert!(under_pos.y.abs() < 0.2, "{under_pos:?}");
        assert!((over_pos.y - 3.4).abs() < 0.2, "{over_pos:?}");
        // Walking under the bridge goes straight; the deck is its own island.
        let filter = QueryFilter::default();
        let (corners, path) = find_path_points(
            mesh,
            Vec3::new(2.0, 0.0, 10.0),
            Vec3::new(18.0, 0.0, 10.0),
            extents,
            &filter,
        )
        .unwrap();
        assert!(path.complete);
        assert_eq!(corners.len(), 2);
        let (_, path) = find_path_points(
            mesh,
            Vec3::new(10.0, 3.4, 2.0),
            Vec3::new(2.0, 0.0, 10.0),
            extents,
            &filter,
        )
        .unwrap();
        assert!(!path.complete, "no way down from the deck");
    }

    #[test]
    fn stairs_connect_levels() {
        let mut g = NavGeometry::default();
        add_box(
            &mut g,
            Vec3::new(0.0, -0.5, 0.0),
            Vec3::new(20.0, 0.0, 10.0),
        );
        // An upper floor at 2 m, and a flight of 0.25 m steps up to it.
        add_box(
            &mut g,
            Vec3::new(12.0, 0.0, 0.0),
            Vec3::new(20.0, 2.0, 10.0),
        );
        for step in 0..8 {
            let x0 = 4.0 + step as f32 * 1.0;
            add_box(
                &mut g,
                Vec3::new(x0, 0.0, 3.0),
                Vec3::new(12.0, (step + 1) as f32 * 0.25, 6.0),
            );
        }
        let s = surface();
        let built = build(&mut g, &s);
        let filter = QueryFilter::default();
        let (corners, path) = find_path_points(
            &built.mesh,
            Vec3::new(1.0, 0.0, 5.0),
            Vec3::new(18.0, 2.0, 5.0),
            Vec3::new(0.5, 1.0, 0.5),
            &filter,
        )
        .expect("a path");
        assert!(path.complete, "up the stairs");
        let end = corners.last().unwrap().pos;
        assert!((end.y - 2.0).abs() < 0.25, "{end:?}");
    }

    #[test]
    fn carving_blocks_a_corridor_and_paths_reroute() {
        let mut g = NavGeometry::default();
        add_box(
            &mut g,
            Vec3::new(0.0, -0.5, 0.0),
            Vec3::new(20.0, 0.0, 20.0),
        );
        // A wall with two gaps: x 2..4 and x 16..18.
        add_box(&mut g, Vec3::new(0.0, 0.0, 9.5), Vec3::new(2.0, 3.0, 10.5));
        add_box(&mut g, Vec3::new(4.0, 0.0, 9.5), Vec3::new(16.0, 3.0, 10.5));
        add_box(
            &mut g,
            Vec3::new(18.0, 0.0, 9.5),
            Vec3::new(20.0, 3.0, 10.5),
        );
        let s = surface();
        let mut built = build(&mut g, &s);
        let filter = QueryFilter::default();
        let extents = Vec3::new(1.0, 2.0, 1.0);
        let (from, to) = (Vec3::new(3.0, 0.0, 4.0), Vec3::new(3.0, 0.0, 16.0));
        let (before, _) = find_path_points(&built.mesh, from, to, extents, &filter).unwrap();
        let near_gap = before.iter().all(|c| c.pos.x < 6.0);
        assert!(near_gap, "uses the near gap: {before:?}");

        // A crate in the near gap.
        let carve = MarkShape::Convex {
            points: vec![
                Vec2::new(1.8, 9.0),
                Vec2::new(4.2, 9.0),
                Vec2::new(4.2, 11.0),
                Vec2::new(1.8, 11.0),
            ],
            min_y: -0.5,
            max_y: 1.5,
        };
        let touched: Vec<(i32, i32)> = built
            .grid
            .tiles_touching(Vec2::new(1.8, 9.0), Vec2::new(4.2, 11.0))
            .collect();
        assert!(!touched.is_empty());
        for (x, z) in touched {
            match built.cache.rebuild(&g, x, z, std::slice::from_ref(&carve)) {
                Some(tile) => {
                    built.mesh.add_tile(tile);
                }
                None => {
                    built.mesh.remove_tile(x, z);
                }
            }
        }
        let (after, path) = find_path_points(&built.mesh, from, to, extents, &filter).unwrap();
        assert!(path.complete);
        assert!(
            after.iter().any(|c| c.pos.x > 15.0),
            "goes round by the far gap: {after:?}"
        );
    }
}

#[cfg(test)]
mod forbidden_ground {
    use super::super::components::NavMeshSurface;
    use super::super::recast::{FORBIDDEN_AREA, WALKABLE_AREA};
    use super::*;

    /// Water 0.48 m above the floor is within one agent climb once the floor rounds up a
    /// cell; marked forbidden, the low-obstacle filter must still leave it alone.
    #[test]
    fn unwalkable_water_in_a_basin_stays_unwalkable() {
        let mut g = NavGeometry::default();
        // Canal bed far below sets the field's floor, as in the market.
        let bed = [
            Vec3::new(-20.0, -3.6, 15.0),
            Vec3::new(20.0, -3.6, 15.0),
            Vec3::new(20.0, -3.6, 17.0),
            Vec3::new(-20.0, -3.6, 17.0),
        ];
        g.add_triangles(&bed, &[[0, 2, 1], [0, 3, 2]], WALKABLE_AREA);
        let slab = [
            Vec3::new(-10.0, 0.0, -10.0),
            Vec3::new(10.0, 0.0, -10.0),
            Vec3::new(10.0, 0.0, 10.0),
            Vec3::new(-10.0, 0.0, 10.0),
        ];
        g.add_triangles(&slab, &[[0, 2, 1], [0, 3, 2]], WALKABLE_AREA);
        // A ring of water (NULL) at 0.48 inside a rim wall up to 0.7.
        let seg = 32;
        let mut water = Vec::new();
        let mut water_tris = Vec::new();
        let mut wall = Vec::new();
        let mut wall_tris = Vec::new();
        for i in 0..seg {
            let a = i as f32 / seg as f32 * std::f32::consts::TAU;
            let (c, s) = (a.cos(), a.sin());
            water.push(Vec3::new(c * 2.95, 0.48, s * 2.95));
            water.push(Vec3::new(c * 0.55, 0.48, s * 0.55));
            wall.push(Vec3::new(c * 2.95, 0.3, s * 2.95));
            wall.push(Vec3::new(c * 2.95, 0.7, s * 2.95));
            wall.push(Vec3::new(c * 3.35, 0.7, s * 3.35));
            wall.push(Vec3::new(c * 3.35, 0.0, s * 3.35));
        }
        for i in 0..seg as u32 {
            let j = (i + 1) % seg as u32;
            water_tris.push([2 * i, 2 * j, 2 * j + 1]);
            water_tris.push([2 * i, 2 * j + 1, 2 * i + 1]);
            for k in 0..3u32 {
                let (a, b) = (4 * i + k, 4 * i + k + 1);
                let (c, d) = (4 * j + k, 4 * j + k + 1);
                wall_tris.push([a, c, d]);
                wall_tris.push([a, d, b]);
            }
        }
        g.add_triangles(&water, &water_tris, FORBIDDEN_AREA);
        g.add_triangles(&wall, &wall_tris, WALKABLE_AREA);
        let surface = NavMeshSurface {
            agent_radius: 0.35,
            agent_max_climb: 0.4,
            cell_size: 0.15,
            cell_height: 0.1,
            tile_size: 48,
            ..Default::default()
        };
        let cfg = surface.recast_config();
        g.finish(cfg.walkable_slope_angle, None, cfg.tile_world_size());
        let built = build_navmesh(&g, &cfg, surface.agent_max_climb, false, None);
        let hit = built
            .mesh
            .find_nearest_poly(Vec3::new(2.0, 0.48, 0.0), Vec3::new(0.05, 0.05, 0.05));
        assert!(hit.is_none(), "walkable water at {hit:?}");
    }
}
