//! # Editor Commands API
//!
//! This module provides functions for Lua scripts to control the editor.
//!
//! ## Features
//!
//! - **Project management** - Load or create projects from scripts
//! - **Command queueing** - Commands are queued and processed by editor
//!
//! ## Use Cases
//!
//! - Creating editor tools and utilities
//! - Automating project workflows
//! - Building custom project launchers
//!
//! ## Notes
//!
//! These commands are typically used in scripts marked with `@editor` or `@tool`
//! annotations, as they only make sense in the editor context.

use mlua::{Lua, Result as LuaResult};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// Editor commands that can be queued from Lua scripts.
///
/// These commands are queued when scripts call editor functions and are
/// processed by the editor's main loop.
#[derive(Clone, Debug)]
pub enum LuaEditorCommand {
    LoadProject(PathBuf),
    CreateProject { name: String, location: PathBuf },
}

/// Global queue of editor commands from Lua plugins
static EDITOR_COMMAND_QUEUE: OnceLock<Mutex<Vec<LuaEditorCommand>>> = OnceLock::new();

fn get_queue() -> &'static Mutex<Vec<LuaEditorCommand>> {
    EDITOR_COMMAND_QUEUE.get_or_init(|| Mutex::new(Vec::new()))
}

/// Drain all pending editor commands from Lua scripts
pub fn drain_editor_commands() -> Vec<LuaEditorCommand> {
    get_queue()
        .lock()
        .map(|mut queue| std::mem::take(&mut *queue))
        .unwrap_or_default()
}

/// Registers editor command API functions with the Lua runtime.
///
/// This function exposes editor control functions to Lua scripts.
///
/// ## Available Functions
///
/// - `load_project(path)` - Request editor to load project from path
/// - `create_project(name, location)` - Request editor to create new project
///
/// # Example Lua usage
///
/// ```lua
/// -- @editor
/// -- Project launcher tool
///
/// function on_ui(self_entity, ui)
///     ui:heading("Project Launcher")
///     ui:separator()
///
///     if ui:button("Load Example Project") then
///         load_project("examples/demo_project")
///     end
///
///     if ui:button("Create New Project") then
///         create_project("MyGame", "C:/Projects")
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
pub(crate) fn register_editor_command_api(lua: &Lua) -> LuaResult<()> {
    let globals = lua.globals();

    // load_project(path: string)
    // Requests the editor to load a project from the given path
    globals.set(
        "load_project",
        lua.create_function(|_, path: String| {
            let path_buf = PathBuf::from(&path);
            log::info!("Lua script requesting project load: {:?}", path_buf);

            if let Ok(mut queue) = get_queue().lock() {
                queue.push(LuaEditorCommand::LoadProject(path_buf));
            }

            Ok(())
        })?,
    )?;

    // create_project(name: string, location: string)
    // Requests the editor to create a new project
    globals.set(
        "create_project",
        lua.create_function(|_, (name, location): (String, String)| {
            let location_buf = PathBuf::from(&location);
            log::info!(
                "Lua script requesting project creation: '{}' at {:?}",
                name,
                location_buf
            );

            if let Ok(mut queue) = get_queue().lock() {
                queue.push(LuaEditorCommand::CreateProject {
                    name,
                    location: location_buf,
                });
            }

            Ok(())
        })?,
    )?;

    Ok(())
}
