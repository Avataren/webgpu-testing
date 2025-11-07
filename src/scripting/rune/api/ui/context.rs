use super::commands::{UiCommand, UiResponse};
use rune::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// UI context that scripts use to build their UI.
///
/// This type uses command recording - all UI operations are recorded as commands
/// and then replayed later with a real egui::Ui context. This solves lifetime
/// issues with egui and allows the UI to be built within the Rune VM.
#[derive(Clone, Any)]
#[rune(item = ::ui)]
pub struct UiContext {
    commands: Rc<RefCell<Vec<UiCommand>>>,
    responses: Rc<RefCell<HashMap<String, UiResponse>>>,
}

impl UiContext {
    /// Create a new UI context.
    pub fn new() -> Self {
        Self {
            commands: Rc::new(RefCell::new(Vec::new())),
            responses: Rc::new(RefCell::new(HashMap::new())),
        }
    }

    /// Get the recorded commands for rendering.
    pub fn take_commands(&self) -> Vec<UiCommand> {
        self.commands.borrow_mut().drain(..).collect()
    }

    /// Set responses from rendering.
    pub fn set_responses(&self, responses: HashMap<String, UiResponse>) {
        *self.responses.borrow_mut() = responses;
    }

    /// Get a response for a specific widget ID.
    fn get_response(&self, id: &str) -> Option<UiResponse> {
        self.responses.borrow().get(id).cloned()
    }

    /// Display a text label.
    #[rune::function(instance)]
    pub fn label(&self, text: rune::Value) {
        use rune::FromValue;
        use rune::runtime::try_result;
        // Convert Value to String inside the function to avoid snapshot issues
        // If conversion fails, use empty string as fallback
        let text_str = match try_result(String::from_value(text)) {
            rune::runtime::VmResult::Ok(s) => s,
            rune::runtime::VmResult::Err(_) => String::new(),
        };
        self.commands.borrow_mut().push(UiCommand::Label { text: text_str });
    }

    /// Display a button and return whether it was clicked.
    #[rune::function(instance)]
    pub fn button(&self, text: String) -> bool {
        let id = format!("button_{}", text);
        self.commands.borrow_mut().push(UiCommand::Button {
            text: text.clone(),
        });

        self.get_response(&id)
            .map(|r| r.clicked)
            .unwrap_or(false)
    }

    /// Display a heading.
    #[rune::function(instance)]
    pub fn heading(&self, text: String) {
        self.commands.borrow_mut().push(UiCommand::Heading { text });
    }

    /// Display a separator line.
    #[rune::function(instance)]
    pub fn separator(&self) {
        self.commands.borrow_mut().push(UiCommand::Separator);
    }

    /// Display a text input field and return the new value.
    #[rune::function(instance)]
    pub fn text_edit(&self, id: String, current_value: rune::Value) -> String {
        use rune::FromValue;
        use rune::runtime::try_result;
        // Convert Value to String inside the function to avoid snapshot issues
        // If conversion fails, use empty string as fallback
        let current_str = match try_result(String::from_value(current_value)) {
            rune::runtime::VmResult::Ok(s) => s,
            rune::runtime::VmResult::Err(_) => String::new(),
        };

        self.commands.borrow_mut().push(UiCommand::TextEdit {
            id: id.clone(),
            current_value: current_str.clone(),
        });

        self.get_response(&id)
            .and_then(|r| r.text_value)
            .unwrap_or(current_str)
    }

    /// Display a multiline text editor and return the new value.
    /// If width and height are not provided, the editor will fill available space.
    #[rune::function(instance)]
    pub fn text_edit_multiline(&self, id: String, current_value: rune::Value, width: Option<f64>, height: Option<f64>) -> String {
        use rune::FromValue;
        use rune::runtime::try_result;
        // Convert Value to String inside the function to avoid snapshot issues
        // If conversion fails, use empty string as fallback
        let current_str = match try_result(String::from_value(current_value)) {
            rune::runtime::VmResult::Ok(s) => s,
            rune::runtime::VmResult::Err(_) => String::new(),
        };

        self.commands.borrow_mut().push(UiCommand::TextEditMultiline {
            id: id.clone(),
            current_value: current_str.clone(),
            width: width.map(|v| v as f32),
            height: height.map(|v| v as f32),
        });

        self.get_response(&id)
            .and_then(|r| r.text_value)
            .unwrap_or(current_str)
    }

    /// Display a slider and return the new value.
    #[rune::function(instance)]
    pub fn slider(&self, id: String, current_value: f64, min: f64, max: f64) -> f64 {
        self.commands.borrow_mut().push(UiCommand::Slider {
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
    #[rune::function(instance)]
    pub fn drag_value(&self, id: String, current_value: f64) -> f64 {
        self.commands.borrow_mut().push(UiCommand::DragValue {
            id: id.clone(),
            current_value,
        });

        self.get_response(&id)
            .and_then(|r| r.float_value)
            .unwrap_or(current_value)
    }

    /// Display a checkbox and return the new value.
    #[rune::function(instance)]
    pub fn checkbox(&self, id: String, current_value: bool, label: String) -> bool {
        self.commands.borrow_mut().push(UiCommand::Checkbox {
            id: id.clone(),
            current_value,
            label,
        });

        self.get_response(&id)
            .and_then(|r| r.bool_value)
            .unwrap_or(current_value)
    }

    /// Display a color picker and return the new RGB values as a tuple.
    #[rune::function(instance)]
    pub fn color_edit(&self, id: String, r: f64, g: f64, b: f64) -> (f64, f64, f64) {
        self.commands.borrow_mut().push(UiCommand::ColorEdit {
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
}

impl Default for UiContext {
    fn default() -> Self {
        Self::new()
    }
}
