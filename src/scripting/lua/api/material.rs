//! # Mesh and Material API
//!
//! Lets scripts give entities primitive meshes and change their materials at runtime,
//! e.g. to recolor a brick when it is hit or to spawn effects.
//!
//! Changes are queued like other script commands and applied before the frame is
//! drawn. Changing the material of an entity that shares it with others gives that
//! entity its own copy first, so only it changes.

use mlua::{Lua, LuaSerdeExt, Result as LuaResult, Value as LuaValue};
use serde_json::json;

use super::voxel::LuaGeneratedMesh;
use crate::scripting::lua::commands::AssetCommand;
use crate::scripting::lua::guards::with_active_commands;

/// Primitive mesh names `set_mesh` and `spawn_primitive` accept.
const PRIMITIVES: &[&str] = &["cube", "sphere", "plane", "cylinder", "cone", "torus"];

fn check_primitive(kind: &str) -> LuaResult<()> {
    if PRIMITIVES.contains(&kind) {
        Ok(())
    } else {
        Err(mlua::Error::RuntimeError(format!(
            "unknown primitive {kind:?}; use one of {}",
            PRIMITIVES.join(", ")
        )))
    }
}

/// Turn a Lua table of material fields into the JSON patch the material commands take.
/// `false` removes a texture (Lua tables cannot hold nil) or clears a flag.
pub(crate) fn material_patch(
    lua: &Lua,
    fields: LuaValue,
    function: &str,
) -> LuaResult<serde_json::Value> {
    let mut patch: serde_json::Value = lua.from_value(fields)?;
    let Some(object) = patch.as_object_mut() else {
        return Err(mlua::Error::RuntimeError(format!(
            "{function} expects a table of material fields"
        )));
    };
    for value in object.values_mut() {
        if *value == json!(false) {
            *value = serde_json::Value::Null;
        }
    }
    for flag in ["unlit", "transparent", "double_sided", "nearest_filtering"] {
        if object.get(flag).is_some_and(serde_json::Value::is_null) {
            object.insert(flag.into(), json!(false));
        }
    }
    Ok(patch)
}

/// Registers the mesh and material functions:
///
/// - `set_material(entity, fields)` - change material fields: `color` ({r, g, b[, a]} in
///   0..1 or "#rrggbb"), `emissive`, `metallic`, `roughness` (0..1), `unlit`,
///   `transparent`, `double_sided`, `nearest_filtering` (booleans), and texture paths
///   `base_color_texture`, `normal_texture`, `emissive_texture`, ... (false removes one)
/// - `set_material_color(entity, r, g, b[, a])` - shorthand for the base color
/// - `set_mesh(entity, mesh)` - replace the geometry with a primitive (by name), a generated
///   mesh (a part of `voxel_model`), or `nil` to remove it
/// - `spawn_primitive(kind[, name[, {material_of = entity}]])` - spawn a visible entity
///   with a primitive mesh and its own material (or `material_of`'s); returns its handle
/// - `share_material(entity, source)` - draw `entity` with `source`'s material, so many
///   entities (sparks of one color) use one material. Changing a shared material with
///   `set_material` gives that entity its own copy again.
pub(crate) fn register_material_api(lua: &Lua) -> LuaResult<()> {
    let globals = lua.globals();

    globals.set(
        "set_material",
        lua.create_function(|lua, (entity, fields): (i64, LuaValue)| {
            let patch = material_patch(lua, fields, "set_material")?;
            with_active_commands(|commands| {
                commands.asset_command(entity, AssetCommand::Material(patch))
            })
        })?,
    )?;

    globals.set(
        "set_material_color",
        lua.create_function(
            |_, (entity, r, g, b, a): (i64, f64, f64, f64, Option<f64>)| {
                let patch = json!({ "color": [r, g, b, a.unwrap_or(1.0)] });
                with_active_commands(|commands| {
                    commands.asset_command(entity, AssetCommand::Material(patch))
                })
            },
        )?,
    )?;

    globals.set(
        "set_mesh",
        lua.create_function(|_, (entity, mesh): (i64, LuaValue)| {
            let command = match &mesh {
                LuaValue::Nil => AssetCommand::Mesh(serde_json::Value::Null),
                LuaValue::String(kind) => {
                    let kind = kind.to_str()?.to_string();
                    check_primitive(&kind)?;
                    AssetCommand::Mesh(json!(kind))
                }
                LuaValue::UserData(data) => {
                    AssetCommand::GeneratedMesh(data.borrow::<LuaGeneratedMesh>()?.0.clone())
                }
                LuaValue::Integer(_) | LuaValue::Number(_) => {
                    let source = match mesh {
                        LuaValue::Integer(source) => source,
                        LuaValue::Number(source) => source as i64,
                        _ => unreachable!(),
                    };
                    return with_active_commands(|commands| {
                        commands.resolve_or_pending(source)?;
                        commands.asset_command(entity, AssetCommand::ShareMesh(source))
                    });
                }
                other => {
                    return Err(mlua::Error::RuntimeError(format!(
                        "set_mesh expects a primitive name, a generated mesh (a voxel model \
                         part's mesh), an entity whose mesh to draw, or nil, not a {}",
                        other.type_name()
                    )))
                }
            };
            with_active_commands(|commands| commands.asset_command(entity, command))
        })?,
    )?;

    globals.set(
        "spawn_primitive",
        lua.create_function(
            |_, (kind, name, options): (String, Option<String>, Option<mlua::Table>)| {
                check_primitive(&kind)?;
                let material_of = match &options {
                    Some(options) => options.get::<Option<i64>>("material_of")?,
                    None => None,
                };
                with_active_commands(|commands| {
                    let entity = commands.spawn_entity(name.or_else(|| Some(kind.clone())));
                    // Shared first, so the mesh change finds a material and makes none.
                    if let Some(source) = material_of {
                        commands.resolve_or_pending(source)?;
                        commands.asset_command(entity, AssetCommand::ShareMaterial(source))?;
                    }
                    commands.asset_command(entity, AssetCommand::Mesh(json!(kind)))?;
                    Ok(entity)
                })
            },
        )?,
    )?;

    globals.set(
        "share_material",
        lua.create_function(|_, (entity, source): (i64, i64)| {
            with_active_commands(|commands| {
                commands.resolve_or_pending(source)?;
                commands.asset_command(entity, AssetCommand::ShareMaterial(source))
            })
        })?,
    )?;

    Ok(())
}
