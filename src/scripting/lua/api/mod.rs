// Lua API functions module
//
// This module contains all the API functions exposed to Lua scripts.

mod clipboard;
mod component;
mod coroutine;
mod editor_commands;
mod entity;
mod events;
mod file_io;
mod hierarchy;
mod input;
mod logging;
mod query;
mod script_access;
mod state;
mod text_editor_bridge;
mod transform;
pub mod ui;

use mlua::{Lua, Result as LuaResult};

pub use clipboard::{ClipboardAccess, ClipboardGuard};
pub(crate) use coroutine::{set_current_coroutine_id, CoroutineGuard};
pub use editor_commands::{drain_editor_commands, LuaEditorCommand};
pub use text_editor_bridge::enqueue_text_editor_request;
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
    script_access::register_script_access_api(lua)?;
    events::register_events_api(lua)?;
    file_io::register_file_io_api(lua)?;
    clipboard::register_clipboard_api(lua)?;
    ui::register_ui_api(lua)?;
    editor_commands::register_editor_command_api(lua)?;
    coroutine::register_coroutine_api(lua)?;
    text_editor_bridge::register_text_editor_bridge_api(lua)?;

    Ok(())
}
