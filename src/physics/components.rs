//! Physics components: what an entity collides as, how it moves, and the character and
//! camera rigs built on them. They are plain data, saved with the scene; the simulation
//! itself ([`super::ScenePhysics`]) is rebuilt from them.

use serde::{Deserialize, Serialize};

/// How a [`RigidBody`] moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyKind {
    /// Moved by the simulation: gravity, collisions and impulses.
    #[default]
    Dynamic,
    /// Never moves on its own; the entity's transform places it (level geometry).
    Fixed,
    /// Follows the entity's transform (set by scripts or animation) and pushes dynamic
    /// bodies out of its way, like a moving platform or a door.
    Kinematic,
}

impl BodyKind {
    pub const ALL: [Self; 3] = [Self::Dynamic, Self::Fixed, Self::Kinematic];

    pub fn label(self) -> &'static str {
        match self {
            Self::Dynamic => "Dynamic",
            Self::Fixed => "Fixed",
            Self::Kinematic => "Kinematic",
        }
    }
}

/// Makes an entity a simulated body. Its shape comes from the entity's [`Collider`] (a
/// body without one collides as [`ColliderShape::Auto`]).
///
/// Dynamic bodies write their position back to the entity's transform each play step;
/// moving such an entity yourself (a script's `set_translation`, the editor) teleports
/// the body there.
///
/// ```
/// use wgpu_cube::physics::{BodyKind, RigidBody};
///
/// let crate_body = RigidBody { mass: 20.0, ..Default::default() };
/// assert_eq!(crate_body.kind, BodyKind::Dynamic);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RigidBody {
    pub kind: BodyKind,
    /// Mass in kg; 0 takes it from the collider's volume and density.
    pub mass: f32,
    /// Slows the body's movement over time (0 = none).
    pub linear_damping: f32,
    /// Slows the body's spin over time (0 = none).
    pub angular_damping: f32,
    /// Multiplies the scene's gravity (0 floats).
    pub gravity_scale: f32,
    /// Collisions never turn the body (crates that should slide rather than tumble).
    pub lock_rotation: bool,
    /// Continuous collision detection, so small fast bodies do not pass through walls.
    pub ccd: bool,
    /// Velocity when the simulation starts (m/s, world space).
    pub initial_velocity: [f32; 3],
    /// Let the body sleep once it comes to rest; it wakes when something touches it.
    pub can_sleep: bool,
}

impl Default for RigidBody {
    fn default() -> Self {
        Self {
            kind: BodyKind::Dynamic,
            mass: 0.0,
            linear_damping: 0.0,
            angular_damping: 0.05,
            gravity_scale: 1.0,
            lock_rotation: false,
            ccd: false,
            initial_velocity: [0.0; 3],
            can_sleep: true,
        }
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

/// The shape of a [`Collider`], in the entity's local units: the entity's scale
/// stretches it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ColliderShape {
    /// Fit the entity's mesh: built-in primitives get their exact shape, other meshes a
    /// triangle mesh (fixed and kinematic bodies) or their convex hull (dynamic bodies).
    /// An entity without a mesh gets a 1 m box.
    #[default]
    Auto,
    /// A box of full size `size`, centred on the entity.
    Box {
        #[serde(default = "unit_box")]
        size: [f32; 3],
    },
    Sphere {
        #[serde(default = "half_metre")]
        radius: f32,
    },
    /// Upright capsule: `height` is the full height, end caps included.
    Capsule {
        #[serde(default = "half_metre")]
        radius: f32,
        #[serde(default = "one_metre")]
        height: f32,
    },
    /// Upright cylinder of full height `height`.
    Cylinder {
        #[serde(default = "half_metre")]
        radius: f32,
        #[serde(default = "one_metre")]
        height: f32,
    },
    /// The mesh's triangles, exactly (for level geometry; dynamic bodies get the hull).
    Mesh,
    /// The smallest convex shape around the mesh.
    ConvexHull,
}

impl ColliderShape {
    pub const KIND_NAMES: [&'static str; 7] = [
        "auto",
        "box",
        "sphere",
        "capsule",
        "cylinder",
        "mesh",
        "convex_hull",
    ];

    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Box { .. } => "box",
            Self::Sphere { .. } => "sphere",
            Self::Capsule { .. } => "capsule",
            Self::Cylinder { .. } => "cylinder",
            Self::Mesh => "mesh",
            Self::ConvexHull => "convex_hull",
        }
    }

    /// A shape of kind `name` with default dimensions.
    pub fn from_kind_name(name: &str) -> Option<Self> {
        Some(match name {
            "auto" => Self::Auto,
            "box" => Self::Box { size: unit_box() },
            "sphere" => Self::Sphere { radius: 0.5 },
            "capsule" => Self::Capsule {
                radius: 0.5,
                height: 1.0,
            },
            "cylinder" => Self::Cylinder {
                radius: 0.5,
                height: 1.0,
            },
            "mesh" => Self::Mesh,
            "convex_hull" => Self::ConvexHull,
            _ => return None,
        })
    }
}

/// What an entity collides as. On its own it is fixed scenery placed by the entity's
/// transform; with a [`RigidBody`] it gives that body its shape.
///
/// ```
/// use wgpu_cube::physics::{Collider, ColliderShape};
///
/// let ball = Collider { shape: ColliderShape::Sphere { radius: 0.25 }, restitution: 0.6, ..Default::default() };
/// assert!(!ball.sensor);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Collider {
    pub shape: ColliderShape,
    /// Where the shape's centre sits relative to the entity (local units).
    pub offset: [f32; 3],
    /// 0 = ice, 1 = rubber.
    pub friction: f32,
    /// Bounciness: 0 keeps none of the impact speed, 1 all of it.
    pub restitution: f32,
    /// kg per cubic metre of shape (gives dynamic bodies without a mass their mass).
    pub density: f32,
    /// Detects overlaps (scripts get `on_trigger_enter` / `on_trigger_exit`) without
    /// blocking anything.
    pub sensor: bool,
    /// Also take the meshes of the entity's descendants (imported models and levels,
    /// whose meshes sit on child nodes). Children with a collider of their own keep it.
    pub include_children: bool,
}

impl Default for Collider {
    fn default() -> Self {
        Self {
            shape: ColliderShape::Auto,
            offset: [0.0; 3],
            friction: 0.6,
            restitution: 0.0,
            density: 1.0,
            sensor: false,
            include_children: false,
        }
    }
}

/// A walking character: an upright capsule moved by move-and-slide (Rapier's kinematic
/// character controller) with gravity, jumping, slopes, steps and ground snapping.
///
/// The entity's origin is the character's feet. Scripts steer it (`character_move`,
/// `character_jump`) and read back what happened (`character_state`); the numbers
/// here tune how it feels. Models are expected to face +Z (glTF's convention); use
/// `yaw_offset` for others.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CharacterController {
    /// Capsule radius (m).
    pub radius: f32,
    /// Capsule height, feet to head (m).
    pub height: f32,
    /// Steepest slope it walks up (degrees); steeper ground is a wall it slides off.
    pub max_slope: f32,
    /// Tallest ledge it steps up onto without jumping (m); 0 disables stepping.
    pub step_height: f32,
    /// Keeps it on the ground walking down slopes and stairs, up to this drop (m).
    pub snap_distance: f32,
    /// Speed at full stick without running (m/s).
    pub walk_speed: f32,
    /// Speed at full stick while running (m/s).
    pub run_speed: f32,
    /// How fast it speeds up and slows down on the ground (m/s²).
    pub acceleration: f32,
    /// Share of `acceleration` it has in the air (0 = no steering mid-jump).
    pub air_control: f32,
    /// Downward acceleration (m/s²); games usually feel better above the real 9.81.
    pub gravity: f32,
    /// How high a jump lifts the feet (m).
    pub jump_height: f32,
    /// Extra jumps in the air (1 = double jump).
    pub air_jumps: u32,
    /// After walking off a ledge, a jump still counts as from the ground for this long (s).
    pub coyote_time: f32,
    /// A jump pressed this long before landing still happens on landing (s).
    pub jump_buffer: f32,
    /// Fastest fall (m/s).
    pub max_fall_speed: f32,
    /// Turn to face the way it moves.
    pub face_movement: bool,
    /// How fast it turns toward its movement (degrees per second).
    pub turn_speed: f32,
    /// Added to the facing (degrees), for models that do not face +Z.
    pub yaw_offset: f32,
    /// Its mass when it pushes dynamic bodies (kg); 0 does not push.
    pub push_mass: f32,
    /// Ride on moving (kinematic) platforms it stands on.
    pub ride_platforms: bool,
}

impl Default for CharacterController {
    fn default() -> Self {
        Self {
            radius: 0.3,
            height: 1.8,
            max_slope: 50.0,
            step_height: 0.35,
            snap_distance: 0.3,
            walk_speed: 2.2,
            run_speed: 6.0,
            acceleration: 40.0,
            air_control: 0.35,
            gravity: 22.0,
            jump_height: 1.3,
            air_jumps: 0,
            coyote_time: 0.12,
            jump_buffer: 0.15,
            max_fall_speed: 45.0,
            face_movement: true,
            turn_speed: 720.0,
            yaw_offset: 0.0,
            push_mass: 80.0,
            ride_platforms: true,
        }
    }
}

/// A third-person camera on a spring arm: it orbits a point above its target at
/// `distance`, follows it with a little lag, and pulls in when scenery comes between
/// them. Give the entity a camera too.
///
/// Mouse look is built in (while the cursor is locked or the right mouse button is
/// held); scripts can turn it with `spring_camera_rotate` instead.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SpringCamera {
    /// Name of the entity to follow; empty follows the camera's parent.
    pub target: String,
    /// The point the camera looks at, relative to the target's origin (world axes, m).
    pub pivot_offset: [f32; 3],
    /// Arm length (m).
    pub distance: f32,
    pub min_distance: f32,
    pub max_distance: f32,
    /// Starting direction around the target (degrees; 0 looks along -Z).
    pub yaw: f32,
    /// Starting tilt (degrees; negative looks down on the target).
    pub pitch: f32,
    pub min_pitch: f32,
    pub max_pitch: f32,
    /// Sideways offset for an over-the-shoulder view (m, + is right).
    pub shoulder_offset: f32,
    /// Turn with the mouse (cursor locked, or right button held). Off, scripts turn it
    /// (`spring_camera_set`); the wheel still zooms.
    pub mouse_look: bool,
    /// Degrees per pixel of mouse movement.
    pub sensitivity: f32,
    pub invert_y: bool,
    /// Metres per scroll-wheel notch (0 disables zoom).
    pub zoom_step: f32,
    /// Seconds the arm's end takes to catch up with the target sideways (0 = rigid).
    pub follow_lag: f32,
    /// Seconds it takes to catch up vertically (jumps look calmer with more).
    pub vertical_lag: f32,
    /// Pull in when scenery would come between the camera and its target.
    pub collision: bool,
    /// Radius kept clear around the camera (m).
    pub probe_radius: f32,
    /// Swing behind the target while it moves (turns per second-ish; 0 = never).
    pub auto_follow: f32,
}

impl Default for SpringCamera {
    fn default() -> Self {
        Self {
            target: String::new(),
            pivot_offset: [0.0, 1.5, 0.0],
            distance: 4.5,
            min_distance: 1.5,
            max_distance: 10.0,
            yaw: 0.0,
            pitch: -15.0,
            min_pitch: -75.0,
            max_pitch: 55.0,
            shoulder_offset: 0.0,
            mouse_look: true,
            sensitivity: 0.15,
            invert_y: false,
            zoom_step: 0.6,
            follow_lag: 0.06,
            vertical_lag: 0.15,
            collision: true,
            probe_radius: 0.2,
            auto_follow: 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_round_trip_with_missing_dimensions() {
        let shape: ColliderShape = serde_json::from_str(r#"{"kind":"capsule"}"#).unwrap();
        assert_eq!(
            shape,
            ColliderShape::Capsule {
                radius: 0.5,
                height: 1.0
            }
        );
        let collider = Collider {
            shape: ColliderShape::Box {
                size: [2.0, 1.0, 3.0],
            },
            ..Default::default()
        };
        let json = serde_json::to_string(&collider).unwrap();
        assert_eq!(serde_json::from_str::<Collider>(&json).unwrap(), collider);
    }

    #[test]
    fn components_load_from_partial_json() {
        let body: RigidBody = serde_json::from_str(r#"{"kind":"fixed"}"#).unwrap();
        assert_eq!(body.kind, BodyKind::Fixed);
        assert_eq!(body.gravity_scale, 1.0);
        let controller: CharacterController = serde_json::from_str(r#"{"run_speed":8}"#).unwrap();
        assert_eq!(controller.run_speed, 8.0);
        assert_eq!(controller.radius, 0.3);
        for name in ColliderShape::KIND_NAMES {
            assert_eq!(
                ColliderShape::from_kind_name(name).unwrap().kind_name(),
                name
            );
        }
    }
}
