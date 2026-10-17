//! Navigation: navmeshes generated from the scene, paths over them, and crowds of
//! agents that walk those paths while steering around each other and around
//! obstacles that move. Behind the `navigation` feature (on by default).
//!
//! - [`NavMeshSurface`] marks a navmesh to build from the scene's static geometry
//!   ([`recast`]: voxels, so floors over floors, bridges and tunnels work). The editor
//!   bakes it into the project; a scene without a bake builds it when play starts.
//! - [`NavAgent`]s walk it: scripts give them destinations, they follow the corridor of
//!   polygons to it ([`query`]), avoid each other (ORCA) and repath when it changes.
//! - [`NavObstacle`]s steer agents aside while moving and carve the navmesh once they
//!   stand still: the tiles they touch are rebuilt from a cache and paths through them
//!   reroute.
//! - [`NavModifier`] leaves geometry out, makes it unwalkable or gives it an area cost.
//! - [`NavLink`]s join places the floor does not (jump-downs, gaps, ladders).

pub mod avoidance;
pub mod builder;
pub mod components;
pub mod crowd;
mod debug;
pub mod edit;
pub mod geometry;
pub mod navmesh;
pub mod query;
pub(crate) mod recast;
mod scene_nav;
pub mod serialize;

pub use builder::{BuildStats, NavMeshBuild, TileCache, TileGrid};
pub use components::{
    NavAgent, NavAgentDrive, NavAreaCost, NavCollect, NavGeometrySource, NavLink, NavMeshSurface,
    NavModifier, NavObstacle, NavObstacleShape,
};
pub use crowd::{AgentParams, AgentState, Crowd, CrowdAgent, CrowdEvent, MovingObstacle};
pub use geometry::{collect_geometry, NavGeometry};
pub use navmesh::{NavMesh, NavPoly, NavTile, OffMeshConnection, PolyKind, PolyRef};
pub use query::{PathCorner, PolyPath, QueryFilter};
pub use recast::{NULL_AREA, WALKABLE_AREA};
pub use scene_nav::{
    AgentStatus, BakeOutcome, BakeProgress, NavCommand, NavDebugDraw, NavEvent, SceneNavigation,
    SurfaceStatus, BAKE_DIR,
};
