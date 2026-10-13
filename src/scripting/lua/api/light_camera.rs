//! # Light and Camera API
//!
//! Lets scripts create lights and cameras and change them at runtime, e.g. a flash
//! when something explodes or a cut to another camera. Lights and cameras sit at
//! their entity's transform and point along its -Z, so `set_translation` and
//! `look_at` aim them.
//!
//! Changes are queued like other script commands and applied before the frame is
//! drawn. The fields are those of the engine tools' `entity_update`.

use mlua::{Lua, LuaSerdeExt, Result as LuaResult, Value as LuaValue};
use serde_json::{json, Map, Value};

use crate::scripting::lua::commands::AssetCommand;
use crate::scripting::lua::guards::with_active_commands;

const LIGHT_FIELDS: &[&str] = &[
    "kind",
    "color",
    "intensity",
    "range",
    "shadow_size",
    "inner_angle_deg",
    "outer_angle_deg",
    "cast_shadow",
];

const CAMERA_FIELDS: &[&str] = &[
    "active",
    "projection",
    "fov_deg",
    "near",
    "far",
    "left",
    "right",
    "bottom",
    "top",
];

/// Convert an optional Lua table of fields, rejecting names not in `allowed`.
fn fields(
    lua: &Lua,
    value: LuaValue,
    allowed: &[&str],
    what: &str,
) -> LuaResult<Map<String, Value>> {
    let value: Value = match value {
        LuaValue::Nil => return Ok(Map::new()),
        value => lua.from_value(value)?,
    };
    let fields = match value {
        Value::Object(fields) => fields,
        // An empty Lua table converts to an empty array.
        Value::Array(items) if items.is_empty() => Map::new(),
        _ => {
            return Err(mlua::Error::RuntimeError(format!(
                "expected a table of {what} fields"
            )))
        }
    };
    if let Some(unknown) = fields.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(mlua::Error::RuntimeError(format!(
            "unknown {what} field {unknown:?}; use {}",
            allowed.join(", ")
        )));
    }
    Ok(fields)
}

fn light_name(kind: &str) -> String {
    match kind {
        "point" => "Point Light".into(),
        "directional" => "Directional Light".into(),
        "spot" => "Spot Light".into(),
        other => other.into(),
    }
}

/// Registers the light and camera functions:
///
/// - `spawn_light(kind[, fields][, name])` - spawn a `"point"`, `"directional"` or
///   `"spot"` light; returns its handle
/// - `set_light(entity, fields)` - change a light: `color`, `intensity`, `range`,
///   `shadow_size`, `inner_angle_deg`, `outer_angle_deg`, `cast_shadow`; `kind` adds a
///   light or switches its kind
/// - `spawn_camera([fields][, name])` - spawn a camera; returns its handle
/// - `set_camera(entity, fields)` - change or add a camera: `projection`
///   (`"perspective"`/`"orthographic"`), `fov_deg`, `near`, `far`, `left`, `right`,
///   `bottom`, `top`, and `active = true` to view the game through it
/// - `set_active_camera(entity)` - view the game through the entity's camera
pub(crate) fn register_light_camera_api(lua: &Lua) -> LuaResult<()> {
    let globals = lua.globals();

    globals.set(
        "spawn_light",
        lua.create_function(
            |lua, (kind, light, name): (String, LuaValue, Option<String>)| {
                let mut light = fields(lua, light, LIGHT_FIELDS, "light")?;
                if !["point", "directional", "spot"].contains(&kind.as_str()) {
                    return Err(mlua::Error::RuntimeError(format!(
                        "unknown light kind {kind:?}; use point, directional or spot"
                    )));
                }
                let name = name.unwrap_or_else(|| light_name(&kind));
                light.insert("kind".into(), json!(kind));
                with_active_commands(|commands| {
                    let entity = commands.spawn_entity(Some(name));
                    commands.asset_command(entity, AssetCommand::Light(Value::Object(light)))?;
                    Ok(entity)
                })
            },
        )?,
    )?;

    globals.set(
        "set_light",
        lua.create_function(|lua, (entity, light): (i64, LuaValue)| {
            let light = fields(lua, light, LIGHT_FIELDS, "light")?;
            with_active_commands(|commands| {
                commands.asset_command(entity, AssetCommand::Light(Value::Object(light)))
            })
        })?,
    )?;

    globals.set(
        "spawn_camera",
        lua.create_function(|lua, (camera, name): (LuaValue, Option<String>)| {
            let camera = fields(lua, camera, CAMERA_FIELDS, "camera")?;
            with_active_commands(|commands| {
                let entity = commands.spawn_entity(Some(name.unwrap_or_else(|| "Camera".into())));
                commands.asset_command(entity, AssetCommand::Camera(Value::Object(camera)))?;
                Ok(entity)
            })
        })?,
    )?;

    globals.set(
        "set_camera",
        lua.create_function(|lua, (entity, camera): (i64, LuaValue)| {
            let camera = fields(lua, camera, CAMERA_FIELDS, "camera")?;
            with_active_commands(|commands| {
                commands.asset_command(entity, AssetCommand::Camera(Value::Object(camera)))
            })
        })?,
    )?;

    globals.set(
        "set_active_camera",
        lua.create_function(|_, entity: i64| {
            with_active_commands(|commands| {
                commands.asset_command(entity, AssetCommand::ActiveCamera)
            })
        })?,
    )?;

    Ok(())
}
