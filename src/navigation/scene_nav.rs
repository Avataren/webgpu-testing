//! The navigation of one scene: its navmeshes (loaded from bakes or built), the crowd
//! walking each, and the obstacles carving them. Derived state like the physics: it
//! is rebuilt from the components, steps in play mode after scripts and before the
//! physics, and nothing in it is saved except bakes.

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use glam::{Quat, Vec2, Vec3};
use hecs::{Entity, World};
use serde::{Deserialize, Serialize};

use super::builder::{build_navmesh, BuildStats, NavMeshBuild, TileCache, TileGrid};
use super::components::{
    NavAgent, NavAgentDrive, NavLink, NavMeshSurface, NavObstacle, NavObstacleShape,
};
use super::crowd::{AgentParams, AgentState, Crowd, CrowdEvent, MovingObstacle};
use super::geometry::{collect_geometry, NavGeometry};
use super::navmesh::{NavMesh, OffMeshConnection};
use super::query::{self, QueryFilter};
use super::recast::MarkShape;
use super::serialize::{self, NavMeshFileInfo};
use crate::asset::Assets;
use crate::physics::{CharacterController, PhysicsCommand};
use crate::renderer::debug_draw::DebugGeometry;
use crate::scene::components::{Name, Parent, TransformComponent};
use crate::scene::Transform;

/// Where bakes are saved, relative to the project.
pub const BAKE_DIR: &str = "content/navigation";
/// Tiles rebuilt per frame at most when obstacles carve (the rest wait).
const TILE_REBUILDS_PER_FRAME: usize = 6;

/// A change scripts (or tools) ask of the navigation, applied at its next step.
#[derive(Debug, Clone, PartialEq)]
pub enum NavCommand {
    SetDestination {
        entity: Entity,
        target: Vec3,
    },
    Stop {
        entity: Entity,
    },
    Warp {
        entity: Entity,
        position: Vec3,
    },
    /// Turn a standing agent to face `yaw` (radians about +Y; 0 faces +Z), at its turn
    /// speed; it keeps the facing until it walks again.
    Face {
        entity: Entity,
        yaw: f32,
    },
    SetDebugDraw(NavDebugDraw),
    /// Build the navmeshes again from the scene (all, or one surface's).
    Rebuild {
        surface: Option<Entity>,
    },
}

/// What the navigation draws over the scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NavDebugDraw {
    /// The polygons, tinted by area, with their outlines.
    pub mesh: bool,
    /// Agents' bodies, velocities and states.
    pub agents: bool,
    /// Agents' paths to their destinations.
    pub paths: bool,
    /// Obstacles' footprints (red when carving).
    pub obstacles: bool,
    /// Also show (faintly) what scenery hides.
    pub x_ray: bool,
}

impl NavDebugDraw {
    pub fn all() -> Self {
        Self {
            mesh: true,
            agents: true,
            paths: true,
            obstacles: true,
            x_ray: false,
        }
    }

    pub fn any(&self) -> bool {
        self.mesh || self.agents || self.paths || self.obstacles
    }
}

/// A script callback: `callback(self, data)` on `entity`'s script.
#[derive(Debug, Clone, PartialEq)]
pub struct NavEvent {
    pub entity: Entity,
    pub callback: &'static str,
    pub data: serde_json::Value,
}

/// An agent as scripts see it.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentStatus {
    pub state: AgentState,
    pub position: Vec3,
    pub velocity: Vec3,
    pub desired_velocity: Vec3,
    pub target: Option<Vec3>,
    pub remaining_distance: Option<f32>,
    /// Its path stops short of the target (blocked for now).
    pub partial: bool,
    pub corners: Vec<Vec3>,
}

/// A navmesh as the inspector and tools see it.
#[derive(Debug, Clone, PartialEq)]
pub struct SurfaceStatus {
    pub entity: Entity,
    pub name: String,
    pub tiles: usize,
    pub polys: usize,
    /// "baked" (from `baked`), "built" (at play start or by a script) or "none".
    pub source: &'static str,
    pub baked: Option<String>,
    /// The bake was made with other settings than the component has now.
    pub stale: bool,
    pub error: Option<String>,
    pub agents: usize,
    pub build_millis: Option<f64>,
}

/// A bake running in the background.
#[derive(Debug, Clone, PartialEq)]
pub struct BakeProgress {
    pub surface: Entity,
    pub tiles_done: usize,
    pub tiles_total: usize,
    pub seconds: f32,
}

/// How a bake ended.
#[derive(Debug, Clone, PartialEq)]
pub struct BakeOutcome {
    pub serial: u32,
    pub surface: Entity,
    /// The project path it was saved to (or "" when no project is open), or the error.
    pub result: Result<String, String>,
    pub stats: Option<BuildStats>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MeshSource {
    None,
    Baked,
    Built,
}

struct SurfaceRuntime {
    entity: Entity,
    settings: NavMeshSurface,
    base: Option<Arc<NavMesh>>,
    /// A copy of `base` with carved tiles, once something carves.
    working: Option<NavMesh>,
    grid: Option<TileGrid>,
    geometry: Option<Arc<NavGeometry>>,
    cache: Option<TileCache>,
    source: MeshSource,
    loaded_path: Option<String>,
    file_settings_hash: Option<u64>,
    error: Option<String>,
    build_attempted: bool,
    crowd: Crowd,
    stats: Option<BuildStats>,
    /// Bumped whenever the mesh changes (debug geometry is cached by it).
    generation: u64,
    dirty_tiles: BTreeSet<(i32, i32)>,
    /// The off-mesh links last put into the mesh, and the tiles they start in.
    applied_links: u64,
    link_tiles: Vec<(i32, i32)>,
}

impl SurfaceRuntime {
    fn new(entity: Entity, settings: NavMeshSurface) -> Self {
        Self {
            entity,
            settings,
            base: None,
            working: None,
            grid: None,
            geometry: None,
            cache: None,
            source: MeshSource::None,
            loaded_path: None,
            file_settings_hash: None,
            error: None,
            build_attempted: false,
            crowd: Crowd::new(),
            stats: None,
            generation: 0,
            dirty_tiles: BTreeSet::new(),
            applied_links: 0,
            link_tiles: Vec::new(),
        }
    }

    fn mesh(&self) -> Option<&NavMesh> {
        self.working.as_ref().or(self.base.as_deref())
    }

    fn install(&mut self, mesh: Arc<NavMesh>, grid: TileGrid, source: MeshSource) {
        self.base = Some(mesh);
        self.working = None;
        self.grid = Some(grid);
        self.cache = None;
        self.source = source;
        self.generation += 1;
        self.crowd = Crowd::new();
        self.dirty_tiles.clear();
        self.applied_links = 0;
        self.link_tiles.clear();
    }

    /// Put `links` into the mesh (replacing the ones put in before) when they changed.
    fn apply_links(&mut self, links: &[OffMeshConnection], signature: u64) {
        if signature == self.applied_links {
            return;
        }
        self.applied_links = signature;
        if links.is_empty() && self.link_tiles.is_empty() {
            return;
        }
        let Some(base) = self.base.clone() else {
            return;
        };
        let mesh = self.working.get_or_insert_with(|| (*base).clone());
        let mut by_tile: HashMap<(i32, i32), Vec<OffMeshConnection>> = HashMap::new();
        for link in links {
            by_tile
                .entry(mesh.tile_at(link.start.x, link.start.z))
                .or_default()
                .push(link.clone());
        }
        let mut tiles: BTreeSet<(i32, i32)> = self.link_tiles.iter().copied().collect();
        tiles.extend(by_tile.keys().copied());
        for (x, z) in tiles {
            mesh.set_off_mesh_links(x, z, by_tile.remove(&(x, z)).unwrap_or_default());
        }
        self.link_tiles = links
            .iter()
            .map(|l| mesh.tile_at(l.start.x, l.start.z))
            .collect();
        self.generation += 1;
        self.crowd.mesh_generation += 1;
    }
}

struct AgentRuntime {
    surface: Entity,
    id: usize,
    component: NavAgent,
    character: bool,
    yaw: f32,
    /// A facing to turn to while standing (`NavCommand::Face`).
    face: Option<f32>,
    jumped: bool,
    landed: bool,
}

struct ObstacleRuntime {
    /// Where it was when it last counted as moved.
    settled: (Vec3, Quat),
    last_position: Vec3,
    velocity: Vec3,
    still: f32,
    carved: Option<MarkShape>,
}

struct BakeJob {
    surface: Entity,
    started: instant::Instant,
    progress: Arc<AtomicUsize>,
    total: usize,
    geometry: Arc<NavGeometry>,
    settings: NavMeshSurface,
    #[cfg(not(target_arch = "wasm32"))]
    handle: Option<std::thread::JoinHandle<NavMeshBuild>>,
    #[cfg(target_arch = "wasm32")]
    result: Option<NavMeshBuild>,
}

/// What is drawn and the path test: viewing choices, not scene data, so they outlive
/// the scene copies the editor makes for play, undo and stop.
type ViewState = (NavDebugDraw, Option<(Vec3, Vec3)>);

fn view_state() -> &'static Mutex<ViewState> {
    static VIEW: OnceLock<Mutex<ViewState>> = OnceLock::new();
    VIEW.get_or_init(|| Mutex::new((NavDebugDraw::default(), None)))
}

/// Decoded bakes, shared by every scene that loads the same file (a scene is rebuilt
/// from a snapshot after each undo and when play stops).
type LoadedBake = (u64, Arc<NavMesh>, NavMeshFileInfo);

fn loaded_bakes() -> &'static Mutex<HashMap<String, LoadedBake>> {
    static CACHE: OnceLock<Mutex<HashMap<String, LoadedBake>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// A stamp of the file's contents, so a re-bake is read again.
fn file_stamp(path: &std::path::Path) -> u64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::fs::metadata(path)
            .ok()
            .map(|m| {
                let modified = m
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_nanos() as u64);
                modified ^ m.len().rotate_left(32)
            })
            .unwrap_or(0)
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = path;
        0
    }
}

fn load_bake(path: &str) -> Result<(Arc<NavMesh>, NavMeshFileInfo), String> {
    let file = crate::project::resolve_project_path(path);
    let stamp = file_stamp(&file);
    if let Ok(cache) = loaded_bakes().lock() {
        if let Some((cached_stamp, mesh, info)) = cache.get(path) {
            if *cached_stamp == stamp {
                return Ok((mesh.clone(), info.clone()));
            }
        }
    }
    let bytes = crate::io::load_binary(&file)?;
    let (mesh, info) = serialize::decode(&bytes).map_err(|err| format!("{path}: {err}"))?;
    let mesh = Arc::new(mesh);
    if let Ok(mut cache) = loaded_bakes().lock() {
        cache.insert(path.to_owned(), (stamp, mesh.clone(), info.clone()));
    }
    Ok((mesh, info))
}

fn world_transform(world: &World, entity: Entity) -> Transform {
    crate::physics::world_transform(world, entity)
}

/// Put `entity` at a world position and rotation (keeping its scale).
fn set_world_pose(world: &mut World, entity: Entity, position: Vec3, rotation: Quat) {
    let parent = world.get::<&Parent>(entity).ok().map(|p| p.0);
    let local = match parent {
        Some(parent) => {
            let parent_world = world_transform(world, parent);
            let inverse = parent_world.rotation.inverse();
            let scale = parent_world.scale.max(Vec3::splat(1e-6));
            (
                inverse * (position - parent_world.translation) / scale,
                inverse * rotation,
            )
        }
        None => (position, rotation),
    };
    if let Ok(mut transform) = world.get::<&mut TransformComponent>(entity) {
        transform.0.translation = local.0;
        transform.0.rotation = local.1;
    }
}

fn yaw_of(rotation: Quat) -> f32 {
    let forward = rotation * Vec3::Z;
    forward.x.atan2(forward.z)
}

fn turn_toward(current: f32, target: f32, max_step: f32) -> f32 {
    let mut delta = (target - current) % std::f32::consts::TAU;
    if delta > std::f32::consts::PI {
        delta -= std::f32::consts::TAU;
    } else if delta < -std::f32::consts::PI {
        delta += std::f32::consts::TAU;
    }
    current + delta.clamp(-max_step, max_step)
}

fn agent_params(agent: &NavAgent, character: bool) -> AgentParams {
    let radius = agent.radius.max(0.05);
    AgentParams {
        radius,
        height: agent.height.max(0.1),
        max_speed: agent.speed.max(0.0),
        max_acceleration: agent.acceleration.max(0.1),
        stopping_distance: agent.stopping_distance.max(0.0),
        avoidance: agent.avoidance,
        priority: agent.priority.clamp(0.0, 1.0),
        separation: agent.separation.max(0.0),
        collision_query_range: (radius * 12.0).max(2.5),
        path_optimization_range: (radius * 30.0).max(8.0),
        external_motion: character,
        filter: QueryFilter::with_costs(&agent.area_costs),
    }
}

/// The 2D convex hull of points (monotone chain), counter-clockwise.
fn convex_hull(mut points: Vec<Vec2>) -> Vec<Vec2> {
    points.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    points.dedup_by(|a, b| a.distance_squared(*b) < 1e-10);
    if points.len() < 3 {
        return points;
    }
    let cross = |o: Vec2, a: Vec2, b: Vec2| (a - o).perp_dot(b - o);
    let mut hull: Vec<Vec2> = Vec::with_capacity(points.len() * 2);
    for &p in &points {
        while hull.len() >= 2 && cross(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0.0 {
            hull.pop();
        }
        hull.push(p);
    }
    let lower = hull.len() + 1;
    for &p in points.iter().rev().skip(1) {
        while hull.len() >= lower && cross(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0.0 {
            hull.pop();
        }
        hull.push(p);
    }
    hull.pop();
    hull
}

/// An obstacle's footprint in the world: the carving shape, and discs covering it for
/// avoidance while it moves.
pub(crate) fn obstacle_footprint(
    world: &World,
    entity: Entity,
    obstacle: &NavObstacle,
    climb: f32,
) -> (MarkShape, Vec<(Vec3, f32, f32)>) {
    let transform = world_transform(world, entity);
    let center = transform
        .matrix()
        .transform_point3(Vec3::from(obstacle.offset));
    match &obstacle.shape {
        NavObstacleShape::Box { size } => {
            let half = Vec3::from(*size).abs() * transform.scale.abs() * 0.5;
            let rotation = transform.rotation;
            let mut corners = Vec::with_capacity(8);
            let (mut min_y, mut max_y) = (f32::MAX, f32::MIN);
            for sx in [-1.0, 1.0] {
                for sy in [-1.0, 1.0] {
                    for sz in [-1.0, 1.0] {
                        let p =
                            center + rotation * Vec3::new(half.x * sx, half.y * sy, half.z * sz);
                        corners.push(Vec2::new(p.x, p.z));
                        min_y = min_y.min(p.y);
                        max_y = max_y.max(p.y);
                    }
                }
            }
            // Discs along the box's longer side for avoidance.
            let axis_x = rotation * Vec3::X;
            let axis_z = rotation * Vec3::Z;
            let (long_axis, long, short) = if half.x >= half.z {
                (axis_x, half.x * 2.0, half.z * 2.0)
            } else {
                (axis_z, half.z * 2.0, half.x * 2.0)
            };
            let count = ((long / short.max(0.1)).ceil() as usize).clamp(1, 8);
            let segment = long / count as f32;
            let radius = 0.5 * (short * short + segment * segment).sqrt();
            let height = max_y - min_y;
            let discs = (0..count)
                .map(|i| {
                    let t = (i as f32 + 0.5) / count as f32 - 0.5;
                    let mut p = center
                        + Vec3::new(long_axis.x, 0.0, long_axis.z).normalize_or_zero() * (t * long);
                    p.y = min_y;
                    (p, radius, height)
                })
                .collect();
            (
                MarkShape::Convex {
                    points: convex_hull(corners),
                    min_y: min_y - climb,
                    max_y,
                },
                discs,
            )
        }
        NavObstacleShape::Cylinder { radius, height } => {
            let scale = transform.scale.abs();
            let radius = radius.abs() * scale.x.max(scale.z);
            let height = height.abs() * scale.y;
            let base = center - Vec3::Y * (height * 0.5);
            (
                MarkShape::Cylinder {
                    base: base - Vec3::Y * climb,
                    radius,
                    height: height + climb,
                },
                vec![(base, radius, height)],
            )
        }
    }
}

/// See the module documentation.
pub struct SceneNavigation {
    surfaces: Vec<SurfaceRuntime>,
    agents: HashMap<Entity, AgentRuntime>,
    obstacles: HashMap<Entity, ObstacleRuntime>,
    commands: Vec<NavCommand>,
    events: Vec<NavEvent>,
    bake: Option<BakeJob>,
    bake_serial: u32,
    last_bake: Option<BakeOutcome>,
    debug_cache: HashMap<Entity, (u64, DebugGeometry)>,
}

impl Default for SceneNavigation {
    fn default() -> Self {
        Self::new()
    }
}

impl SceneNavigation {
    pub fn new() -> Self {
        Self {
            surfaces: Vec::new(),
            agents: HashMap::new(),
            obstacles: HashMap::new(),
            commands: Vec::new(),
            events: Vec::new(),
            bake: None,
            bake_serial: 0,
            last_bake: None,
            debug_cache: HashMap::new(),
        }
    }

    /// Forget agents, obstacles and carving (play starts or stops); navmeshes stay.
    pub fn reset(&mut self) {
        for surface in &mut self.surfaces {
            surface.crowd = Crowd::new();
            if surface.working.take().is_some() {
                surface.generation += 1;
            }
            surface.applied_links = 0;
            surface.link_tiles.clear();
            surface.dirty_tiles.clear();
            if surface.source == MeshSource::Built {
                // Built for the last play; built again for the next.
                surface.build_attempted = false;
            }
        }
        self.agents.clear();
        self.obstacles.clear();
        self.commands.clear();
        self.events.clear();
    }

    pub fn queue(&mut self, commands: impl IntoIterator<Item = NavCommand>) {
        self.commands.extend(commands);
    }

    pub fn take_events(&mut self) -> Vec<NavEvent> {
        std::mem::take(&mut self.events)
    }

    /// What is drawn (shared by every scene: it is a viewing choice).
    pub fn debug_draw(&self) -> NavDebugDraw {
        view_state().lock().map(|v| v.0).unwrap_or_default()
    }

    pub fn set_debug_draw(&mut self, debug: NavDebugDraw) {
        if let Ok(mut view) = view_state().lock() {
            view.0 = debug;
        }
    }

    /// Show a path between two points (the editor's path test), or none.
    pub fn set_test_path(&mut self, path: Option<(Vec3, Vec3)>) {
        if let Ok(mut view) = view_state().lock() {
            view.1 = path;
        }
    }

    pub fn test_path(&self) -> Option<(Vec3, Vec3)> {
        view_state().lock().ok().and_then(|v| v.1)
    }

    pub fn surface_count(&self) -> usize {
        self.surfaces.len()
    }

    /// The navmesh agents of `agent_type` walk (the first one when none matches).
    pub fn mesh(&self, agent_type: &str) -> Option<&NavMesh> {
        self.surface_for(agent_type)
            .and_then(|i| self.surfaces[i].mesh())
    }

    fn surface_for(&self, agent_type: &str) -> Option<usize> {
        self.surfaces
            .iter()
            .position(|s| s.settings.agent_type == agent_type && s.mesh().is_some())
            .or_else(|| self.surfaces.iter().position(|s| s.mesh().is_some()))
            .or_else(|| {
                self.surfaces
                    .iter()
                    .position(|s| s.settings.agent_type == agent_type)
            })
            .or(if self.surfaces.is_empty() {
                None
            } else {
                Some(0)
            })
    }

    fn surface_index(&self, entity: Entity) -> Option<usize> {
        self.surfaces.iter().position(|s| s.entity == entity)
    }

    pub fn surface_statuses(&self, world: &World) -> Vec<SurfaceStatus> {
        self.surfaces
            .iter()
            .map(|s| SurfaceStatus {
                entity: s.entity,
                name: world
                    .get::<&Name>(s.entity)
                    .map(|n| n.0.clone())
                    .unwrap_or_else(|_| format!("{:?}", s.entity)),
                tiles: s.mesh().map_or(0, NavMesh::tile_count),
                polys: s.mesh().map_or(0, NavMesh::poly_count),
                source: match s.source {
                    MeshSource::None => "none",
                    MeshSource::Baked => "baked",
                    MeshSource::Built => "built",
                },
                baked: s.settings.baked.clone(),
                stale: s.source == MeshSource::Baked
                    && s.file_settings_hash
                        .is_some_and(|h| h != s.settings.build_settings_hash()),
                error: s.error.clone(),
                agents: s.crowd.agent_count(),
                build_millis: s.stats.as_ref().map(|st| st.millis),
            })
            .collect()
    }

    pub fn agent_count(&self) -> usize {
        self.agents.len()
    }

    pub fn obstacle_count(&self) -> (usize, usize) {
        let carving = self
            .obstacles
            .values()
            .filter(|o| o.carved.is_some())
            .count();
        (self.obstacles.len(), carving)
    }

    pub fn bake_progress(&self) -> Option<BakeProgress> {
        self.bake.as_ref().map(|job| BakeProgress {
            surface: job.surface,
            tiles_done: job.progress.load(Ordering::Relaxed),
            tiles_total: job.total,
            seconds: job.started.elapsed().as_secs_f32(),
        })
    }

    pub fn last_bake(&self) -> Option<&BakeOutcome> {
        self.last_bake.as_ref()
    }

    // ---------------------------------------------------------------- surfaces

    /// Match the surfaces to the components: load bakes, and (`playing`) build the
    /// ones without a bake that ask for it.
    fn sync_surfaces(&mut self, world: &World, assets: &Assets, playing: bool) {
        let mut wanted: Vec<(Entity, NavMeshSurface)> = world
            .query::<&NavMeshSurface>()
            .iter()
            .map(|(e, s)| (e, s.clone()))
            .collect();
        wanted.sort_by_key(|(e, _)| e.to_bits());
        self.surfaces
            .retain(|s| wanted.iter().any(|(e, _)| *e == s.entity));
        for (entity, settings) in wanted {
            let index = match self.surface_index(entity) {
                Some(i) => i,
                None => {
                    self.surfaces
                        .push(SurfaceRuntime::new(entity, settings.clone()));
                    self.surfaces.len() - 1
                }
            };
            let surface = &mut self.surfaces[index];
            surface.settings = settings.clone();
            if let Some(path) = settings.baked.as_deref() {
                if surface.loaded_path.as_deref() != Some(path) {
                    surface.loaded_path = Some(path.to_owned());
                    match load_bake(path) {
                        Ok((mesh, info)) => {
                            log::info!(
                                "Navigation: loaded {path} ({} tiles, {} polygons)",
                                mesh.tile_count(),
                                mesh.poly_count()
                            );
                            surface.file_settings_hash = Some(info.settings_hash);
                            surface.install(mesh, info.grid, MeshSource::Baked);
                            surface.error = None;
                        }
                        Err(error) => {
                            log::warn!("Navigation: cannot load {path}: {error}");
                            surface.error = Some(error);
                        }
                    }
                }
            } else if surface.source == MeshSource::Baked {
                // The bake was cleared.
                surface.base = None;
                surface.working = None;
                surface.source = MeshSource::None;
                surface.loaded_path = None;
                surface.generation += 1;
            }
            if playing
                && surface.base.is_none()
                && settings.build_on_start
                && !surface.build_attempted
            {
                surface.build_attempted = true;
                let geometry = collect_geometry(world, assets, entity, &settings);
                let cfg = settings.recast_config();
                let built = build_navmesh(&geometry, &cfg, settings.agent_max_climb, false, None);
                log::info!(
                    "Navigation: built a navmesh at play start ({} tiles, {} polygons from {} triangles in {:.0} ms); bake it in the editor to skip this",
                    built.stats.tiles,
                    built.stats.polys,
                    built.stats.triangles,
                    built.stats.millis
                );
                surface.stats = Some(built.stats.clone());
                surface.install(Arc::new(built.mesh), built.grid, MeshSource::Built);
                surface.geometry = Some(Arc::new(geometry));
                surface.cache = Some(built.cache);
            }
        }
        self.sync_links(world);
    }

    /// The scene's off-mesh links into every navmesh (when they changed).
    fn sync_links(&mut self, world: &World) {
        use std::hash::{Hash, Hasher};
        let mut links: Vec<OffMeshConnection> = world
            .query::<&NavLink>()
            .iter()
            .map(|(entity, link)| {
                let transform = world_transform(world, entity);
                OffMeshConnection {
                    start: transform.translation,
                    end: transform.matrix().transform_point3(Vec3::from(link.end)),
                    radius: link.radius.max(0.05),
                    bidirectional: link.bidirectional,
                    area: link.area.clamp(1, super::recast::WALKABLE_AREA),
                    user_id: entity.to_bits().get(),
                }
            })
            .collect();
        links.sort_by_key(|l| l.user_id);
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for link in &links {
            for v in [link.start, link.end] {
                v.x.to_bits().hash(&mut hasher);
                v.y.to_bits().hash(&mut hasher);
                v.z.to_bits().hash(&mut hasher);
            }
            link.radius.to_bits().hash(&mut hasher);
            (link.bidirectional, link.area, link.user_id).hash(&mut hasher);
        }
        // Never 0, which stands for "not applied yet".
        let signature = hasher.finish() | 1;
        for surface in &mut self.surfaces {
            surface.apply_links(&links, signature);
        }
    }

    /// Start baking a surface (or the first one) in the background. The result is
    /// installed, and saved into the project, by [`Self::poll_bake`].
    pub fn start_bake(
        &mut self,
        world: &World,
        assets: &Assets,
        surface: Option<Entity>,
    ) -> Result<Entity, String> {
        if self.bake.is_some() {
            return Err("a bake is already running".into());
        }
        let (entity, settings) = match surface {
            Some(entity) => {
                let settings = world
                    .get::<&NavMeshSurface>(entity)
                    .map(|s| (*s).clone())
                    .map_err(|_| "that entity has no NavMeshSurface")?;
                (entity, settings)
            }
            None => {
                let mut query = world.query::<&NavMeshSurface>();
                let (entity, settings) = query
                    .iter()
                    .min_by_key(|(e, _)| e.to_bits())
                    .ok_or("the scene has no NavMeshSurface (add one to an entity first)")?;
                (entity, settings.clone())
            }
        };
        let geometry = Arc::new(collect_geometry(world, assets, entity, &settings));
        if geometry.is_empty() {
            return Err(
                "no geometry to build from: no visible static meshes (or colliders) were found"
                    .into(),
            );
        }
        let cfg = settings.recast_config();
        let grid = TileGrid::new(&geometry, &cfg);
        let total = (grid.tiles_x * grid.tiles_z).max(0) as usize;
        let progress = Arc::new(AtomicUsize::new(0));
        log::info!(
            "Navigation: baking {} triangles into {} tiles",
            geometry.triangle_count(),
            total
        );
        #[cfg(not(target_arch = "wasm32"))]
        let handle = {
            let geometry = geometry.clone();
            let progress = progress.clone();
            let climb = settings.agent_max_climb;
            Some(
                std::thread::Builder::new()
                    .name("navmesh-bake".into())
                    .spawn(move || build_navmesh(&geometry, &cfg, climb, false, Some(&progress)))
                    .map_err(|err| format!("cannot start the bake: {err}"))?,
            )
        };
        self.bake = Some(BakeJob {
            surface: entity,
            started: instant::Instant::now(),
            progress: progress.clone(),
            total,
            geometry: geometry.clone(),
            settings: settings.clone(),
            #[cfg(not(target_arch = "wasm32"))]
            handle,
            #[cfg(target_arch = "wasm32")]
            result: Some(build_navmesh(
                &geometry,
                &cfg,
                settings.agent_max_climb,
                false,
                Some(&progress),
            )),
        });
        Ok(entity)
    }

    /// Finish a bake that is done: install it, save it into the project and point the
    /// surface at the file. Returns the outcome once.
    pub fn poll_bake(&mut self, world: &mut World) -> Option<BakeOutcome> {
        let job = self.bake.as_mut()?;
        #[cfg(not(target_arch = "wasm32"))]
        let built = {
            if !job.handle.as_ref().is_some_and(|h| h.is_finished()) {
                return None;
            }
            job.handle.take().map(|h| h.join())
        };
        #[cfg(target_arch = "wasm32")]
        let built = job.result.take().map(Ok::<_, ()>);
        let job = self.bake.take()?;
        self.bake_serial += 1;
        let serial = self.bake_serial;
        let outcome = match built {
            Some(Ok(built)) => {
                let stats = built.stats.clone();
                let saved = save_bake(world, job.surface, &built, &job.settings);
                if let Some(index) = self.surface_index(job.surface) {
                    let surface = &mut self.surfaces[index];
                    surface.stats = Some(stats.clone());
                    surface.file_settings_hash = Some(job.settings.build_settings_hash());
                    surface.install(Arc::new(built.mesh), built.grid, MeshSource::Baked);
                    surface.geometry = Some(job.geometry);
                    surface.error = None;
                    if let Ok(path) = &saved {
                        surface.loaded_path = if path.is_empty() {
                            None
                        } else {
                            Some(path.clone())
                        };
                    }
                } else {
                    let mut surface = SurfaceRuntime::new(job.surface, job.settings.clone());
                    surface.stats = Some(stats.clone());
                    surface.install(Arc::new(built.mesh), built.grid, MeshSource::Baked);
                    surface.geometry = Some(job.geometry);
                    if let Ok(path) = &saved {
                        surface.loaded_path = if path.is_empty() {
                            None
                        } else {
                            Some(path.clone())
                        };
                    }
                    self.surfaces.push(surface);
                }
                log::info!(
                    "Navigation: baked {} tiles, {} polygons from {} triangles in {:.0} ms",
                    stats.tiles,
                    stats.polys,
                    stats.triangles,
                    stats.millis
                );
                BakeOutcome {
                    serial,
                    surface: job.surface,
                    result: saved,
                    stats: Some(stats),
                }
            }
            _ => BakeOutcome {
                serial,
                surface: job.surface,
                result: Err("the bake thread failed".into()),
                stats: None,
            },
        };
        self.last_bake = Some(outcome.clone());
        Some(outcome)
    }

    /// Build a surface (or every one) again now, from the scene as it is.
    fn rebuild_now(&mut self, world: &World, assets: &Assets, only: Option<Entity>) {
        for surface in &mut self.surfaces {
            if only.is_some_and(|e| e != surface.entity) {
                continue;
            }
            let geometry = collect_geometry(world, assets, surface.entity, &surface.settings);
            let cfg = surface.settings.recast_config();
            let built = build_navmesh(
                &geometry,
                &cfg,
                surface.settings.agent_max_climb,
                false,
                None,
            );
            surface.stats = Some(built.stats.clone());
            let crowd = std::mem::take(&mut surface.crowd);
            surface.install(Arc::new(built.mesh), built.grid, MeshSource::Built);
            surface.crowd = crowd;
            surface.crowd.mesh_generation += 1;
            surface.geometry = Some(Arc::new(geometry));
            surface.cache = Some(built.cache);
        }
    }

    // ---------------------------------------------------------------- per frame

    /// Editor mode: keep navmeshes loaded for display and finish bakes.
    pub fn update_editor(&mut self, world: &mut World, assets: &Assets) {
        self.sync_surfaces(world, assets, false);
        self.apply_debug_commands();
        if let Some(outcome) = self.poll_bake(world) {
            self.events.push(NavEvent {
                entity: outcome.surface,
                callback: "on_nav_baked",
                data: serde_json::Value::Null,
            });
        }
    }

    fn apply_debug_commands(&mut self) {
        let mut rest = Vec::new();
        for command in std::mem::take(&mut self.commands) {
            match command {
                NavCommand::SetDebugDraw(debug) => self.set_debug_draw(debug),
                other => rest.push(other),
            }
        }
        // Agent commands only mean something while playing.
        let _ = rest;
    }

    /// Play mode: run agents and obstacles for `dt` seconds. Returns the steering for
    /// agents moved by character controllers, for this frame's physics step.
    pub fn step(&mut self, world: &mut World, assets: &Assets, dt: f32) -> Vec<PhysicsCommand> {
        self.sync_surfaces(world, assets, true);
        if let Some(outcome) = self.poll_bake(world) {
            self.events.push(NavEvent {
                entity: outcome.surface,
                callback: "on_nav_baked",
                data: serde_json::Value::Null,
            });
        }
        if self.surfaces.is_empty() {
            self.commands.clear();
            return Vec::new();
        }
        self.sync_agents(world);
        self.apply_commands(world, assets);
        let moving = self.update_obstacles(world, assets, dt);
        self.rebuild_dirty_tiles(world, assets);

        // Agents moved by character controllers report where physics put them.
        for (&entity, agent) in &self.agents {
            if !agent.character {
                continue;
            }
            let Some(index) = self.surface_index(agent.surface) else {
                continue;
            };
            let surface = &mut self.surfaces[index];
            let Some(mesh) = surface.working.as_ref().or(surface.base.as_deref()) else {
                continue;
            };
            let pos = world_transform(world, entity).translation;
            surface.crowd.set_agent_position(mesh, agent.id, pos);
        }

        let mut crowd_events: Vec<(Entity, CrowdEvent)> = Vec::new();
        for surface in &mut self.surfaces {
            let obstacles = &moving;
            let Some(mesh) = surface.working.as_ref().or(surface.base.as_deref()) else {
                continue;
            };
            for event in surface.crowd.update(mesh, dt, obstacles) {
                crowd_events.push((surface.entity, event));
            }
        }

        let physics = self.write_back(world, dt);
        self.feed_animation_trees(world);
        self.dispatch(crowd_events);
        physics
    }

    fn sync_agents(&mut self, world: &World) {
        let present: Vec<(Entity, NavAgent)> = world
            .query::<&NavAgent>()
            .iter()
            .map(|(e, a)| (e, a.clone()))
            .collect();
        let gone: Vec<Entity> = self
            .agents
            .keys()
            .filter(|e| !present.iter().any(|(p, _)| p == *e))
            .copied()
            .collect();
        for entity in gone {
            if let Some(agent) = self.agents.remove(&entity) {
                if let Some(index) = self.surface_index(agent.surface) {
                    self.surfaces[index].crowd.remove_agent(agent.id);
                }
            }
        }
        for (entity, component) in present {
            let has_controller = world
                .satisfies::<&CharacterController>(entity)
                .unwrap_or(false);
            let character = match component.drive {
                NavAgentDrive::Auto => has_controller,
                NavAgentDrive::Transform => false,
                NavAgentDrive::Character => has_controller,
            };
            if let Some(existing) = self.agents.get_mut(&entity) {
                if existing.component != component || existing.character != character {
                    if let Some(index) = self
                        .surfaces
                        .iter()
                        .position(|s| s.entity == existing.surface)
                    {
                        if let Some(agent) = self.surfaces[index].crowd.agent_mut(existing.id) {
                            agent.params = agent_params(&component, character);
                        }
                    }
                    existing.component = component;
                    existing.character = character;
                }
                continue;
            }
            let Some(index) = self.surface_for(&component.agent_type) else {
                continue;
            };
            let surface = &mut self.surfaces[index];
            let Some(mesh) = surface.working.as_ref().or(surface.base.as_deref()) else {
                continue;
            };
            let transform = world_transform(world, entity);
            let id = surface.crowd.add_agent(
                mesh,
                transform.translation,
                agent_params(&component, character),
            );
            if let Some(target) = component.destination {
                surface
                    .crowd
                    .request_move_target(mesh, id, Vec3::from(target));
            }
            self.agents.insert(
                entity,
                AgentRuntime {
                    surface: surface.entity,
                    id,
                    yaw: yaw_of(transform.rotation) - component.yaw_offset.to_radians(),
                    component,
                    character,
                    face: None,
                    jumped: false,
                    landed: false,
                },
            );
        }
    }

    fn apply_commands(&mut self, world: &World, assets: &Assets) {
        for command in std::mem::take(&mut self.commands) {
            match command {
                NavCommand::SetDebugDraw(debug) => self.set_debug_draw(debug),
                NavCommand::Rebuild { surface } => self.rebuild_now(world, assets, surface),
                NavCommand::SetDestination { entity, target } => {
                    let Some(agent) = self.agents.get(&entity) else {
                        log::warn!(
                            "Navigation: {entity:?} is not a NavAgent (or has no navmesh to walk)"
                        );
                        continue;
                    };
                    let (surface, id) = (agent.surface, agent.id);
                    let Some(index) = self.surface_index(surface) else {
                        continue;
                    };
                    let s = &mut self.surfaces[index];
                    let Some(mesh) = s.working.as_ref().or(s.base.as_deref()) else {
                        continue;
                    };
                    if !s.crowd.request_move_target(mesh, id, target) {
                        self.events.push(NavEvent {
                            entity,
                            callback: "on_nav_failed",
                            data: serde_json::json!("the destination is not on the navmesh"),
                        });
                    }
                }
                NavCommand::Stop { entity } => {
                    if let Some(agent) = self.agents.get(&entity) {
                        if let Some(index) = self.surface_index(agent.surface) {
                            self.surfaces[index].crowd.reset_move_target(agent.id);
                        }
                    }
                }
                NavCommand::Face { entity, yaw } => {
                    if let Some(agent) = self.agents.get_mut(&entity) {
                        agent.face = Some(yaw - agent.component.yaw_offset.to_radians());
                    }
                }
                NavCommand::Warp { entity, position } => {
                    if let Some(agent) = self.agents.get(&entity) {
                        let (surface, id) = (agent.surface, agent.id);
                        if let Some(index) = self.surface_index(surface) {
                            let s = &mut self.surfaces[index];
                            if let Some(mesh) = s.working.as_ref().or(s.base.as_deref()) {
                                s.crowd.warp(mesh, id, position);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Track obstacles: discs for the ones moving (agents steer around them), carving
    /// for the ones standing still. Returns the moving discs.
    fn update_obstacles(
        &mut self,
        world: &World,
        _assets: &Assets,
        dt: f32,
    ) -> Vec<MovingObstacle> {
        let present: Vec<(Entity, NavObstacle)> = world
            .query::<&NavObstacle>()
            .iter()
            .map(|(e, o)| (e, o.clone()))
            .collect();
        let climb = self
            .surfaces
            .iter()
            .map(|s| s.settings.agent_max_climb)
            .fold(0.0f32, f32::max);
        // Obstacles that went away stop carving.
        let gone: Vec<Entity> = self
            .obstacles
            .keys()
            .filter(|e| !present.iter().any(|(p, _)| p == *e))
            .copied()
            .collect();
        for entity in gone {
            if let Some(runtime) = self.obstacles.remove(&entity) {
                if let Some(shape) = runtime.carved {
                    self.mark_dirty(&shape);
                }
            }
        }
        let mut moving = Vec::new();
        for (entity, obstacle) in present {
            let transform = world_transform(world, entity);
            let pose = (transform.translation, transform.rotation);
            let runtime = self
                .obstacles
                .entry(entity)
                .or_insert_with(|| ObstacleRuntime {
                    settled: pose,
                    last_position: pose.0,
                    velocity: Vec3::ZERO,
                    still: f32::MAX,
                    carved: None,
                });
            if dt > 0.0 {
                let velocity = (pose.0 - runtime.last_position) / dt;
                runtime.velocity = runtime.velocity.lerp(velocity, 0.5);
            }
            runtime.last_position = pose.0;
            let threshold = obstacle.move_threshold.max(0.001);
            let moved = pose.0.distance(runtime.settled.0) > threshold
                || pose.1.angle_between(runtime.settled.1) > threshold;
            if moved {
                runtime.settled = pose;
                runtime.still = 0.0;
            } else if runtime.still < f32::MAX {
                runtime.still += dt;
            }
            let (shape, discs) = obstacle_footprint(world, entity, &obstacle, climb);
            let wants_carve = obstacle.carve
                && (!obstacle.carve_only_stationary || runtime.still >= obstacle.stationary_time);
            let mut dirty = Vec::new();
            if wants_carve {
                if runtime.carved.as_ref() != Some(&shape)
                    && (runtime.carved.is_none() || moved || !obstacle.carve_only_stationary)
                {
                    if let Some(old) = runtime.carved.replace(shape.clone()) {
                        dirty.push(old);
                    }
                    dirty.push(shape);
                }
            } else if let Some(old) = runtime.carved.take() {
                dirty.push(old);
            }
            let carved = runtime.carved.is_some();
            let velocity = runtime.velocity;
            for shape in dirty {
                self.mark_dirty(&shape);
            }
            // A carved obstacle is a wall in the navmesh; until its tiles are rebuilt
            // (and while moving) agents steer around its discs.
            if !carved || self.surfaces.iter().any(|s| !s.dirty_tiles.is_empty()) {
                for (position, radius, height) in discs {
                    moving.push(MovingObstacle {
                        position,
                        velocity,
                        radius,
                        height,
                    });
                }
            }
        }
        moving
    }

    fn mark_dirty(&mut self, shape: &MarkShape) {
        let (lo, hi) = shape.bounds_xz();
        for surface in &mut self.surfaces {
            let Some(grid) = surface.grid.as_ref() else {
                continue;
            };
            let pad = surface.settings.agent_radius + surface.settings.cell_size * 4.0;
            let touched: Vec<(i32, i32)> = grid
                .tiles_touching(lo - Vec2::splat(pad), hi + Vec2::splat(pad))
                .collect();
            surface.dirty_tiles.extend(touched);
        }
    }

    fn rebuild_dirty_tiles(&mut self, world: &World, assets: &Assets) {
        let carves: Vec<MarkShape> = self
            .obstacles
            .values()
            .filter_map(|o| o.carved.clone())
            .collect();
        let mut budget = TILE_REBUILDS_PER_FRAME;
        for surface in &mut self.surfaces {
            if surface.dirty_tiles.is_empty() || budget == 0 {
                continue;
            }
            let (Some(base), Some(grid)) = (surface.base.clone(), surface.grid.clone()) else {
                surface.dirty_tiles.clear();
                continue;
            };
            let cfg = surface.settings.recast_config();
            if (cfg.tile_world_size() - grid.tile_size).abs() > 1e-3 {
                log::warn!("Navigation: the bake's tiles do not match the surface's settings; bake again to let obstacles carve");
                surface.dirty_tiles.clear();
                continue;
            }
            if surface.geometry.is_none() {
                surface.geometry = Some(Arc::new(collect_geometry(
                    world,
                    assets,
                    surface.entity,
                    &surface.settings,
                )));
            }
            let geometry = surface.geometry.clone().unwrap_or_default();
            let cache = surface
                .cache
                .get_or_insert_with(|| TileCache::new(cfg, grid));
            let working = surface.working.get_or_insert_with(|| (*base).clone());
            while budget > 0 {
                let Some(&(x, z)) = surface.dirty_tiles.iter().next() else {
                    break;
                };
                surface.dirty_tiles.remove(&(x, z));
                budget -= 1;
                match cache.rebuild(&geometry, x, z, &carves) {
                    Some(tile) => {
                        working.add_tile(tile);
                    }
                    None => {
                        working.remove_tile(x, z);
                    }
                }
            }
            surface.generation += 1;
            surface.crowd.mesh_generation += 1;
        }
    }

    /// Move transform-driven agents and turn them; steering commands for the rest.
    fn write_back(&mut self, world: &mut World, dt: f32) -> Vec<PhysicsCommand> {
        let mut physics = Vec::new();
        for (&entity, runtime) in self.agents.iter_mut() {
            let Some(index) = self
                .surfaces
                .iter()
                .position(|s| s.entity == runtime.surface)
            else {
                continue;
            };
            let Some(agent) = self.surfaces[index].crowd.agent(runtime.id) else {
                continue;
            };
            let component = &runtime.component;
            let velocity = if runtime.character {
                agent.new_vel
            } else {
                agent.vel
            };
            let horizontal = Vec3::new(velocity.x, 0.0, velocity.z);
            let walking = agent.state == AgentState::Moving || agent.state == AgentState::OffMesh;
            if walking && runtime.face.is_some() && horizontal.length() > 0.3 {
                runtime.face = None;
            }
            if component.face_movement && horizontal.length() > 0.08 && runtime.face.is_none() {
                let target = horizontal.x.atan2(horizontal.z);
                runtime.yaw =
                    turn_toward(runtime.yaw, target, component.turn_speed.to_radians() * dt);
            } else if let Some(target) = runtime.face {
                runtime.yaw = turn_toward(
                    runtime.yaw,
                    target,
                    component.turn_speed.to_radians() * dt * 0.6,
                );
            }
            if runtime.character {
                let Ok(controller) = world
                    .get::<&CharacterController>(entity)
                    .map(|c| (*c).clone())
                else {
                    continue;
                };
                let speed = horizontal.length();
                let run = speed > controller.walk_speed * 1.05;
                let reference = if run {
                    controller.run_speed
                } else {
                    controller.walk_speed
                }
                .max(0.01);
                physics.push(PhysicsCommand::CharacterMove {
                    entity,
                    direction: (horizontal / reference).clamp_length_max(1.0),
                    run,
                });
                continue;
            }
            if agent.state == AgentState::Lost {
                continue;
            }
            let rotation = Quat::from_rotation_y(runtime.yaw + component.yaw_offset.to_radians());
            set_world_pose(world, entity, agent.pos, rotation);
        }
        physics
    }

    /// Agents' speeds into the `AnimationTree`s that read from their character.
    fn feed_animation_trees(&mut self, world: &mut World) {
        use crate::scene::skeletal::AnimationTree;
        if self.agents.is_empty() {
            return;
        }
        let trees: Vec<Entity> = world
            .query::<&AnimationTree>()
            .iter()
            .filter(|(_, tree)| tree.from_character)
            .map(|(e, _)| e)
            .collect();
        for entity in trees {
            let mut current = Some(entity);
            let mut owner = None;
            for _ in 0..64 {
                let Some(candidate) = current else { break };
                if self.agents.contains_key(&candidate) {
                    owner = Some(candidate);
                    break;
                }
                current = world.get::<&Parent>(candidate).ok().map(|p| p.0);
            }
            let Some(owner) = owner else {
                continue;
            };
            let runtime = self.agents.get_mut(&owner).expect("found above");
            if runtime.character {
                // The character controller feeds its own tree.
                continue;
            }
            let Some(surface) = self.surfaces.iter().find(|s| s.entity == runtime.surface) else {
                continue;
            };
            let Some(agent) = surface.crowd.agent(runtime.id) else {
                continue;
            };
            let Ok(mut tree) = world.get::<&mut AnimationTree>(entity) else {
                continue;
            };
            let yaw = runtime.yaw;
            let forward = Vec3::new(yaw.sin(), 0.0, yaw.cos());
            let right = Vec3::new(-yaw.cos(), 0.0, yaw.sin());
            let horizontal = Vec3::new(agent.vel.x, 0.0, agent.vel.z);
            let in_air = agent.state == AgentState::OffMesh;
            tree.set("speed", horizontal.length());
            tree.set("forward_speed", horizontal.dot(forward));
            tree.set("side_speed", horizontal.dot(right));
            tree.set("vertical_speed", 0.0);
            tree.set("grounded", if in_air { 0.0 } else { 1.0 });
            tree.set("air_time", 0.0);
            if std::mem::take(&mut runtime.jumped) {
                tree.trigger("jumped");
            }
            if std::mem::take(&mut runtime.landed) {
                tree.trigger("landed");
            }
        }
    }

    fn dispatch(&mut self, crowd_events: Vec<(Entity, CrowdEvent)>) {
        let by_id: HashMap<(Entity, usize), Entity> = self
            .agents
            .iter()
            .map(|(e, a)| ((a.surface, a.id), *e))
            .collect();
        for (surface, event) in crowd_events {
            let (id, callback, data) = match event {
                CrowdEvent::Arrived(id) => (id, "on_nav_arrived", serde_json::Value::Null),
                CrowdEvent::Failed(id) => (
                    id,
                    "on_nav_failed",
                    serde_json::json!("no path to the destination"),
                ),
                CrowdEvent::OffMeshStart(id, start, end, user) => (
                    id,
                    "on_nav_link_start",
                    serde_json::json!({
                        "start": {"x": start.x, "y": start.y, "z": start.z},
                        "finish": {"x": end.x, "y": end.y, "z": end.z},
                        "link": user,
                    }),
                ),
                CrowdEvent::OffMeshEnd(id) => (id, "on_nav_link_end", serde_json::Value::Null),
            };
            let Some(&entity) = by_id.get(&(surface, id)) else {
                continue;
            };
            if let Some(runtime) = self.agents.get_mut(&entity) {
                match callback {
                    "on_nav_link_start" => runtime.jumped = true,
                    "on_nav_link_end" => runtime.landed = true,
                    _ => {}
                }
            }
            self.events.push(NavEvent {
                entity,
                callback,
                data,
            });
        }
    }

    // ---------------------------------------------------------------- queries

    /// The agent walking as `entity`.
    pub fn agent_status(&self, entity: Entity) -> Option<AgentStatus> {
        let runtime = self.agents.get(&entity)?;
        let surface = self.surfaces.iter().find(|s| s.entity == runtime.surface)?;
        let agent = surface.crowd.agent(runtime.id)?;
        Some(AgentStatus {
            state: agent.state,
            position: agent.pos,
            velocity: agent.vel,
            desired_velocity: agent.desired_vel,
            target: agent.target.map(|t| t.1),
            remaining_distance: agent.remaining_distance(),
            partial: agent.partial,
            corners: agent.corners.iter().map(|c| c.pos).collect(),
        })
    }

    pub fn is_agent(&self, entity: Entity) -> bool {
        self.agents.contains_key(&entity)
    }

    /// A path between two points: its corners, whether it reaches `to`, and its length.
    pub fn find_path(
        &self,
        from: Vec3,
        to: Vec3,
        agent_type: &str,
        filter: &QueryFilter,
    ) -> Option<(Vec<Vec3>, bool, f32)> {
        let mesh = self.mesh(agent_type)?;
        let (corners, path) =
            query::find_path_points(mesh, from, to, Vec3::new(2.0, 2.0, 2.0), filter)?;
        let points: Vec<Vec3> = corners.iter().map(|c| c.pos).collect();
        let length = points.windows(2).map(|w| w[0].distance(w[1])).sum();
        Some((points, path.complete, length))
    }

    /// The navmesh point nearest `pos` (within `extents`).
    pub fn closest_point(&self, pos: Vec3, extents: Vec3, agent_type: &str) -> Option<Vec3> {
        self.mesh(agent_type)?
            .find_nearest_poly(pos, extents)
            .map(|(_, p)| p)
    }

    /// A random navmesh point: reachable from `around` within `radius`, or anywhere.
    pub fn random_point(&self, around: Option<(Vec3, f32)>, agent_type: &str) -> Option<Vec3> {
        let mesh = self.mesh(agent_type)?;
        let filter = QueryFilter::default();
        let mut rng = rand::thread_rng();
        let from =
            around.and_then(|(center, _)| mesh.find_nearest_poly(center, Vec3::new(2.0, 2.0, 2.0)));
        let radius = around.map(|(_, r)| r);
        query::random_point(mesh, from, radius, &filter, &mut rng).map(|(_, p)| p)
    }

    /// Walk a straight line over the floor: (blocked, end point, wall normal).
    pub fn raycast(&self, from: Vec3, to: Vec3, agent_type: &str) -> Option<(bool, Vec3, Vec3)> {
        let mesh = self.mesh(agent_type)?;
        let (start, start_pos) = mesh.find_nearest_poly(from, Vec3::new(1.0, 2.0, 1.0))?;
        let filter = QueryFilter::default();
        let hit = query::raycast(mesh, start, start_pos, to, &filter);
        if hit.t == f32::MAX {
            let end_ref = hit.path.last().copied().unwrap_or(start);
            let mut end = to;
            end.y = mesh.poly_height(end_ref, end).unwrap_or(to.y);
            Some((false, end, Vec3::ZERO))
        } else {
            let mut point = start_pos.lerp(to, hit.t);
            if let Some(&last) = hit.path.last() {
                point.y = mesh.poly_height(last, point).unwrap_or(point.y);
            }
            Some((true, point, hit.normal))
        }
    }

    /// Where a ray (a mouse click) meets the navmesh first.
    pub fn pick(&self, origin: Vec3, direction: Vec3) -> Option<Vec3> {
        let mut best: Option<f32> = None;
        for surface in &self.surfaces {
            let Some(mesh) = surface.mesh() else {
                continue;
            };
            for tile in mesh.tiles() {
                for i in 0..tile.ground_count {
                    for [a, b, c] in tile.detail_triangles(i) {
                        if let Some(t) = ray_triangle(origin, direction, a, b, c) {
                            if best.is_none_or(|bt| t < bt) {
                                best = Some(t);
                            }
                        }
                    }
                }
            }
        }
        best.map(|t| origin + direction * t)
    }

    // ---------------------------------------------------------------- debug

    /// Whether anything would be drawn for this selection.
    pub fn wants_debug_geometry(&self, world: &World, selected: &[Entity]) -> bool {
        self.debug_draw().any()
            || self.test_path().is_some()
            || selected.iter().any(|&e| {
                world
                    .satisfies::<hecs::Or<&NavMeshSurface, hecs::Or<&NavAgent, &NavObstacle>>>(e)
                    .unwrap_or(false)
            })
    }

    /// What to draw this frame: the requested debug layers, plus what the editor's
    /// `selected` entities are (a surface's navmesh, an agent's body, an obstacle's
    /// footprint).
    pub fn debug_geometry(&mut self, world: &World, selected: &[Entity]) -> DebugGeometry {
        let mut out = DebugGeometry::default();
        let debug = self.debug_draw();
        for &entity in selected {
            if let Ok(agent) = world.get::<&NavAgent>(entity) {
                if !debug.agents {
                    let pos = world_transform(world, entity).translation;
                    out.circle(
                        pos + Vec3::Y * 0.03,
                        agent.radius,
                        2.0,
                        super::debug::IDLE_COLOR,
                    );
                    out.circle(
                        pos + Vec3::Y * agent.height,
                        agent.radius,
                        1.0,
                        super::debug::IDLE_COLOR,
                    );
                }
            }
            if let Ok(obstacle) = world.get::<&NavObstacle>(entity) {
                if !debug.obstacles {
                    let (shape, _) = obstacle_footprint(world, entity, &obstacle, 0.0);
                    let carved = self
                        .obstacles
                        .get(&entity)
                        .is_some_and(|o| o.carved.is_some());
                    super::debug::obstacle_geometry(&shape, carved, &mut out);
                }
            }
        }
        for surface in &self.surfaces {
            let show = debug.mesh || selected.contains(&surface.entity);
            if !show {
                continue;
            }
            let Some(mesh) = surface.mesh() else {
                continue;
            };
            let cached = self
                .debug_cache
                .get(&surface.entity)
                .filter(|(generation, _)| *generation == surface.generation);
            let geometry = match cached {
                Some((_, geometry)) => geometry.clone(),
                None => {
                    let geometry = super::debug::mesh_geometry(mesh);
                    self.debug_cache
                        .insert(surface.entity, (surface.generation, geometry.clone()));
                    geometry
                }
            };
            out.append(geometry);
        }
        if debug.agents || debug.paths {
            for (entity, runtime) in &self.agents {
                let Some(surface) = self.surfaces.iter().find(|s| s.entity == runtime.surface)
                else {
                    continue;
                };
                let Some(agent) = surface.crowd.agent(runtime.id) else {
                    continue;
                };
                super::debug::agent_geometry(agent, debug, &mut out);
                let _ = entity;
            }
            if self.agents.is_empty() {
                // Editing: the agents' bodies where their entities stand.
                for (entity, agent) in world.query::<&NavAgent>().iter() {
                    let pos = world_transform(world, entity).translation;
                    out.circle(pos, agent.radius, 2.0, super::debug::IDLE_COLOR);
                }
            }
        }
        if debug.obstacles {
            for (entity, obstacle) in world.query::<&NavObstacle>().iter() {
                let (shape, _) = obstacle_footprint(world, entity, obstacle, 0.0);
                let carved = self
                    .obstacles
                    .get(&entity)
                    .is_some_and(|o| o.carved.is_some());
                super::debug::obstacle_geometry(&shape, carved, &mut out);
            }
        }
        if let Some((from, to)) = self.test_path() {
            let mesh = self.mesh("");
            super::debug::test_path_geometry(mesh, from, to, &mut out);
        }
        if debug.x_ray {
            out.x_ray = 0.25;
        }
        out
    }
}

/// The distance along a ray to a triangle (Möller–Trumbore), either facing.
fn ray_triangle(origin: Vec3, dir: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
    let e1 = b - a;
    let e2 = c - a;
    let p = dir.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-9 {
        return None;
    }
    let inv = 1.0 / det;
    let s = origin - a;
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = dir.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = e2.dot(q) * inv;
    (t > 0.0).then_some(t)
}

/// Write a finished bake to `content/navigation/<surface name>.navmesh` and point the
/// surface at it. Returns the project path ("" without a project).
fn save_bake(
    world: &mut World,
    surface: Entity,
    built: &NavMeshBuild,
    settings: &NavMeshSurface,
) -> Result<String, String> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (world, surface, built, settings);
        Ok(String::new())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let Some(root) = crate::project::active_project_root() else {
            log::warn!("Navigation: no project is open, so the bake is kept in memory only");
            return Ok(String::new());
        };
        let name = world
            .get::<&Name>(surface)
            .map(|n| n.0.clone())
            .unwrap_or_else(|_| "navmesh".into());
        let stem: String = name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c.to_ascii_lowercase()
                } else {
                    '_'
                }
            })
            .collect();
        let stem = if stem.trim_matches('_').is_empty() {
            "navmesh".to_string()
        } else {
            stem
        };
        let shared = world
            .query::<(&NavMeshSurface, &Name)>()
            .iter()
            .any(|(other, (_, n))| other != surface && n.0.eq_ignore_ascii_case(&name));
        let stem = if shared {
            format!("{stem}_{}", surface.id())
        } else {
            stem
        };
        let path = format!("{BAKE_DIR}/{stem}.navmesh");
        let file = root.join(&path);
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).map_err(|err| format!("{}: {err}", dir.display()))?;
        }
        let info = NavMeshFileInfo {
            settings_hash: settings.build_settings_hash(),
            grid: built.grid.clone(),
        };
        let bytes = serialize::encode(&built.mesh, &info);
        std::fs::write(&file, &bytes).map_err(|err| format!("{}: {err}", file.display()))?;
        if let Ok(mut cache) = loaded_bakes().lock() {
            cache.remove(&path);
        }
        if let Ok(mut component) = world.get::<&mut NavMeshSurface>(surface) {
            component.baked = Some(path.clone());
        }
        log::info!(
            "Navigation: saved {path} ({} KB)",
            bytes.len().div_ceil(1024)
        );
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hulls_and_turning() {
        let hull = convex_hull(vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(0.0, 1.0),
            Vec2::new(0.5, 0.5),
        ]);
        assert_eq!(hull.len(), 4);
        let yaw = turn_toward(3.0, -3.0, 0.1);
        assert!((yaw - 3.1).abs() < 1e-5, "turns the short way round: {yaw}");
    }

    #[test]
    fn rays_hit_triangles() {
        let t = ray_triangle(
            Vec3::new(0.2, 5.0, 0.2),
            Vec3::NEG_Y,
            Vec3::ZERO,
            Vec3::X,
            Vec3::Z,
        );
        assert!((t.unwrap() - 5.0).abs() < 1e-5);
    }
}
