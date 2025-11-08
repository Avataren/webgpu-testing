use mlua::{Lua, Result as LuaResult};

use crate::scene::Name;
use crate::scripting::lua::commands::entity_bits;
use crate::scripting::lua::guards::{with_active_commands, with_active_world};

/// Register entity management API functions with the Lua runtime.
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
