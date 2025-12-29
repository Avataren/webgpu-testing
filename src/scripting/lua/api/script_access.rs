//! # Script Access API
//!
//! This module provides read-only access to Lua script metadata and sources
//! attached to entities.
//!
//! ## Features
//!
//! - **Script discovery** - List scripts attached to an entity
//! - **Source inspection** - Read inline source or file-backed script contents
//! - **Reload helpers** - Re-read script source data for change detection

use hecs::Entity;
use mlua::{Lua, Result as LuaResult, Table as LuaTable};

use crate::scripting::lua::guards::with_active_world;
use crate::scripting::{LuaScriptComponent, LuaScriptSource};

#[cfg(not(target_arch = "wasm32"))]
fn read_script_source(source: &LuaScriptSource) -> Result<String, mlua::Error> {
    match source {
        LuaScriptSource::Inline { source, .. } => Ok(source.clone()),
        LuaScriptSource::File { path } => std::fs::read_to_string(path).map_err(|err| {
            mlua::Error::RuntimeError(format!(
                "Failed to read script file '{}': {}",
                path.display(),
                err
            ))
        }),
    }
}

#[cfg(target_arch = "wasm32")]
fn read_script_source(source: &LuaScriptSource) -> Result<String, mlua::Error> {
    match source {
        LuaScriptSource::Inline { source, .. } => Ok(source.clone()),
        LuaScriptSource::File { path } => Err(mlua::Error::RuntimeError(format!(
            "Script file access is not supported on WASM: {}",
            path.display()
        ))),
    }
}

fn script_source_for_entity(world: &hecs::World, entity_bits: i64) -> Option<LuaScriptSource> {
    let entity = Entity::from_bits(entity_bits as u64)?;
    let script = world.get::<&LuaScriptComponent>(entity).ok()?;
    Some(script.source().clone())
}

/// Registers script access API functions with the Lua runtime.
///
/// ## Available Functions
///
/// - `get_entity_scripts(entity)` - Returns a 1-indexed table of script descriptors
/// - `read_script_source(entity)` - Returns script source text or nil
/// - `reload_script(entity)` - Re-reads the script source and returns its contents
///
/// # Example Lua usage
///
/// ```lua
/// local scripts = get_entity_scripts(self_entity)
/// if #scripts > 0 then
///     local script = scripts[1]
///     log_info("Script kind: " .. script.kind)
///     local source = read_script_source(self_entity)
///     if source then
///         log_info("Script length: " .. #source)
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
pub(crate) fn register_script_access_api(lua: &Lua) -> LuaResult<()> {
    let globals = lua.globals();

    // get_entity_scripts(entity: number) -> table
    globals.set(
        "get_entity_scripts",
        lua.create_function(|lua, entity_bits: i64| {
            with_active_world(|world| {
                let table = lua.create_table()?;

                let Some(source) = script_source_for_entity(world, entity_bits) else {
                    return Ok(table);
                };

                let descriptor: LuaTable = lua.create_table()?;
                match source {
                    LuaScriptSource::Inline { name, source } => {
                        descriptor.set("kind", "inline")?;
                        descriptor.set("name", name)?;
                        descriptor.set("source", source)?;
                    }
                    LuaScriptSource::File { path } => {
                        descriptor.set("kind", "file")?;
                        descriptor.set("path", path.to_string_lossy().to_string())?;
                    }
                }

                table.raw_set(1, descriptor)?;
                Ok(table)
            })
        })?,
    )?;

    // read_script_source(entity: number) -> string | nil
    globals.set(
        "read_script_source",
        lua.create_function(|_, entity_bits: i64| {
            with_active_world(|world| {
                let Some(source) = script_source_for_entity(world, entity_bits) else {
                    return Ok(None);
                };

                read_script_source(&source).map(Some)
            })
        })?,
    )?;

    // reload_script(entity: number) -> string | nil
    globals.set(
        "reload_script",
        lua.create_function(|_, entity_bits: i64| {
            with_active_world(|world| {
                let Some(source) = script_source_for_entity(world, entity_bits) else {
                    return Ok(None);
                };

                read_script_source(&source).map(Some)
            })
        })?,
    )?;

    Ok(())
}
