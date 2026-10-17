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
//! - `path_traced()` is true while the player has path tracing (and ray tracing) on and the
//!   GPU can run it, so a game can swap lighting it fakes for the raster renderer (glow
//!   planes, fill lights) for emissive surfaces the path tracer lights the scene with
//! - `ray_traced_gi()` is true while ray-traced global illumination lights the scene: the
//!   player has it on, the GPU can run it and path tracing (which wins) is off. Bounce
//!   and sky light then arrive traced, so fill lights faked for the raster lighting can go
//! - `graphics_settings()` returns the settings as the page shows them (`anti_aliasing`,
//!   `render_scale`, `upscaler`, `ray_tracing` (off: every effect raster, light probes
//!   baked), `ray_traced_shadows`, `reflections`, `ambient_occlusion`,
//!   `global_illumination`, `path_tracing`, `vsync`, `shadow_atlas_size`,
//!   `shadow_updates_per_frame`, `shadow_cascades`, `shadow_cascade_size`, ...) plus
//!   `capabilities`
//!   (`ray_tracing`, `path_tracing`, `upscaling`), e.g. for a benchmark's report
//! - `set_graphics_settings(fields[, { save = false }])` changes the named settings like
//!   the page does (unknown names and values are errors). They are saved as the player's
//!   choice unless `save = false`, which keeps the change to this run: a benchmark trying
//!   configurations sets the player's own settings back when it is done

use mlua::{Lua, LuaSerdeExt, Result as LuaResult, Table, Value};

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
        "path_traced",
        lua.create_function(|_, ()| {
            let state = graphics_settings();
            Ok(state.settings.ray_tracing
                && state.settings.path_tracing
                && state.capabilities.path_tracing)
        })?,
    )?;
    globals.set(
        "ray_traced_gi",
        lua.create_function(|_, ()| {
            let state = graphics_settings();
            Ok(state.settings.ray_tracing
                && state.settings.global_illumination
                    == crate::graphics_settings::GlobalIllumination::RayTraced
                && !state.settings.path_tracing
                && state.capabilities.path_tracing)
        })?,
    )?;
    globals.set(
        "graphics_settings",
        lua.create_function(|lua, ()| {
            let state = graphics_settings();
            let Value::Table(table) = lua.to_value(&state.settings)? else {
                return Err(mlua::Error::RuntimeError(
                    "graphics settings are not a table".into(),
                ));
            };
            let capabilities = lua.create_table()?;
            capabilities.set("ray_tracing", state.capabilities.ray_tracing)?;
            capabilities.set("path_tracing", state.capabilities.path_tracing)?;
            capabilities.set("upscaling", state.capabilities.upscaling)?;
            table.set("capabilities", capabilities)?;
            Ok(table)
        })?,
    )?;
    globals.set(
        "set_graphics_settings",
        lua.create_function(|lua, (fields, options): (Value, Option<Table>)| {
            let patch: serde_json::Value = lua.from_value(fields)?;
            let Some(patch) = patch.as_object() else {
                return Err(mlua::Error::RuntimeError(
                    "set_graphics_settings expects a table of settings".into(),
                ));
            };
            let save = match &options {
                Some(options) => options.get::<Option<bool>>("save")?.unwrap_or(true),
                None => true,
            };
            let mut state = graphics_settings();
            let settings = state
                .settings
                .patched(patch)
                .map_err(mlua::Error::RuntimeError)?;
            if save {
                state.set(settings);
            } else {
                state.set_unsaved(settings);
            }
            Ok(())
        })?,
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
