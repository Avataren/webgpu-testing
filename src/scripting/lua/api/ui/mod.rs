/// UI module for exposing egui to Lua scripts.
///
/// This module provides a safe interface for Lua scripts to create
/// egui-based user interfaces that can run in both editor and play modes.

mod commands;
mod context;

pub use commands::{UiCommand, UiResponse};
pub use context::UiContext;

use mlua::{Lua, UserData, UserDataMethods};

/// Register UI API with Lua.
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
        methods.add_method("button", |_, this, text: String| {
            Ok(this.button(text))
        });

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
        methods.add_method("text_edit", |_, this, (id, current_value): (String, String)| {
            Ok(this.text_edit(id, current_value))
        });

        // Display a multiline text editor
        methods.add_method("text_edit_multiline", |_, this, (id, current_value, width, height): (String, String, Option<f64>, Option<f64>)| {
            Ok(this.text_edit_multiline(id, current_value, width, height))
        });

        // Display a slider
        methods.add_method("slider", |_, this, (id, current_value, min, max): (String, f64, f64, f64)| {
            Ok(this.slider(id, current_value, min, max))
        });

        // Display a drag value widget
        methods.add_method("drag_value", |_, this, (id, current_value): (String, f64)| {
            Ok(this.drag_value(id, current_value))
        });

        // Display a checkbox
        methods.add_method("checkbox", |_, this, (id, current_value, label): (String, bool, String)| {
            Ok(this.checkbox(id, current_value, label))
        });

        // Display a color picker (returns table with r, g, b)
        methods.add_method("color_edit", |lua, this, (id, r, g, b): (String, f64, f64, f64)| {
            let (new_r, new_g, new_b) = this.color_edit(id, r, g, b);
            let table = lua.create_table()?;
            table.set("r", new_r)?;
            table.set("g", new_g)?;
            table.set("b", new_b)?;
            Ok(table)
        });
    }
}
