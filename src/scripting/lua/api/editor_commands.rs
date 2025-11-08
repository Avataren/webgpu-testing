use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use mlua::{Lua, Result as LuaResult};

/// Editor commands that can be queued from Lua scripts
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

/// Register editor command API with Lua.
/// These functions allow Lua scripts to send commands to the editor.
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
            log::info!("Lua script requesting project creation: '{}' at {:?}", name, location_buf);

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
