//! Navmesh generation: a Rust take on [Recast](https://github.com/recastnavigation)'s
//! voxel pipeline, one tile at a time.
//!
//! 1. Triangles are rasterized into a heightfield of solid spans per column
//!    ([`heightfield`]); floors above floors stay separate spans.
//! 2. Filters mark where an agent fits and can step ([`Heightfield`] filters).
//! 3. The open space above walkable spans becomes the compact heightfield
//!    ([`compact`]); obstacles carve it, then it is eroded by the agent's radius.
//! 4. Watershed regions ([`region`]) are traced and simplified to outlines
//!    ([`contour`]), triangulated and merged into convex polygons ([`polymesh`]), and
//!    given height detail ([`detail`]).

pub(crate) mod compact;
pub(crate) mod contour;
pub(crate) mod detail;
pub(crate) mod heightfield;
pub(crate) mod polymesh;
pub(crate) mod region;

use glam::{Vec2, Vec3};

pub(crate) use compact::CompactHeightfield;
pub(crate) use detail::PolyMeshDetail;
pub(crate) use heightfield::Heightfield;
pub(crate) use polymesh::PolyMesh;

/// Not walkable.
pub const NULL_AREA: u8 = 0;
/// The default walkable area id (custom areas are 1..62).
pub const WALKABLE_AREA: u8 = 63;
/// Geometry that is never walked on (a `NavModifier` with `walkable: false`): unlike
/// [`NULL_AREA`] tops, which the low-obstacle filter may still let agents step onto (a
/// curb), these stay unwalkable whatever is below them.
pub const FORBIDDEN_AREA: u8 = 0xff;
pub(crate) const SPAN_MAX_HEIGHT: i32 = u16::MAX as i32;
pub(crate) const NOT_CONNECTED: u8 = 0x3f;
pub(crate) const BORDER_REG: u16 = 0x8000;

/// Cell steps per direction: 0 = -x, 1 = +z, 2 = +x, 3 = -z.
pub(crate) fn dir_offset_x(dir: usize) -> i32 {
    [-1, 0, 1, 0][dir & 3]
}

pub(crate) fn dir_offset_z(dir: usize) -> i32 {
    [0, 1, 0, -1][dir & 3]
}

/// The generator's settings in cells (derived from [`super::NavMeshSettings`]).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RecastConfig {
    pub cs: f32,
    pub ch: f32,
    pub walkable_slope_angle: f32,
    pub walkable_height: i32,
    pub walkable_climb: i32,
    pub walkable_radius: i32,
    pub max_edge_len: i32,
    pub max_simplification_error: f32,
    pub min_region_area: i32,
    pub merge_region_area: i32,
    pub max_verts_per_poly: usize,
    /// World units.
    pub detail_sample_dist: f32,
    pub detail_sample_max_error: f32,
    /// Tile edge in cells, and the border built around it (cells).
    pub tile_size: i32,
    pub border_size: i32,
}

impl RecastConfig {
    pub fn tile_world_size(&self) -> f32 {
        self.tile_size as f32 * self.cs
    }
}

/// A shape that marks the compact heightfield: carving (area [`NULL_AREA`]) happens
/// before erosion, so holes grow by the agent's radius; other areas after it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum MarkShape {
    /// A convex prism over x/z `points`.
    Convex {
        points: Vec<Vec2>,
        min_y: f32,
        max_y: f32,
    },
    /// An upright cylinder standing on `base`.
    Cylinder {
        base: Vec3,
        radius: f32,
        height: f32,
    },
}

impl MarkShape {
    /// The shape's world x/z bounds.
    pub fn bounds_xz(&self) -> (Vec2, Vec2) {
        match self {
            Self::Convex { points, .. } => {
                let mut lo = Vec2::splat(f32::MAX);
                let mut hi = Vec2::splat(f32::MIN);
                for p in points {
                    lo = lo.min(*p);
                    hi = hi.max(*p);
                }
                (lo, hi)
            }
            Self::Cylinder { base, radius, .. } => (
                Vec2::new(base.x - radius, base.z - radius),
                Vec2::new(base.x + radius, base.z + radius),
            ),
        }
    }

    fn apply(&self, chf: &mut CompactHeightfield, area: u8) {
        match self {
            Self::Convex {
                points,
                min_y,
                max_y,
            } => chf.mark_convex_area(points, *min_y, *max_y, area),
            Self::Cylinder {
                base,
                radius,
                height,
            } => chf.mark_cylinder_area(*base, *radius, *height, area),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AreaMark {
    pub shape: MarkShape,
    pub area: u8,
}

/// Mark triangles too steep to walk on as [`NULL_AREA`] (either facing counts: imported
/// meshes are not always wound the same way).
pub(crate) fn mark_walkable_triangles(
    verts: &[Vec3],
    tris: &[[u32; 3]],
    slope_degrees: f32,
    areas: &mut [u8],
) {
    let threshold = slope_degrees.to_radians().cos();
    for (tri, area) in tris.iter().zip(areas.iter_mut()) {
        let v0 = verts[tri[0] as usize];
        let v1 = verts[tri[1] as usize];
        let v2 = verts[tri[2] as usize];
        let normal = (v1 - v0).cross(v2 - v0).normalize_or_zero();
        if normal.y.abs() <= threshold && *area != FORBIDDEN_AREA {
            *area = NULL_AREA;
        }
    }
}

/// Stage one of a tile: rasterize and filter its triangles (already marked by slope)
/// into the compact heightfield of the tile plus its border. This is what a tile cache
/// keeps to rebuild the tile quickly when obstacles change. `None` when nothing is
/// walkable.
pub(crate) fn rasterize_tile(
    cfg: &RecastConfig,
    bmin: Vec3,
    bmax: Vec3,
    verts: &[Vec3],
    tris: &[[u32; 3]],
    areas: &[u8],
) -> Option<CompactHeightfield> {
    let size = cfg.tile_size + cfg.border_size * 2;
    let mut hf = Heightfield::new(size, size, bmin, bmax, cfg.cs, cfg.ch);
    hf.rasterize_triangles(verts, tris, areas, cfg.walkable_climb);
    hf.filter_low_hanging_obstacles(cfg.walkable_climb);
    hf.filter_ledge_spans(cfg.walkable_height, cfg.walkable_climb);
    hf.filter_low_height_spans(cfg.walkable_height);
    let chf = CompactHeightfield::build(&hf, cfg.walkable_height, cfg.walkable_climb);
    (chf.span_count() > 0).then_some(chf)
}

/// Stage two: carve obstacles, erode, mark areas, partition and build the polygons and
/// their detail. `None` when no polygon is left.
pub(crate) fn build_tile_polys(
    cfg: &RecastConfig,
    mut chf: CompactHeightfield,
    carves: &[MarkShape],
    marks: &[AreaMark],
) -> Option<(PolyMesh, PolyMeshDetail)> {
    for carve in carves {
        carve.apply(&mut chf, NULL_AREA);
    }
    chf.erode_walkable_area(cfg.walkable_radius);
    for mark in marks {
        mark.shape.apply(&mut chf, mark.area);
    }
    chf.build_distance_field();
    chf.build_regions(cfg.border_size, cfg.min_region_area, cfg.merge_region_area);
    let cset = contour::ContourSet::build(&chf, cfg.max_simplification_error, cfg.max_edge_len);
    if cset.conts.is_empty() {
        return None;
    }
    let mesh = PolyMesh::build(&cset, cfg.max_verts_per_poly);
    if mesh.npolys == 0 {
        return None;
    }
    let detail = PolyMeshDetail::build(
        &mesh,
        &chf,
        cfg.detail_sample_dist,
        cfg.detail_sample_max_error,
    );
    Some((mesh, detail))
}
