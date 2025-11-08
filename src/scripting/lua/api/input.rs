//! # Input API
//!
//! This module provides functions for reading keyboard and mouse input state.
//!
//! ## Features
//!
//! - **Keyboard input** - Check key states (pressed, just pressed, just released)
//! - **Mouse buttons** - Check mouse button states
//! - **Mouse position** - Get screen-space cursor position
//! - **Mouse delta** - Get frame-to-frame mouse movement
//! - **Scroll wheel** - Get scroll delta
//!
//! ## Key Names
//!
//! Key names are strings like: "W", "A", "S", "D", "Space", "Escape", "Return", etc.
//!
//! ## Mouse Buttons
//!
//! Mouse buttons are numbered:
//! - **0** - Left button
//! - **1** - Right button
//! - **2** - Middle button

use mlua::{Lua, Result as LuaResult};

use crate::scripting::lua::guards::with_active_input_state;

/// Registers input API functions with the Lua runtime.
///
/// This function exposes keyboard and mouse input functions to Lua scripts.
///
/// ## Available Functions
///
/// ### Keyboard
/// - `is_key_pressed(key)` - Returns true if key is currently held down
/// - `is_key_just_pressed(key)` - Returns true if key was pressed this frame
/// - `is_key_just_released(key)` - Returns true if key was released this frame
///
/// ### Mouse Buttons
/// - `is_mouse_button_pressed(button)` - Returns true if button is held
/// - `is_mouse_button_just_pressed(button)` - Returns true if pressed this frame
/// - `is_mouse_button_just_released(button)` - Returns true if released this frame
///
/// ### Mouse Movement
/// - `get_mouse_position()` - Returns {x, y} table of cursor position
/// - `get_mouse_delta()` - Returns {x, y} table of movement since last frame
/// - `get_mouse_scroll_delta()` - Returns {x, y} table of scroll wheel delta
///
/// # Example Lua usage
///
/// ```lua
/// function update(self_entity, dt)
///     -- Keyboard movement
///     if is_key_pressed("W") then
///         translate(self_entity, 0, 0, -dt * 5)
///     end
///     if is_key_just_pressed("Space") then
///         log_info("Jump!")
///     end
///
///     -- Mouse look
///     local delta = get_mouse_delta()
///     if is_mouse_button_pressed(1) then  -- Right button
///         local yaw = get_f64("yaw", 0) + delta.x * 0.002
///         set_f64("yaw", yaw)
///         set_rotation(self_entity, yaw, 0, 0)
///     end
///
///     -- Scroll zoom
///     local scroll = get_mouse_scroll_delta()
///     if scroll.y ~= 0 then
///         local zoom = get_f64("zoom", 10) - scroll.y
///         set_f64("zoom", zoom)
///     end
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
pub(crate) fn register_input_api(lua: &Lua) -> LuaResult<()> {
    let globals = lua.globals();

    // is_key_pressed(key: string) -> boolean
    globals.set(
        "is_key_pressed",
        lua.create_function(|_, key: String| {
            with_active_input_state(|input_state| Ok(input_state.is_key_pressed(&key)))
        })?,
    )?;

    // is_key_just_pressed(key: string) -> boolean
    globals.set(
        "is_key_just_pressed",
        lua.create_function(|_, key: String| {
            with_active_input_state(|input_state| Ok(input_state.is_key_just_pressed(&key)))
        })?,
    )?;

    // is_key_just_released(key: string) -> boolean
    globals.set(
        "is_key_just_released",
        lua.create_function(|_, key: String| {
            with_active_input_state(|input_state| Ok(input_state.is_key_just_released(&key)))
        })?,
    )?;

    // is_mouse_button_pressed(button: number) -> boolean
    globals.set(
        "is_mouse_button_pressed",
        lua.create_function(|_, button: i64| {
            with_active_input_state(|input_state| {
                Ok(input_state.is_mouse_button_pressed(button as u32))
            })
        })?,
    )?;

    // is_mouse_button_just_pressed(button: number) -> boolean
    globals.set(
        "is_mouse_button_just_pressed",
        lua.create_function(|_, button: i64| {
            with_active_input_state(|input_state| {
                Ok(input_state.is_mouse_button_just_pressed(button as u32))
            })
        })?,
    )?;

    // is_mouse_button_just_released(button: number) -> boolean
    globals.set(
        "is_mouse_button_just_released",
        lua.create_function(|_, button: i64| {
            with_active_input_state(|input_state| {
                Ok(input_state.is_mouse_button_just_released(button as u32))
            })
        })?,
    )?;

    // get_mouse_position() -> {x: number, y: number}
    globals.set(
        "get_mouse_position",
        lua.create_function(|lua, ()| {
            with_active_input_state(|input_state| {
                let pos = input_state.mouse_position();
                let table = lua.create_table()?;
                table.set("x", pos.x as f64)?;
                table.set("y", pos.y as f64)?;
                Ok(table)
            })
        })?,
    )?;

    // get_mouse_delta() -> {x: number, y: number}
    globals.set(
        "get_mouse_delta",
        lua.create_function(|lua, ()| {
            with_active_input_state(|input_state| {
                let delta = input_state.mouse_delta();
                let table = lua.create_table()?;
                table.set("x", delta.x as f64)?;
                table.set("y", delta.y as f64)?;
                Ok(table)
            })
        })?,
    )?;

    // get_mouse_scroll_delta() -> {x: number, y: number}
    globals.set(
        "get_mouse_scroll_delta",
        lua.create_function(|lua, ()| {
            with_active_input_state(|input_state| {
                let scroll = input_state.scroll_delta();
                let table = lua.create_table()?;
                table.set("x", scroll.x as f64)?;
                table.set("y", scroll.y as f64)?;
                Ok(table)
            })
        })?,
    )?;

    Ok(())
}
