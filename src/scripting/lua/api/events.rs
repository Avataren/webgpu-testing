use mlua::{Lua, Result as LuaResult, Value as LuaValue};

use crate::scripting::lua::guards::{get_active_entity, with_active_commands, with_active_event_queue};
use crate::scripting::lua::types::ScriptEvent;

/// Convert Lua value to serde_json::Value for event data.
fn lua_to_json_for_event(lua: &Lua, value: LuaValue) -> LuaResult<serde_json::Value> {
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
                    arr.push(lua_to_json_for_event(lua, val)?);
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
                    map.insert(key, lua_to_json_for_event(lua, v)?);
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

/// Register events API functions with the Lua runtime.
pub(crate) fn register_events_api(lua: &Lua) -> LuaResult<()> {
    let globals = lua.globals();

    // emit_event(event_name: string, data: any)
    globals.set(
        "emit_event",
        lua.create_function(|lua, (event_name, data): (String, LuaValue)| {
            let json_data = lua_to_json_for_event(lua, data)?;
            with_active_event_queue(|queue| {
                queue.push(ScriptEvent {
                    name: event_name,
                    data: json_data,
                });
                Ok(())
            })
        })?,
    )?;

    // subscribe_event(event_name: string, callback_name: string)
    globals.set(
        "subscribe_event",
        lua.create_function(|_, (event_name, callback_name): (String, String)| {
            // Get the current entity from the active context
            let entity_bits = get_active_entity()?;

            with_active_commands(|commands| {
                commands.subscribe_event(entity_bits, event_name, callback_name)
            })
        })?,
    )?;

    // unsubscribe_event(event_name: string)
    globals.set(
        "unsubscribe_event",
        lua.create_function(|_, event_name: String| {
            // Get the current entity from the active context
            let entity_bits = get_active_entity()?;

            with_active_commands(|commands| commands.unsubscribe_event(entity_bits, event_name))
        })?,
    )?;

    Ok(())
}
