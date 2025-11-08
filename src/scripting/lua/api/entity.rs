//! # Entity Management API
//!
//! This module provides functions for creating, finding, and managing entities
//! in the ECS (Entity Component System).
//!
//! ## Entity Handles
//!
//! Entities are represented as 64-bit integers (i64) in Lua. These handles
//! can be stored in variables and passed to other functions.
//!
//! ## Features
//!
//! - **Entity spawning** - Create new entities with optional names
//! - **Name management** - Set and search entities by name
//! - **Script attachment** - Dynamically attach scripts to entities
//! - **GLTF import** - Load 3D models and attach them to entities

use mlua::{Lua, Result as LuaResult};

use crate::scene::Name;
use crate::scripting::lua::commands::entity_bits;
use crate::scripting::lua::guards::{with_active_commands, with_active_world};

/// Registers entity management API functions with the Lua runtime.
///
/// This function exposes entity creation, naming, and management functions
/// to Lua scripts.
///
/// ## Available Functions
///
/// - `spawn_entity(name)` - Create new entity, returns handle (i64)
/// - `set_name(entity, name)` - Set entity name
/// - `find_entity_by_name(name)` - Search by name, returns entity or nil
/// - `attach_inline_script(entity, name, source)` - Attach Lua code to entity
/// - `attach_script(entity, path)` - Load and attach script from file
/// - `import_gltf(entity, path, scale)` - Import 3D model as child of entity
///
/// # Example Lua usage
///
/// ```lua
/// -- Create a new entity
/// local cube = spawn_entity("MyCube")
/// set_translation(cube, 0, 5, 0)
///
/// -- Find existing entity
/// local player = find_entity_by_name("Player")
/// if player then
///     log_info("Found player entity")
/// end
///
/// -- Dynamically attach a script
/// attach_inline_script(cube, "rotator", [[
///     function update(self_entity, dt)
///         rotate(self_entity, 0, 1, 0, dt)
///     end
/// ]])
///
/// -- Import a 3D model
/// local model_parent = spawn_entity("ModelContainer")
/// import_gltf(model_parent, "assets/models/character.gltf", 1.0)
/// ```
///
/// # Arguments
///
/// * `lua` - The Lua runtime to register functions with
///
/// # Returns
///
/// `Ok(())` on success, or a Lua error if registration fails.
pub(crate) fn register_entity_api(lua: &Lua) -> LuaResult<()> {
    let globals = lua.globals();

    // spawn_entity(name: string | nil) -> number
    globals.set(
        "spawn_entity",
        lua.create_function(|_, name: Option<String>| {
            with_active_commands(|commands| Ok(commands.spawn_entity(name)))
        })?,
    )?;

    // set_name(entity: number, name: string)
    globals.set(
        "set_name",
        lua.create_function(|_, (entity, name): (i64, String)| {
            with_active_commands(|commands| {
                commands.set_name(entity, name)?;
                Ok(())
            })
        })?,
    )?;

    // attach_inline_script(entity: number, name: string, source: string)
    globals.set(
        "attach_inline_script",
        lua.create_function(|_, (entity, name, source): (i64, String, String)| {
            with_active_commands(|commands| commands.attach_inline_script(entity, name, source))
        })?,
    )?;

    // attach_script(entity: number, path: string)
    globals.set(
        "attach_script",
        lua.create_function(|_, (entity, path): (i64, String)| {
            with_active_commands(|commands| commands.attach_file_script(entity, path))
        })?,
    )?;

    // import_gltf(entity: number, path: string, scale: number)
    globals.set(
        "import_gltf",
        lua.create_function(|_, (entity, path, scale): (i64, String, f64)| {
            with_active_commands(|commands| {
                commands.import_gltf(entity, path, scale as f32)?;
                Ok(())
            })
        })?,
    )?;

    // find_entity_by_name(name: string) -> number | nil
    globals.set(
        "find_entity_by_name",
        lua.create_function(|_, name: String| {
            with_active_world(|world| {
                for (entity, entity_name) in world.query::<&Name>().iter() {
                    if entity_name.0 == name {
                        return Ok(Some(entity_bits(entity)));
                    }
                }
                Ok(None)
            })
        })?,
    )?;

    Ok(())
}
