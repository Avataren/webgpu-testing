//! # Component API
//!
//! This module provides functions for managing entity components in the ECS.
//!
//! ## Features
//!
//! - **Component queries** - Check if an entity has a component
//! - **Component access** - Get component data
//! - **Component modification** - Set, add, or remove components
//!
//! ## Component Data Format
//!
//! Component values are passed as Lua tables that match the component's structure.
//! The system automatically serializes Lua values to JSON for component storage.
//!
use hecs::Entity;
use mlua::{Lua, LuaSerdeExt, Result as LuaResult, Value as LuaValue};

use crate::scripting::lua::commands::ScriptCommands;
use crate::scripting::lua::guards::{with_active_commands, with_active_world};

/// Registers component API functions with the Lua runtime.
///
/// This function exposes component management functions to Lua scripts.
///
/// ## Available Functions
///
/// - `has_component(entity, component_name)` - Returns true if entity has component
/// - `get_component(entity, component_name)` - Get component data
/// - `set_component(entity, component_name, value)` - Set component data
/// - `add_component(entity, component_name, value)` - Add new component
/// - `remove_component(entity, component_name)` - Remove component from entity
///
/// # Example Lua usage
///
/// ```lua
/// -- Check if entity has a component
/// if has_component(entity, "Health") then
///     log_info("Entity has health component")
/// end
///
/// -- Add a component
/// add_component(entity, "Health", {
///     current = 100,
///     max = 100
/// })
///
/// -- Modify a component
/// set_component(entity, "Health", {
///     current = 50,
///     max = 100
/// })
///
/// -- Remove a component
/// remove_component(entity, "Health")
///
/// -- Get component data
/// local transform = get_component(entity, "Transform")
/// if transform then
///     log_info("Position: " .. transform.translation[1] .. ", " .. transform.translation[2])
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
pub(crate) fn register_component_api(lua: &Lua) -> LuaResult<()> {
    let globals = lua.globals();

    // has_component(entity: number, component_name: string) -> boolean
    globals.set(
        "has_component",
        lua.create_function(|_, (entity_bits, component_name): (i64, String)| {
            with_active_world(|world| {
                let Some(entity) = Entity::from_bits(entity_bits as u64) else {
                    return Ok(false);
                };

                if world.entity(entity).is_err() {
                    return Ok(false);
                }

                ScriptCommands::component_exists(world, entity, &component_name)
                    .map_err(mlua::Error::RuntimeError)
            })
        })?,
    )?;

    // get_component(entity: number, component_name: string) -> any | nil
    globals.set(
        "get_component",
        lua.create_function(|lua, (entity_bits, component_name): (i64, String)| {
            with_active_world(|world| {
                let Some(entity) = Entity::from_bits(entity_bits as u64) else {
                    return Ok(LuaValue::Nil);
                };

                if world.entity(entity).is_err() {
                    return Ok(LuaValue::Nil);
                }

                match ScriptCommands::component_to_json(world, entity, &component_name)
                    .map_err(mlua::Error::RuntimeError)?
                {
                    Some(value) => lua.to_value(&value),
                    None => Ok(LuaValue::Nil),
                }
            })
        })?,
    )?;

    // set_component(entity: number, component_name: string, value: any)
    globals.set(
        "set_component",
        lua.create_function(
            |lua, (entity_bits, component_name, value): (i64, String, LuaValue)| {
                // Use mlua's built-in serde integration
                let json_value: serde_json::Value = lua.from_value(value)?;
                with_active_commands(|commands| {
                    commands.set_component(entity_bits, component_name, json_value)
                })
            },
        )?,
    )?;

    // add_component(entity: number, component_name: string, value: any)
    globals.set(
        "add_component",
        lua.create_function(
            |lua, (entity_bits, component_name, value): (i64, String, LuaValue)| {
                // Use mlua's built-in serde integration
                let json_value: serde_json::Value = lua.from_value(value)?;
                with_active_commands(|commands| {
                    commands.add_component(entity_bits, component_name, json_value)
                })
            },
        )?,
    )?;

    // remove_component(entity: number, component_name: string)
    globals.set(
        "remove_component",
        lua.create_function(|_, (entity_bits, component_name): (i64, String)| {
            with_active_commands(|commands| commands.remove_component(entity_bits, component_name))
        })?,
    )?;

    Ok(())
}
