use mlua::{Lua, LuaSerdeExt, Result as LuaResult, Value as LuaValue};

use crate::scripting::lua::guards::{
    get_active_entity, with_active_commands, with_active_event_queue,
};
use crate::scripting::lua::types::ScriptEvent;

/// Register events API functions with the Lua runtime.
pub(crate) fn register_events_api(lua: &Lua) -> LuaResult<()> {
    let globals = lua.globals();

    // emit_event(event_name: string, data: any)
    globals.set(
        "emit_event",
        lua.create_function(|lua, (event_name, data): (String, LuaValue)| {
            // Use mlua's built-in serde integration
            let json_data: serde_json::Value = lua.from_value(data)?;
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
