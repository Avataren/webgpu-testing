//! # Events API
//!
//! This module provides an event system for inter-script communication.
//!
//! ## Features
//!
//! - **Event emission** - Broadcast events with arbitrary data to all subscribers
//! - **Event subscription** - Register callbacks to handle specific events
//! - **JSON-serializable data** - Event data can be any Lua value (tables, strings, numbers)
//!
//! ## Event Flow
//!
//! 1. Scripts emit events using `emit_event(name, data)`
//! 2. Events are queued and distributed to subscribers
//! 3. Subscribed scripts receive events via their callback functions
//!
//! ## Known Limitations
//!
//! Event data must be JSON-serializable for serde conversion to work.

use mlua::{Lua, LuaSerdeExt, Result as LuaResult, Value as LuaValue};

use crate::scripting::lua::guards::{
    get_active_entity, with_active_commands, with_active_event_queue,
};
use crate::scripting::lua::types::ScriptEvent;

/// Registers events API functions with the Lua runtime.
///
/// This function exposes event emission and subscription functions to Lua scripts.
///
/// ## Available Functions
///
/// - `emit_event(event_name, data)` - Emit event with JSON-serializable data
/// - `subscribe_event(event_name, callback_name)` - Subscribe to event (current entity)
/// - `unsubscribe_event(event_name)` - Unsubscribe from event (current entity)
///
/// # Example Lua usage
///
/// ```lua
/// -- Emitter script
/// function update(self_entity, dt)
///     if is_key_just_pressed("Space") then
///         emit_event("player_jumped", {
///             height = 10,
///             entity = self_entity
///         })
///     end
/// end
///
/// -- Listener script
/// function on_created(self_entity)
///     subscribe_event("player_jumped", "on_player_jumped")
/// end
///
/// function on_player_jumped(event_data)
///     log_info("Player jumped to height: " .. event_data.height)
/// end
///
/// -- Cleanup when done
/// function on_destroyed(self_entity)
///     unsubscribe_event("player_jumped")
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
