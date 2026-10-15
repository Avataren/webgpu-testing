//! # Post-processing API
//!
//! Lets scripts change post-processing at run time (a damage vignette, a bloom pulse, a
//! desaturated death screen). Post-processing lives on post-process volume entities, so
//! this edits one of them with the same settings the engine tools' `postprocess_set`
//! takes.
//!
//! Changes are queued like other script commands and applied before the frame is drawn.
//! They are undone when play mode stops, along with a volume a script created.
//!
//! `set_environment` changes the scene's environment the same way: its ambient light,
//! background colour and HDR sky (a night falling, a path-traced look that wants a darker
//! sky than the raster one).

use mlua::{Lua, LuaSerdeExt, Result as LuaResult, Value as LuaValue};
use serde_json::Value;

use crate::scripting::lua::guards::with_active_commands;

/// Registers `set_postprocess` and `set_environment`:
///
/// - `set_postprocess(settings)` - change the scene's infinite post-process volume
///   (created if there is none): `bloom = { intensity = 1.2 }`, `vignette = { enabled =
///   true, intensity = 0.4 }`, `tonemapping = "agx"`, `color = { saturation = 0.2 }`, ...
/// - `set_postprocess(entity, settings)` - change that post-process volume entity
///
/// Only the settings given change. A `volume = { priority = .., weight = .. }` field edits
/// the volume itself. Unknown setting names are an error.
///
/// - `set_environment(settings)` - change the scene's environment: `ambient_intensity`,
///   `clear_color` (`{r, g, b[, a]}` or `"#rrggbb"`), `hdr = { enabled, intensity, path }`
pub(crate) fn register_postprocess_api(lua: &Lua) -> LuaResult<()> {
    lua.globals().set(
        "set_postprocess",
        lua.create_function(|lua, (first, second): (LuaValue, Option<LuaValue>)| {
            let (entity, settings) = match (first, second) {
                (LuaValue::Integer(entity), Some(settings)) => (Some(entity), settings),
                (LuaValue::Number(entity), Some(settings)) if entity.fract() == 0.0 => {
                    (Some(entity as i64), settings)
                }
                (settings @ LuaValue::Table(_), None) => (None, settings),
                _ => {
                    return Err(mlua::Error::RuntimeError(
                        "set_postprocess expects (settings) or (volume_entity, settings)".into(),
                    ))
                }
            };
            let patch: Value = match lua.from_value(settings)? {
                object @ Value::Object(_) => object,
                // An empty Lua table converts to an empty array.
                Value::Array(items) if items.is_empty() => Value::Object(Default::default()),
                _ => {
                    return Err(mlua::Error::RuntimeError(
                        "set_postprocess expects a table of settings, e.g. { bloom = { intensity = 1 } }"
                            .into(),
                    ))
                }
            };
            crate::remote::handlers::post_process_patch(&patch)
                .map_err(mlua::Error::RuntimeError)?;
            with_active_commands(|commands| commands.post_process(entity, patch))
        })?,
    )?;
    lua.globals().set(
        "set_environment",
        lua.create_function(|lua, settings: LuaValue| {
            let params: Value = match lua.from_value(settings)? {
                object @ Value::Object(_) => object,
                _ => {
                    return Err(mlua::Error::RuntimeError(
                        "set_environment expects a table, e.g. { ambient_intensity = 0.05 }"
                            .into(),
                    ))
                }
            };
            let patch = crate::remote::handlers::environment_patch(&params)
                .map_err(mlua::Error::RuntimeError)?;
            with_active_commands(|commands| {
                commands.environment(patch);
                Ok(())
            })
        })?,
    )
}
