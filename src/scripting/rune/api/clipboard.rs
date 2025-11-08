/// Clipboard API for RuneScript
///
/// Provides functions to copy text to the system clipboard.
/// Note: Reading from clipboard is not supported by egui for security reasons.
/// Users must paste text using Ctrl+V into text fields.
use std::cell::RefCell;

thread_local! {
    static CLIPBOARD_CONTEXT: RefCell<Option<ClipboardAccess>> = RefCell::new(None);
}

/// Internal struct to hold clipboard access
pub struct ClipboardAccess {
    pub set_fn: Box<dyn Fn(String)>,
}

/// Guard for setting up clipboard context
#[allow(dead_code)]
pub struct ClipboardGuard;

impl ClipboardGuard {
    #[cfg(feature = "egui")]
    #[allow(dead_code)]
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
    #[allow(dead_code)]
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

/// Get text from the clipboard.
/// NOTE: This function always returns an empty string because egui does not provide
/// clipboard read access for security reasons. Users should paste text using Ctrl+V
/// or Cmd+V directly into text edit fields.
#[rune::function]
pub fn get_clipboard() -> String {
    String::new()
}

/// Set text to the clipboard (copy).
/// The text can then be pasted elsewhere using Ctrl+V or Cmd+V.
#[rune::function]
pub fn set_clipboard(text: String) {
    CLIPBOARD_CONTEXT.with(|cell| {
        if let Some(access) = cell.borrow().as_ref() {
            (access.set_fn)(text);
        }
    });
}
