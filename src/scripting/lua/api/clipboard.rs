/// Clipboard API for Lua scripts.
///
/// Provides functions to copy text to the system clipboard.
/// Note: Reading from clipboard is not supported by egui for security reasons.
/// Users must paste text using Ctrl+V into text fields.

use std::cell::RefCell;

use mlua::{Lua, Result as LuaResult};

thread_local! {
    static CLIPBOARD_CONTEXT: RefCell<Option<ClipboardAccess>> = RefCell::new(None);
}

/// Internal struct to hold clipboard access
pub struct ClipboardAccess {
    pub set_fn: Box<dyn Fn(String)>,
}

/// Guard for setting up clipboard context
pub struct ClipboardGuard;

impl ClipboardGuard {
    #[cfg(feature = "egui")]
    pub fn enter(ctx: &egui::Context) -> Self {
        CLIPBOARD_CONTEXT.with(|cell| {
            let ctx_clone = ctx.clone();

            *cell.borrow_mut() = Some(ClipboardAccess {
                set_fn: Box::new(move |text| ctx_clone.copy_text(text)),
            });
        });
        Self
    }

    #[cfg(not(feature = "egui"))]
    pub fn enter() -> Self {
        Self
    }
}

impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        CLIPBOARD_CONTEXT.with(|cell| {
            *cell.borrow_mut() = None;
        });
    }
}

/// Register clipboard API functions with the Lua runtime.
pub(crate) fn register_clipboard_api(lua: &Lua) -> LuaResult<()> {
    let globals = lua.globals();

    // get_clipboard() -> string
    // NOTE: This function always returns an empty string because egui does not provide
    // clipboard read access for security reasons. Users should paste text using Ctrl+V
    // or Cmd+V directly into text edit fields.
    globals.set(
        "get_clipboard",
        lua.create_function(|_, ()| Ok(String::new()))?,
    )?;

    // set_clipboard(text: string)
    globals.set(
        "set_clipboard",
        lua.create_function(|_, text: String| {
            CLIPBOARD_CONTEXT.with(|cell| {
                if let Some(access) = cell.borrow().as_ref() {
                    (access.set_fn)(text);
                }
            });
            Ok(())
        })?,
    )?;

    Ok(())
}
