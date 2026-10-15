// Lua API functions module
//
// This module contains all the API functions exposed to Lua scripts.

mod animation;
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
pub(crate) mod modules;
mod particles;
mod postprocess;
mod query;
mod save_data;
mod script_access;
mod settings;
mod state;
mod text_editor_bridge;
mod transform;
pub mod ui;
mod view;
pub(crate) mod voxel;

use mlua::{Lua, Result as LuaResult};

pub use clipboard::{ClipboardAccess, ClipboardGuard};
pub(crate) use coroutine::{resume_coroutine, CoroutineGuard};
pub use editor_commands::{drain_editor_commands, LuaEditorCommand};
pub use file_io::grant_path_access;
pub use save_data::set_save_namespace;
pub(crate) use save_data::{load_user_file, namespace as save_namespace, save_user_file};
pub use text_editor_bridge::enqueue_text_editor_request;
pub use ui::{Anchor, UiCameraGuard, UiCommand, UiContext, UiResponse};
pub(crate) use view::ScriptView;

/// Whether scripts on a runtime are currently being run for play mode. Stored as Lua
/// app data by the scripting state each time it runs scripts.
#[derive(Clone, Copy)]
pub(crate) struct PlayMode(pub bool);

/// Seconds of script time (the sum of the frame steps scripts have been given), stored
/// as Lua app data by the scripting state. It restarts when play starts or stops.
#[derive(Clone, Copy, Default)]
pub(crate) struct ScriptClock(pub f64);

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
    postprocess::register_postprocess_api(lua)?;
    audio::register_audio_api(lua)?;
    animation::register_animation_api(lua)?;
    save_data::register_save_data_api(lua)?;
    settings::register_settings_api(lua)?;
    particles::register_particles_api(lua)?;
    view::register_view_api(lua)?;
    query::register_query_api(lua)?;
    script_access::register_script_access_api(lua)?;
    events::register_events_api(lua)?;
    file_io::register_file_io_api(lua)?;
    clipboard::register_clipboard_api(lua)?;
    ui::register_ui_api(lua)?;
    editor_commands::register_editor_command_api(lua)?;
    coroutine::register_coroutine_api(lua)?;
    voxel::register_voxel_api(lua)?;
    lua.globals().set(
        "is_playing",
        lua.create_function(|lua, ()| {
            Ok(lua.app_data_ref::<PlayMode>().is_some_and(|mode| mode.0))
        })?,
    )?;
    lua.globals().set(
        "get_time",
        lua.create_function(|lua, ()| {
            Ok(lua
                .app_data_ref::<ScriptClock>()
                .map_or(0.0, |clock| clock.0))
        })?,
    )?;
    text_editor_bridge::register_text_editor_bridge_api(lua)?;

    Ok(())
}
