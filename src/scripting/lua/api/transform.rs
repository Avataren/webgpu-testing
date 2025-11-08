//! # Transform API
//!
//! This module provides functions for manipulating entity transforms including
//! position (translation), rotation, and scale.
//!
//! ## Coordinate System
//!
//! The engine uses a **right-handed 3D coordinate system**:
//! - **X axis** - Right
//! - **Y axis** - Up
//! - **Z axis** - Back (toward camera in default view)
//!
//! ## Rotation Conventions
//!
//! ### Euler Angles (YXZ order)
//!
//! The `set_rotation(entity, yaw, pitch, roll)` function uses **YXZ** rotation order:
//! 1. **Yaw** - Rotation around Y axis (left/right)
//! 2. **Pitch** - Rotation around X axis (up/down)
//! 3. **Roll** - Rotation around Z axis (tilt/roll)
//!
//! All angles are in **radians**:
//! - 90 degrees = π/2 ≈ 1.5708 radians
//! - 180 degrees = π ≈ 3.1416 radians
//! - 360 degrees = 2π ≈ 6.2832 radians
//!
//! ### Axis-Angle Rotation
//!
//! The `rotate(entity, axis_x, axis_y, axis_z, angle)` function applies an
//! incremental rotation around an arbitrary axis (axis vector does not need
//! to be normalized).
//!
//! ## World vs Local Transforms
//!
//! - `get_world_translation()` and `get_world_rotation()` return transforms
//!   in world space (accounting for parent transforms)
//! - Setter functions (`set_translation`, `set_rotation`) set local transforms
//!   relative to the parent

use glam::{EulerRot, Quat, Vec3};
use hecs::Entity;
use mlua::{Lua, Result as LuaResult};

use crate::scene::{TransformComponent, WorldTransform};
use crate::scripting::lua::guards::{with_active_commands, with_active_world};

/// Registers transform API functions with the Lua runtime.
///
/// This function exposes position, rotation, and scale manipulation functions
/// to Lua scripts.
///
/// ## Available Functions
///
/// ### Translation
/// - `translate(entity, x, y, z)` - Apply delta to position
/// - `set_translation(entity, x, y, z)` - Set absolute position
/// - `get_world_translation(entity)` - Get world position as {x, y, z} table
///
/// ### Rotation
/// - `rotate(entity, axis_x, axis_y, axis_z, angle)` - Rotate around axis (radians)
/// - `set_rotation(entity, yaw, pitch, roll)` - Set via Euler angles (YXZ, radians)
/// - `get_world_rotation(entity)` - Get world rotation as {yaw, pitch, roll} table
/// - `look_at(entity, target_x, target_y, target_z)` - Orient to look at point
///
/// ### Scale
/// - `set_scale(entity, x, y, z)` - Set scale factors
///
/// # Example Lua usage
///
/// ```lua
/// -- Position an entity
/// set_translation(entity, 10, 0, 5)
/// translate(entity, 0, 1, 0)  -- Move up by 1 unit
///
/// -- Rotate using Euler angles (radians)
/// set_rotation(entity, 0, 1.5708, 0)  -- 90 degrees pitch
///
/// -- Rotate incrementally around Y axis
/// rotate(entity, 0, 1, 0, dt * 2.0)  -- Spin at 2 rad/sec
///
/// -- Orient to look at a point
/// look_at(entity, target_x, target_y, target_z)
///
/// -- Scale entity
/// set_scale(entity, 2.0, 2.0, 2.0)  -- Double size
///
/// -- Get world position
/// local pos = get_world_translation(entity)
/// if pos then
///     log_info("Position: " .. pos.x .. ", " .. pos.y .. ", " .. pos.z)
/// end
/// ```
///
/// # Arguments
///
/// * `lua` - The Lua runtime to register functions with
///
/// # Returns
///
/// `Ok(())` on success, or a Lua error if registration fails.
pub(crate) fn register_transform_api(lua: &Lua) -> LuaResult<()> {
    let globals = lua.globals();

    // translate(entity: number, x: number, y: number, z: number)
    globals.set(
        "translate",
        lua.create_function(|_, (entity, x, y, z): (i64, f64, f64, f64)| {
            with_active_commands(|commands| {
                commands.translate(entity, Vec3::new(x as f32, y as f32, z as f32))
            })
        })?,
    )?;

    // set_translation(entity: number, x: number, y: number, z: number)
    globals.set(
        "set_translation",
        lua.create_function(|_, (entity, x, y, z): (i64, f64, f64, f64)| {
            with_active_commands(|commands| {
                commands.set_translation(entity, Vec3::new(x as f32, y as f32, z as f32))
            })
        })?,
    )?;

    // rotate(entity: number, axis_x: number, axis_y: number, axis_z: number, angle: number)
    globals.set(
        "rotate",
        lua.create_function(
            |_, (entity, axis_x, axis_y, axis_z, angle): (i64, f64, f64, f64, f64)| {
                with_active_commands(|commands| {
                    commands.rotate(
                        entity,
                        Vec3::new(axis_x as f32, axis_y as f32, axis_z as f32),
                        angle as f32,
                    )
                })
            },
        )?,
    )?;

    // set_rotation(entity: number, yaw: number, pitch: number, roll: number)
    globals.set(
        "set_rotation",
        lua.create_function(|_, (entity, yaw, pitch, roll): (i64, f64, f64, f64)| {
            let rotation = Quat::from_euler(EulerRot::YXZ, yaw as f32, pitch as f32, roll as f32);
            with_active_commands(|commands| commands.set_rotation(entity, rotation))
        })?,
    )?;

    // set_scale(entity: number, x: number, y: number, z: number)
    globals.set(
        "set_scale",
        lua.create_function(|_, (entity, x, y, z): (i64, f64, f64, f64)| {
            with_active_commands(|commands| {
                commands.set_scale(entity, Vec3::new(x as f32, y as f32, z as f32))
            })
        })?,
    )?;

    // look_at(entity: number, target_x: number, target_y: number, target_z: number)
    globals.set(
        "look_at",
        lua.create_function(
            |_, (entity, target_x, target_y, target_z): (i64, f64, f64, f64)| {
                with_active_commands(|commands| {
                    commands.look_at(
                        entity,
                        Vec3::new(target_x as f32, target_y as f32, target_z as f32),
                    )
                })
            },
        )?,
    )?;

    // get_world_translation(entity: number) -> {x: number, y: number, z: number} | nil
    globals.set(
        "get_world_translation",
        lua.create_function(|lua, entity_bits: i64| {
            with_active_world(|world| {
                let entity = match Entity::from_bits(entity_bits as u64) {
                    Some(e) => e,
                    None => return Ok(None),
                };

                // Try to get world transform first, fall back to local if not available
                let translation = if let Ok(world_transform) = world.get::<&WorldTransform>(entity) {
                    world_transform.0.translation
                } else if let Ok(transform) = world.get::<&TransformComponent>(entity) {
                    transform.0.translation
                } else {
                    return Ok(None);
                };

                let table = lua.create_table()?;
                table.set("x", translation.x as f64)?;
                table.set("y", translation.y as f64)?;
                table.set("z", translation.z as f64)?;
                Ok(Some(table))
            })
        })?,
    )?;

    // get_world_rotation(entity: number) -> {yaw: number, pitch: number, roll: number} | nil
    globals.set(
        "get_world_rotation",
        lua.create_function(|lua, entity_bits: i64| {
            with_active_world(|world| {
                let entity = match Entity::from_bits(entity_bits as u64) {
                    Some(e) => e,
                    None => return Ok(None),
                };

                // Try to get world transform first, fall back to local if not available
                let rotation = if let Ok(world_transform) = world.get::<&WorldTransform>(entity) {
                    world_transform.0.rotation
                } else if let Ok(transform) = world.get::<&TransformComponent>(entity) {
                    transform.0.rotation
                } else {
                    return Ok(None);
                };

                let (yaw, pitch, roll) = rotation.to_euler(EulerRot::YXZ);
                let table = lua.create_table()?;
                table.set("yaw", yaw as f64)?;
                table.set("pitch", pitch as f64)?;
                table.set("roll", roll as f64)?;
                Ok(Some(table))
            })
        })?,
    )?;

    Ok(())
}
