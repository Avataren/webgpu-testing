//! # Physics API
//!
//! Scene queries, bodies, characters and spring cameras (see the `RigidBody`, `Collider`,
//! `CharacterController` and `SpringCamera` components; their settings are read and
//! changed with `get_component` / `set_component`, e.g.
//! `set_component(player, "CharacterController", { run_speed = 8 })`).
//!
//! The simulation runs in play mode after every script's `update`, so changes made here
//! take effect in this frame's step and queries see the world as of the last step.
//!
//! ## Queries
//! - `physics_raycast(ox, oy, oz, dx, dy, dz[, max_distance[, options]])` returns
//!   `{entity, distance, point = {x, y, z}, normal = {x, y, z}}` for the first collider
//!   hit, or nil. `options`: `exclude` (an entity or a list of them), `sensors` (also hit
//!   trigger volumes), `ignore_dynamic` (only scenery and characters).
//! - `physics_spherecast(ox, oy, oz, dx, dy, dz, radius[, max_distance[, options]])`: the
//!   same for a ball swept along the ray.
//! - `physics_overlap_sphere(x, y, z, radius[, options])` returns the entities whose
//!   colliders overlap the ball.
//!
//! ## Bodies
//! - `get_velocity(entity)` / `get_angular_velocity(entity)` return `{x, y, z}` (m/s,
//!   rad/s) for a body or character, or nil.
//! - `set_velocity(entity, x, y, z)`, `apply_impulse(entity, x, y, z)` (kg·m/s; on a
//!   character it adds m/s to its velocity), `set_angular_velocity(entity, x, y, z)`,
//!   `apply_torque_impulse(entity, x, y, z)`.
//! - `set_gravity(x, y, z)`, `get_gravity()`; `physics_debug_draw(enabled)` shows every
//!   collider.
//!
//! ## Characters
//! - `character_move(entity, x, z[, run])`: steer this frame: a world-space horizontal
//!   direction whose length (0..1) is the share of walking (or with `run`, running) speed.
//!   Steering holds until changed, so send (0, 0) to stop.
//! - `character_jump(entity)`: jump if it can (or as soon as it lands, within the
//!   controller's `jump_buffer`).
//! - `character_face(entity, yaw[, instant])`: face `yaw` (radians about +Y; 0 faces +Z,
//!   `math.atan(x, z)` faces a direction), turning at the controller's turn speed or at
//!   once. It then stops turning toward its movement (strafing, walking backwards) until
//!   `character_face(entity, nil)`.
//! - `character_state(entity)` returns `{grounded, velocity, speed, horizontal_speed,
//!   vertical_speed, ground_normal, ground_entity, air_time, just_landed, landing_speed,
//!   just_jumped, sliding, running, yaw}`.
//! - Scripts on a character get `on_jumped(self)` and `on_landed(self, speed)`; any
//!   scripted entity gets `on_collision_enter(self, other)` / `on_collision_exit` and,
//!   for sensors, `on_trigger_enter(self, other)` / `on_trigger_exit`.
//!
//! ## Spring cameras
//! - `spring_camera_state(entity)` returns `{yaw, pitch, distance, pivot, forward, right,
//!   target}`: `forward` and `right` are the view's horizontal axes, for camera-relative
//!   movement.
//! - `spring_camera_rotate(entity, yaw_degrees, pitch_degrees)` turns it.
//! - `spring_camera_set(entity, {yaw, pitch, distance, target, snap})` sets its view
//!   (`target` an entity, or false for the component's; `snap` skips the lag once).
//!
//! ## Cursor
//! - `set_cursor_locked(locked)` captures the mouse for mouse look (hidden, unbounded
//!   movement through `get_mouse_delta()`); Escape or leaving play releases it.
//! - `is_cursor_locked()`.

use glam::Vec3;
use hecs::Entity;
use mlua::{Lua, Result as LuaResult, Table, Value};

use crate::physics::{PhysicsCommand, QueryOptions, RayHit};
use crate::scripting::lua::commands::entity_bits;
use crate::scripting::lua::guards::{
    resolve_entity, with_active_commands, with_active_input_state, with_active_physics,
    with_active_world,
};

fn error(message: impl Into<String>) -> mlua::Error {
    mlua::Error::RuntimeError(message.into())
}

fn entity_of(handle: i64) -> Option<Entity> {
    resolve_entity(handle)
}

fn vec3_table(lua: &Lua, v: Vec3) -> LuaResult<Table> {
    let table = lua.create_table()?;
    table.set("x", v.x)?;
    table.set("y", v.y)?;
    table.set("z", v.z)?;
    Ok(table)
}

fn hit_table(lua: &Lua, hit: RayHit) -> LuaResult<Table> {
    let table = lua.create_table()?;
    table.set("entity", entity_bits(hit.entity))?;
    table.set("distance", hit.distance)?;
    table.set("point", vec3_table(lua, hit.point)?)?;
    table.set("normal", vec3_table(lua, hit.normal)?)?;
    Ok(table)
}

fn query_options(value: Option<Table>) -> LuaResult<QueryOptions> {
    let mut options = QueryOptions::default();
    let Some(table) = value else {
        return Ok(options);
    };
    match table.get::<Value>("exclude")? {
        Value::Nil => {}
        Value::Integer(handle) => options.exclude.extend(entity_of(handle)),
        Value::Number(handle) => options.exclude.extend(entity_of(handle as i64)),
        Value::Table(list) => {
            for handle in list.sequence_values::<i64>() {
                options.exclude.extend(entity_of(handle?));
            }
        }
        _ => return Err(error("exclude must be an entity or a list of entities")),
    }
    options.sensors = table.get::<Option<bool>>("sensors")?.unwrap_or(false);
    options.ignore_dynamic = table
        .get::<Option<bool>>("ignore_dynamic")?
        .unwrap_or(false);
    Ok(options)
}

/// Queue a change for the simulation; `handle` must name a spawned entity.
fn queue(handle: i64, make: impl FnOnce(Entity) -> PhysicsCommand) -> LuaResult<()> {
    with_active_commands(|commands| {
        let bits = commands.resolve_entity_bits(handle)?;
        let entity = Entity::from_bits(bits).ok_or_else(|| error("invalid entity handle"))?;
        commands.physics.push(make(entity));
        Ok(())
    })
}

fn queue_global(command: PhysicsCommand) -> LuaResult<()> {
    with_active_commands(|commands| {
        commands.physics.push(command);
        Ok(())
    })
}

fn vector(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3::new(x as f32, y as f32, z as f32)
}

pub(crate) fn register_physics_api(lua: &Lua) -> LuaResult<()> {
    let globals = lua.globals();

    globals.set(
        "physics_raycast",
        lua.create_function(
            |lua,
             (ox, oy, oz, dx, dy, dz, max_distance, options): (
                f64,
                f64,
                f64,
                f64,
                f64,
                f64,
                Option<f64>,
                Option<Table>,
            )| {
                let options = query_options(options)?;
                let hit = with_active_physics(|physics| {
                    Ok(physics.raycast(
                        vector(ox, oy, oz),
                        vector(dx, dy, dz),
                        max_distance.unwrap_or(1000.0) as f32,
                        &options,
                    ))
                })?;
                hit.map(|hit| hit_table(lua, hit)).transpose()
            },
        )?,
    )?;

    globals.set(
        "physics_spherecast",
        lua.create_function(
            |lua,
             (ox, oy, oz, dx, dy, dz, radius, max_distance, options): (
                f64,
                f64,
                f64,
                f64,
                f64,
                f64,
                f64,
                Option<f64>,
                Option<Table>,
            )| {
                let options = query_options(options)?;
                let hit = with_active_physics(|physics| {
                    Ok(physics.spherecast(
                        vector(ox, oy, oz),
                        vector(dx, dy, dz),
                        radius as f32,
                        max_distance.unwrap_or(1000.0) as f32,
                        &options,
                    ))
                })?;
                hit.map(|hit| hit_table(lua, hit)).transpose()
            },
        )?,
    )?;

    globals.set(
        "physics_overlap_sphere",
        lua.create_function(
            |lua, (x, y, z, radius, options): (f64, f64, f64, f64, Option<Table>)| {
                let options = query_options(options)?;
                let entities = with_active_physics(|physics| {
                    Ok(physics.overlap_sphere(vector(x, y, z), radius as f32, &options))
                })?;
                lua.create_sequence_from(entities.into_iter().map(entity_bits))
            },
        )?,
    )?;

    globals.set(
        "get_velocity",
        lua.create_function(|lua, entity: i64| {
            let velocity = with_active_physics(|physics| {
                Ok(entity_of(entity).and_then(|entity| physics.velocity(entity)))
            })?;
            velocity.map(|v| vec3_table(lua, v)).transpose()
        })?,
    )?;

    globals.set(
        "get_angular_velocity",
        lua.create_function(|lua, entity: i64| {
            let velocity = with_active_physics(|physics| {
                Ok(entity_of(entity).and_then(|entity| physics.angular_velocity(entity)))
            })?;
            velocity.map(|v| vec3_table(lua, v)).transpose()
        })?,
    )?;

    globals.set(
        "set_velocity",
        lua.create_function(|_, (entity, x, y, z): (i64, f64, f64, f64)| {
            queue(entity, |entity| PhysicsCommand::SetVelocity {
                entity,
                velocity: vector(x, y, z),
            })
        })?,
    )?;

    globals.set(
        "apply_impulse",
        lua.create_function(|_, (entity, x, y, z): (i64, f64, f64, f64)| {
            queue(entity, |entity| PhysicsCommand::ApplyImpulse {
                entity,
                impulse: vector(x, y, z),
            })
        })?,
    )?;

    globals.set(
        "set_angular_velocity",
        lua.create_function(|_, (entity, x, y, z): (i64, f64, f64, f64)| {
            queue(entity, |entity| PhysicsCommand::SetAngularVelocity {
                entity,
                velocity: vector(x, y, z),
            })
        })?,
    )?;

    globals.set(
        "apply_torque_impulse",
        lua.create_function(|_, (entity, x, y, z): (i64, f64, f64, f64)| {
            queue(entity, |entity| PhysicsCommand::ApplyTorqueImpulse {
                entity,
                impulse: vector(x, y, z),
            })
        })?,
    )?;

    globals.set(
        "set_gravity",
        lua.create_function(|_, (x, y, z): (f64, f64, f64)| {
            queue_global(PhysicsCommand::SetGravity(vector(x, y, z)))
        })?,
    )?;

    globals.set(
        "get_gravity",
        lua.create_function(|lua, ()| {
            let gravity = with_active_physics(|physics| Ok(physics.gravity()))?;
            vec3_table(lua, gravity)
        })?,
    )?;

    globals.set(
        "physics_debug_draw",
        lua.create_function(|_, enabled: bool| {
            queue_global(PhysicsCommand::SetDebugDraw(enabled))
        })?,
    )?;

    globals.set(
        "character_move",
        lua.create_function(|_, (entity, x, z, run): (i64, f64, f64, Option<bool>)| {
            queue(entity, |entity| PhysicsCommand::CharacterMove {
                entity,
                direction: vector(x, 0.0, z),
                run: run.unwrap_or(false),
            })
        })?,
    )?;

    globals.set(
        "character_jump",
        lua.create_function(|_, entity: i64| {
            queue(entity, |entity| PhysicsCommand::CharacterJump { entity })
        })?,
    )?;

    globals.set(
        "character_face",
        lua.create_function(
            |_, (entity, yaw, instant): (i64, Option<f64>, Option<bool>)| {
                queue(entity, |entity| PhysicsCommand::CharacterFace {
                    entity,
                    yaw: yaw.map(|yaw| yaw as f32),
                    instant: instant.unwrap_or(false),
                })
            },
        )?,
    )?;

    globals.set(
        "character_state",
        lua.create_function(|lua, entity: i64| {
            let status = with_active_physics(|physics| {
                Ok(entity_of(entity).and_then(|entity| physics.character_status(entity)))
            })?;
            let Some(status) = status else {
                return Ok(Value::Nil);
            };
            let table = lua.create_table()?;
            let horizontal = Vec3::new(status.velocity.x, 0.0, status.velocity.z).length();
            table.set("grounded", status.grounded)?;
            table.set("velocity", vec3_table(lua, status.velocity)?)?;
            table.set("speed", status.velocity.length())?;
            table.set("horizontal_speed", horizontal)?;
            table.set("vertical_speed", status.velocity.y)?;
            table.set("ground_normal", vec3_table(lua, status.ground_normal)?)?;
            table.set("ground_entity", status.ground_entity.map(entity_bits))?;
            table.set("air_time", status.air_time)?;
            table.set("just_landed", status.just_landed)?;
            table.set("landing_speed", status.landing_speed)?;
            table.set("just_jumped", status.just_jumped)?;
            table.set("sliding", status.sliding)?;
            table.set("running", status.running)?;
            table.set("move_input", vec3_table(lua, status.move_input)?)?;
            table.set("yaw", status.yaw)?;
            Ok(Value::Table(table))
        })?,
    )?;

    globals.set(
        "spring_camera_state",
        lua.create_function(|lua, entity: i64| {
            let status = with_active_world(|world| {
                with_active_physics(|physics| {
                    Ok(entity_of(entity)
                        .and_then(|entity| physics.spring_camera_status(world, entity)))
                })
            })?;
            let Some(status) = status else {
                return Ok(Value::Nil);
            };
            let table = lua.create_table()?;
            table.set("yaw", status.yaw)?;
            table.set("pitch", status.pitch)?;
            table.set("distance", status.distance)?;
            table.set("pivot", vec3_table(lua, status.pivot)?)?;
            table.set("forward", vec3_table(lua, status.forward)?)?;
            table.set("right", vec3_table(lua, status.right)?)?;
            table.set("target", status.target.map(entity_bits))?;
            Ok(Value::Table(table))
        })?,
    )?;

    globals.set(
        "spring_camera_rotate",
        lua.create_function(|_, (entity, yaw, pitch): (i64, f64, Option<f64>)| {
            queue(entity, |entity| PhysicsCommand::CameraRotate {
                entity,
                yaw: yaw as f32,
                pitch: pitch.unwrap_or(0.0) as f32,
            })
        })?,
    )?;

    globals.set(
        "spring_camera_set",
        lua.create_function(|_, (entity, options): (i64, Table)| {
            let target = match options.get::<Value>("target")? {
                Value::Nil => None,
                Value::Boolean(false) => Some(None),
                Value::Integer(handle) => Some(entity_of(handle)),
                Value::Number(handle) => Some(entity_of(handle as i64)),
                _ => return Err(error("target must be an entity or false")),
            };
            let yaw = options.get::<Option<f32>>("yaw")?;
            let pitch = options.get::<Option<f32>>("pitch")?;
            let distance = options.get::<Option<f32>>("distance")?;
            let snap = options.get::<Option<bool>>("snap")?.unwrap_or(false);
            queue(entity, |entity| PhysicsCommand::CameraSet {
                entity,
                yaw,
                pitch,
                distance,
                target,
                snap,
            })
        })?,
    )?;

    globals.set(
        "set_cursor_locked",
        lua.create_function(|_, locked: bool| {
            crate::scene::input_state::request_cursor_lock(locked);
            Ok(())
        })?,
    )?;

    globals.set(
        "is_cursor_locked",
        lua.create_function(|_, ()| with_active_input_state(|input| Ok(input.cursor_locked())))?,
    )?;

    Ok(())
}
