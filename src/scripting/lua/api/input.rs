use mlua::{Lua, Result as LuaResult};

use crate::scripting::lua::guards::with_active_input_state;

/// Register input API functions with the Lua runtime.
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
