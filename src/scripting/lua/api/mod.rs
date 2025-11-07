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
pub mod ui;

use mlua::{Lua, Result as LuaResult};

pub use clipboard::{ClipboardAccess, ClipboardGuard};
pub use ui::{UiCommand, UiContext, UiResponse};

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
    ui::register_ui_api(lua)?;

    Ok(())
}
