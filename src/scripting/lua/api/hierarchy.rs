use hecs::Entity;
use mlua::{Lua, Result as LuaResult};

use crate::scene::components::{Children, Parent};
use crate::scripting::lua::commands::entity_bits;
use crate::scripting::lua::guards::{with_active_commands, with_active_world};

/// Register hierarchy API functions with the Lua runtime.
pub(crate) fn register_hierarchy_api(lua: &Lua) -> LuaResult<()> {
    let globals = lua.globals();

    // set_parent(entity: number, parent: number | nil)
    globals.set(
        "set_parent",
        lua.create_function(|_, (entity, parent): (i64, Option<i64>)| {
            with_active_commands(|commands| commands.set_parent(entity, parent))
        })?,
    )?;

    // get_parent(entity: number) -> number | nil
    globals.set(
        "get_parent",
        lua.create_function(|_, entity_handle: i64| {
            with_active_world(|world| {
                let entity = match Entity::from_bits(entity_handle as u64) {
                    Some(e) => e,
                    None => return Ok(None),
                };

                if let Ok(parent) = world.get::<&Parent>(entity) {
                    return Ok(Some(entity_bits(parent.0)));
                }

                Ok(None)
            })
        })?,
    )?;

    // get_children(entity: number) -> table | nil
    globals.set(
        "get_children",
        lua.create_function(|lua, entity_handle: i64| {
            with_active_world(|world| {
                let entity = match Entity::from_bits(entity_handle as u64) {
                    Some(e) => e,
                    None => return Ok(None),
                };

                if let Ok(children) = world.get::<&Children>(entity) {
                    let table = lua.create_table()?;
                    for (i, &child) in children.0.iter().enumerate() {
                        table.raw_set(i + 1, entity_bits(child))?;
                    }
                    return Ok(Some(table));
                }

                Ok(None)
            })
        })?,
    )?;

    Ok(())
}
