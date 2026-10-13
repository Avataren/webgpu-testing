// Lua API functions module
//
// This module contains all the API functions exposed to Lua scripts.

mod audio;
mod clipboard;
mod component;
mod coroutine;
mod editor_commands;
mod entity;
mod events;
mod file_io;
mod hierarchy;
mod input;
mod light_camera;
mod logging;
mod material;
mod query;
mod script_access;
mod state;
mod text_editor_bridge;
mod transform;
pub mod ui;

use mlua::{Lua, Result as LuaResult};

pub use clipboard::{ClipboardAccess, ClipboardGuard};
pub(crate) use coroutine::{resume_coroutine, CoroutineGuard};
pub use editor_commands::{drain_editor_commands, LuaEditorCommand};
pub use file_io::grant_path_access;
pub use text_editor_bridge::enqueue_text_editor_request;
pub use ui::{Anchor, UiCameraGuard, UiCommand, UiContext, UiResponse};

/// Whether scripts on a runtime are currently being run for play mode. Stored as Lua
/// app data by the scripting state each time it runs scripts.
#[derive(Clone, Copy)]
pub(crate) struct PlayMode(pub bool);

/// Register all Lua API functions with the runtime.
pub(crate) fn register_all_apis(lua: &Lua) -> LuaResult<()> {
    logging::register_logging_api(lua)?;
    state::register_state_api(lua)?;
    entity::register_entity_api(lua)?;
    transform::register_transform_api(lua)?;
    hierarchy::register_hierarchy_api(lua)?;
    input::register_input_api(lua)?;
    component::register_component_api(lua)?;
    material::register_material_api(lua)?;
    light_camera::register_light_camera_api(lua)?;
    audio::register_audio_api(lua)?;
    query::register_query_api(lua)?;
    script_access::register_script_access_api(lua)?;
    events::register_events_api(lua)?;
    file_io::register_file_io_api(lua)?;
    clipboard::register_clipboard_api(lua)?;
    ui::register_ui_api(lua)?;
    editor_commands::register_editor_command_api(lua)?;
    coroutine::register_coroutine_api(lua)?;
    lua.globals().set(
        "is_playing",
        lua.create_function(|lua, ()| {
            Ok(lua.app_data_ref::<PlayMode>().is_some_and(|mode| mode.0))
        })?,
    )?;
    text_editor_bridge::register_text_editor_bridge_api(lua)?;

    Ok(())
}
