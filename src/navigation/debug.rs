//! Debug drawing: navmesh polygons tinted by area, their outlines (walls thicker),
//! off-mesh links as arcs, agents with their velocities and paths, and obstacles'
//! footprints.

use glam::Vec3;

use super::crowd::{AgentState, CrowdAgent};
use super::navmesh::{NavMesh, PolyKind, EXT_LINK};
use super::query::{find_path_points, QueryFilter};
use super::recast::{MarkShape, WALKABLE_AREA};
use super::scene_nav::NavDebugDraw;
use crate::renderer::debug_draw::{DebugColor, DebugGeometry};

const WALKABLE_COLOR: DebugColor = [0.12, 0.62, 0.82, 0.30];
const INNER_EDGE_COLOR: DebugColor = [0.55, 0.85, 1.0, 0.22];
const WALL_COLOR: DebugColor = [0.02, 0.22, 0.32, 0.95];
const LINK_COLOR: DebugColor = [1.0, 0.85, 0.2, 0.95];
pub(crate) const IDLE_COLOR: DebugColor = [0.75, 0.75, 0.78, 0.9];
const MOVING_COLOR: DebugColor = [0.3, 1.0, 0.45, 0.95];
const ARRIVED_COLOR: DebugColor = [0.35, 0.6, 1.0, 0.95];
const FAILED_COLOR: DebugColor = [1.0, 0.25, 0.2, 0.95];
const PLANNING_COLOR: DebugColor = [1.0, 0.9, 0.3, 0.95];
const PATH_COLOR: DebugColor = [1.0, 0.6, 0.15, 0.9];
const VELOCITY_COLOR: DebugColor = [0.95, 1.0, 0.95, 0.9];
const CARVED_COLOR: DebugColor = [1.0, 0.2, 0.25, 0.95];
const MOVING_OBSTACLE_COLOR: DebugColor = [1.0, 0.65, 0.1, 0.95];
const TEST_PATH_COLOR: DebugColor = [1.0, 0.3, 0.9, 1.0];

/// A distinct tint per custom area id.
fn area_color(area: u8) -> DebugColor {
    if area == WALKABLE_AREA {
        return WALKABLE_COLOR;
    }
    let hue = (f32::from(area) * 0.618_034) % 1.0;
    let (r, g, b) = hsv(hue, 0.6, 0.9);
    [r, g, b, 0.35]
}

fn hsv(h: f32, s: f32, v: f32) -> (f32, f32, f32) {
    let i = (h * 6.0).floor();
    let f = h * 6.0 - i;
    let p = v * (1.0 - s);
    let q = v * (1.0 - f * s);
    let t = v * (1.0 - (1.0 - f) * s);
    match i as i32 % 6 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    }
}

/// The whole navmesh.
pub(crate) fn mesh_geometry(mesh: &NavMesh) -> DebugGeometry {
    let mut out = DebugGeometry::default();
    for tile in mesh.tiles() {
        // A slight checker between tiles shows where they meet.
        let shade = if (tile.x + tile.z).rem_euclid(2) == 0 {
            1.0
        } else {
            0.85
        };
        for (index, poly) in tile.polys.iter().enumerate().take(tile.ground_count) {
            let base = area_color(poly.area);
            let color = [base[0] * shade, base[1] * shade, base[2] * shade, base[3]];
            for [a, b, c] in tile.detail_triangles(index) {
                out.triangle(a, b, c, color);
            }
            let n = usize::from(poly.vert_count);
            let links = &tile.links[index];
            for j in 0..n {
                let a = tile.verts[usize::from(poly.verts[j])];
                let b = tile.verts[usize::from(poly.verts[(j + 1) % n])];
                let linked = links.iter().any(|link| usize::from(link.edge) == j);
                let nei = poly.neis[j];
                if linked || (nei != 0 && nei & EXT_LINK == 0) {
                    // Only draw each inner edge once.
                    if nei & EXT_LINK != 0 || usize::from(nei) > index {
                        out.line(a, b, 1.0, INNER_EDGE_COLOR);
                    }
                } else {
                    out.line(a, b, 2.5, WALL_COLOR);
                }
            }
        }
        for link in &tile.off_mesh {
            arc(&mut out, link.start, link.end, LINK_COLOR);
            out.circle(link.start, link.radius.max(0.1), 1.5, LINK_COLOR);
            out.circle(link.end, link.radius.max(0.1), 1.5, LINK_COLOR);
        }
        let _ = PolyKind::Ground;
    }
    out
}

fn arc(out: &mut DebugGeometry, start: Vec3, end: Vec3, color: DebugColor) {
    const SEGMENTS: usize = 12;
    let height = 0.25 + start.distance(end) * 0.15 + (end.y - start.y).max(0.0) * 0.5;
    let point = |i: usize| {
        let t = i as f32 / SEGMENTS as f32;
        let mut p = start.lerp(end, t);
        p.y += height * 4.0 * t * (1.0 - t);
        p
    };
    for i in 0..SEGMENTS {
        out.line(point(i), point(i + 1), 2.0, color);
    }
}

fn state_color(state: AgentState) -> DebugColor {
    match state {
        AgentState::Idle => IDLE_COLOR,
        AgentState::Planning => PLANNING_COLOR,
        AgentState::Moving | AgentState::OffMesh => MOVING_COLOR,
        AgentState::Arrived => ARRIVED_COLOR,
        AgentState::Failed | AgentState::Lost => FAILED_COLOR,
    }
}

pub(crate) fn agent_geometry(agent: &CrowdAgent, debug: NavDebugDraw, out: &mut DebugGeometry) {
    let feet = agent.pos + Vec3::Y * 0.03;
    if debug.agents {
        let color = state_color(agent.state);
        out.circle(feet, agent.params.radius, 2.0, color);
        let speed = agent.vel.length();
        if speed > 0.05 {
            out.arrow(feet, feet + agent.vel * 0.6, 1.5, VELOCITY_COLOR);
        }
        if agent.partial {
            out.circle(feet, agent.params.radius * 0.5, 1.5, FAILED_COLOR);
        }
    }
    if debug.paths && matches!(agent.state, AgentState::Moving | AgentState::Planning) {
        let mut at = feet;
        for corner in &agent.corners {
            let p = corner.pos + Vec3::Y * 0.05;
            out.line(at, p, 1.5, PATH_COLOR);
            at = p;
        }
        if let Some((_, target)) = agent.target {
            let t = target + Vec3::Y * 0.05;
            if agent.corners.is_empty() || agent.corners.len() >= 4 {
                out.line(
                    at,
                    t,
                    1.0,
                    [PATH_COLOR[0], PATH_COLOR[1], PATH_COLOR[2], 0.35],
                );
            }
            let s = 0.2;
            out.line(
                t + Vec3::new(-s, 0.0, -s),
                t + Vec3::new(s, 0.0, s),
                2.0,
                PATH_COLOR,
            );
            out.line(
                t + Vec3::new(-s, 0.0, s),
                t + Vec3::new(s, 0.0, -s),
                2.0,
                PATH_COLOR,
            );
        }
    }
}

pub(crate) fn obstacle_geometry(shape: &MarkShape, carved: bool, out: &mut DebugGeometry) {
    let color = if carved {
        CARVED_COLOR
    } else {
        MOVING_OBSTACLE_COLOR
    };
    match shape {
        MarkShape::Convex {
            points,
            min_y,
            max_y,
        } => {
            let n = points.len();
            for i in 0..n {
                let (a, b) = (points[i], points[(i + 1) % n]);
                out.line(
                    Vec3::new(a.x, *min_y, a.y),
                    Vec3::new(b.x, *min_y, b.y),
                    2.5,
                    color,
                );
                out.line(
                    Vec3::new(a.x, *max_y, a.y),
                    Vec3::new(b.x, *max_y, b.y),
                    1.5,
                    color,
                );
                out.line(
                    Vec3::new(a.x, *min_y, a.y),
                    Vec3::new(a.x, *max_y, a.y),
                    1.0,
                    color,
                );
            }
        }
        MarkShape::Cylinder {
            base,
            radius,
            height,
        } => {
            out.circle(*base, *radius, 2.5, color);
            out.circle(*base + Vec3::Y * *height, *radius, 1.5, color);
        }
    }
}

pub(crate) fn test_path_geometry(
    mesh: Option<&NavMesh>,
    from: Vec3,
    to: Vec3,
    out: &mut DebugGeometry,
) {
    out.circle(from, 0.3, 3.0, TEST_PATH_COLOR);
    out.circle(to, 0.3, 3.0, TEST_PATH_COLOR);
    let Some(mesh) = mesh else {
        return;
    };
    let filter = QueryFilter::default();
    let Some((corners, path)) = find_path_points(mesh, from, to, Vec3::new(2.0, 2.0, 2.0), &filter)
    else {
        return;
    };
    let color = if path.complete {
        TEST_PATH_COLOR
    } else {
        FAILED_COLOR
    };
    for pair in corners.windows(2) {
        out.line(
            pair[0].pos + Vec3::Y * 0.08,
            pair[1].pos + Vec3::Y * 0.08,
            3.5,
            color,
        );
    }
    for corner in &corners {
        out.circle(corner.pos + Vec3::Y * 0.08, 0.08, 2.0, color);
    }
}
