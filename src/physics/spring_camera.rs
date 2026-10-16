//! Spring-arm cameras: orbit a point above the target, follow it with lag, and pull in
//! when scenery comes between camera and target.

use glam::{EulerRot, Quat, Vec3};
use hecs::Entity;
use rapier3d::parry::query::ShapeCastOptions;
use rapier3d::prelude as rp;

use super::components::SpringCamera;
use super::{pose, to_rp};
use crate::scene::InputState;

/// Seconds the arm takes to grow back out after scenery stops blocking it.
const ARM_RECOVERY: f32 = 0.25;
/// A target that jumps further than this in one frame was teleported: no lag then.
const TELEPORT_DISTANCE: f32 = 8.0;
/// Below this speed (m/s) the camera does not swing behind the target.
const AUTO_FOLLOW_MIN_SPEED: f32 = 0.5;

/// A spring camera's live state; kept by the physics world, not saved.
#[derive(Debug, Clone, Default)]
pub(crate) struct CameraRuntime {
    pub initialized: bool,
    /// Degrees.
    pub yaw: f32,
    pub pitch: f32,
    /// Arm length asked for (zoom).
    pub distance: f32,
    /// Share of the arm in use after collisions (0..1).
    pub extent: f32,
    /// The smoothed point the camera looks at.
    pub pivot: Vec3,
    /// Where the target's pivot was last frame (for auto-follow).
    pub last_goal: Vec3,
    /// Target set by a script, overriding the component's.
    pub target: Option<Entity>,
    /// Skip the lag once (after a script moves the camera).
    pub snap: bool,
}

/// A spring camera's state as scripts see it (`spring_camera_state`).
#[derive(Debug, Clone, PartialEq)]
pub struct SpringCameraStatus {
    /// Degrees.
    pub yaw: f32,
    pub pitch: f32,
    /// Arm length in use (after collisions).
    pub distance: f32,
    pub pivot: Vec3,
    /// Horizontal unit vectors of the view: where "forward" and "right" on a stick go.
    pub forward: Vec3,
    pub right: Vec3,
    pub target: Option<Entity>,
}

impl CameraRuntime {
    pub(crate) fn status(&self, target: Option<Entity>) -> SpringCameraStatus {
        let yaw = self.yaw.to_radians();
        SpringCameraStatus {
            yaw: self.yaw,
            pitch: self.pitch,
            distance: self.distance * self.extent,
            pivot: self.pivot,
            forward: Vec3::new(-yaw.sin(), 0.0, -yaw.cos()),
            right: Vec3::new(yaw.cos(), 0.0, -yaw.sin()),
            target,
        }
    }
}

/// Where a spring camera goes this frame.
pub(crate) struct CameraPlacement {
    pub position: Vec3,
    pub rotation: Quat,
    pub focus: Vec3,
}

fn smooth(current: f32, goal: f32, lag: f32, dt: f32) -> f32 {
    if lag <= 1.0e-4 {
        goal
    } else {
        current + (goal - current) * (1.0 - (-dt / lag).exp())
    }
}

/// Move the camera for one frame. `target` is the followed entity's world position;
/// `exclude` the body the arm should see through (the target's own).
#[allow(clippy::too_many_arguments)]
pub(crate) fn update(
    physics: &rp::PhysicsWorld,
    config: &SpringCamera,
    state: &mut CameraRuntime,
    target: Vec3,
    exclude: Option<rp::RigidBodyHandle>,
    input: Option<&InputState>,
    cursor_locked: bool,
    dt: f32,
) -> CameraPlacement {
    let goal = target + Vec3::from(config.pivot_offset);
    if !state.initialized {
        state.initialized = true;
        state.yaw = config.yaw;
        state.pitch = config.pitch;
        state.distance = config.distance;
        state.extent = 1.0;
        state.snap = true;
    }
    if state.snap || goal.distance(state.pivot) > TELEPORT_DISTANCE {
        state.pivot = goal;
        state.last_goal = goal;
        state.snap = false;
    }

    let mut looked = false;
    if let Some(input) = input {
        if config.mouse_look && (cursor_locked || input.is_mouse_button_pressed(1)) {
            let delta = input.mouse_delta() * config.sensitivity;
            let invert = if config.invert_y { -1.0 } else { 1.0 };
            state.yaw -= delta.x;
            state.pitch -= delta.y * invert;
            looked = delta.length_squared() > 0.0;
        }
        if config.zoom_step > 0.0 {
            state.distance -= input.scroll_delta().y * config.zoom_step;
        }
    }
    let (min_pitch, max_pitch) = (
        config.min_pitch.min(config.max_pitch),
        config.max_pitch.max(config.min_pitch),
    );
    state.pitch = state.pitch.clamp(min_pitch.max(-89.0), max_pitch.min(89.0));
    state.yaw = (state.yaw + 180.0).rem_euclid(360.0) - 180.0;
    let (min_distance, max_distance) = (
        config.min_distance.max(0.0),
        config.max_distance.max(config.min_distance.max(0.0)),
    );
    state.distance = state.distance.clamp(min_distance, max_distance);

    // Swing behind a target that moves, unless the player is looking around.
    let travel = Vec3::new(goal.x - state.last_goal.x, 0.0, goal.z - state.last_goal.z);
    state.last_goal = goal;
    if config.auto_follow > 0.0 && !looked && dt > 0.0 {
        let speed = travel.length() / dt;
        if speed > AUTO_FOLLOW_MIN_SPEED {
            let behind = (-travel.x).atan2(-travel.z).to_degrees();
            let delta = (behind - state.yaw + 180.0).rem_euclid(360.0) - 180.0;
            let rate = (config.auto_follow * dt * (speed / 4.0).min(1.0)).min(1.0);
            state.yaw += delta * rate;
        }
    }

    state.pivot.x = smooth(state.pivot.x, goal.x, config.follow_lag, dt);
    state.pivot.z = smooth(state.pivot.z, goal.z, config.follow_lag, dt);
    state.pivot.y = smooth(state.pivot.y, goal.y, config.vertical_lag, dt);

    let rotation = Quat::from_euler(
        EulerRot::YXZ,
        state.yaw.to_radians(),
        state.pitch.to_radians(),
        0.0,
    );
    let arm = rotation * Vec3::X * config.shoulder_offset + rotation * Vec3::Z * state.distance;

    // Shorten the arm to the first scenery in its way; grow back slowly.
    let mut reach = 1.0;
    if config.collision && arm.length_squared() > 1.0e-6 {
        let mut filter = rp::QueryFilter::from(
            rp::QueryFilterFlags::EXCLUDE_DYNAMIC
                | rp::QueryFilterFlags::EXCLUDE_KINEMATIC
                | rp::QueryFilterFlags::EXCLUDE_SENSORS,
        );
        if let Some(body) = exclude {
            filter = filter.exclude_rigid_body(body);
        }
        let probe = rp::Ball::new(config.probe_radius.max(0.01));
        let options = ShapeCastOptions {
            max_time_of_impact: 1.0,
            target_distance: 0.0,
            stop_at_penetration: false,
            compute_impact_geometry_on_penetration: false,
        };
        if let Some((_, hit)) = physics.query_pipeline_with_filter(filter).cast_shape(
            &pose(state.pivot, Quat::IDENTITY),
            to_rp(arm),
            &probe,
            options,
        ) {
            reach = hit.time_of_impact.clamp(0.0, 1.0);
        }
    }
    state.extent = if reach < state.extent {
        reach
    } else {
        smooth(state.extent, reach, ARM_RECOVERY, dt)
    };

    let position = state.pivot + arm * state.extent;
    CameraPlacement {
        position,
        rotation,
        focus: state.pivot + rotation * Vec3::X * config.shoulder_offset,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_axes_follow_the_yaw() {
        let state = CameraRuntime {
            yaw: 90.0,
            extent: 1.0,
            distance: 4.0,
            ..Default::default()
        };
        let status = state.status(None);
        // Yaw 90 looks along -X.
        assert!(
            (status.forward - Vec3::NEG_X).length() < 1e-5,
            "{:?}",
            status.forward
        );
        assert!(
            (status.right - Vec3::NEG_Z).length() < 1e-5,
            "{:?}",
            status.right
        );
    }

    #[test]
    fn the_arm_pulls_in_behind_a_wall() {
        let mut physics = rp::PhysicsWorld::default();
        // A wall 2 m behind the pivot (the camera sits on +Z at yaw 0).
        physics.insert(
            rp::RigidBodyBuilder::fixed().translation(rp::Vector::new(0.0, 1.5, 2.0)),
            rp::ColliderBuilder::cuboid(5.0, 5.0, 0.1),
        );
        physics.step();
        let config = SpringCamera {
            pitch: 0.0,
            follow_lag: 0.0,
            vertical_lag: 0.0,
            ..Default::default()
        };
        let mut state = CameraRuntime::default();
        let placement = update(
            &physics,
            &config,
            &mut state,
            Vec3::ZERO,
            None,
            None,
            false,
            1.0 / 60.0,
        );
        assert!(placement.position.z < 1.9, "{:?}", placement.position);
        assert!(placement.position.z > 1.0, "{:?}", placement.position);
    }
}
