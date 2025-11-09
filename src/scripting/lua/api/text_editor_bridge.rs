use mlua::{Lua, Result as LuaResult};
use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

static TEXT_EDITOR_QUEUE: OnceLock<Mutex<VecDeque<String>>> = OnceLock::new();

fn queue() -> &'static Mutex<VecDeque<String>> {
    TEXT_EDITOR_QUEUE.get_or_init(|| Mutex::new(VecDeque::new()))
}

/// Register Lua helpers that allow scripts to fetch pending text-editor requests.
pub(crate) fn register_text_editor_bridge_api(lua: &Lua) -> LuaResult<()> {
    let globals = lua.globals();
    globals.set(
        "fetch_text_editor_request",
        lua.create_function(|_, ()| {
            let mut queue = queue().lock().unwrap();
            Ok(queue.pop_front())
        })?,
    )?;
    Ok(())
}

/// Enqueue a file path for the text editor plugin to open.
pub fn enqueue_text_editor_request(path: impl AsRef<Path>) {
    let path = path.as_ref();
    let display_path = path.to_string_lossy().into_owned();
    let mut queue = queue().lock().unwrap();
    queue.push_back(display_path);
}
