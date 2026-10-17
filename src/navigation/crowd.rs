//! Crowds: many agents walking the navmesh at once, after Detour's `dtCrowd`.
//!
//! Each agent keeps a corridor of polygons to its target and walks toward the next
//! corners of the straightest path through it. Every frame the corridor follows the
//! agent (sliding along walls), shortcuts open up by raycasting ahead, neighbours and
//! moving obstacles are avoided (ORCA, with the nearby navmesh walls as obstacles),
//! overlaps are pushed apart, and agents whose corridor went stale (a tile was rebuilt
//! under it) or that are stuck plan again.

use std::collections::{HashMap, VecDeque};

use glam::{Vec2, Vec3};

use super::avoidance::{compute_velocity, AvoidanceInput, Neighbour};
use super::navmesh::{NavMesh, PolyKind, PolyRef};
use super::query::{
    find_local_neighbourhood, find_path, find_straight_path, move_along_surface,
    poly_wall_segments, raycast, smooth_corridor, PathCorner, QueryFilter,
};

/// Corners closer than this to the agent are passed.
const MIN_TARGET_DIST: f32 = 0.01;
const MAX_CORNERS: usize = 4;
const MAX_NEIGHBOURS: usize = 8;
const MAX_WALLS: usize = 10;
const MAX_PATH_NODES: usize = 4096;
/// Repath an agent whose path stopped short of its target this often (s), in case the
/// way opened.
const PARTIAL_REPATH_INTERVAL: f32 = 1.5;
/// An agent that wants to move but makes less than this share of its speed for
/// `STUCK_TIME` plans again.
const STUCK_SPEED_SHARE: f32 = 0.1;
const STUCK_TIME: f32 = 2.5;
/// How far agents keep from the navmesh's edges (m), which already stop the surface's
/// agent radius short of walls.
const WALL_CLEARANCE: f32 = 0.05;
/// How far agents lean to one side when someone comes the other way (share of their
/// speed).
const KEEP_SIDE: f32 = 0.25;
/// An agent stuck this long (s) face to face with oncoming agents steps back and aside
/// for `GIVE_WAY_TIME`: two pairs walking abreast into each other on a narrow bridge
/// fall into single file, keeping to their sides.
const GIVE_WAY_AFTER: f32 = 1.0;
const GIVE_WAY_TIME: f32 = 0.8;
/// Stuck this long (s) within two radii of its end, an agent has arrived.
const BLOCKED_ARRIVAL_TIME: f32 = 0.75;

fn xz(v: Vec3) -> Vec2 {
    Vec2::new(v.x, v.z)
}

fn dist_xz(a: Vec3, b: Vec3) -> f32 {
    xz(a).distance(xz(b))
}

/// How an agent moves and avoids, in world units.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentParams {
    pub radius: f32,
    pub height: f32,
    pub max_speed: f32,
    pub max_acceleration: f32,
    /// Within this of the end it has arrived.
    pub stopping_distance: f32,
    pub avoidance: bool,
    /// 0..1: who makes way for whom.
    pub priority: f32,
    pub separation: f32,
    /// Neighbours and walls within this are considered.
    pub collision_query_range: f32,
    /// How far ahead shortcuts are looked for.
    pub path_optimization_range: f32,
    /// The agent's position comes from outside (a character controller moves it); the
    /// crowd only steers.
    pub external_motion: bool,
    pub filter: QueryFilter,
}

impl Default for AgentParams {
    fn default() -> Self {
        Self::with_radius(0.35)
    }
}

impl AgentParams {
    pub fn with_radius(radius: f32) -> Self {
        Self {
            radius,
            height: 1.8,
            max_speed: 1.5,
            max_acceleration: 8.0,
            stopping_distance: 0.15,
            avoidance: true,
            priority: 0.5,
            separation: 1.0,
            collision_query_range: radius * 12.0,
            path_optimization_range: radius * 30.0,
            external_motion: false,
            filter: QueryFilter::default(),
        }
    }
}

/// Where an agent is in its walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AgentState {
    /// No destination.
    #[default]
    Idle,
    /// Waiting for its path to be planned.
    Planning,
    Moving,
    Arrived,
    /// No way to the destination (or it is off the navmesh).
    Failed,
    /// Crossing an off-mesh link (a jump).
    OffMesh,
    /// Not on any navmesh.
    Lost,
}

impl AgentState {
    pub fn name(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Planning => "planning",
            Self::Moving => "moving",
            Self::Arrived => "arrived",
            Self::Failed => "failed",
            Self::OffMesh => "off_mesh",
            Self::Lost => "lost",
        }
    }
}

/// The corridor of polygons an agent walks, and where it is in it.
#[derive(Debug, Clone, Default)]
pub struct PathCorridor {
    pub pos: Vec3,
    pub target: Vec3,
    pub path: Vec<PolyRef>,
}

impl PathCorridor {
    pub fn reset(&mut self, r: PolyRef, pos: Vec3) {
        self.pos = pos;
        self.target = pos;
        self.path = vec![r];
    }

    pub fn first_poly(&self) -> PolyRef {
        self.path.first().copied().unwrap_or(PolyRef::NONE)
    }

    pub fn last_poly(&self) -> PolyRef {
        self.path.last().copied().unwrap_or(PolyRef::NONE)
    }

    /// The next corners to walk to, the ones already reached removed.
    pub fn find_corners(&self, mesh: &NavMesh, max: usize) -> Vec<PathCorner> {
        let mut corners = find_straight_path(mesh, self.pos, self.target, &self.path, max + 1);
        while let Some(first) = corners.first() {
            if first.flags & PathCorner::OFF_MESH != 0
                || dist_xz(first.pos, self.pos) > MIN_TARGET_DIST
            {
                break;
            }
            corners.remove(0);
        }
        if let Some(i) = corners
            .iter()
            .position(|c| c.flags & PathCorner::OFF_MESH != 0)
        {
            corners.truncate(i + 1);
        }
        corners.truncate(max);
        corners
    }

    /// Move along the floor toward `npos`; the corridor's start follows.
    pub fn move_position(&mut self, mesh: &NavMesh, npos: Vec3, filter: &QueryFilter) -> bool {
        let (result, visited) = move_along_surface(mesh, self.first_poly(), self.pos, npos, filter);
        if visited.is_empty() {
            return false;
        }
        self.path = merge_corridor_start_moved(&self.path, &visited);
        let h = mesh
            .poly_height(self.first_poly(), result)
            .unwrap_or(npos.y);
        self.pos = Vec3::new(result.x, h, result.z);
        true
    }

    /// Look ahead toward `next` (up to `range`): if the floor is open, the polygons the
    /// ray crosses replace the start of the corridor.
    pub fn optimize_path_visibility(
        &mut self,
        mesh: &NavMesh,
        next: Vec3,
        range: f32,
        filter: &QueryFilter,
    ) {
        let dist = dist_xz(self.pos, next);
        if dist < 0.01 {
            return;
        }
        let dist = (dist + 0.01).min(range);
        let delta = next - self.pos;
        let goal = self.pos + delta * (range / dist);
        let hit = raycast(mesh, self.first_poly(), self.pos, goal, filter);
        if hit.path.len() > 1 && hit.t > 0.99 {
            self.path = merge_corridor_start_shortcut(&self.path, &hit.path);
        }
    }

    /// Whether the first `look_ahead` polygons all still exist.
    pub fn is_valid(&self, mesh: &NavMesh, look_ahead: usize) -> bool {
        self.path.iter().take(look_ahead).all(|&r| mesh.is_valid(r))
    }
}

fn merge_corridor_start_moved(path: &[PolyRef], visited: &[PolyRef]) -> Vec<PolyRef> {
    let mut furthest = None;
    'outer: for i in (0..path.len()).rev() {
        for j in (0..visited.len()).rev() {
            if path[i] == visited[j] {
                furthest = Some((i, j));
                break 'outer;
            }
        }
    }
    let Some((fp, fv)) = furthest else {
        // Moved off the corridor altogether: start it where we are.
        let mut fresh = vec![*visited.last().unwrap_or(&PolyRef::NONE)];
        fresh.extend_from_slice(path);
        fresh.dedup();
        return fresh;
    };
    let mut merged: Vec<PolyRef> = visited[fv..].iter().rev().copied().collect();
    merged.extend_from_slice(&path[(fp + 1).min(path.len())..]);
    merged
}

fn merge_corridor_start_shortcut(path: &[PolyRef], visited: &[PolyRef]) -> Vec<PolyRef> {
    let mut furthest = None;
    'outer: for i in (0..path.len()).rev() {
        for j in (0..visited.len()).rev() {
            if path[i] == visited[j] {
                furthest = Some((i, j));
                break 'outer;
            }
        }
    }
    let Some((fp, fv)) = furthest else {
        return path.to_vec();
    };
    if fv == 0 {
        return path.to_vec();
    }
    let mut merged = visited[..fv].to_vec();
    merged.extend_from_slice(&path[fp..]);
    merged
}

/// The walls near an agent, updated when it has moved far enough.
#[derive(Debug, Clone, Default)]
struct LocalBoundary {
    center: Vec3,
    first_poly: PolyRef,
    walls: Vec<(Vec2, Vec2)>,
    valid: bool,
}

/// An agent crossing an off-mesh link.
#[derive(Debug, Clone)]
struct OffMeshTraversal {
    from: Vec3,
    start: Vec3,
    end: Vec3,
    t: f32,
    duration: f32,
    /// How high the arc rises above the straight line (m).
    arc: f32,
}

/// Something that happened to an agent this update.
#[derive(Debug, Clone, PartialEq)]
pub enum CrowdEvent {
    Arrived(usize),
    /// No path to its destination.
    Failed(usize),
    /// Started across an off-mesh link (agent, start, end, the link's user id).
    OffMeshStart(usize, Vec3, Vec3, u64),
    OffMeshEnd(usize),
}

#[derive(Debug, Clone)]
pub struct CrowdAgent {
    pub params: AgentParams,
    pub state: AgentState,
    pub pos: Vec3,
    /// Velocity it moves at.
    pub vel: Vec3,
    /// Velocity it would like (toward its next corner).
    pub desired_vel: Vec3,
    /// Velocity avoidance chose.
    pub new_vel: Vec3,
    pub corridor: PathCorridor,
    pub corners: Vec<PathCorner>,
    /// The destination asked for, and where it lies on the navmesh.
    pub target: Option<(PolyRef, Vec3)>,
    pub requested_target: Option<Vec3>,
    /// The path ends short of the target (unreachable for now).
    pub partial: bool,
    /// Driven by a velocity (a player's input) instead of toward a target: that velocity.
    pub steer: Option<Vec3>,
    boundary: LocalBoundary,
    neighbours: Vec<usize>,
    traversal: Option<OffMeshTraversal>,
    repath_timer: f32,
    stuck_time: f32,
    /// Seconds left stepping aside for oncoming agents it is stuck against.
    give_way: f32,
    path_generation: u64,
    displacement: Vec3,
}

impl CrowdAgent {
    /// Path length still to walk (along the corners), if moving.
    pub fn remaining_distance(&self) -> Option<f32> {
        if self.state != AgentState::Moving {
            return None;
        }
        let mut length = 0.0;
        let mut at = self.pos;
        for corner in &self.corners {
            length += at.distance(corner.pos);
            at = corner.pos;
        }
        if self
            .corners
            .last()
            .is_some_and(|c| c.flags & PathCorner::END == 0)
        {
            length += at.distance(self.corridor.target);
        }
        Some(length)
    }
}

/// A moving obstacle agents steer around (a disc; long boxes become several).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MovingObstacle {
    pub position: Vec3,
    pub velocity: Vec3,
    pub radius: f32,
    pub height: f32,
}

/// Every agent walking one navmesh.
pub struct Crowd {
    agents: Vec<Option<CrowdAgent>>,
    free: Vec<usize>,
    path_queue: VecDeque<usize>,
    /// Paths planned per update at most (the rest wait).
    pub max_paths_per_update: usize,
    /// Bumped whenever the navmesh changes, so agents with partial paths retry.
    pub mesh_generation: u64,
}

impl Default for Crowd {
    fn default() -> Self {
        Self::new()
    }
}

impl Crowd {
    pub fn new() -> Self {
        Self {
            agents: Vec::new(),
            free: Vec::new(),
            path_queue: VecDeque::new(),
            max_paths_per_update: 24,
            mesh_generation: 0,
        }
    }

    pub fn agent(&self, id: usize) -> Option<&CrowdAgent> {
        self.agents.get(id)?.as_ref()
    }

    pub fn agent_mut(&mut self, id: usize) -> Option<&mut CrowdAgent> {
        self.agents.get_mut(id)?.as_mut()
    }

    pub fn agents(&self) -> impl Iterator<Item = (usize, &CrowdAgent)> {
        self.agents
            .iter()
            .enumerate()
            .filter_map(|(i, a)| a.as_ref().map(|a| (i, a)))
    }

    pub fn agent_count(&self) -> usize {
        self.agents.iter().flatten().count()
    }

    fn extents(params: &AgentParams) -> Vec3 {
        Vec3::new(
            params.radius * 2.0 + 0.5,
            params.height * 0.75,
            params.radius * 2.0 + 0.5,
        )
    }

    /// Add an agent at `pos` (snapped to the navmesh when it is near).
    pub fn add_agent(&mut self, mesh: &NavMesh, pos: Vec3, params: AgentParams) -> usize {
        let mut agent = CrowdAgent {
            state: AgentState::Idle,
            pos,
            vel: Vec3::ZERO,
            desired_vel: Vec3::ZERO,
            new_vel: Vec3::ZERO,
            corridor: PathCorridor::default(),
            corners: Vec::new(),
            target: None,
            requested_target: None,
            partial: false,
            steer: None,
            boundary: LocalBoundary::default(),
            neighbours: Vec::new(),
            traversal: None,
            repath_timer: 0.0,
            stuck_time: 0.0,
            give_way: 0.0,
            path_generation: 0,
            displacement: Vec3::ZERO,
            params,
        };
        match mesh.find_nearest_poly(pos, Self::extents(&agent.params)) {
            Some((r, nearest)) => {
                agent.corridor.reset(r, nearest);
                agent.pos = nearest;
            }
            None => {
                agent.corridor.reset(PolyRef::NONE, pos);
                agent.state = AgentState::Lost;
            }
        }
        match self.free.pop() {
            Some(id) => {
                self.agents[id] = Some(agent);
                id
            }
            None => {
                self.agents.push(Some(agent));
                self.agents.len() - 1
            }
        }
    }

    pub fn remove_agent(&mut self, id: usize) {
        if let Some(slot) = self.agents.get_mut(id) {
            if slot.take().is_some() {
                self.free.push(id);
            }
        }
        self.path_queue.retain(|&queued| queued != id);
    }

    /// Head for `target`; the path is planned during the next updates.
    pub fn request_move_target(&mut self, mesh: &NavMesh, id: usize, target: Vec3) -> bool {
        let Some(agent) = self.agents.get_mut(id).and_then(Option::as_mut) else {
            return false;
        };
        agent.steer = None;
        agent.requested_target = Some(target);
        let extents = Vec3::new(2.0, agent.params.height.max(2.0), 2.0);
        match mesh.find_nearest_poly(target, extents) {
            Some((r, nearest)) => {
                agent.target = Some((r, nearest));
                if agent.state != AgentState::OffMesh {
                    agent.state = AgentState::Planning;
                }
                if !self.path_queue.contains(&id) {
                    self.path_queue.push_back(id);
                }
                true
            }
            None => {
                agent.target = None;
                agent.state = AgentState::Failed;
                false
            }
        }
    }

    /// Stop where it is.
    pub fn reset_move_target(&mut self, id: usize) {
        if let Some(agent) = self.agents.get_mut(id).and_then(Option::as_mut) {
            agent.target = None;
            agent.requested_target = None;
            agent.partial = false;
            agent.steer = None;
            let r = agent.corridor.first_poly();
            let pos = agent.corridor.pos;
            agent.corridor.reset(r, pos);
            agent.corners.clear();
            if agent.state != AgentState::OffMesh {
                agent.state = AgentState::Idle;
            }
        }
        self.path_queue.retain(|&queued| queued != id);
    }

    /// Move at `velocity` (world space, m/s; its height is ignored) instead of toward a
    /// target, the way a player's input drives a character: it keeps to the navmesh (walls
    /// and edges stop it), the others make way for it, and it avoids nobody itself. It
    /// lasts until a target or `reset_move_target`; ask again every update.
    pub fn request_move_velocity(&mut self, id: usize, velocity: Vec3) -> bool {
        let Some(agent) = self.agents.get_mut(id).and_then(Option::as_mut) else {
            return false;
        };
        agent.steer = Some(Vec3::new(velocity.x, 0.0, velocity.z));
        agent.target = None;
        agent.requested_target = None;
        agent.partial = false;
        agent.corners.clear();
        if agent.state != AgentState::OffMesh && agent.state != AgentState::Lost {
            agent.state = AgentState::Moving;
        }
        self.path_queue.retain(|&queued| queued != id);
        true
    }

    /// Put an agent somewhere else at once.
    pub fn warp(&mut self, mesh: &NavMesh, id: usize, pos: Vec3) -> bool {
        let Some(agent) = self.agents.get_mut(id).and_then(Option::as_mut) else {
            return false;
        };
        let Some((r, nearest)) = mesh.find_nearest_poly(pos, Self::extents(&agent.params)) else {
            return false;
        };
        agent.corridor.reset(r, nearest);
        agent.pos = nearest;
        agent.vel = Vec3::ZERO;
        agent.traversal = None;
        agent.boundary.valid = false;
        if agent.steer.is_some() {
            agent.state = AgentState::Moving;
        } else if let Some(target) = agent.requested_target {
            agent.state = AgentState::Planning;
            let _ = target;
            if !self.path_queue.contains(&id) {
                self.path_queue.push_back(id);
            }
        } else {
            agent.state = AgentState::Idle;
        }
        true
    }

    /// For agents moved from outside (`external_motion`): where they are now.
    pub fn set_agent_position(&mut self, mesh: &NavMesh, id: usize, pos: Vec3) {
        let Some(agent) = self.agents.get_mut(id).and_then(Option::as_mut) else {
            return;
        };
        if agent.state == AgentState::OffMesh || agent.state == AgentState::Lost {
            agent.pos = pos;
            return;
        }
        let filter = agent.params.filter.clone();
        if agent.corridor.move_position(mesh, pos, &filter) {
            agent.pos = Vec3::new(pos.x, pos.y, pos.z);
        }
    }

    /// Advance every agent by `dt` seconds.
    pub fn update(
        &mut self,
        mesh: &NavMesh,
        dt: f32,
        obstacles: &[MovingObstacle],
    ) -> Vec<CrowdEvent> {
        let mut events = Vec::new();
        if dt <= 0.0 {
            return events;
        }
        self.check_path_validity(mesh, dt);
        self.plan_paths(mesh, &mut events);
        self.find_neighbours();

        let ids: Vec<usize> = self.agents().map(|(i, _)| i).collect();
        for &id in &ids {
            self.prepare_agent(mesh, id, &mut events);
        }
        // Avoidance reads every agent's current velocity, so new velocities are
        // computed first and applied after.
        let new_velocities: Vec<(usize, Vec3)> = ids
            .iter()
            .map(|&id| (id, self.avoid(id, dt, obstacles)))
            .collect();
        for (id, nvel) in new_velocities {
            let Some(agent) = self.agent_mut(id) else {
                continue;
            };
            agent.new_vel = nvel;
            if agent.state == AgentState::OffMesh || agent.params.external_motion {
                continue;
            }
            let dv = nvel - agent.vel;
            let max_dv = agent.params.max_acceleration * dt;
            agent.vel += dv.clamp_length_max(max_dv);
            agent.pos += agent.vel * dt;
        }
        self.resolve_collisions(obstacles);
        for &id in &ids {
            self.move_on_mesh(mesh, id, dt);
        }
        self.update_traversals(mesh, dt, &mut events);
        events
    }

    fn check_path_validity(&mut self, mesh: &NavMesh, dt: f32) {
        let generation = self.mesh_generation;
        for id in 0..self.agents.len() {
            let Some(agent) = self.agents[id].as_mut() else {
                continue;
            };
            if agent.state == AgentState::OffMesh {
                continue;
            }
            let mut replan = false;
            if !mesh.is_valid(agent.corridor.first_poly()) {
                match mesh.find_nearest_poly(agent.pos, Self::extents(&agent.params)) {
                    Some((r, nearest)) => {
                        agent.corridor.reset(r, nearest);
                        agent.pos = nearest;
                        agent.boundary.valid = false;
                        if agent.state == AgentState::Lost {
                            agent.state = AgentState::Idle;
                        }
                        replan = agent.target.is_some();
                    }
                    None => {
                        agent.corridor.reset(PolyRef::NONE, agent.pos);
                        agent.state = AgentState::Lost;
                        continue;
                    }
                }
            }
            if let Some((target_ref, target_pos)) = agent.target {
                if !mesh.is_valid(target_ref) {
                    let extents = Vec3::new(2.0, agent.params.height.max(2.0), 2.0);
                    match mesh
                        .find_nearest_poly(agent.requested_target.unwrap_or(target_pos), extents)
                    {
                        Some(found) => {
                            agent.target = Some(found);
                            replan = true;
                        }
                        None => {
                            agent.target = None;
                            agent.state = AgentState::Failed;
                            continue;
                        }
                    }
                }
            }
            if matches!(agent.state, AgentState::Moving) {
                if !agent.corridor.is_valid(mesh, 16) {
                    replan = true;
                }
                if agent.partial {
                    agent.repath_timer += dt;
                    if agent.repath_timer > PARTIAL_REPATH_INTERVAL
                        || agent.path_generation != generation
                    {
                        replan = true;
                    }
                }
                // Stuck behind others or in a corner (or pushed back): give way to
                // whoever it is face to face with, then try another path.
                agent.give_way = (agent.give_way - dt).max(0.0);
                let want = agent.desired_vel.length();
                let progress = if want > 0.0 {
                    agent.vel.dot(agent.desired_vel) / want
                } else {
                    0.0
                };
                if want > 0.05 && progress < want * STUCK_SPEED_SHARE {
                    agent.stuck_time += dt;
                    if agent.stuck_time > GIVE_WAY_AFTER && agent.give_way <= 0.0 {
                        agent.give_way = GIVE_WAY_TIME;
                    }
                    if agent.stuck_time > STUCK_TIME {
                        agent.stuck_time = 0.0;
                        replan = true;
                    }
                } else {
                    agent.stuck_time = 0.0;
                }
            }
            if replan && agent.target.is_some() && !self.path_queue.contains(&id) {
                agent.repath_timer = 0.0;
                self.path_queue.push_back(id);
            }
        }
    }

    fn plan_paths(&mut self, mesh: &NavMesh, events: &mut Vec<CrowdEvent>) {
        let generation = self.mesh_generation;
        for _ in 0..self.max_paths_per_update {
            let Some(id) = self.path_queue.pop_front() else {
                break;
            };
            let Some(agent) = self.agents.get_mut(id).and_then(Option::as_mut) else {
                continue;
            };
            if agent.state == AgentState::OffMesh {
                // Plan once it has landed.
                self.path_queue.push_back(id);
                continue;
            }
            let Some((target_ref, target_pos)) = agent.target else {
                continue;
            };
            let start = agent.corridor.first_poly();
            if !mesh.is_valid(start) {
                agent.state = AgentState::Lost;
                continue;
            }
            let filter = &agent.params.filter;
            let found = find_path(
                mesh,
                start,
                target_ref,
                agent.pos,
                target_pos,
                filter,
                MAX_PATH_NODES,
            );
            if found.polys.is_empty() {
                agent.state = AgentState::Failed;
                events.push(CrowdEvent::Failed(id));
                continue;
            }
            let end = if found.complete {
                target_pos
            } else {
                let last = *found.polys.last().unwrap_or(&start);
                mesh.closest_point_on_poly(last, target_pos)
                    .map_or(target_pos, |c| c.0)
            };
            let polys = smooth_corridor(mesh, found.polys, agent.pos, end, filter);
            agent.corridor.path = polys;
            agent.corridor.target = end;
            agent.partial = !found.complete;
            agent.path_generation = generation;
            agent.repath_timer = 0.0;
            agent.state = AgentState::Moving;
            agent.boundary.valid = false;
        }
    }

    fn find_neighbours(&mut self) {
        // A hash grid with cells as large as the widest query range.
        let mut cell = 1.0f32;
        for agent in self.agents.iter().flatten() {
            cell = cell.max(agent.params.collision_query_range);
        }
        let key = |p: Vec3| ((p.x / cell).floor() as i32, (p.z / cell).floor() as i32);
        let mut grid: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
        for (id, agent) in self.agents.iter().enumerate() {
            if let Some(agent) = agent {
                grid.entry(key(agent.pos)).or_default().push(id);
            }
        }
        for id in 0..self.agents.len() {
            let Some(agent) = self.agents[id].as_ref() else {
                continue;
            };
            let (cx, cz) = key(agent.pos);
            let range = agent.params.collision_query_range;
            let mut found: Vec<(f32, usize)> = Vec::new();
            for dz in -1..=1 {
                for dx in -1..=1 {
                    let Some(list) = grid.get(&(cx + dx, cz + dz)) else {
                        continue;
                    };
                    for &other in list {
                        if other == id {
                            continue;
                        }
                        let Some(o) = self.agents[other].as_ref() else {
                            continue;
                        };
                        // Agents on another floor (a bridge overhead) are not neighbours.
                        if (o.pos.y - agent.pos.y).abs()
                            > (agent.params.height + o.params.height) * 0.5
                        {
                            continue;
                        }
                        let d = dist_xz(agent.pos, o.pos);
                        if d < range {
                            found.push((d, other));
                        }
                    }
                }
            }
            found.sort_by(|a, b| a.0.total_cmp(&b.0));
            found.truncate(MAX_NEIGHBOURS);
            if let Some(agent) = self.agents[id].as_mut() {
                agent.neighbours = found.into_iter().map(|(_, other)| other).collect();
            }
        }
    }

    /// Corners, shortcuts, walls, off-mesh triggers and the desired velocity.
    fn prepare_agent(&mut self, mesh: &NavMesh, id: usize, events: &mut Vec<CrowdEvent>) {
        let separation_disp = self.separation(id);
        let Some(agent) = self.agents.get_mut(id).and_then(Option::as_mut) else {
            return;
        };
        if agent.state == AgentState::OffMesh || agent.state == AgentState::Lost {
            agent.desired_vel = Vec3::ZERO;
            return;
        }
        let first = agent.corridor.first_poly();
        // Driven by a velocity: the corridor is only the polygon it stands on (moving on
        // the mesh grows it), and it goes the way it is sent.
        if let Some(velocity) = agent.steer {
            let pos = agent.corridor.pos;
            agent.corridor.reset(first, pos);
            agent.desired_vel = velocity;
            return;
        }
        let filter = agent.params.filter.clone();

        // Walls nearby.
        let range = agent.params.collision_query_range;
        if !agent.boundary.valid
            || agent.boundary.first_poly != first
            || dist_xz(agent.boundary.center, agent.pos) > range * 0.25
        {
            update_boundary(mesh, agent, &filter);
        }

        if agent.state == AgentState::Moving {
            agent.corners = agent.corridor.find_corners(mesh, MAX_CORNERS);
            if let Some(next) = agent
                .corners
                .get(1.min(agent.corners.len().saturating_sub(1)))
            {
                let next = next.pos;
                let range = agent.params.path_optimization_range;
                agent
                    .corridor
                    .optimize_path_visibility(mesh, next, range, &filter);
                agent.corners = agent.corridor.find_corners(mesh, MAX_CORNERS);
            }
            // Off-mesh link ahead and close: start across it.
            if let Some(last) = agent.corners.last().copied() {
                let trigger = agent.params.radius * 2.25;
                if last.flags & PathCorner::OFF_MESH != 0
                    && agent.corners.len() <= 2
                    && dist_xz(last.pos, agent.pos) < trigger
                {
                    if let Some(traversal) = start_off_mesh(mesh, agent, last.poly) {
                        events.push(CrowdEvent::OffMeshStart(
                            id,
                            traversal.0,
                            traversal.1,
                            traversal.2,
                        ));
                        return;
                    }
                }
            }
            // Arrived?
            let end = agent
                .corners
                .last()
                .filter(|c| c.flags & PathCorner::END != 0)
                .map(|c| c.pos);
            let at_end = agent.corners.is_empty()
                || end.is_some_and(|e| {
                    let d = dist_xz(e, agent.pos);
                    // Someone standing on the spot: this is as near as it gets.
                    d <= agent.params.stopping_distance.max(0.05)
                        || (d <= agent.params.radius * 2.0
                            && agent.stuck_time > BLOCKED_ARRIVAL_TIME)
                });
            if at_end && !agent.partial {
                agent.state = AgentState::Arrived;
                agent.corners.clear();
                events.push(CrowdEvent::Arrived(id));
            } else if at_end && agent.partial {
                // As near as it can get for now; keep retrying the rest.
                agent.desired_vel = Vec3::ZERO;
                return;
            }
        }

        agent.desired_vel = if agent.state == AgentState::Moving && !agent.corners.is_empty() {
            let dir = smooth_steer_direction(agent);
            let mut speed = agent.params.max_speed;
            let mut arriving = false;
            if let Some(end) = agent
                .corners
                .last()
                .filter(|c| c.flags & PathCorner::END != 0)
            {
                let slow_down = agent.params.radius * 2.0;
                let d = dist_xz(end.pos, agent.pos);
                speed *= (d / slow_down).min(1.0);
                arriving = d < slow_down;
            }
            let mut dvel = dir * speed;
            // Not on the last steps: next to someone standing by its spot, the push
            // away would cancel the pull in and leave it short, not even stuck.
            if separation_disp != Vec3::ZERO && !arriving {
                dvel += separation_disp;
                dvel = dvel.clamp_length_max(agent.params.max_speed);
            }
            dvel
        } else {
            Vec3::ZERO
        };
    }

    /// A push away from neighbours closer than they should be.
    fn separation(&self, id: usize) -> Vec3 {
        let Some(agent) = self.agent(id) else {
            return Vec3::ZERO;
        };
        if agent.params.separation <= 0.0 || agent.state != AgentState::Moving {
            return Vec3::ZERO;
        }
        let mut disp = Vec3::ZERO;
        let mut count = 0.0;
        for &other in &agent.neighbours {
            let Some(o) = self.agent(other) else {
                continue;
            };
            let mut diff = agent.pos - o.pos;
            diff.y = 0.0;
            let separation = (agent.params.radius + o.params.radius) * 1.6;
            let dist = diff.length();
            if dist > separation || dist < 1e-4 {
                continue;
            }
            let weight = agent.params.separation * (1.0 - (dist / separation).powi(2));
            disp += diff * (weight / dist);
            count += 1.0;
        }
        if count > 0.0 {
            disp * (agent.params.max_speed * 0.5 / count)
        } else {
            Vec3::ZERO
        }
    }

    fn avoid(&self, id: usize, dt: f32, obstacles: &[MovingObstacle]) -> Vec3 {
        let Some(agent) = self.agent(id) else {
            return Vec3::ZERO;
        };
        if agent.state == AgentState::OffMesh || agent.state == AgentState::Lost {
            return agent.vel;
        }
        // A steered agent goes where it is sent; the others make way.
        if !agent.params.avoidance || agent.steer.is_some() {
            return agent.desired_vel;
        }
        let moving = agent.state == AgentState::Moving;
        let heading = xz(agent.desired_vel);
        let mut oncoming = false;
        let mut neighbours: Vec<Neighbour> = Vec::with_capacity(agent.neighbours.len() + 4);
        for &other in &agent.neighbours {
            let Some(o) = self.agent(other) else {
                continue;
            };
            oncoming |=
                xz(o.pos - agent.pos).dot(heading) > 0.0 && xz(o.desired_vel).dot(heading) < 0.0;
            let other_moving = o.state == AgentState::Moving || o.state == AgentState::OffMesh;
            // Standing agents leave most of the avoiding to those walking; priority
            // shifts the share between walkers. Everyone makes way for a steered agent.
            let responsibility = match (moving, other_moving) {
                _ if o.steer.is_some() => 1.0,
                (true, false) => 0.85,
                (false, true) => 0.15,
                _ => (0.5 + (o.params.priority - agent.params.priority) * 0.5).clamp(0.1, 0.9),
            };
            neighbours.push(Neighbour {
                position: xz(o.pos),
                velocity: xz(o.vel),
                radius: o.params.radius,
                responsibility,
            });
        }
        for obstacle in obstacles {
            if (obstacle.position.y - agent.pos.y).abs() > obstacle.height.max(agent.params.height)
            {
                continue;
            }
            let d = dist_xz(obstacle.position, agent.pos) - obstacle.radius;
            if d > agent.params.collision_query_range {
                continue;
            }
            oncoming |= xz(obstacle.position - agent.pos).dot(heading) > 0.0
                && xz(obstacle.velocity).dot(heading) < 0.0;
            neighbours.push(Neighbour {
                position: xz(obstacle.position),
                velocity: xz(obstacle.velocity),
                radius: obstacle.radius,
                responsibility: 1.0,
            });
        }
        if neighbours.is_empty() && agent.boundary.walls.is_empty() {
            return agent.desired_vel;
        }
        // Meeting someone, everyone keeps to the same side, as people do on a path:
        // head-on meetings are symmetric and ORCA would only ever slow down for them,
        // and where crowds cross on a narrow bridge they form lanes instead of a jam.
        let mut preferred = heading;
        if oncoming && preferred.length_squared() > 1e-6 {
            preferred = if agent.give_way > 0.0 {
                (heading.perp() - heading * 0.5) * 0.5
            } else {
                heading + heading.perp() * KEEP_SIDE
            };
        }
        let v = compute_velocity(&AvoidanceInput {
            position: xz(agent.pos),
            velocity: xz(agent.vel),
            preferred_velocity: preferred,
            radius: agent.params.radius,
            max_speed: agent.params.max_speed,
            time_horizon: 2.0,
            time_horizon_obstacles: 0.6,
            time_step: dt.max(1.0 / 240.0),
            neighbours: &neighbours,
            walls: &agent.boundary.walls,
            wall_radius: WALL_CLEARANCE,
        });
        if !v.is_finite() {
            return agent.desired_vel;
        }
        Vec3::new(v.x, 0.0, v.y)
    }

    /// Push overlapping agents apart (moving obstacles push agents out entirely).
    fn resolve_collisions(&mut self, obstacles: &[MovingObstacle]) {
        const ITERATIONS: usize = 4;
        const RESOLVE_FACTOR: f32 = 0.7;
        for iteration in 0..ITERATIONS {
            for id in 0..self.agents.len() {
                let Some(agent) = self.agents[id].as_ref() else {
                    continue;
                };
                if agent.state == AgentState::OffMesh || agent.params.external_motion {
                    continue;
                }
                let mut disp = Vec3::ZERO;
                let mut weight = 0.0;
                for &other in &agent.neighbours {
                    let Some(o) = self.agents[other].as_ref() else {
                        continue;
                    };
                    let mut diff = agent.pos - o.pos;
                    diff.y = 0.0;
                    let dist_sq = diff.length_squared();
                    let combined = agent.params.radius + o.params.radius;
                    if dist_sq > combined * combined {
                        continue;
                    }
                    let dist = dist_sq.sqrt();
                    let mut pen = combined - dist;
                    if dist < 0.0001 {
                        // On top of each other: pick a side by id.
                        diff = if id > other {
                            Vec3::new(-agent.desired_vel.z, 0.0, agent.desired_vel.x)
                        } else {
                            Vec3::new(agent.desired_vel.z, 0.0, -agent.desired_vel.x)
                        };
                        if diff.length_squared() < 1e-8 {
                            diff = Vec3::new(((id * 7 + iteration) % 5) as f32 - 2.0, 0.0, 1.0);
                        }
                        diff = diff.normalize();
                        pen = 0.01;
                    } else {
                        pen = (1.0 / dist) * (pen * 0.5) * RESOLVE_FACTOR;
                    }
                    disp += diff * pen;
                    weight += 1.0;
                }
                for obstacle in obstacles {
                    if (obstacle.position.y - agent.pos.y).abs()
                        > obstacle.height.max(agent.params.height)
                    {
                        continue;
                    }
                    let mut diff = agent.pos - obstacle.position;
                    diff.y = 0.0;
                    let combined = agent.params.radius + obstacle.radius;
                    let dist = diff.length();
                    if dist >= combined || dist < 1e-4 {
                        continue;
                    }
                    disp += diff * ((combined - dist) / dist);
                    weight += 1.0;
                }
                if weight > 0.0 {
                    disp /= weight;
                }
                if let Some(agent) = self.agents[id].as_mut() {
                    agent.displacement = disp;
                }
            }
            for agent in self.agents.iter_mut().flatten() {
                agent.pos += agent.displacement;
                agent.displacement = Vec3::ZERO;
            }
        }
    }

    fn move_on_mesh(&mut self, mesh: &NavMesh, id: usize, dt: f32) {
        let Some(agent) = self.agents.get_mut(id).and_then(Option::as_mut) else {
            return;
        };
        if agent.state == AgentState::OffMesh
            || agent.state == AgentState::Lost
            || agent.params.external_motion
        {
            return;
        }
        let before = agent.corridor.pos;
        let filter = agent.params.filter.clone();
        if agent.corridor.move_position(mesh, agent.pos, &filter) {
            agent.pos = agent.corridor.pos;
        } else {
            agent.pos = before;
        }
        // Walls or others stopped it: what it really moved is its velocity.
        if dt > 0.0 {
            let actual = (agent.pos - before) / dt;
            let actual = Vec3::new(actual.x, 0.0, actual.z);
            if actual.length_squared() < agent.vel.length_squared() {
                agent.vel = actual;
            }
        }
    }

    fn update_traversals(&mut self, mesh: &NavMesh, dt: f32, events: &mut Vec<CrowdEvent>) {
        for id in 0..self.agents.len() {
            let Some(agent) = self.agents[id].as_mut() else {
                continue;
            };
            let Some(traversal) = agent.traversal.as_mut() else {
                continue;
            };
            traversal.t += dt;
            let lead = traversal.duration * 0.15;
            if traversal.t < lead {
                let u = traversal.t / lead;
                agent.pos = traversal.from.lerp(traversal.start, u);
            } else if traversal.t < traversal.duration {
                let u = (traversal.t - lead) / (traversal.duration - lead);
                let mut p = traversal.start.lerp(traversal.end, u);
                p.y += traversal.arc * 4.0 * u * (1.0 - u);
                agent.pos = p;
            } else {
                agent.pos = traversal.end;
                agent.traversal = None;
                agent.vel = Vec3::ZERO;
                agent.state = if agent.target.is_some() || agent.steer.is_some() {
                    AgentState::Moving
                } else {
                    AgentState::Idle
                };
                agent.boundary.valid = false;
                if let Some(h) = mesh.poly_height(agent.corridor.first_poly(), agent.pos) {
                    agent.pos.y = h;
                }
                events.push(CrowdEvent::OffMeshEnd(id));
                continue;
            }
            let span = (traversal.end - traversal.start).length().max(1e-3);
            let dir = (traversal.end - traversal.start) / span;
            agent.vel = Vec3::new(dir.x, 0.0, dir.z) * (span / traversal.duration.max(1e-3));
        }
    }
}

fn update_boundary(mesh: &NavMesh, agent: &mut CrowdAgent, filter: &QueryFilter) {
    let range = agent.params.collision_query_range;
    let first = agent.corridor.first_poly();
    let polys = find_local_neighbourhood(mesh, first, agent.pos, range, filter, 32);
    let mut walls: Vec<(f32, Vec2, Vec2)> = Vec::new();
    let center = xz(agent.pos);
    for r in polys {
        let verts = mesh.poly_vertices(r);
        let inside =
            verts.iter().fold(Vec2::ZERO, |sum, v| sum + xz(*v)) / verts.len().max(1) as f32;
        for (a, b) in poly_wall_segments(mesh, r, filter) {
            let (mut a2, mut b2) = (xz(a), xz(b));
            // The walkable side on the right, as avoidance takes walls.
            if (a2 - inside).perp_dot(b2 - a2) > 0.0 {
                std::mem::swap(&mut a2, &mut b2);
            }
            let ab = b2 - a2;
            let t = if ab.length_squared() > 0.0 {
                ((center - a2).dot(ab) / ab.length_squared()).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let d = center.distance(a2 + ab * t);
            if d <= range {
                walls.push((d, a2, b2));
            }
        }
    }
    walls.sort_by(|a, b| a.0.total_cmp(&b.0));
    walls.truncate(MAX_WALLS);
    agent.boundary = LocalBoundary {
        center: agent.pos,
        first_poly: first,
        walls: walls.into_iter().map(|(_, a, b)| (a, b)).collect(),
        valid: true,
    };
}

/// Steer toward the next corner, swinging wide ahead of a turn.
fn smooth_steer_direction(agent: &CrowdAgent) -> Vec3 {
    let p0 = agent.corners[0].pos;
    let p1 = agent.corners[1.min(agent.corners.len() - 1)].pos;
    let mut dir0 = p0 - agent.pos;
    let mut dir1 = p1 - agent.pos;
    dir0.y = 0.0;
    dir1.y = 0.0;
    let len0 = dir0.length();
    let len1 = dir1.length();
    if len1 > 0.001 {
        dir1 /= len1;
    }
    let dir = Vec3::new(
        dir0.x - dir1.x * len0 * 0.5,
        0.0,
        dir0.z - dir1.z * len0 * 0.5,
    );
    let dir = dir.normalize_or_zero();
    if dir == Vec3::ZERO {
        dir0.normalize_or_zero()
    } else {
        dir
    }
}

/// Begin crossing the off-mesh link `link` (the next corner's polygon): the corridor
/// moves past it and the agent animates from start to end.
fn start_off_mesh(
    mesh: &NavMesh,
    agent: &mut CrowdAgent,
    link: PolyRef,
) -> Option<(Vec3, Vec3, u64)> {
    let index = agent.corridor.path.iter().position(|&r| r == link)?;
    let connection = mesh.off_mesh_connection(link)?;
    let (_, poly) = mesh.tile_and_poly(link)?;
    if poly.kind != PolyKind::OffMesh {
        return None;
    }
    let landing = *agent.corridor.path.get(index + 1)?;
    // Which way round: the end nearer the landing polygon is where it lands.
    let (start, end) = {
        let land = mesh
            .closest_point_on_poly(landing, connection.end)
            .map(|c| c.0.distance(connection.end));
        let back = mesh
            .closest_point_on_poly(landing, connection.start)
            .map(|c| c.0.distance(connection.start));
        match (land, back) {
            (Some(a), Some(b)) if b < a => (connection.end, connection.start),
            _ => (connection.start, connection.end),
        }
    };
    agent.corridor.path.drain(..=index);
    agent.corridor.pos = end;
    let drop = (end.y - start.y).abs();
    let distance = start.distance(end);
    let speed = agent.params.max_speed.max(0.5);
    agent.traversal = Some(OffMeshTraversal {
        from: agent.pos,
        start,
        end,
        t: 0.0,
        duration: (distance / speed).clamp(0.35, 3.0) + drop * 0.12,
        arc: 0.25 + 0.15 * distance.min(4.0) + if end.y > start.y { drop * 0.5 } else { 0.0 },
    });
    agent.state = AgentState::OffMesh;
    agent.corners.clear();
    Some((start, end, connection.user_id))
}

#[cfg(test)]
mod tests {
    use super::super::builder::build_navmesh;
    use super::super::components::NavMeshSurface;
    use super::super::geometry::NavGeometry;
    use super::super::recast::WALKABLE_AREA;
    use super::*;

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

    fn mesh_with(boxes: &[(Vec3, Vec3)]) -> NavMesh {
        let mut g = NavGeometry::default();
        add_box(
            &mut g,
            Vec3::new(0.0, -0.5, 0.0),
            Vec3::new(20.0, 0.0, 20.0),
        );
        for &(lo, hi) in boxes {
            add_box(&mut g, lo, hi);
        }
        let surface = NavMeshSurface {
            tile_size: 32,
            ..Default::default()
        };
        let cfg = surface.recast_config();
        g.finish(cfg.walkable_slope_angle, None, cfg.tile_world_size());
        build_navmesh(&g, &cfg, surface.agent_max_climb, false, None).mesh
    }

    fn run(crowd: &mut Crowd, mesh: &NavMesh, seconds: f32) -> Vec<CrowdEvent> {
        let mut events = Vec::new();
        let steps = (seconds * 60.0) as usize;
        for _ in 0..steps {
            events.extend(crowd.update(mesh, 1.0 / 60.0, &[]));
        }
        events
    }

    #[test]
    fn an_agent_walks_around_a_wall_and_arrives() {
        let mesh = mesh_with(&[(Vec3::new(0.0, 0.0, 9.5), Vec3::new(16.0, 3.0, 10.5))]);
        let mut crowd = Crowd::new();
        let id = crowd.add_agent(&mesh, Vec3::new(4.0, 0.0, 4.0), AgentParams::default());
        assert!(crowd.request_move_target(&mesh, id, Vec3::new(4.0, 0.0, 16.0)));
        let events = run(&mut crowd, &mesh, 30.0);
        assert!(events.contains(&CrowdEvent::Arrived(id)), "{events:?}");
        let agent = crowd.agent(id).unwrap();
        assert!(
            dist_xz(agent.pos, Vec3::new(4.0, 0.0, 16.0)) < 0.3,
            "{:?}",
            agent.pos
        );
    }

    #[test]
    fn agents_crossing_paths_do_not_overlap() {
        let mesh = mesh_with(&[]);
        let mut crowd = Crowd::new();
        let mut ids = Vec::new();
        // Two lines of agents walking through each other.
        for i in 0..6 {
            let z = 7.0 + i as f32 * 1.2;
            let a = crowd.add_agent(&mesh, Vec3::new(2.0, 0.0, z), AgentParams::default());
            crowd.request_move_target(&mesh, a, Vec3::new(18.0, 0.0, z));
            let b = crowd.add_agent(&mesh, Vec3::new(18.0, 0.0, z), AgentParams::default());
            crowd.request_move_target(&mesh, b, Vec3::new(2.0, 0.0, z));
            ids.push(a);
            ids.push(b);
        }
        let mut worst_overlap = 0.0f32;
        for _ in 0..(40 * 60) {
            crowd.update(&mesh, 1.0 / 60.0, &[]);
            let positions: Vec<Vec3> = ids.iter().map(|&i| crowd.agent(i).unwrap().pos).collect();
            for i in 0..positions.len() {
                for j in i + 1..positions.len() {
                    let overlap = 0.7 - dist_xz(positions[i], positions[j]);
                    worst_overlap = worst_overlap.max(overlap);
                }
            }
        }
        let arrived = ids
            .iter()
            .filter(|&&i| crowd.agent(i).unwrap().state == AgentState::Arrived)
            .count();
        assert!(
            arrived >= ids.len() - 1,
            "{arrived} of {} arrived",
            ids.len()
        );
        assert!(worst_overlap < 0.25, "agents overlapped by {worst_overlap}");
    }

    #[test]
    fn crowds_pass_each_other_on_a_narrow_bridge() {
        // Railings 1.84 m apart: once the navmesh keeps clear of them there is room
        // for two people side by side, if they keep to one side.
        let mesh = mesh_with(&[
            (Vec3::new(5.0, 0.0, 8.0), Vec3::new(15.0, 1.0, 9.08)),
            (Vec3::new(5.0, 0.0, 10.92), Vec3::new(15.0, 1.0, 12.0)),
        ]);
        let mut crowd = Crowd::new();
        let mut ids = Vec::new();
        for i in 0..12 {
            let z = 7.0 + (i % 6) as f32 * 1.2;
            let x = 0.8 + (i / 6) as f32 * 1.4;
            let east = crowd.add_agent(&mesh, Vec3::new(x, 0.0, z), AgentParams::default());
            crowd.request_move_target(&mesh, east, Vec3::new(18.5, 0.0, 3.0 + i as f32));
            let west = crowd.add_agent(&mesh, Vec3::new(20.0 - x, 0.0, z), AgentParams::default());
            crowd.request_move_target(&mesh, west, Vec3::new(1.5, 0.0, 3.0 + i as f32));
            ids.push(east);
            ids.push(west);
        }
        // About 12 s each on their own; jammed for good without keeping to a side
        // and giving way.
        run(&mut crowd, &mesh, 50.0);
        let arrived = ids
            .iter()
            .filter(|&&i| crowd.agent(i).unwrap().state == AgentState::Arrived)
            .count();
        assert_eq!(arrived, ids.len(), "{arrived} of {} crossed", ids.len());
    }

    #[test]
    fn agents_jump_down_an_off_mesh_link() {
        // A platform 2 m up (too high to step down) with a link down to the ground.
        let mut mesh = mesh_with(&[(Vec3::new(0.0, 0.0, 0.0), Vec3::new(8.0, 2.0, 20.0))]);
        let start = Vec3::new(7.3, 2.0, 10.0);
        let (tx, tz) = mesh.tile_at(start.x, start.z);
        mesh.set_off_mesh_links(
            tx,
            tz,
            vec![super::super::navmesh::OffMeshConnection {
                start,
                end: Vec3::new(9.6, 0.0, 10.0),
                radius: 0.5,
                bidirectional: false,
                area: WALKABLE_AREA,
                user_id: 7,
            }],
        );
        let mut crowd = Crowd::new();
        let id = crowd.add_agent(&mesh, Vec3::new(2.0, 2.0, 10.0), AgentParams::default());
        assert!(crowd.request_move_target(&mesh, id, Vec3::new(15.0, 0.0, 12.0)));
        let events = run(&mut crowd, &mesh, 25.0);
        assert!(
            events
                .iter()
                .any(|e| matches!(e, CrowdEvent::OffMeshStart(a, _, _, 7) if *a == id)),
            "{events:?}"
        );
        assert!(events.contains(&CrowdEvent::OffMeshEnd(id)));
        assert!(events.contains(&CrowdEvent::Arrived(id)), "{events:?}");
        assert!(crowd.agent(id).unwrap().pos.y.abs() < 0.3);
    }

    #[test]
    fn a_steered_agent_goes_where_it_is_sent_and_keeps_to_the_mesh() {
        let mesh = mesh_with(&[(Vec3::new(0.0, 0.0, 9.5), Vec3::new(16.0, 3.0, 10.5))]);
        let mut crowd = Crowd::new();
        let id = crowd.add_agent(&mesh, Vec3::new(4.0, 0.0, 4.0), AgentParams::default());
        let steer = |crowd: &mut Crowd, velocity: Vec3, seconds: f32| {
            for _ in 0..(seconds * 60.0) as usize {
                assert!(crowd.request_move_velocity(id, velocity));
                crowd.update(&mesh, 1.0 / 60.0, &[]);
            }
        };
        // Faster than its max_speed, which only walks to targets.
        steer(&mut crowd, Vec3::new(0.0, 0.0, 3.0), 1.0);
        let agent = crowd.agent(id).unwrap();
        assert_eq!(agent.state, AgentState::Moving);
        assert!((agent.vel.z - 3.0).abs() < 0.05, "{:?}", agent.vel);
        // On into the wall: it stops at the mesh's edge.
        steer(&mut crowd, Vec3::new(0.0, 0.0, 3.0), 3.0);
        let pos = crowd.agent(id).unwrap().pos;
        assert!(pos.z > 8.5 && pos.z < 9.5, "{pos:?}");
        assert!((pos.x - 4.0).abs() < 0.05, "{pos:?}");
        // Along the wall and off the end of the floor.
        steer(&mut crowd, Vec3::new(-3.0, 0.0, 0.0), 4.0);
        let agent = crowd.agent(id).unwrap();
        assert!(agent.pos.x > 0.0 && agent.pos.x < 0.8, "{:?}", agent.pos);
        assert!(agent.corridor.path.len() <= 2, "{:?}", agent.corridor.path);
        // A target ends the steering.
        assert!(crowd.request_move_target(&mesh, id, Vec3::new(4.0, 0.0, 4.0)));
        let events = run(&mut crowd, &mesh, 10.0);
        assert!(events.contains(&CrowdEvent::Arrived(id)), "{events:?}");
        assert!(crowd.agent(id).unwrap().steer.is_none());
    }

    #[test]
    fn everyone_makes_way_for_a_steered_agent() {
        let mesh = mesh_with(&[]);
        let mut crowd = Crowd::new();
        let player = crowd.add_agent(&mesh, Vec3::new(3.0, 0.0, 10.0), AgentParams::default());
        // Someone standing in the way, and someone walking straight at it.
        let standing = crowd.add_agent(&mesh, Vec3::new(9.0, 0.0, 10.0), AgentParams::default());
        let walker = crowd.add_agent(&mesh, Vec3::new(17.0, 0.0, 10.0), AgentParams::default());
        crowd.request_move_target(&mesh, walker, Vec3::new(2.0, 0.0, 10.0));
        let mut closest = f32::MAX;
        let mut drift = 0.0f32;
        for _ in 0..(5 * 60) {
            crowd.request_move_velocity(player, Vec3::new(2.5, 0.0, 0.0));
            crowd.update(&mesh, 1.0 / 60.0, &[]);
            let p = crowd.agent(player).unwrap().pos;
            drift = drift.max((p.z - 10.0).abs());
            for other in [standing, walker] {
                closest = closest.min(dist_xz(p, crowd.agent(other).unwrap().pos));
            }
        }
        assert!(crowd.agent(player).unwrap().pos.x > 14.0);
        assert!(drift < 0.1, "the steered agent was pushed {drift} m aside");
        assert!(closest > 0.6, "came within {closest}");
    }

    #[test]
    fn a_moving_obstacle_is_avoided() {
        let mesh = mesh_with(&[]);
        let mut crowd = Crowd::new();
        let id = crowd.add_agent(&mesh, Vec3::new(2.0, 0.0, 10.0), AgentParams::default());
        crowd.request_move_target(&mesh, id, Vec3::new(18.0, 0.0, 10.0));
        // A cart rolling toward the agent along the same line.
        let mut cart = MovingObstacle {
            position: Vec3::new(16.0, 0.0, 10.0),
            velocity: Vec3::new(-1.0, 0.0, 0.0),
            radius: 1.0,
            height: 1.5,
        };
        let mut closest = f32::MAX;
        for _ in 0..(20 * 60) {
            cart.position += cart.velocity / 60.0;
            crowd.update(&mesh, 1.0 / 60.0, &[cart]);
            closest = closest.min(dist_xz(crowd.agent(id).unwrap().pos, cart.position));
        }
        assert!(closest > 1.0 + 0.35 - 0.1, "came within {closest}");
        assert_eq!(crowd.agent(id).unwrap().state, AgentState::Arrived);
    }
}
