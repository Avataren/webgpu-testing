use mlua::{Lua, Result as LuaResult, Value as LuaValue};

use crate::scripting::lua::guards::{with_active_entity, with_active_state};

/// Convert a Lua value to a serde_json::Value for storage.
fn lua_to_json(lua: &Lua, value: LuaValue) -> LuaResult<serde_json::Value> {
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
                    arr.push(lua_to_json(lua, val)?);
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
                    map.insert(key, lua_to_json(lua, v)?);
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

/// Convert a serde_json::Value to a Lua value.
fn json_to_lua(lua: &Lua, value: &serde_json::Value) -> LuaResult<LuaValue> {
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
                table.raw_set(i + 1, json_to_lua(lua, val)?)?;
            }
            Ok(LuaValue::Table(table))
        }
        serde_json::Value::Object(map) => {
            let table = lua.create_table()?;
            for (k, v) in map.iter() {
                table.raw_set(k.as_str(), json_to_lua(lua, v)?)?;
            }
            Ok(LuaValue::Table(table))
        }
    }
}

/// Register state management API functions with the Lua runtime.
pub(crate) fn register_state_api(lua: &Lua) -> LuaResult<()> {
    let globals = lua.globals();

    // set_state(key: string, value: any)
    globals.set(
        "set_state",
        lua.create_function(|lua, (key, value): (String, LuaValue)| {
            let json_value = lua_to_json(lua, value)?;
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
            let default_json = lua_to_json(lua, default)?;
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
            json_to_lua(lua, &result)
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
                Some(val) => json_to_lua(lua, &val),
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
                        serde_json::Value::Number(
                            serde_json::Number::from_f64(value).ok_or_else(|| {
                                mlua::Error::RuntimeError("Number is not finite".to_string())
                            })?,
                        ),
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
                with_active_state(|state| {
                    match state.get(&(entity, key.clone())) {
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
                    }
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
            let json_value = lua_to_json(lua, value)?;
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
            let default_json = lua_to_json(lua, default)?;
            let result = with_active_state(|state| match state.get(&(entity, key)) {
                Some(value) => Ok(value.clone()),
                None => Ok(default_json),
            })?;
            json_to_lua(lua, &result)
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
                Some(val) => json_to_lua(lua, &val),
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
                    serde_json::Value::Number(
                        serde_json::Number::from_f64(value).ok_or_else(|| {
                            mlua::Error::RuntimeError("Number is not finite".to_string())
                        })?,
                    ),
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
