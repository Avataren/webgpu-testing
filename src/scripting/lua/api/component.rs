use hecs::Entity;
use log::{error, warn};
use mlua::{Lua, LuaSerdeExt, Result as LuaResult, Value as LuaValue};

use crate::scripting::component_registry::ComponentRegistryError;
use crate::scripting::lua::guards::{with_active_commands, with_active_registry, with_active_world};

/// Register component API functions with the Lua runtime.
pub(crate) fn register_component_api(lua: &Lua) -> LuaResult<()> {
    let globals = lua.globals();

    // has_component(entity: number, component_name: string) -> boolean
    globals.set(
        "has_component",
        lua.create_function(|_, (entity_bits, component_name): (i64, String)| {
            with_active_world(|world| {
                with_active_registry(|registry| {
                    let entity = match Entity::from_bits(entity_bits as u64) {
                        Some(e) => e,
                        None => return Ok(false),
                    };

                    match registry.has_component(world, entity, &component_name) {
                        Ok(has) => Ok(has),
                        Err(ComponentRegistryError::UnknownComponent(name)) => {
                            warn!(target: "script", "Unknown component type: {}", name);
                            Ok(false)
                        }
                        Err(e) => {
                            error!(target: "script", "Failed to check component: {}", e);
                            Ok(false)
                        }
                    }
                })
            })
        })?,
    )?;

    // get_component(entity: number, component_name: string) -> any | nil
    // TODO: This requires rune::Value -> serde_json::Value conversion
    // For Phase 2, we'll stub this out - it will be implemented in a future update
    globals.set(
        "get_component",
        lua.create_function(|lua, (_entity_bits, component_name): (i64, String)| {
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
            with_active_commands(|commands| {
                commands.remove_component(entity_bits, component_name)
            })
        })?,
    )?;

    Ok(())
}
