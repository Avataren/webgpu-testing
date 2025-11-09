//! # Component API
//!
//! This module provides functions for managing entity components in the ECS.
//!
//! ## Features
//!
//! - **Component queries** - Check if an entity has a component
//! - **Component access** - Get component data (Phase 2 feature, currently stubbed)
//! - **Component modification** - Set, add, or remove components
//!
//! ## Component Data Format
//!
//! Component values are passed as Lua tables that match the component's structure.
//! The system automatically serializes Lua values to JSON for component storage.
//!
//! ## Known Limitations
//!
//! - `get_component()` is not yet fully implemented and returns `nil` (Phase 2)

use hecs::Entity;
use log::{error, warn};
use mlua::{Lua, LuaSerdeExt, Result as LuaResult, Value as LuaValue};

use crate::scripting::lua::guards::{
    with_active_commands, with_active_world,
};

/// Registers component API functions with the Lua runtime.
///
/// This function exposes component management functions to Lua scripts.
///
/// ## Available Functions
///
/// - `has_component(entity, component_name)` - Returns true if entity has component
/// - `get_component(entity, component_name)` - Get component data (not yet implemented)
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
/// -- Get component (Phase 2 - currently returns nil)
/// local health = get_component(entity, "Health")
/// if health then
///     log_info("Health: " .. health.current .. "/" .. health.max)
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
    // TODO: This feature is not yet fully implemented for Lua
    // The rune-based component registry has been removed
    globals.set(
        "has_component",
        lua.create_function(|_, (entity_bits, component_name): (i64, String)| {
            warn!(target: "script", "has_component not yet fully implemented for Lua (entity: {}, component: {})", entity_bits, component_name);
            // Return false for now - this will be implemented when we add a Lua-specific component system
            Ok(false)
        })?,
    )?;

    // get_component(entity: number, component_name: string) -> any | nil
    // TODO: This requires rune::Value -> serde_json::Value conversion
    // For Phase 2, we'll stub this out - it will be implemented in a future update
    globals.set(
        "get_component",
        lua.create_function(|_lua, (_entity_bits, component_name): (i64, String)| {
            warn!(target: "script", "get_component not yet fully implemented for Lua (component: {})", component_name);
            // Return nil for now - this will be implemented when we add value conversion
            Ok(mlua::Value::Nil)
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
