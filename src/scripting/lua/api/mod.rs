// Lua API functions module
//
// This module contains all the API functions exposed to Lua scripts.

mod clipboard;
mod component;
mod entity;
mod events;
mod file_io;
mod hierarchy;
mod input;
mod logging;
mod query;
mod state;
mod transform;

use mlua::{Lua, Result as LuaResult};

pub use clipboard::{ClipboardAccess, ClipboardGuard};

/// Register all Lua API functions with the runtime.
pub(crate) fn register_all_apis(lua: &Lua) -> LuaResult<()> {
    logging::register_logging_api(lua)?;
    state::register_state_api(lua)?;
    entity::register_entity_api(lua)?;
    transform::register_transform_api(lua)?;
    hierarchy::register_hierarchy_api(lua)?;
    input::register_input_api(lua)?;
    component::register_component_api(lua)?;
    query::register_query_api(lua)?;
    events::register_events_api(lua)?;
    file_io::register_file_io_api(lua)?;
    clipboard::register_clipboard_api(lua)?;

    // TODO: Future API modules
    // - state: State management (set_state, get_state, etc.)
    // - entity: Entity operations (spawn_entity, set_name, etc.)
    // - component: Component access (get_component, set_component, etc.)
    // - transform: Transform operations (translate, rotate, scale, etc.)
    // - hierarchy: Parent/child relationships
    // - input: Keyboard and mouse input
    // - event: Event emission and subscription
    // - query: Entity queries
    // - clipboard: Clipboard access
    // - file_io: File operations
    // - script_access: Metaprogramming API
    // - ui: UI rendering functions

    Ok(())
}
