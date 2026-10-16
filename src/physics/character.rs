//! Walking characters: velocity from the script's steering, move-and-slide through
//! Rapier's kinematic character controller, then the state scripts read back.
//!
//! The capsule floats: it starts `step_height` above the feet, and a sphere probed down
//! from it finds the ground and holds the feet on it. Anything lower than a step passes
//! under the capsule, so stairs, kerbs and slopes are climbed by the probe rather than by
//! pushing a rounded capsule over their edges, at any frame rate and speed.

use glam::{Quat, Vec3};
use hecs::Entity;
use rapier3d::control::{CharacterLength, KinematicCharacterController};
use rapier3d::parry::query::ShapeCastOptions;
use rapier3d::prelude as rp;

use super::components::CharacterController;
use super::shapes::upright_capsule;
use super::{from_rp, pose, to_rp};

/// Gap kept between the capsule and what it touches (m).
const SKIN: f32 = 0.02;
/// Input shorter than this does not turn the character.
const TURN_DEADZONE: f32 = 0.05;
/// The ground probe is this much narrower than the capsule, so it does not catch walls
/// the capsule is pressed against.
const PROBE_SCALE: f32 = 0.85;

/// What a script has asked of a character and what happened to it; kept by the physics
/// world, not saved.
#[derive(Debug, Clone, Default)]
pub(crate) struct CharacterRuntime {
    /// World-space steering, horizontal, length 0..1 (a fraction of the speed).
    pub move_input: Vec3,
    pub run: bool,
    /// Seconds a pressed jump keeps waiting for the ground.
    pub jump_buffer: f32,
    /// Replaces the velocity on the next step.
    pub velocity_override: Option<Vec3>,
    /// Added to the velocity on the next step (m/s).
    pub impulse: Vec3,
    pub velocity: Vec3,
    pub grounded: bool,
    pub ground_normal: Vec3,
    pub ground_entity: Option<Entity>,
    /// Seconds since it last stood on the ground.
    pub air_time: f32,
    pub air_jumps_used: u32,
    /// Left the ground by jumping (or being launched): no coyote jump then.
    pub jumped: bool,
    pub just_landed: bool,
    /// Downward speed it landed with (m/s).
    pub landing_speed: f32,
    pub just_jumped: bool,
    pub sliding: bool,
    /// Facing about +Y (radians); the model's +Z points this way.
    pub yaw: f32,
    /// Facing a script asked for (`character_face`), and whether to turn there at once;
    /// while set, the character no longer turns toward its movement.
    pub facing: Option<(f32, bool)>,
    /// Velocity of the moving platform it stands on.
    pub platform_velocity: Vec3,
}

/// A character's state as scripts see it (`character_state`).
#[derive(Debug, Clone, PartialEq)]
pub struct CharacterStatus {
    pub velocity: Vec3,
    pub grounded: bool,
    pub ground_normal: Vec3,
    pub ground_entity: Option<Entity>,
    pub air_time: f32,
    pub just_landed: bool,
    pub landing_speed: f32,
    pub just_jumped: bool,
    pub sliding: bool,
    pub running: bool,
    /// The steering it was last given (world space, length 0..1).
    pub move_input: Vec3,
    /// Facing about +Y (radians).
    pub yaw: f32,
}

impl CharacterRuntime {
    pub(crate) fn status(&self) -> CharacterStatus {
        CharacterStatus {
            velocity: self.velocity,
            grounded: self.grounded,
            ground_normal: self.ground_normal,
            ground_entity: self.ground_entity,
            air_time: self.air_time,
            just_landed: self.just_landed,
            landing_speed: self.landing_speed,
            just_jumped: self.just_jumped,
            sliding: self.sliding,
            running: self.run,
            move_input: self.move_input,
            yaw: self.yaw,
        }
    }
}

/// The capsule a character moves with, and the height of its centre above the feet:
/// from `step_height` (clamped so some body remains) up to the head.
pub(crate) fn capsule(config: &CharacterController) -> (rp::SharedShape, f32) {
    let radius = config.radius.max(0.05);
    let height = config.height.max(radius * 2.0);
    let lift = step_lift(config);
    (
        upright_capsule(radius, height - lift),
        lift + (height - lift) * 0.5,
    )
}

/// The capsule the character's body is, feet to head: what sensors, props and scene
/// queries meet. Its centre's height above the feet comes with it.
pub(crate) fn body_capsule(config: &CharacterController) -> (rp::SharedShape, f32) {
    let radius = config.radius.max(0.05);
    let height = config.height.max(radius * 2.0);
    (upright_capsule(radius, height), height * 0.5)
}

/// How far the moving capsule floats above the feet.
fn step_lift(config: &CharacterController) -> f32 {
    let radius = config.radius.max(0.05);
    let height = config.height.max(radius * 2.0);
    config
        .step_height
        .clamp(0.0, (height - radius * 2.0).max(0.0) * 0.6)
}

/// Where a character ended up after one step.
pub(crate) struct CharacterMove {
    pub position: Vec3,
    pub rotation: Quat,
    pub landed: bool,
    pub jumped: bool,
}

fn move_towards(current: Vec3, target: Vec3, max_delta: f32) -> Vec3 {
    let delta = target - current;
    let distance = delta.length();
    if distance <= max_delta || distance <= f32::EPSILON {
        target
    } else {
        current + delta / distance * max_delta
    }
}

/// Turn the angle `current` toward `target` by at most `max_delta` (radians), the short
/// way round.
fn turn_towards(current: f32, target: f32, max_delta: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    let delta = (target - current + std::f32::consts::PI).rem_euclid(tau) - std::f32::consts::PI;
    if delta.abs() <= max_delta {
        target
    } else {
        current + delta.signum() * max_delta
    }
}

/// Move-and-slide for the floating capsule. Steps, snapping and slopes underfoot are the
/// ground probe's job, so Rapier only slides it along walls and ceilings.
fn rapier_controller(config: &CharacterController) -> KinematicCharacterController {
    let max_slope = config.max_slope.clamp(0.0, 89.0).to_radians();
    KinematicCharacterController {
        up: rp::Vector::Y,
        offset: CharacterLength::Absolute(SKIN),
        slide: true,
        autostep: None,
        max_slope_climb_angle: max_slope,
        min_slope_slide_angle: max_slope,
        snap_to_ground: None,
        normal_nudge_factor: 1.0e-4,
    }
}

/// The ground under the feet, as the probe finds it.
struct Ground {
    /// Height of the ground under the character.
    height: f32,
    normal: Vec3,
    collider: rp::ColliderHandle,
    point: Vec3,
}

/// Sweep a sphere down from the capsule's bottom to `reach` below the feet.
fn probe_ground(
    physics: &rp::PhysicsWorld,
    filter: rp::QueryFilter<'_>,
    config: &CharacterController,
    feet: Vec3,
    reach: f32,
) -> Option<Ground> {
    let radius = config.radius.max(0.05) * PROBE_SCALE;
    let lift = step_lift(config);
    let start = feet + Vec3::Y * (lift + radius);
    let ball = rp::Ball::new(radius);
    let options = ShapeCastOptions {
        max_time_of_impact: lift + reach.max(0.0),
        target_distance: 0.0,
        stop_at_penetration: false,
        compute_impact_geometry_on_penetration: true,
    };
    let (collider, hit) = physics.query_pipeline_with_filter(filter).cast_shape(
        &pose(start, Quat::IDENTITY),
        to_rp(Vec3::NEG_Y),
        &ball,
        options,
    )?;
    let center = start - Vec3::Y * hit.time_of_impact;
    let mut normal = from_rp(hit.normal1).normalize_or(Vec3::Y);
    let point = center - normal * radius;
    // A sphere resting on the edge of a step reports the edge's slanted normal; the
    // surface just past the contact says whether it is a step top or a steep slope.
    let max_slope_cos = config.max_slope.clamp(0.0, 89.0).to_radians().cos();
    if normal.y < max_slope_cos {
        let inward = (-Vec3::new(normal.x, 0.0, normal.z)).normalize_or_zero() * 0.03;
        let ray = rp::Ray::new(to_rp(point + inward + Vec3::Y * 0.05), to_rp(Vec3::NEG_Y));
        if let Some((_, surface)) = physics
            .query_pipeline_with_filter(filter)
            .cast_ray_and_get_normal(&ray, 0.1, true)
        {
            let surface = from_rp(surface.normal);
            if surface.y >= max_slope_cos {
                normal = surface;
            }
        }
    }
    Some(Ground {
        height: center.y - radius,
        normal,
        collider,
        point,
    })
}

/// Run one step of the character `body`, standing at `feet` (world space). The body's
/// collider is its whole capsule; it moves with the shorter floating one.
pub(crate) fn step(
    physics: &mut rp::PhysicsWorld,
    body: rp::RigidBodyHandle,
    config: &CharacterController,
    state: &mut CharacterRuntime,
    feet: Vec3,
    dt: f32,
    entity_of: impl Fn(rp::ColliderHandle, &rp::ColliderSet) -> Option<Entity>,
) -> CharacterMove {
    let up = Vec3::Y;
    state.just_landed = false;
    state.just_jumped = false;
    let was_grounded = state.grounded;
    let fall_speed = -state.velocity.y;

    // Steering: ease the horizontal velocity toward the requested one.
    let mut input = Vec3::new(state.move_input.x, 0.0, state.move_input.z);
    if input.length_squared() > 1.0 {
        input = input.normalize();
    }
    let speed = if state.run {
        config.run_speed
    } else {
        config.walk_speed
    };
    let control = if state.grounded {
        1.0
    } else {
        config.air_control.max(0.0)
    };
    let horizontal = move_towards(
        Vec3::new(state.velocity.x, 0.0, state.velocity.z),
        input * speed.max(0.0),
        config.acceleration.max(0.0) * control * dt,
    );
    let mut vertical = state.velocity.y;

    // Jumps: from the ground (or just after leaving it), else one of the air jumps. A
    // press waits up to `jump_buffer` for the ground (and is tried at least this step).
    state.jump_buffer = state.jump_buffer.min(config.jump_buffer.max(dt));
    if state.jump_buffer > 0.0 {
        let from_ground = !state.jumped && (state.grounded || state.air_time <= config.coyote_time);
        if from_ground || state.air_jumps_used < config.air_jumps {
            if !from_ground {
                state.air_jumps_used += 1;
            }
            vertical = (2.0 * config.gravity.max(0.0) * config.jump_height.max(0.0)).sqrt();
            state.jumped = true;
            state.jump_buffer = 0.0;
            state.just_jumped = true;
        }
    }
    state.jump_buffer = (state.jump_buffer - dt).max(0.0);

    if !state.grounded || vertical > 0.0 {
        vertical -= config.gravity * dt;
    }
    vertical = vertical.max(-config.max_fall_speed.abs());

    let mut velocity = horizontal + up * vertical;
    if let Some(forced) = state.velocity_override.take() {
        velocity = forced;
    }
    velocity += std::mem::take(&mut state.impulse);
    let rising = velocity.y > 0.0;
    if rising && state.grounded && !state.just_jumped {
        // Launched upward by a script: it left the ground without a jump.
        state.jumped = true;
    }

    let platform = if config.ride_platforms && state.grounded {
        state.platform_velocity
    } else {
        Vec3::ZERO
    };

    let (shape, center_height) = capsule(config);
    let filter = rp::QueryFilter::default()
        .exclude_rigid_body(body)
        .exclude_sensors();
    let controller = rapier_controller(config);

    // Slide the capsule along walls (and, rising, ceilings). Falling is the probe's.
    let desired = Vec3::new(velocity.x, velocity.y.max(0.0), velocity.z) * dt + platform * dt;
    let mut collisions = Vec::new();
    let movement = {
        let queries = physics.query_pipeline_with_filter(filter);
        controller.move_shape(
            dt,
            &queries,
            shape.as_ref(),
            &pose(feet + up * center_height, Quat::IDENTITY),
            to_rp(desired),
            |collision| collisions.push(collision),
        )
    };
    if config.push_mass > 0.0 && !collisions.is_empty() {
        let mut queries = physics.broad_phase.as_query_pipeline_mut(
            physics.narrow_phase.query_dispatcher(),
            &mut physics.bodies,
            &mut physics.colliders,
            filter,
        );
        controller.solve_character_collision_impulses(
            dt,
            &mut queries,
            shape.as_ref(),
            config.push_mass,
            &collisions,
        );
    }
    let moved = from_rp(movement.translation);
    let mut position = feet + moved;

    // Walls take the speed that goes into them; a ceiling ends the rise.
    let achieved = Vec3::new(moved.x, 0.0, moved.z) / dt - Vec3::new(platform.x, 0.0, platform.z);
    let wanted = Vec3::new(velocity.x, 0.0, velocity.z);
    if achieved.length_squared() < wanted.length_squared() {
        velocity.x = achieved.x;
        velocity.z = achieved.z;
    }
    if rising && moved.y < desired.y - 1.0e-4 {
        velocity.y = 0.0;
    }

    // Find the ground: far enough to catch this step's fall, and to keep walking down
    // stairs and slopes within the snap distance.
    let max_slope_cos = config.max_slope.clamp(0.0, 89.0).to_radians().cos();
    let fall = (-velocity.y * dt).max(0.0);
    let snap = if was_grounded && !rising {
        config.snap_distance.max(0.0)
    } else {
        0.0
    };
    let ground = probe_ground(physics, filter, config, position, fall + snap + SKIN);
    let mut grounded = false;
    state.sliding = false;
    state.ground_normal = up;
    state.ground_entity = None;
    state.platform_velocity = Vec3::ZERO;
    match ground.filter(|_| !rising) {
        Some(ground) if ground.normal.y >= max_slope_cos => {
            grounded = true;
            // Climb onto steps quickly but not in one jump; settle down at once.
            let rise = ground.height - position.y;
            let max_rise = (config.run_speed.max(config.walk_speed) * 1.5).max(8.0) * dt;
            position.y += if rise > 0.0 { rise.min(max_rise) } else { rise };
            velocity.y = 0.0;
            state.ground_normal = ground.normal;
            state.ground_entity = entity_of(ground.collider, &physics.colliders);
            if let Some(body) = physics.colliders[ground.collider]
                .parent()
                .and_then(|parent| physics.bodies.get(parent))
                .filter(|body| !body.is_fixed())
            {
                state.platform_velocity = from_rp(body.velocity_at_point(to_rp(ground.point)));
            }
        }
        Some(ground) => {
            // Too steep to stand on: fall, but not into it, and slide down its slope.
            position.y = (position.y - fall).max(ground.height);
            state.sliding = true;
            let downhill = Vec3::new(ground.normal.x, 0.0, ground.normal.z);
            velocity += downhill.normalize_or_zero() * config.gravity * dt;
            if position.y <= ground.height + 1.0e-4 && velocity.y < 0.0 {
                velocity.y *= 0.5;
            }
        }
        // Rising, `moved` already holds the climb (stopped by any ceiling).
        None if !rising => position.y -= fall,
        None => {}
    }

    state.grounded = grounded;
    if grounded {
        if !was_grounded {
            state.just_landed = true;
            state.landing_speed = fall_speed.max(0.0);
        }
        state.air_time = 0.0;
        state.air_jumps_used = 0;
        state.jumped = false;
    } else {
        state.air_time += dt;
    }
    state.velocity = velocity;

    let max_turn = config.turn_speed.max(0.0).to_radians() * dt;
    if let Some((facing, instant)) = state.facing {
        let target = facing + config.yaw_offset.to_radians();
        state.yaw = if instant {
            target
        } else {
            turn_towards(state.yaw, target, max_turn)
        };
    } else if config.face_movement && input.length() > TURN_DEADZONE {
        let target = input.x.atan2(input.z) + config.yaw_offset.to_radians();
        state.yaw = turn_towards(state.yaw, target, max_turn);
    }

    CharacterMove {
        position,
        rotation: Quat::from_rotation_y(state.yaw),
        landed: state.just_landed,
        jumped: state.just_jumped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turning_takes_the_short_way() {
        let pi = std::f32::consts::PI;
        let turned = turn_towards(0.9 * pi, -0.9 * pi, 0.1);
        assert!(turned > 0.9 * pi, "{turned}");
        assert_eq!(turn_towards(0.0, 0.05, 0.1), 0.05);
    }

    #[test]
    fn velocity_eases_toward_its_target() {
        let v = move_towards(Vec3::ZERO, Vec3::new(4.0, 0.0, 0.0), 1.0);
        assert_eq!(v, Vec3::new(1.0, 0.0, 0.0));
        assert_eq!(move_towards(v, v, 1.0), v);
    }

    #[test]
    fn the_capsule_floats_a_step_above_the_feet() {
        let config = CharacterController::default();
        let (shape, center) = capsule(&config);
        let capsule = shape.as_capsule().expect("a capsule");
        let bottom = center - capsule.half_height() - capsule.radius;
        let top = center + capsule.half_height() + capsule.radius;
        assert!((bottom - config.step_height).abs() < 1e-4, "{bottom}");
        assert!((top - config.height).abs() < 1e-4, "{top}");
    }
}
