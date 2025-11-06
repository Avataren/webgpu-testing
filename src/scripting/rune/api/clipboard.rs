/// Clipboard API for RuneScript
///
/// Provides functions to access the system clipboard for copy/paste operations.
/// Uses thread-local context to access the egui clipboard provider.

use std::cell::RefCell;

thread_local! {
    static CLIPBOARD_CONTEXT: RefCell<Option<ClipboardAccess>> = RefCell::new(None);
}

/// Internal struct to hold clipboard access
pub struct ClipboardAccess {
    pub get_fn: Box<dyn Fn() -> Option<String>>,
    pub set_fn: Box<dyn Fn(String)>,
}

/// Guard for setting up clipboard context
pub struct ClipboardGuard;

impl ClipboardGuard {
    #[cfg(feature = "egui")]
    pub fn enter(ctx: &egui::Context) -> Self {
        CLIPBOARD_CONTEXT.with(|cell| {
            let ctx_clone = ctx.clone();
            let ctx_clone2 = ctx.clone();

            *cell.borrow_mut() = Some(ClipboardAccess {
                get_fn: Box::new(move || ctx_clone.input(|i| i.raw.clipboard_text.clone())),
                set_fn: Box::new(move |text| ctx_clone2.copy_text(text)),
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

/// Get text from the clipboard.
/// Returns an empty string if clipboard is unavailable or empty.
#[rune::function]
pub fn get_clipboard() -> String {
    CLIPBOARD_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|access| (access.get_fn)())
            .unwrap_or_default()
    })
}

/// Set text to the clipboard.
#[rune::function]
pub fn set_clipboard(text: String) {
    CLIPBOARD_CONTEXT.with(|cell| {
        if let Some(access) = cell.borrow().as_ref() {
            (access.set_fn)(text);
        }
    });
}
