//! Navigation components: the navmesh a scene walks on, the agents that walk it, the
//! obstacles that move about on it and the modifiers that shape it. Plain data, saved
//! with the scene; the runtime ([`super::SceneNavigation`]) is built from them.

use serde::{Deserialize, Serialize};

use super::recast::{RecastConfig, WALKABLE_AREA};

/// Which meshes a [`NavMeshSurface`] is built from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NavGeometrySource {
    /// The visible meshes (static ones: not skinned, not dynamic bodies, not agents or
    /// obstacles).
    #[default]
    RenderMeshes,
    /// The shapes of fixed physics colliders.
    Colliders,
    /// Both.
    Both,
}

impl NavGeometrySource {
    pub const ALL: [Self; 3] = [Self::RenderMeshes, Self::Colliders, Self::Both];

    pub fn label(self) -> &'static str {
        match self {
            Self::RenderMeshes => "Render meshes",
            Self::Colliders => "Physics colliders",
            Self::Both => "Meshes and colliders",
        }
    }
}

/// Which entities a [`NavMeshSurface`] collects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NavCollect {
    /// Everything in the scene (inside `bounds`, when set).
    #[default]
    All,
    /// Only the surface entity and its descendants.
    Children,
}

impl NavCollect {
    pub const ALL: [Self; 2] = [Self::All, Self::Children];

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "Whole scene",
            Self::Children => "This entity's children",
        }
    }
}

/// A navmesh: the walkable surface of the scene for agents of one size, built from the
/// scene's static geometry. Floors above floors (bridges, balconies, tunnels) are kept
/// apart, so agents walk over and under overhangs.
///
/// The editor bakes it into `content/navigation/*.navmesh` (`baked`); a scene played
/// without a bake builds it when play starts.
///
/// ```
/// use wgpu_cube::navigation::NavMeshSurface;
///
/// let wide = NavMeshSurface { agent_radius: 0.6, ..Default::default() };
/// // A bake made for other agents is stale.
/// assert_ne!(wide.build_settings_hash(), NavMeshSurface::default().build_settings_hash());
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NavMeshSurface {
    /// The agents' radius (m): walls and drops keep this far from their centre.
    pub agent_radius: f32,
    /// The agents' height (m): lower ceilings are not walkable.
    pub agent_height: f32,
    /// Tallest step (m) agents walk up or down.
    pub agent_max_climb: f32,
    /// Steepest slope (degrees) agents walk on.
    pub agent_max_slope: f32,
    /// Voxel size across (m); smaller follows walls more closely but builds slower.
    pub cell_size: f32,
    /// Voxel height (m).
    pub cell_height: f32,
    /// Tile edge in voxels. Obstacles rebuild the tiles they touch, so smaller tiles
    /// update faster.
    pub tile_size: u32,
    /// Walkable islands smaller than this (m²) are dropped.
    pub min_region_area: f32,
    /// Regions smaller than this (m²) merge into a neighbour.
    pub merge_region_area: f32,
    /// Longest wall edge (m) before it is split (0: no limit).
    pub max_edge_length: f32,
    /// How far (in voxels) simplified walls may stray from the voxels.
    pub max_edge_error: f32,
    /// Spacing (m) of the height samples that make the mesh follow the floor (0: off).
    pub detail_sample_distance: f32,
    /// How far (m) the mesh may float above or sink below the floor.
    pub detail_sample_max_error: f32,
    pub geometry: NavGeometrySource,
    pub collect: NavCollect,
    /// Build only inside this box (full size, centred on the entity, world axes);
    /// zero builds over everything collected.
    pub bounds: [f32; 3],
    /// Project path of the baked navmesh.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baked: Option<String>,
    /// Build the navmesh when play starts if it has no bake.
    pub build_on_start: bool,
    /// Agents with the same `agent_type` walk this navmesh ("" for the default).
    pub agent_type: String,
}

impl Default for NavMeshSurface {
    fn default() -> Self {
        Self {
            agent_radius: 0.4,
            agent_height: 1.8,
            agent_max_climb: 0.45,
            agent_max_slope: 45.0,
            cell_size: 0.2,
            cell_height: 0.1,
            tile_size: 48,
            min_region_area: 1.0,
            merge_region_area: 8.0,
            max_edge_length: 12.0,
            max_edge_error: 1.3,
            detail_sample_distance: 1.2,
            detail_sample_max_error: 0.1,
            geometry: NavGeometrySource::RenderMeshes,
            collect: NavCollect::All,
            bounds: [0.0; 3],
            baked: None,
            build_on_start: true,
            agent_type: String::new(),
        }
    }
}

impl NavMeshSurface {
    /// The settings that decide the navmesh's shape (not where it is saved), for
    /// telling whether a bake is stale.
    pub fn build_settings_hash(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for value in [
            self.agent_radius,
            self.agent_height,
            self.agent_max_climb,
            self.agent_max_slope,
            self.cell_size,
            self.cell_height,
            self.min_region_area,
            self.merge_region_area,
            self.max_edge_length,
            self.max_edge_error,
            self.detail_sample_distance,
            self.detail_sample_max_error,
        ] {
            value.to_bits().hash(&mut hasher);
        }
        self.tile_size.hash(&mut hasher);
        hasher.finish()
    }

    /// The generator's settings in voxels.
    pub(crate) fn recast_config(&self) -> RecastConfig {
        let cs = self.cell_size.max(0.01);
        let ch = self.cell_height.max(0.01);
        let walkable_radius = (self.agent_radius.max(0.0) / cs).ceil() as i32;
        let detail_sample_dist = if self.detail_sample_distance < 0.9 * cs {
            0.0
        } else {
            self.detail_sample_distance
        };
        RecastConfig {
            cs,
            ch,
            walkable_slope_angle: self.agent_max_slope.clamp(0.0, 89.0),
            walkable_height: (self.agent_height.max(ch) / ch).ceil() as i32,
            walkable_climb: (self.agent_max_climb.max(0.0) / ch).floor() as i32,
            walkable_radius,
            max_edge_len: (self.max_edge_length.max(0.0) / cs) as i32,
            max_simplification_error: self.max_edge_error.max(0.1),
            min_region_area: (self.min_region_area.max(0.0) / (cs * cs)).round() as i32,
            merge_region_area: (self.merge_region_area.max(0.0) / (cs * cs)).round() as i32,
            max_verts_per_poly: super::navmesh::MAX_VERTS_PER_POLY,
            detail_sample_dist,
            detail_sample_max_error: self.detail_sample_max_error.max(0.01),
            tile_size: self.tile_size.clamp(16, 256) as i32,
            border_size: walkable_radius + 3,
        }
    }
}

/// What an agent walking over an area costs, relative to plain ground.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct NavAreaCost {
    /// Area id (1..63; 63 is plain walkable ground).
    pub area: u8,
    /// Multiplies the distance walked there; 0 or less keeps the agent out.
    pub cost: f32,
}

/// How an agent moves its entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NavAgentDrive {
    /// Steer the entity's `CharacterController` when it has one, else move it.
    #[default]
    Auto,
    /// Move the entity along the navmesh (cheap, for crowds).
    Transform,
    /// Steer its `CharacterController` (physics: pushing, gravity, colliders).
    Character,
}

impl NavAgentDrive {
    pub const ALL: [Self; 3] = [Self::Auto, Self::Transform, Self::Character];

    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Transform => "Move transform",
            Self::Character => "Character controller",
        }
    }
}

/// Something that walks the navmesh to destinations scripts give it
/// (`nav_set_destination`), steering around other agents and obstacles.
///
/// The entity's origin is the agent's feet. Models are expected to face +Z; use
/// `yaw_offset` for others. A character model's `AnimationTree` with "from character"
/// set gets the agent's speeds, like a character controller's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NavAgent {
    /// Body radius (m), for avoidance.
    pub radius: f32,
    pub height: f32,
    /// Top speed (m/s).
    pub speed: f32,
    /// How fast it speeds up, slows down and swerves (m/s²).
    pub acceleration: f32,
    /// How fast it turns to face its movement (degrees per second).
    pub turn_speed: f32,
    /// Arrive this close to the destination (m).
    pub stopping_distance: f32,
    /// Steer around other agents and moving obstacles.
    pub avoidance: bool,
    /// Higher-priority agents expect others to make way (0..1).
    pub priority: f32,
    /// How strongly it keeps its distance from neighbours.
    pub separation: f32,
    /// Turn toward its movement.
    pub face_movement: bool,
    /// Added to the facing (degrees), for models that do not face +Z.
    pub yaw_offset: f32,
    pub drive: NavAgentDrive,
    /// Walks the [`NavMeshSurface`] with the same agent type.
    pub agent_type: String,
    /// Where it heads when play starts (world space), if anywhere.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination: Option<[f32; 3]>,
    /// Costs of areas other than plain ground.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub area_costs: Vec<NavAreaCost>,
}

impl Default for NavAgent {
    fn default() -> Self {
        Self {
            radius: 0.35,
            height: 1.8,
            speed: 1.5,
            acceleration: 8.0,
            turn_speed: 540.0,
            stopping_distance: 0.15,
            avoidance: true,
            priority: 0.5,
            separation: 1.0,
            face_movement: true,
            yaw_offset: 0.0,
            drive: NavAgentDrive::Auto,
            agent_type: String::new(),
            destination: None,
            area_costs: Vec::new(),
        }
    }
}

/// The footprint of a [`NavObstacle`], in the entity's local units.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NavObstacleShape {
    Box {
        #[serde(default = "unit_box")]
        size: [f32; 3],
    },
    Cylinder {
        #[serde(default = "half_metre")]
        radius: f32,
        #[serde(default = "one_metre")]
        height: f32,
    },
}

impl Default for NavObstacleShape {
    fn default() -> Self {
        Self::Box { size: unit_box() }
    }
}

fn unit_box() -> [f32; 3] {
    [1.0; 3]
}

fn half_metre() -> f32 {
    0.5
}

fn one_metre() -> f32 {
    1.0
}

/// Something that moves about on the navmesh (a cart, a door, a crate): agents steer
/// around it while it moves, and once it stands still it cuts a hole into the navmesh,
/// so paths go around it (and reroute when it blocks the way).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NavObstacle {
    pub shape: NavObstacleShape,
    /// Where the shape's centre sits relative to the entity (local units).
    pub offset: [f32; 3],
    /// Cut a hole into the navmesh (otherwise agents only steer around it).
    pub carve: bool,
    /// Carve only once it has stood still for `stationary_time`.
    pub carve_only_stationary: bool,
    /// It counts as moved after going this far (m) or turning about as much.
    pub move_threshold: f32,
    /// Seconds without moving before it carves.
    pub stationary_time: f32,
}

impl Default for NavObstacle {
    fn default() -> Self {
        Self {
            shape: NavObstacleShape::default(),
            offset: [0.0; 3],
            carve: true,
            carve_only_stationary: true,
            move_threshold: 0.1,
            stationary_time: 0.5,
        }
    }
}

/// A way between two points the floor does not connect: a jump down from a wall, a gap
/// to leap, a ladder. It starts at the entity and ends at `end`; agents whose path uses
/// it cross it in an arc (scripts hear `on_nav_link_start` / `on_nav_link_end`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NavLink {
    /// Where it lands, relative to the entity (local units).
    pub end: [f32; 3],
    /// Crossable both ways (a ladder, a gap) or only from the entity to `end` (a drop).
    pub bidirectional: bool,
    /// How far from either end it reaches for the floor (m).
    pub radius: f32,
    /// Area id, for agents' area costs (63 is plain ground).
    pub area: u8,
}

impl Default for NavLink {
    fn default() -> Self {
        Self {
            end: [0.0, 0.0, 2.0],
            bidirectional: false,
            radius: 0.5,
            area: WALKABLE_AREA,
        }
    }
}

/// Changes how an entity's geometry (and its children's) goes into navmeshes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NavModifier {
    /// Leave it out of the navmesh entirely.
    pub ignore: bool,
    /// Its tops are walkable (off: solid but never walked on, like rooftops).
    pub walkable: bool,
    /// The area its walkable tops get (1..63; 63 is plain ground). Agents can give
    /// areas costs (roads cheaper, mud dearer).
    pub area: u8,
    /// Apply to the entity's descendants too.
    pub include_children: bool,
}

impl Default for NavModifier {
    fn default() -> Self {
        Self {
            ignore: false,
            walkable: true,
            area: WALKABLE_AREA,
            include_children: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn components_load_from_partial_json() {
        let agent: NavAgent = serde_json::from_str(r#"{"speed": 3}"#).unwrap();
        assert_eq!(agent.speed, 3.0);
        assert_eq!(agent.radius, 0.35);
        let obstacle: NavObstacle =
            serde_json::from_str(r#"{"shape": {"kind": "cylinder", "radius": 0.8}}"#).unwrap();
        assert_eq!(
            obstacle.shape,
            NavObstacleShape::Cylinder {
                radius: 0.8,
                height: 1.0
            }
        );
        let surface: NavMeshSurface = serde_json::from_str(r#"{"agent_radius": 0.3}"#).unwrap();
        assert_eq!(surface.cell_size, 0.2);
        assert!(surface.baked.is_none());
    }

    #[test]
    fn config_converts_to_voxels() {
        let surface = NavMeshSurface::default();
        let cfg = surface.recast_config();
        assert_eq!(cfg.walkable_radius, 2);
        assert_eq!(cfg.walkable_height, 18);
        assert_eq!(cfg.walkable_climb, 4);
        assert_eq!(cfg.border_size, 5);
        assert_ne!(
            surface.build_settings_hash(),
            NavMeshSurface {
                agent_radius: 0.5,
                ..surface.clone()
            }
            .build_settings_hash()
        );
    }
}
