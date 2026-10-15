//! # Settings page API
//!
//! The engine's graphics settings page (display, anti-aliasing, upscaling, ray tracing,
//! FPS readout), which Escape opens and closes by default:
//!
//! - `open_settings()` / `close_settings()` show or hide it, e.g. from a pause menu
//! - `settings_open()` is true while it shows (pause the game then if it should)
//! - `set_settings_hotkey(enabled)` turns Escape's toggle off for games that use Escape
//!   themselves (and back on); the page stays available through `open_settings()`
//! - `settings_hotkey()` is whether Escape toggles it (to restore it in `on_destroyed`)

use mlua::{Lua, Result as LuaResult};

use crate::graphics_settings::graphics_settings;

pub(crate) fn register_settings_api(lua: &Lua) -> LuaResult<()> {
    let globals = lua.globals();
    globals.set(
        "open_settings",
        lua.create_function(|_, ()| {
            graphics_settings().open = true;
            Ok(())
        })?,
    )?;
    globals.set(
        "close_settings",
        lua.create_function(|_, ()| {
            graphics_settings().open = false;
            Ok(())
        })?,
    )?;
    globals.set(
        "settings_open",
        lua.create_function(|_, ()| Ok(graphics_settings().open))?,
    )?;
    globals.set(
        "settings_hotkey",
        lua.create_function(|_, ()| Ok(graphics_settings().hotkey_enabled))?,
    )?;
    globals.set(
        "set_settings_hotkey",
        lua.create_function(|_, enabled: bool| {
            graphics_settings().hotkey_enabled = enabled;
            Ok(())
        })?,
    )?;
    Ok(())
}
