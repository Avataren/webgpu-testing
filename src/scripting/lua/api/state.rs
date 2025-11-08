//! # State Management API
//!
//! This module provides persistent state storage for Lua scripts.
//! State is stored per-entity and persists across script reloads.
//!
//! ## Features
//!
//! - **Generic state storage** - Store any Lua value (tables, numbers, strings, booleans)
//! - **Typed accessors** - Optimized functions for specific types (f64, bool, string)
//! - **Per-entity state** - Operate on the current entity or any arbitrary entity
//! - **Default values** - Graceful fallback when keys don't exist
//!
//! ## Performance
//!
//! For simple types, use the typed functions (`set_f64`, `get_f64`, etc.) as they
//! are more efficient than the generic `set_state`/`get_state`.

use mlua::{Lua, LuaSerdeExt, Result as LuaResult, Value as LuaValue};

use crate::scripting::lua::guards::{with_active_entity, with_active_state};

/// Registers state management API functions with the Lua runtime.
///
/// This function exposes state storage and retrieval functions to Lua scripts.
/// State is stored as JSON values and persists across script reloads.
///
/// ## Current Entity Functions
///
/// - `set_state(key, value)` - Store any value for current entity
/// - `get_state(key, default)` - Retrieve with default fallback
/// - `try_get_state(key)` - Return nil if not found
/// - `set_f64(key, number)` - Optimized number storage
/// - `get_f64(key)` - Retrieve number (panics if missing)
/// - `set_bool(key, boolean)` - Store boolean
/// - `get_bool(key, default)` - Retrieve boolean with default
/// - `set_string(key, string)` - Store string
/// - `get_string(key, default)` - Retrieve string with default
///
/// ## Arbitrary Entity Functions
///
/// - `set_state_for(entity, key, value)`
/// - `get_state_for(entity, key, default)`
/// - `try_get_state_for(entity, key)`
/// - `set_f64_for(entity, key, number)`
/// - `get_f64_for(entity, key, default)`
///
/// # Example Lua usage
///
/// ```lua
/// -- Store various types
/// set_f64("speed", 5.0)
/// set_bool("active", true)
/// set_state("config", {x = 10, y = 20})
///
/// -- Retrieve with defaults
/// local speed = get_f64("speed")
/// local enabled = get_bool("enabled", false)
/// local config = get_state("config", {x = 0, y = 0})
///
/// -- Operate on other entities
/// set_state_for(other_entity, "color", {r = 1.0, g = 0.0, b = 0.0})
/// ```
///
/// # Arguments
///
/// * `lua` - The Lua runtime to register functions with
///
/// # Returns
///
/// `Ok(())` on success, or a Lua error if registration fails.
pub(crate) fn register_state_api(lua: &Lua) -> LuaResult<()> {
    let globals = lua.globals();

    // set_state(key: string, value: any)
    globals.set(
        "set_state",
        lua.create_function(|lua, (key, value): (String, LuaValue)| {
            // Use mlua's built-in serde integration for conversion
            let json_value: serde_json::Value = lua.from_value(value)?;
            with_active_entity(|entity| {
                with_active_state(|state| {
                    state.insert((entity, key), json_value);
                    Ok(())
                })
            })
        })?,
    )?;

    // get_state(key: string, default: any) -> any
    globals.set(
        "get_state",
        lua.create_function(|lua, (key, default): (String, LuaValue)| {
            let default_json: serde_json::Value = lua.from_value(default)?;
            let result = with_active_entity(|entity| {
                with_active_state(|state| {
                    let entry_key = (entity, key.clone());
                    match state.get(&entry_key) {
                        Some(value) => Ok(value.clone()),
                        None => {
                            state.insert(entry_key, default_json.clone());
                            Ok(default_json.clone())
                        }
                    }
                })
            })?;
            // Convert serde_json::Value back to Lua value using mlua's serde integration
            lua.to_value(&result)
        })?,
    )?;

    // try_get_state(key: string) -> any | nil
    globals.set(
        "try_get_state",
        lua.create_function(|lua, key: String| {
            let result = with_active_entity(|entity| {
                with_active_state(|state| {
                    let entry_key = (entity, key);
                    match state.get(&entry_key) {
                        Some(value) => Ok(Some(value.clone())),
                        None => Ok(None),
                    }
                })
            })?;
            match result {
                Some(val) => lua.to_value(&val),
                None => Ok(LuaValue::Nil),
            }
        })?,
    )?;

    // set_f64(key: string, value: number)
    globals.set(
        "set_f64",
        lua.create_function(|_lua, (key, value): (String, f64)| {
            with_active_entity(|entity| {
                with_active_state(|state| {
                    state.insert(
                        (entity, key),
                        serde_json::Value::Number(serde_json::Number::from_f64(value).ok_or_else(
                            || mlua::Error::RuntimeError("Number is not finite".to_string()),
                        )?),
                    );
                    Ok(())
                })
            })
        })?,
    )?;

    // get_f64(key: string) -> number
    globals.set(
        "get_f64",
        lua.create_function(|_lua, key: String| {
            with_active_entity(|entity| {
                with_active_state(|state| match state.get(&(entity, key.clone())) {
                    Some(serde_json::Value::Number(n)) => {
                        if let Some(f) = n.as_f64() {
                            Ok(f)
                        } else {
                            Err(mlua::Error::RuntimeError(
                                "Value is not a number".to_string(),
                            ))
                        }
                    }
                    Some(_) => Err(mlua::Error::RuntimeError(
                        "Value is not a number".to_string(),
                    )),
                    None => Err(mlua::Error::RuntimeError(format!(
                        "State key not found: {}",
                        key
                    ))),
                })
            })
        })?,
    )?;

    // set_bool(key: string, value: boolean)
    globals.set(
        "set_bool",
        lua.create_function(|_lua, (key, value): (String, bool)| {
            with_active_entity(|entity| {
                with_active_state(|state| {
                    state.insert((entity, key), serde_json::Value::Bool(value));
                    Ok(())
                })
            })
        })?,
    )?;

    // get_bool(key: string, default: boolean) -> boolean
    globals.set(
        "get_bool",
        lua.create_function(|_lua, (key, default): (String, bool)| {
            with_active_entity(|entity| {
                with_active_state(|state| match state.get(&(entity, key)) {
                    Some(serde_json::Value::Bool(b)) => Ok(*b),
                    Some(_) | None => Ok(default),
                })
            })
        })?,
    )?;

    // set_string(key: string, value: string)
    globals.set(
        "set_string",
        lua.create_function(|_lua, (key, value): (String, String)| {
            with_active_entity(|entity| {
                with_active_state(|state| {
                    state.insert((entity, key), serde_json::Value::String(value));
                    Ok(())
                })
            })
        })?,
    )?;

    // get_string(key: string, default: string) -> string
    globals.set(
        "get_string",
        lua.create_function(|_lua, (key, default): (String, String)| {
            with_active_entity(|entity| {
                with_active_state(|state| match state.get(&(entity, key)) {
                    Some(serde_json::Value::String(s)) => Ok(s.clone()),
                    Some(_) | None => Ok(default),
                })
            })
        })?,
    )?;

    // For-entity versions (operate on arbitrary entities)

    // set_state_for(entity: number, key: string, value: any)
    globals.set(
        "set_state_for",
        lua.create_function(|lua, (entity, key, value): (i64, String, LuaValue)| {
            let json_value: serde_json::Value = lua.from_value(value)?;
            with_active_state(|state| {
                state.insert((entity, key), json_value);
                Ok(())
            })
        })?,
    )?;

    // get_state_for(entity: number, key: string, default: any) -> any
    globals.set(
        "get_state_for",
        lua.create_function(|lua, (entity, key, default): (i64, String, LuaValue)| {
            let default_json: serde_json::Value = lua.from_value(default)?;
            let result = with_active_state(|state| match state.get(&(entity, key)) {
                Some(value) => Ok(value.clone()),
                None => Ok(default_json),
            })?;
            lua.to_value(&result)
        })?,
    )?;

    // try_get_state_for(entity: number, key: string) -> any | nil
    globals.set(
        "try_get_state_for",
        lua.create_function(|lua, (entity, key): (i64, String)| {
            let result = with_active_state(|state| match state.get(&(entity, key)) {
                Some(value) => Ok(Some(value.clone())),
                None => Ok(None),
            })?;
            match result {
                Some(val) => lua.to_value(&val),
                None => Ok(LuaValue::Nil),
            }
        })?,
    )?;

    // set_f64_for(entity: number, key: string, value: number)
    globals.set(
        "set_f64_for",
        lua.create_function(|_lua, (entity, key, value): (i64, String, f64)| {
            with_active_state(|state| {
                state.insert(
                    (entity, key),
                    serde_json::Value::Number(serde_json::Number::from_f64(value).ok_or_else(
                        || mlua::Error::RuntimeError("Number is not finite".to_string()),
                    )?),
                );
                Ok(())
            })
        })?,
    )?;

    // get_f64_for(entity: number, key: string, default: number) -> number
    globals.set(
        "get_f64_for",
        lua.create_function(|_lua, (entity, key, default): (i64, String, f64)| {
            with_active_state(|state| match state.get(&(entity, key)) {
                Some(serde_json::Value::Number(n)) => {
                    if let Some(f) = n.as_f64() {
                        Ok(f)
                    } else {
                        Ok(default)
                    }
                }
                Some(_) | None => Ok(default),
            })
        })?,
    )?;

    Ok(())
}
