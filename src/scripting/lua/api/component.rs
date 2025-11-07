use hecs::Entity;
use log::{error, warn};
use mlua::{Lua, Result as LuaResult, Value as LuaValue};

use crate::scripting::component_registry::ComponentRegistryError;
use crate::scripting::lua::guards::{with_active_commands, with_active_registry, with_active_world};

/// Convert a Lua value to serde_json::Value for components.
fn lua_to_json_for_component(lua: &Lua, value: LuaValue) -> LuaResult<serde_json::Value> {
    match value {
        LuaValue::Nil => Ok(serde_json::Value::Null),
        LuaValue::Boolean(b) => Ok(serde_json::Value::Bool(b)),
        LuaValue::Integer(i) => Ok(serde_json::Value::Number(i.into())),
        LuaValue::Number(n) => {
            if let Some(num) = serde_json::Number::from_f64(n) {
                Ok(serde_json::Value::Number(num))
            } else {
                Err(mlua::Error::RuntimeError(
                    "Number is not finite".to_string(),
                ))
            }
        }
        LuaValue::String(s) => Ok(serde_json::Value::String(s.to_str()?.to_string())),
        LuaValue::Table(t) => {
            // Check if it's an array or an object
            let len = t.raw_len();
            if len > 0 {
                // Treat as array
                let mut arr = Vec::new();
                for i in 1..=len {
                    let val: LuaValue = t.raw_get(i)?;
                    arr.push(lua_to_json_for_component(lua, val)?);
                }
                Ok(serde_json::Value::Array(arr))
            } else {
                // Treat as object
                let mut map = serde_json::Map::new();
                for pair in t.pairs::<LuaValue, LuaValue>() {
                    let (k, v) = pair?;
                    let key = match k {
                        LuaValue::String(s) => s.to_str()?.to_string(),
                        LuaValue::Integer(i) => i.to_string(),
                        LuaValue::Number(n) => n.to_string(),
                        _ => {
                            return Err(mlua::Error::RuntimeError(
                                "Table keys must be strings or numbers".to_string(),
                            ))
                        }
                    };
                    map.insert(key, lua_to_json_for_component(lua, v)?);
                }
                Ok(serde_json::Value::Object(map))
            }
        }
        _ => Err(mlua::Error::RuntimeError(format!(
            "Unsupported value type: {}",
            value.type_name()
        ))),
    }
}

/// Convert a serde_json::Value from component to Lua value.
fn json_to_lua_for_component(lua: &Lua, value: &serde_json::Value) -> LuaResult<LuaValue> {
    match value {
        serde_json::Value::Null => Ok(LuaValue::Nil),
        serde_json::Value::Bool(b) => Ok(LuaValue::Boolean(*b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(LuaValue::Integer(i))
            } else if let Some(f) = n.as_f64() {
                Ok(LuaValue::Number(f))
            } else {
                Err(mlua::Error::RuntimeError("Invalid number".to_string()))
            }
        }
        serde_json::Value::String(s) => Ok(LuaValue::String(lua.create_string(s)?)),
        serde_json::Value::Array(arr) => {
            let table = lua.create_table()?;
            for (i, val) in arr.iter().enumerate() {
                table.raw_set(i + 1, json_to_lua_for_component(lua, val)?)?;
            }
            Ok(LuaValue::Table(table))
        }
        serde_json::Value::Object(map) => {
            let table = lua.create_table()?;
            for (k, v) in map.iter() {
                table.raw_set(k.as_str(), json_to_lua_for_component(lua, v)?)?;
            }
            Ok(LuaValue::Table(table))
        }
    }
}

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
                let json_value = lua_to_json_for_component(lua, value)?;
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
                let json_value = lua_to_json_for_component(lua, value)?;
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
