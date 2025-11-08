//! # Hierarchy API
//!
//! This module provides functions for managing parent-child relationships
//! between entities in the scene graph.
//!
//! ## Features
//!
//! - **Parenting** - Set or clear parent-child relationships
//! - **Queries** - Get parent or list all children
//! - **Transform propagation** - Child transforms are automatically computed
//!   relative to parent world transforms
//!
//! ## Notes
//!
//! - Children arrays use **1-based indexing** (Lua convention)
//! - Setting parent to `nil` unparents the entity
//! - World transforms are automatically updated when hierarchy changes

use hecs::Entity;
use mlua::{Lua, Result as LuaResult};

use crate::scene::components::{Children, Parent};
use crate::scripting::lua::commands::entity_bits;
use crate::scripting::lua::guards::{with_active_commands, with_active_world};

/// Registers hierarchy API functions with the Lua runtime.
///
/// This function exposes parent-child relationship management to Lua scripts.
///
/// ## Available Functions
///
/// - `set_parent(entity, parent)` - Set parent (or nil to unparent)
/// - `get_parent(entity)` - Returns parent handle or nil
/// - `get_children(entity)` - Returns 1-indexed table array or nil
///
/// # Example Lua usage
///
/// ```lua
/// -- Create a hierarchy
/// local parent = spawn_entity("Parent")
/// local child1 = spawn_entity("Child1")
/// local child2 = spawn_entity("Child2")
///
/// set_parent(child1, parent)
/// set_parent(child2, parent)
///
/// -- Query relationships
/// local p = get_parent(child1)
/// log_info("Child1's parent: " .. tostring(p))
///
/// local children = get_children(parent)
/// if children then
///     log_info("Parent has " .. #children .. " children")
///     for i = 1, #children do
///         log_info("Child " .. i .. ": " .. children[i])
///     end
/// end
///
/// -- Unparent an entity
/// set_parent(child1, nil)
/// ```
///
/// # Arguments
///
/// * `lua` - The Lua runtime to register functions with
///
/// # Returns
///
/// `Ok(())` on success, or a Lua error if registration fails.
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
