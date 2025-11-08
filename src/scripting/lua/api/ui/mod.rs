//! # UI API
//!
//! This module provides a user interface API for Lua scripts using egui.
//!
//! ## Features
//!
//! - **Widgets** - Buttons, labels, text inputs, sliders, checkboxes, color pickers
//! - **Layout** - Headings, separators, menu bars
//! - **Editor integration** - Create custom editor tools and panels
//!
//! ## Usage
//!
//! UI functions are only available in the `on_ui(self_entity, ui)` callback,
//! which is called for scripts marked with `@editor` or `@tool` annotations.
//!
//! The `ui` parameter provides methods to create widgets. Most widgets return
//! updated values, enabling immediate-mode GUI patterns.
//!
//! ## Script Modes
//!
//! - **`@editor`** - Script runs only in editor mode, has access to UI
//! - **`@tool`** - Script runs in editor and play modes (currently same as @editor)
//! - **No annotation** - Script runs only in play mode, no UI access
//!
//! ## Example
//!
//! ```lua
//! -- @editor
//!
//! function on_ui(self_entity, ui)
//!     ui:heading("My Tool")
//!     ui:separator()
//!
//!     if ui:button("Click Me") then
//!         log_info("Button clicked!")
//!     end
//!
//!     local value = ui:slider("speed", get_f64("speed", 5.0), 0, 10)
//!     set_f64("speed", value)
//! end
//! ```

mod commands;
mod context;

pub use commands::{UiCommand, UiResponse};
pub use context::UiContext;

use mlua::{Lua, UserData, UserDataMethods};

/// Registers UI API with the Lua runtime.
///
/// The UI API works differently from other APIs - instead of registering global
/// functions, it passes a `UiContext` userdata object to the `on_ui()` callback.
///
/// Scripts call methods on the `ui` parameter like `ui:button("text")`.
///
/// ## Available UI Methods
///
/// - `ui:label(text)` - Display text label
/// - `ui:heading(text)` - Display heading
/// - `ui:separator()` - Display horizontal line
/// - `ui:button(text)` - Display button, returns true if clicked
/// - `ui:checkbox(id, value, label)` - Display checkbox, returns new value
/// - `ui:text_edit(id, value)` - Single-line text input, returns new value
/// - `ui:text_edit_multiline(id, value, width, height)` - Multi-line text, returns new value
/// - `ui:slider(id, value, min, max)` - Numeric slider, returns new value
/// - `ui:drag_value(id, value)` - Numeric drag input, returns new value
/// - `ui:color_edit(id, r, g, b)` - Color picker, returns {r, g, b} table
/// - `ui:menu_bar(callback)` - Create menu bar, calls callback to add menus
/// - `ui:menu(text, callback)` - Create menu in menu bar, calls callback for items
/// - `ui:menu_item(id, text)` - Create menu item, returns true if clicked
///
/// # Arguments
///
/// * `lua` - The Lua runtime (unused, kept for API consistency)
///
/// # Returns
///
/// `Ok(())` always succeeds.
pub fn register_ui_api(_lua: &Lua) -> mlua::Result<()> {
    // Register UiContext as a userdata type that Lua scripts can interact with
    // The actual context will be passed to on_ui() callback

    // Note: We don't register global UI functions here because UI is provided
    // via the `ui` parameter in the on_ui(self_entity, ui) callback.
    // The UiContext is passed as userdata and scripts call methods on it.

    Ok(())
}

/// Implement Lua userdata for UiContext so it can be passed to scripts.
impl UserData for UiContext {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // Display a text label
        methods.add_method("label", |_, this, text: String| {
            this.label(text);
            Ok(())
        });

        // Display a button and return whether it was clicked
        methods.add_method("button", |_, this, text: String| Ok(this.button(text)));

        // Display a heading
        methods.add_method("heading", |_, this, text: String| {
            this.heading(text);
            Ok(())
        });

        // Display a separator
        methods.add_method("separator", |_, this, ()| {
            this.separator();
            Ok(())
        });

        // Display a text edit field
        methods.add_method(
            "text_edit",
            |_, this, (id, current_value): (String, String)| Ok(this.text_edit(id, current_value)),
        );

        // Display a multiline text editor
        methods.add_method("text_edit_multiline", |_, this, (id, current_value, width, height): (String, String, Option<f64>, Option<f64>)| {
            Ok(this.text_edit_multiline(id, current_value, width, height))
        });

        // Display a slider
        methods.add_method(
            "slider",
            |_, this, (id, current_value, min, max): (String, f64, f64, f64)| {
                Ok(this.slider(id, current_value, min, max))
            },
        );

        // Display a drag value widget
        methods.add_method(
            "drag_value",
            |_, this, (id, current_value): (String, f64)| Ok(this.drag_value(id, current_value)),
        );

        // Display a checkbox
        methods.add_method(
            "checkbox",
            |_, this, (id, current_value, label): (String, bool, String)| {
                Ok(this.checkbox(id, current_value, label))
            },
        );

        // Display a color picker (returns table with r, g, b)
        methods.add_method(
            "color_edit",
            |lua, this, (id, r, g, b): (String, f64, f64, f64)| {
                let (new_r, new_g, new_b) = this.color_edit(id, r, g, b);
                let table = lua.create_table()?;
                table.set("r", new_r)?;
                table.set("g", new_g)?;
                table.set("b", new_b)?;
                Ok(table)
            },
        );

        // Display a menu bar with menus
        methods.add_method("menu_bar", |_, this, callback: mlua::Function| {
            this.menu_bar(callback)
        });

        // Display a menu with a title
        methods.add_method(
            "menu",
            |_, this, (text, callback): (String, mlua::Function)| this.menu(text, callback),
        );

        // Display a menu item (like a button in a menu)
        methods.add_method("menu_item", |_, this, (id, text): (String, String)| {
            Ok(this.menu_item(id, text))
        });
    }
}
