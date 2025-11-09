use super::commands::{UiCommand, UiResponse};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// UI context that Lua scripts use to build their UI.
///
/// This type uses command recording - all UI operations are recorded as commands
/// and then replayed later with a real egui::Ui context. This solves lifetime
/// issues with egui and allows the UI to be built within the Lua VM.
///
/// Note: Uses Arc/Mutex instead of Rc/RefCell to support mlua's Send requirement.
#[derive(Clone)]
pub struct UiContext {
    commands: Arc<Mutex<Vec<UiCommand>>>,
    responses: Arc<Mutex<HashMap<String, UiResponse>>>,
    /// Viewport dimensions for in-game UI positioning (in logical points)
    viewport_width: Arc<Mutex<Option<f32>>>,
    viewport_height: Arc<Mutex<Option<f32>>>,
    /// DPI scaling factor (pixels per point)
    pixels_per_point: Arc<Mutex<Option<f32>>>,
}

impl UiContext {
    /// Create a new UI context.
    pub fn new() -> Self {
        Self {
            commands: Arc::new(Mutex::new(Vec::new())),
            responses: Arc::new(Mutex::new(HashMap::new())),
            viewport_width: Arc::new(Mutex::new(None)),
            viewport_height: Arc::new(Mutex::new(None)),
            pixels_per_point: Arc::new(Mutex::new(None)),
        }
    }

    /// Set viewport information for in-game UI positioning.
    pub fn set_viewport_info(&self, width: f32, height: f32, pixels_per_point: f32) {
        *self.viewport_width.lock().unwrap() = Some(width);
        *self.viewport_height.lock().unwrap() = Some(height);
        *self.pixels_per_point.lock().unwrap() = Some(pixels_per_point);
    }

    /// Get the viewport size (width, height) in logical points.
    /// Returns (0, 0) if viewport info is not set.
    pub fn get_viewport_size(&self) -> (f32, f32) {
        let width = self.viewport_width.lock().unwrap().unwrap_or(0.0);
        let height = self.viewport_height.lock().unwrap().unwrap_or(0.0);
        (width, height)
    }

    /// Get the DPI scaling factor (pixels per point).
    /// Returns 1.0 if not set.
    pub fn get_pixels_per_point(&self) -> f32 {
        self.pixels_per_point.lock().unwrap().unwrap_or(1.0)
    }

    /// Get the recorded commands for rendering.
    pub fn take_commands(&self) -> Vec<UiCommand> {
        self.commands.lock().unwrap().drain(..).collect()
    }

    /// Set responses from rendering.
    pub fn set_responses(&self, responses: HashMap<String, UiResponse>) {
        *self.responses.lock().unwrap() = responses;
    }

    /// Get a response for a specific widget ID.
    fn get_response(&self, id: &str) -> Option<UiResponse> {
        self.responses.lock().unwrap().get(id).cloned()
    }

    /// Display a text label.
    pub fn label(&self, text: String) {
        self.commands
            .lock()
            .unwrap()
            .push(UiCommand::Label { text });
    }

    /// Display a button and return whether it was clicked.
    pub fn button(&self, text: String) -> bool {
        let id = format!("button_{}", text);
        self.commands
            .lock()
            .unwrap()
            .push(UiCommand::Button { text: text.clone() });

        self.get_response(&id).map(|r| r.clicked).unwrap_or(false)
    }

    /// Display a heading.
    pub fn heading(&self, text: String) {
        self.commands
            .lock()
            .unwrap()
            .push(UiCommand::Heading { text });
    }

    /// Display a separator line.
    pub fn separator(&self) {
        self.commands.lock().unwrap().push(UiCommand::Separator);
    }

    /// Display a text input field and return the new value.
    pub fn text_edit(&self, id: String, current_value: String) -> String {
        // Check if we have a response from the previous frame
        // If so, use that value to avoid lag/jumping
        let command_value = self
            .get_response(&id)
            .and_then(|r| r.text_value.clone())
            .unwrap_or_else(|| current_value.clone());

        self.commands.lock().unwrap().push(UiCommand::TextEdit {
            id: id.clone(),
            current_value: command_value.clone(),
        });

        // Return the command value we just used
        command_value
    }

    /// Display a multiline text editor and return the new value.
    /// If width and height are not provided, the editor will fill available space.
    pub fn text_edit_multiline(
        &self,
        id: String,
        current_value: String,
        width: Option<f64>,
        height: Option<f64>,
    ) -> String {
        // Check if we have a response from the previous frame
        // If so, use that value to avoid lag/jumping
        let command_value = self
            .get_response(&id)
            .and_then(|r| r.text_value.clone())
            .unwrap_or_else(|| current_value.clone());

        self.commands
            .lock()
            .unwrap()
            .push(UiCommand::TextEditMultiline {
                id: id.clone(),
                current_value: command_value.clone(),
                width: width.map(|v| v as f32),
                height: height.map(|v| v as f32),
            });

        // Return the command value we just used
        command_value
    }

    /// Display a slider and return the new value.
    pub fn slider(&self, id: String, current_value: f64, min: f64, max: f64) -> f64 {
        self.commands.lock().unwrap().push(UiCommand::Slider {
            id: id.clone(),
            current_value,
            min,
            max,
        });

        self.get_response(&id)
            .and_then(|r| r.float_value)
            .unwrap_or(current_value)
    }

    /// Display a drag value widget and return the new value.
    pub fn drag_value(&self, id: String, current_value: f64) -> f64 {
        self.commands.lock().unwrap().push(UiCommand::DragValue {
            id: id.clone(),
            current_value,
        });

        self.get_response(&id)
            .and_then(|r| r.float_value)
            .unwrap_or(current_value)
    }

    /// Display a checkbox and return the new value.
    pub fn checkbox(&self, id: String, current_value: bool, label: String) -> bool {
        self.commands.lock().unwrap().push(UiCommand::Checkbox {
            id: id.clone(),
            current_value,
            label,
        });

        self.get_response(&id)
            .and_then(|r| r.bool_value)
            .unwrap_or(current_value)
    }

    /// Display a color picker and return the new RGB values as a table.
    pub fn color_edit(&self, id: String, r: f64, g: f64, b: f64) -> (f64, f64, f64) {
        self.commands.lock().unwrap().push(UiCommand::ColorEdit {
            id: id.clone(),
            r: r as f32,
            g: g as f32,
            b: b as f32,
        });

        if let Some(response) = self.get_response(&id) {
            if let Some((new_r, new_g, new_b)) = response.color_value {
                return (new_r as f64, new_g as f64, new_b as f64);
            }
        }

        (r, g, b)
    }

    /// Render a block of UI centered horizontally with an optional fixed width.
    pub fn centered_area(&self, width: f64, callback: mlua::Function) -> mlua::Result<()> {
        let temp_context = UiContext::new();
        temp_context.set_responses(self.responses.lock().unwrap().clone());
        // Copy viewport info to temporary context
        let (vp_width, vp_height) = self.get_viewport_size();
        let ppp = self.get_pixels_per_point();
        if vp_width > 0.0 && vp_height > 0.0 {
            temp_context.set_viewport_info(vp_width, vp_height, ppp);
        }

        callback.call::<()>(temp_context.clone())?;
        let commands = temp_context.take_commands();

        self.commands.lock().unwrap().push(UiCommand::CenteredArea {
            width: if width > 0.0 {
                Some(width as f32)
            } else {
                None
            },
            items: commands,
        });

        Ok(())
    }

    /// Display a menu bar. The callback function will be called to add menu items.
    pub fn menu_bar(&self, callback: mlua::Function) -> mlua::Result<()> {
        // Collect all commands from the callback
        let temp_context = UiContext::new();

        // Copy responses from parent context so menu items can check them!
        temp_context.set_responses(self.responses.lock().unwrap().clone());
        // Copy viewport info
        let (vp_width, vp_height) = self.get_viewport_size();
        let ppp = self.get_pixels_per_point();
        if vp_width > 0.0 && vp_height > 0.0 {
            temp_context.set_viewport_info(vp_width, vp_height, ppp);
        }

        callback.call::<()>(temp_context.clone())?;
        let menu_items = temp_context.take_commands();

        self.commands
            .lock()
            .unwrap()
            .push(UiCommand::MenuBar { items: menu_items });

        Ok(())
    }

    /// Display a menu with a title. The callback function will be called to add menu items.
    pub fn menu(&self, text: String, callback: mlua::Function) -> mlua::Result<()> {
        // Collect all commands from the callback
        let temp_context = UiContext::new();

        // Copy responses from parent context so menu items can check them!
        temp_context.set_responses(self.responses.lock().unwrap().clone());
        // Copy viewport info
        let (vp_width, vp_height) = self.get_viewport_size();
        let ppp = self.get_pixels_per_point();
        if vp_width > 0.0 && vp_height > 0.0 {
            temp_context.set_viewport_info(vp_width, vp_height, ppp);
        }

        callback.call::<()>(temp_context.clone())?;
        let menu_items = temp_context.take_commands();

        self.commands.lock().unwrap().push(UiCommand::Menu {
            text,
            items: menu_items,
        });

        Ok(())
    }

    /// Display a menu item (like a button in a menu).
    pub fn menu_item(&self, id: String, text: String) -> bool {
        self.commands.lock().unwrap().push(UiCommand::MenuItem {
            id: id.clone(),
            text,
        });

        self.get_response(&id).map(|r| r.clicked).unwrap_or(false)
    }
}

impl Default for UiContext {
    fn default() -> Self {
        Self::new()
    }
}
