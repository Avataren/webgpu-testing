//! Queries on a [`NavMesh`]: shortest paths (A* over polygons, then the funnel
//! algorithm for the corners), raycasts along the floor, constrained movement and
//! random points. Modelled on Detour's `dtNavMeshQuery`.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet, VecDeque};

use glam::Vec3;
use rand::Rng;

use super::navmesh::{
    distance_pt_seg_sqr_xz, point_in_polygon_xz, NavMesh, PolyKind, PolyRef, EXT_LINK,
};
use super::recast::NULL_AREA;

/// Slightly under 1, so A* prefers nodes nearer the goal among equal costs.
const H_SCALE: f32 = 0.999;

/// Which areas a query may cross and what each costs.
#[derive(Debug, Clone, PartialEq)]
pub struct QueryFilter {
    /// Cost multiplier per area id; 0 or less excludes the area.
    pub area_cost: [f32; 64],
}

impl Default for QueryFilter {
    fn default() -> Self {
        let mut area_cost = [1.0; 64];
        area_cost[usize::from(NULL_AREA)] = 0.0;
        Self { area_cost }
    }
}

impl QueryFilter {
    /// A filter with the agent's area costs.
    pub fn with_costs(costs: &[super::NavAreaCost]) -> Self {
        let mut filter = Self::default();
        for cost in costs {
            if let Some(slot) = filter.area_cost.get_mut(usize::from(cost.area)) {
                *slot = cost.cost;
            }
        }
        filter
    }

    pub fn passes(&self, area: u8) -> bool {
        self.area_cost
            .get(usize::from(area))
            .is_some_and(|&c| c > 0.0)
    }

    fn cost(&self, a: Vec3, b: Vec3, area: u8) -> f32 {
        a.distance(b)
            * self
                .area_cost
                .get(usize::from(area))
                .copied()
                .unwrap_or(1.0)
    }
}

/// A corner of a straight path.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PathCorner {
    pub pos: Vec3,
    /// The polygon entered at this corner (none at the end).
    pub poly: PolyRef,
    pub flags: u8,
}

impl PathCorner {
    pub const START: u8 = 1;
    pub const END: u8 = 2;
    /// The corner is the start of an off-mesh link.
    pub const OFF_MESH: u8 = 4;
}

/// What [`find_path`] found.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PolyPath {
    pub polys: Vec<PolyRef>,
    /// Whether it reaches the goal's polygon (else it ends as near as it could get).
    pub complete: bool,
}

#[derive(Clone, Copy)]
struct Node {
    r: PolyRef,
    pos: Vec3,
    cost: f32,
    total: f32,
    parent: Option<usize>,
    closed: bool,
}

#[derive(Clone, Copy, PartialEq)]
struct Open {
    total: f32,
    node: usize,
}

impl Eq for Open {}

impl Ord for Open {
    fn cmp(&self, other: &Self) -> Ordering {
        // Smallest total first.
        other
            .total
            .total_cmp(&self.total)
            .then_with(|| other.node.cmp(&self.node))
    }
}

impl PartialOrd for Open {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// The cheapest corridor of polygons from `start` to `end` (A*, at most `max_nodes`
/// polygons explored). When the goal cannot be reached the corridor ends at the
/// polygon nearest it.
pub fn find_path(
    mesh: &NavMesh,
    start: PolyRef,
    end: PolyRef,
    start_pos: Vec3,
    end_pos: Vec3,
    filter: &QueryFilter,
    max_nodes: usize,
) -> PolyPath {
    if !mesh.is_valid(start) || !mesh.is_valid(end) {
        return PolyPath::default();
    }
    if start == end {
        return PolyPath {
            polys: vec![start],
            complete: true,
        };
    }
    let mut nodes: Vec<Node> = Vec::with_capacity(256);
    let mut index: HashMap<PolyRef, usize> = HashMap::with_capacity(256);
    let mut open = BinaryHeap::new();
    let heuristic = start_pos.distance(end_pos) * H_SCALE;
    nodes.push(Node {
        r: start,
        pos: start_pos,
        cost: 0.0,
        total: heuristic,
        parent: None,
        closed: false,
    });
    index.insert(start, 0);
    open.push(Open {
        total: heuristic,
        node: 0,
    });
    let mut best_node = 0;
    let mut best_cost = heuristic;

    while let Some(Open {
        total,
        node: current,
    }) = open.pop()
    {
        if nodes[current].closed || total > nodes[current].total {
            continue;
        }
        nodes[current].closed = true;
        let current_ref = nodes[current].r;
        if current_ref == end {
            best_node = current;
            break;
        }
        let parent_ref = nodes[current].parent.map(|p| nodes[p].r);
        let Some(current_area) = mesh.poly_area(current_ref) else {
            continue;
        };
        for link in mesh.links(current_ref) {
            let neighbour = link.target;
            if Some(neighbour) == parent_ref {
                continue;
            }
            let Some(area) = mesh.poly_area(neighbour) else {
                continue;
            };
            if !filter.passes(area) {
                continue;
            }
            let existing = index.get(&neighbour).copied();
            if existing.is_none() && nodes.len() >= max_nodes {
                continue;
            }
            let neighbour_pos = match existing {
                Some(n) => nodes[n].pos,
                None => match mesh.edge_mid_point(current_ref, neighbour) {
                    Some(p) => p,
                    None => continue,
                },
            };
            let (cost, heuristic) = if neighbour == end {
                let through = filter.cost(nodes[current].pos, neighbour_pos, current_area);
                let to_goal = filter.cost(neighbour_pos, end_pos, area);
                (nodes[current].cost + through + to_goal, 0.0)
            } else {
                let through = filter.cost(nodes[current].pos, neighbour_pos, current_area);
                (
                    nodes[current].cost + through,
                    neighbour_pos.distance(end_pos) * H_SCALE,
                )
            };
            let total = cost + heuristic;
            let n = match existing {
                Some(n) => {
                    if total >= nodes[n].total {
                        continue;
                    }
                    n
                }
                None => {
                    nodes.push(Node {
                        r: neighbour,
                        pos: neighbour_pos,
                        cost,
                        total,
                        parent: None,
                        closed: false,
                    });
                    index.insert(neighbour, nodes.len() - 1);
                    nodes.len() - 1
                }
            };
            let node = &mut nodes[n];
            node.parent = Some(current);
            node.cost = cost;
            node.total = total;
            node.closed = false;
            open.push(Open { total, node: n });
            if heuristic < best_cost {
                best_cost = heuristic;
                best_node = n;
            }
        }
    }

    let mut polys = Vec::new();
    let mut at = Some(best_node);
    while let Some(n) = at {
        polys.push(nodes[n].r);
        at = nodes[n].parent;
    }
    polys.reverse();
    PolyPath {
        complete: polys.last() == Some(&end),
        polys,
    }
}

/// Twice the signed area of a, b, c on the x/z plane, as Detour measures it.
fn tri_area_2d(a: Vec3, b: Vec3, c: Vec3) -> f32 {
    let abx = b.x - a.x;
    let abz = b.z - a.z;
    let acx = c.x - a.x;
    let acz = c.z - a.z;
    acx * abz - abx * acz
}

fn vequal(a: Vec3, b: Vec3) -> bool {
    a.distance_squared(b) < (1.0 / 16384.0) * (1.0 / 16384.0)
}

/// The nearest point to `pos` on polygon `r`'s outline, or `pos` when it is inside.
pub fn closest_point_on_poly_boundary(mesh: &NavMesh, r: PolyRef, pos: Vec3) -> Option<Vec3> {
    let points = mesh.poly_vertices(r);
    if points.is_empty() {
        return None;
    }
    if points.len() >= 3 && point_in_polygon_xz(pos, &points) {
        return Some(pos);
    }
    Some(super::navmesh::closest_point_on_boundary_xz(pos, &points))
}

/// The corners of the straightest walk through a corridor (the funnel algorithm),
/// from `start` to `end`, at most `max_corners`.
pub fn find_straight_path(
    mesh: &NavMesh,
    start: Vec3,
    end: Vec3,
    path: &[PolyRef],
    max_corners: usize,
) -> Vec<PathCorner> {
    let mut out: Vec<PathCorner> = Vec::new();
    if path.is_empty() || max_corners == 0 {
        return out;
    }
    let append = |out: &mut Vec<PathCorner>, pos: Vec3, flags: u8, poly: PolyRef| -> bool {
        if let Some(last) = out.last_mut() {
            if vequal(last.pos, pos) {
                last.flags = flags;
                last.poly = poly;
                return out.len() < max_corners;
            }
        }
        out.push(PathCorner { pos, poly, flags });
        if flags == PathCorner::END {
            return false;
        }
        out.len() < max_corners
    };
    let Some(closest_start) = closest_point_on_poly_boundary(mesh, path[0], start) else {
        return out;
    };
    let Some(mut closest_end) = closest_point_on_poly_boundary(mesh, path[path.len() - 1], end)
    else {
        return out;
    };
    if !append(&mut out, closest_start, PathCorner::START, path[0]) {
        return out;
    }
    if path.len() > 1 {
        let mut apex = closest_start;
        let mut portal_left = apex;
        let mut portal_right = apex;
        let (mut left_index, mut right_index) = (0usize, 0usize);
        let mut left_kind = PolyKind::Ground;
        let mut right_kind = PolyKind::Ground;
        let mut left_ref = path[0];
        let mut right_ref = path[0];
        let mut i = 0;
        while i < path.len() {
            let (left, right, to_kind);
            if i + 1 < path.len() {
                match mesh.portal_points(path[i], path[i + 1]) {
                    Some((l, r, _, kind)) => {
                        left = l;
                        right = r;
                        to_kind = kind;
                    }
                    None => {
                        // A broken corridor: end at the polygon reached.
                        if let Some(p) = closest_point_on_poly_boundary(mesh, path[i], end) {
                            closest_end = p;
                        }
                        append(&mut out, closest_end, PathCorner::END, PolyRef::NONE);
                        return out;
                    }
                }
                // Starting right on the first portal: skip it.
                if i == 0 && distance_pt_seg_sqr_xz(apex, left, right).0 < 0.001 * 0.001 {
                    i += 1;
                    continue;
                }
            } else {
                left = closest_end;
                right = closest_end;
                to_kind = PolyKind::Ground;
            }

            // Right side of the funnel.
            if tri_area_2d(apex, portal_right, right) <= 0.0 {
                if vequal(apex, portal_right) || tri_area_2d(apex, portal_left, right) > 0.0 {
                    portal_right = right;
                    right_ref = path.get(i + 1).copied().unwrap_or(PolyRef::NONE);
                    right_kind = to_kind;
                    right_index = i;
                } else {
                    apex = portal_left;
                    let apex_index = left_index;
                    let flags = if left_ref.is_none() {
                        PathCorner::END
                    } else if left_kind == PolyKind::OffMesh {
                        PathCorner::OFF_MESH
                    } else {
                        0
                    };
                    if !append(&mut out, apex, flags, left_ref) {
                        return out;
                    }
                    portal_left = apex;
                    portal_right = apex;
                    left_index = apex_index;
                    right_index = apex_index;
                    i = apex_index + 1;
                    continue;
                }
            }

            // Left side.
            if tri_area_2d(apex, portal_left, left) >= 0.0 {
                if vequal(apex, portal_left) || tri_area_2d(apex, portal_right, left) < 0.0 {
                    portal_left = left;
                    left_ref = path.get(i + 1).copied().unwrap_or(PolyRef::NONE);
                    left_kind = to_kind;
                    left_index = i;
                } else {
                    apex = portal_right;
                    let apex_index = right_index;
                    let flags = if right_ref.is_none() {
                        PathCorner::END
                    } else if right_kind == PolyKind::OffMesh {
                        PathCorner::OFF_MESH
                    } else {
                        0
                    };
                    if !append(&mut out, apex, flags, right_ref) {
                        return out;
                    }
                    portal_left = apex;
                    portal_right = apex;
                    left_index = apex_index;
                    right_index = apex_index;
                    i = apex_index + 1;
                    continue;
                }
            }
            i += 1;
        }
    }
    append(&mut out, closest_end, PathCorner::END, PolyRef::NONE);
    out
}

/// Where a segment enters and leaves a convex polygon (x/z): the parameters and the
/// edges crossed (`None` when it starts or ends inside).
fn intersect_segment_poly_2d(
    p0: Vec3,
    p1: Vec3,
    verts: &[Vec3],
) -> Option<(f32, f32, Option<usize>, Option<usize>)> {
    const EPS: f32 = 0.000001;
    let mut tmin = 0.0f32;
    let mut tmax = 1.0f32;
    let mut seg_min = None;
    let mut seg_max = None;
    let dir = p1 - p0;
    let perp = |u: Vec3, v: Vec3| u.z * v.x - u.x * v.z;
    let n = verts.len();
    let mut j = n - 1;
    for i in 0..n {
        let edge = verts[i] - verts[j];
        let diff = p0 - verts[j];
        let num = perp(edge, diff);
        let den = perp(dir, edge);
        if den.abs() < EPS {
            if num < 0.0 {
                return None;
            }
            j = i;
            continue;
        }
        let t = num / den;
        if den < 0.0 {
            if t > tmin {
                tmin = t;
                seg_min = Some(j);
                if tmin > tmax {
                    return None;
                }
            }
        } else if t < tmax {
            tmax = t;
            seg_max = Some(j);
            if tmax < tmin {
                return None;
            }
        }
        j = i;
    }
    Some((tmin, tmax, seg_min, seg_max))
}

/// What a [`raycast`] along the floor found.
#[derive(Debug, Clone, PartialEq)]
pub struct RaycastHit {
    /// How far along the segment the wall is (`f32::MAX`: no wall, the end was reached).
    pub t: f32,
    /// The wall's normal (x/z), zero when nothing was hit.
    pub normal: Vec3,
    /// The polygons crossed.
    pub path: Vec<PolyRef>,
}

/// Walk a straight line over the floor from `start` (on polygon `start_ref`) toward
/// `end`, stopping at the first wall.
pub fn raycast(
    mesh: &NavMesh,
    start_ref: PolyRef,
    start: Vec3,
    end: Vec3,
    filter: &QueryFilter,
) -> RaycastHit {
    let mut hit = RaycastHit {
        t: 0.0,
        normal: Vec3::ZERO,
        path: Vec::new(),
    };
    let mut current = start_ref;
    let mut guard = 0;
    while !current.is_none() {
        guard += 1;
        if guard > 4096 {
            break;
        }
        let verts = mesh.poly_vertices(current);
        if verts.len() < 3 {
            return hit;
        }
        let Some((_, tmax, _, seg_max)) = intersect_segment_poly_2d(start, end, &verts) else {
            return hit;
        };
        if tmax > hit.t {
            hit.t = tmax;
        }
        hit.path.push(current);
        let Some(edge) = seg_max else {
            hit.t = f32::MAX;
            return hit;
        };
        let mut next = PolyRef::NONE;
        for link in mesh.links(current) {
            if usize::from(link.edge) != edge {
                continue;
            }
            let Some((_, poly)) = mesh.tile_and_poly(link.target) else {
                continue;
            };
            if poly.kind == PolyKind::OffMesh || !filter.passes(poly.area) {
                continue;
            }
            if link.side == 0xff || (link.tmin <= 0.0 && link.tmax >= 1.0) {
                next = link.target;
                break;
            }
            // A link along part of a tile-border edge: is the crossing inside it?
            let (left, right) = (verts[edge], verts[(edge + 1) % verts.len()]);
            let crossing = start.lerp(end, tmax);
            let (lo, hi, at) = if link.side == 0 || link.side == 2 {
                (
                    left.z + (right.z - left.z) * link.tmin,
                    left.z + (right.z - left.z) * link.tmax,
                    crossing.z,
                )
            } else {
                (
                    left.x + (right.x - left.x) * link.tmin,
                    left.x + (right.x - left.x) * link.tmax,
                    crossing.x,
                )
            };
            let (lo, hi) = if lo > hi { (hi, lo) } else { (lo, hi) };
            if at >= lo && at <= hi {
                next = link.target;
                break;
            }
        }
        if next.is_none() {
            let (a, b) = (verts[edge], verts[(edge + 1) % verts.len()]);
            let (dx, dz) = (b.x - a.x, b.z - a.z);
            hit.normal = Vec3::new(dz, 0.0, -dx).normalize_or_zero();
            return hit;
        }
        current = next;
    }
    hit
}

/// Move from `start` (on `start_ref`) toward `end` along the floor, sliding along walls:
/// returns where it got to (height not adjusted) and the polygons visited, start first.
pub fn move_along_surface(
    mesh: &NavMesh,
    start_ref: PolyRef,
    start: Vec3,
    end: Vec3,
    filter: &QueryFilter,
) -> (Vec3, Vec<PolyRef>) {
    if !mesh.is_valid(start_ref) {
        return (start, Vec::new());
    }
    struct Visit {
        r: PolyRef,
        parent: Option<usize>,
    }
    let search_pos = (start + end) * 0.5;
    let search_rad_sqr = {
        let r = start.distance(end) * 0.5 + 0.001;
        r * r
    };
    let mut visits = vec![Visit {
        r: start_ref,
        parent: None,
    }];
    let mut closed: HashSet<PolyRef> = HashSet::from([start_ref]);
    let mut queue: VecDeque<usize> = VecDeque::from([0]);
    let mut best_pos = start;
    let mut best_dist = f32::MAX;
    let mut best_node = 0;
    while let Some(current) = queue.pop_front() {
        let current_ref = visits[current].r;
        let verts = mesh.poly_vertices(current_ref);
        if verts.len() < 3 {
            continue;
        }
        if point_in_polygon_xz(end, &verts) {
            best_node = current;
            best_pos = end;
            break;
        }
        let n = verts.len();
        let links = mesh.links(current_ref);
        let Some((tile, poly)) = mesh.tile_and_poly(current_ref) else {
            continue;
        };
        let mut j = n - 1;
        for i in 0..n {
            // Neighbours across edge j (from vertex j to i).
            let mut neighbours: Vec<PolyRef> = Vec::new();
            let nei = poly.neis[j];
            if nei & EXT_LINK != 0 || nei != 0 {
                for link in links {
                    if usize::from(link.edge) == j {
                        if let Some((_, other)) = mesh.tile_and_poly(link.target) {
                            if other.kind == PolyKind::Ground && filter.passes(other.area) {
                                neighbours.push(link.target);
                            }
                        }
                    }
                }
            }
            let _ = tile;
            let (vj, vi) = (verts[j], verts[i]);
            if neighbours.is_empty() {
                let (d, t) = distance_pt_seg_sqr_xz(end, vj, vi);
                if d < best_dist {
                    best_pos = vj.lerp(vi, t);
                    best_dist = d;
                    best_node = current;
                }
            } else {
                for neighbour in neighbours {
                    if closed.contains(&neighbour) {
                        continue;
                    }
                    let (d, _) = distance_pt_seg_sqr_xz(search_pos, vj, vi);
                    if d > search_rad_sqr {
                        continue;
                    }
                    closed.insert(neighbour);
                    visits.push(Visit {
                        r: neighbour,
                        parent: Some(current),
                    });
                    queue.push_back(visits.len() - 1);
                }
            }
            j = i;
        }
    }
    let mut visited = Vec::new();
    let mut at = Some(best_node);
    while let Some(n) = at {
        visited.push(visits[n].r);
        at = visits[n].parent;
    }
    visited.reverse();
    (best_pos, visited)
}

/// The wall segments of polygon `r`: its edges, minus the parts that lead to
/// neighbours the filter lets through.
pub fn poly_wall_segments(mesh: &NavMesh, r: PolyRef, filter: &QueryFilter) -> Vec<(Vec3, Vec3)> {
    let mut walls = Vec::new();
    let Some((_, poly)) = mesh.tile_and_poly(r) else {
        return walls;
    };
    if poly.kind != PolyKind::Ground {
        return walls;
    }
    let verts = mesh.poly_vertices(r);
    let n = verts.len();
    let links = mesh.links(r);
    for j in 0..n {
        let (a, b) = (verts[j], verts[(j + 1) % n]);
        let mut open: Vec<(f32, f32)> = links
            .iter()
            .filter(|link| usize::from(link.edge) == j)
            .filter(|link| {
                mesh.tile_and_poly(link.target).is_some_and(|(_, other)| {
                    other.kind == PolyKind::Ground && filter.passes(other.area)
                })
            })
            .map(|link| (link.tmin, link.tmax))
            .collect();
        if open.is_empty() {
            walls.push((a, b));
            continue;
        }
        open.sort_by(|x, y| x.0.total_cmp(&y.0));
        let mut t = 0.0f32;
        for (lo, hi) in open {
            if lo > t + 0.01 {
                walls.push((a.lerp(b, t), a.lerp(b, lo)));
            }
            t = t.max(hi);
        }
        if t < 0.99 {
            walls.push((a.lerp(b, t), b));
        }
    }
    walls
}

/// The polygons around `start_ref` whose edges come within `radius` of `center`
/// (breadth first, at most `max_polys`).
pub fn find_local_neighbourhood(
    mesh: &NavMesh,
    start_ref: PolyRef,
    center: Vec3,
    radius: f32,
    filter: &QueryFilter,
    max_polys: usize,
) -> Vec<PolyRef> {
    let mut result = vec![start_ref];
    if !mesh.is_valid(start_ref) {
        return result;
    }
    let radius_sqr = radius * radius;
    let mut seen: HashSet<PolyRef> = HashSet::from([start_ref]);
    let mut queue = VecDeque::from([start_ref]);
    while let Some(current) = queue.pop_front() {
        for link in mesh.links(current) {
            let neighbour = link.target;
            if seen.contains(&neighbour) {
                continue;
            }
            let Some((_, poly)) = mesh.tile_and_poly(neighbour) else {
                continue;
            };
            if poly.kind != PolyKind::Ground || !filter.passes(poly.area) {
                continue;
            }
            let Some((left, right, _, _)) = mesh.portal_points(current, neighbour) else {
                continue;
            };
            if distance_pt_seg_sqr_xz(center, left, right).0 > radius_sqr {
                continue;
            }
            // Keep to the agent's own floor (portals far above or below are another level).
            if (left.y.min(right.y) - center.y).abs() > radius * 2.0
                && (left.y.max(right.y) - center.y).abs() > radius * 2.0
            {
                continue;
            }
            seen.insert(neighbour);
            result.push(neighbour);
            if result.len() >= max_polys {
                return result;
            }
            queue.push_back(neighbour);
        }
    }
    result
}

fn polygon_area_xz(points: &[Vec3]) -> f32 {
    let mut area = 0.0;
    for i in 1..points.len().saturating_sub(1) {
        area += tri_area_2d(points[0], points[i], points[i + 1]).abs() * 0.5;
    }
    area
}

fn random_point_in_polygon(points: &[Vec3], rng: &mut impl Rng) -> Vec3 {
    let total = polygon_area_xz(points);
    let mut pick = rng.gen::<f32>() * total;
    let mut tri = (points[0], points[1], points[2.min(points.len() - 1)]);
    for i in 1..points.len().saturating_sub(1) {
        let a = tri_area_2d(points[0], points[i], points[i + 1]).abs() * 0.5;
        tri = (points[0], points[i], points[i + 1]);
        if pick <= a {
            break;
        }
        pick -= a;
    }
    let (mut s, mut t) = (rng.gen::<f32>(), rng.gen::<f32>());
    if s + t > 1.0 {
        s = 1.0 - s;
        t = 1.0 - t;
    }
    tri.0 + (tri.1 - tri.0) * s + (tri.2 - tri.0) * t
}

/// A random point on the floor, area weighted: anywhere reachable from `from`
/// (within `radius` of it when given), or anywhere on the navmesh.
pub fn random_point(
    mesh: &NavMesh,
    from: Option<(PolyRef, Vec3)>,
    radius: Option<f32>,
    filter: &QueryFilter,
    rng: &mut impl Rng,
) -> Option<(PolyRef, Vec3)> {
    let mut candidates: Vec<(PolyRef, f32)> = Vec::new();
    match from {
        Some((start, center)) if mesh.is_valid(start) => {
            let radius_sqr = radius.map(|r| r * r);
            let mut seen: HashSet<PolyRef> = HashSet::from([start]);
            let mut queue = VecDeque::from([start]);
            while let Some(current) = queue.pop_front() {
                let points = mesh.poly_vertices(current);
                candidates.push((current, polygon_area_xz(&points)));
                for link in mesh.links(current) {
                    let neighbour = link.target;
                    if seen.contains(&neighbour) {
                        continue;
                    }
                    let Some((_, poly)) = mesh.tile_and_poly(neighbour) else {
                        continue;
                    };
                    if poly.kind != PolyKind::Ground || !filter.passes(poly.area) {
                        continue;
                    }
                    if let Some(radius_sqr) = radius_sqr {
                        let Some((left, right, _, _)) = mesh.portal_points(current, neighbour)
                        else {
                            continue;
                        };
                        if distance_pt_seg_sqr_xz(center, left, right).0 > radius_sqr {
                            continue;
                        }
                    }
                    seen.insert(neighbour);
                    queue.push_back(neighbour);
                }
            }
        }
        _ => {
            for (slot, tile) in mesh.tiles_with_slots() {
                for i in 0..tile.ground_count {
                    if !filter.passes(tile.polys[i].area) {
                        continue;
                    }
                    let points: Vec<Vec3> = tile.poly_points(i).collect();
                    candidates.push((mesh.poly_ref(slot, i), polygon_area_xz(&points)));
                }
            }
        }
    }
    let total: f32 = candidates.iter().map(|c| c.1).sum();
    if candidates.is_empty() || total <= 0.0 {
        return None;
    }
    for _ in 0..8 {
        let mut pick = rng.gen::<f32>() * total;
        let mut chosen = candidates[candidates.len() - 1].0;
        for &(r, area) in &candidates {
            if pick <= area {
                chosen = r;
                break;
            }
            pick -= area;
        }
        let points = mesh.poly_vertices(chosen);
        let mut p = random_point_in_polygon(&points, rng);
        p.y = mesh.poly_height(chosen, p).unwrap_or(p.y);
        let inside = match (from, radius) {
            (Some((_, center)), Some(r)) => {
                let d = Vec3::new(p.x - center.x, 0.0, p.z - center.z);
                d.length_squared() <= r * r
            }
            _ => true,
        };
        if inside {
            return Some((chosen, p));
        }
    }
    None
}

/// Straighten a corridor: wherever the corners either side of a corner see each other
/// along the floor (a raycast reaches), the corridor between them is replaced by the
/// polygons the ray crossed, and the corner goes. A* over polygons can pick a corridor
/// that bends through a tile corner the funnel then has to follow; this removes those.
pub fn smooth_corridor(
    mesh: &NavMesh,
    path: Vec<PolyRef>,
    start: Vec3,
    end: Vec3,
    filter: &QueryFilter,
) -> Vec<PolyRef> {
    let mut path = path;
    for _ in 0..32 {
        let corners = find_straight_path(mesh, start, end, &path, 256);
        if corners.len() <= 2 {
            break;
        }
        let index_of = |r: PolyRef, from: usize, path: &[PolyRef]| {
            path[from..].iter().position(|&p| p == r).map(|i| i + from)
        };
        let mut improved = false;
        for k in 1..corners.len() - 1 {
            let a = corners[k - 1];
            let c = corners[k + 1];
            if a.flags & PathCorner::OFF_MESH != 0 || corners[k].flags & PathCorner::OFF_MESH != 0 {
                continue;
            }
            let a_ref = if k == 1 { path[0] } else { a.poly };
            let Some(ai) = index_of(a_ref, 0, &path) else {
                continue;
            };
            let hit = raycast(mesh, a_ref, a.pos, c.pos, filter);
            if hit.t != f32::MAX {
                continue;
            }
            let Some(&last) = hit.path.last() else {
                continue;
            };
            let Some(ci) = index_of(last, ai, &path) else {
                continue;
            };
            let mut spliced = path[..ai].to_vec();
            spliced.extend_from_slice(&hit.path);
            spliced.extend_from_slice(&path[ci + 1..]);
            // A ray can step back over the portal it starts on: drop the loop.
            spliced.dedup();
            if spliced != path {
                path = spliced;
                improved = true;
                break;
            }
        }
        if !improved {
            break;
        }
    }
    path
}

/// A whole path between two points: the nearest polygons to each (within
/// `half_extents`), the corridor and its corners.
pub fn find_path_points(
    mesh: &NavMesh,
    start: Vec3,
    end: Vec3,
    half_extents: Vec3,
    filter: &QueryFilter,
) -> Option<(Vec<PathCorner>, PolyPath)> {
    let (start_ref, start_pos) = mesh.find_nearest_poly(start, half_extents)?;
    let (end_ref, end_pos) = mesh.find_nearest_poly(end, half_extents)?;
    let mut path = find_path(mesh, start_ref, end_ref, start_pos, end_pos, filter, 8192);
    if path.polys.is_empty() {
        return None;
    }
    // A partial path ends at the reachable point nearest the goal.
    let target = if path.complete {
        end_pos
    } else {
        let last = *path.polys.last()?;
        mesh.closest_point_on_poly(last, end_pos)
            .map_or(end_pos, |c| c.0)
    };
    path.polys = smooth_corridor(
        mesh,
        std::mem::take(&mut path.polys),
        start_pos,
        target,
        filter,
    );
    let corners = find_straight_path(mesh, start_pos, target, &path.polys, 512);
    Some((corners, path))
}
