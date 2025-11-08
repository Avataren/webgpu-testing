use glam::{EulerRot, Quat, Vec3};
use hecs::Entity;
use mlua::{Lua, Result as LuaResult};

use crate::scene::TransformComponent;
use crate::scripting::lua::guards::{with_active_commands, with_active_world};

/// Register transform API functions with the Lua runtime.
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

                // Get local transform for now
                if let Ok(transform) = world.get::<&TransformComponent>(entity) {
                    let translation = transform.0.translation;
                    let table = lua.create_table()?;
                    table.set("x", translation.x as f64)?;
                    table.set("y", translation.y as f64)?;
                    table.set("z", translation.z as f64)?;
                    return Ok(Some(table));
                }

                Ok(None)
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

                // Get local transform for now
                if let Ok(transform) = world.get::<&TransformComponent>(entity) {
                    let (yaw, pitch, roll) = transform.0.rotation.to_euler(EulerRot::YXZ);
                    let table = lua.create_table()?;
                    table.set("yaw", yaw as f64)?;
                    table.set("pitch", pitch as f64)?;
                    table.set("roll", roll as f64)?;
                    return Ok(Some(table));
                }

                Ok(None)
            })
        })?,
    )?;

    Ok(())
}
