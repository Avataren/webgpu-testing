/// UI commands that are recorded during Lua script execution and replayed with real egui::Ui.
///
/// This pattern solves the lifetime issues with egui::Ui by deferring actual rendering
/// until after the Lua VM has completed execution.

#[derive(Debug, Clone)]
pub enum UiCommand {
    Label { text: String },
    Button { text: String },
    Heading { text: String },
    Separator,
    TextEdit { id: String, current_value: String },
    TextEditMultiline { id: String, current_value: String, width: Option<f32>, height: Option<f32> },
    Slider { id: String, current_value: f64, min: f64, max: f64 },
    DragValue { id: String, current_value: f64 },
    Checkbox { id: String, current_value: bool, label: String },
    ColorEdit { id: String, r: f32, g: f32, b: f32 },
}

impl UiCommand {
    /// Render this command using a real egui::Ui context.
    #[cfg(feature = "egui")]
    pub fn render(&self, ui: &mut egui::Ui) -> Option<UiResponse> {
        match self {
            UiCommand::Label { text } => {
                ui.label(text);
                None
            }
            UiCommand::Button { text } => {
                let response = ui.button(text);
                Some(UiResponse {
                    clicked: response.clicked(),
                    hovered: response.hovered(),
                    ..Default::default()
                })
            }
            UiCommand::Heading { text } => {
                ui.heading(text);
                None
            }
            UiCommand::Separator => {
                ui.separator();
                None
            }
            UiCommand::TextEdit { id: _, current_value } => {
                let mut text = current_value.clone();
                let response = ui.text_edit_singleline(&mut text);
                Some(UiResponse {
                    clicked: response.clicked(),
                    hovered: response.hovered(),
                    changed: response.changed(),
                    text_value: Some(text),
                    ..Default::default()
                })
            }
            UiCommand::TextEditMultiline { id: _, current_value, width, height } => {
                let mut text = current_value.clone();
                let text_edit = egui::TextEdit::multiline(&mut text)
                    .code_editor();

                // Calculate size based on provided dimensions or use available space
                let size = match (width, height) {
                    (Some(w), Some(h)) => egui::vec2(*w, *h),
                    (Some(w), None) => egui::vec2(*w, ui.available_height()),
                    (None, Some(h)) => egui::vec2(ui.available_width(), *h),
                    (None, None) => ui.available_size(),
                };

                let response = ui.add_sized(size, text_edit);
                Some(UiResponse {
                    clicked: response.clicked(),
                    hovered: response.hovered(),
                    changed: response.changed(),
                    text_value: Some(text),
                    ..Default::default()
                })
            }
            UiCommand::Slider { id: _, current_value, min, max } => {
                let mut value = *current_value;
                let response = ui.add(egui::Slider::new(&mut value, *min..=*max));
                Some(UiResponse {
                    clicked: response.clicked(),
                    hovered: response.hovered(),
                    changed: response.changed(),
                    float_value: Some(value),
                    ..Default::default()
                })
            }
            UiCommand::DragValue { id: _, current_value } => {
                let mut value = *current_value;
                let response = ui.add(egui::DragValue::new(&mut value));
                Some(UiResponse {
                    clicked: response.clicked(),
                    hovered: response.hovered(),
                    changed: response.changed(),
                    float_value: Some(value),
                    ..Default::default()
                })
            }
            UiCommand::Checkbox { id: _, current_value, label } => {
                let mut checked = *current_value;
                let response = ui.checkbox(&mut checked, label);
                Some(UiResponse {
                    clicked: response.clicked(),
                    hovered: response.hovered(),
                    changed: response.changed(),
                    bool_value: Some(checked),
                    ..Default::default()
                })
            }
            UiCommand::ColorEdit { id: _, r, g, b } => {
                let mut color = [*r, *g, *b];
                let response = ui.color_edit_button_rgb(&mut color);
                Some(UiResponse {
                    clicked: response.clicked(),
                    hovered: response.hovered(),
                    changed: response.changed(),
                    color_value: Some((color[0], color[1], color[2])),
                    ..Default::default()
                })
            }
        }
    }
}

/// Response from a UI widget.
#[derive(Debug, Clone, Default)]
pub struct UiResponse {
    pub clicked: bool,
    pub hovered: bool,
    pub changed: bool,
    pub text_value: Option<String>,
    pub float_value: Option<f64>,
    pub bool_value: Option<bool>,
    pub color_value: Option<(f32, f32, f32)>,
}

/// Convert Lua UiCommand to Rune UiCommand.
/// Since the types are identical in structure, we can convert field by field.
impl From<UiCommand> for crate::scripting::rune::api::ui::UiCommand {
    fn from(cmd: UiCommand) -> Self {
        match cmd {
            UiCommand::Label { text } => crate::scripting::rune::api::ui::UiCommand::Label { text },
            UiCommand::Button { text } => crate::scripting::rune::api::ui::UiCommand::Button { text },
            UiCommand::Heading { text } => crate::scripting::rune::api::ui::UiCommand::Heading { text },
            UiCommand::Separator => crate::scripting::rune::api::ui::UiCommand::Separator,
            UiCommand::TextEdit { id, current_value } => {
                crate::scripting::rune::api::ui::UiCommand::TextEdit { id, current_value }
            }
            UiCommand::TextEditMultiline { id, current_value, width, height } => {
                crate::scripting::rune::api::ui::UiCommand::TextEditMultiline { id, current_value, width, height }
            }
            UiCommand::Slider { id, current_value, min, max } => {
                crate::scripting::rune::api::ui::UiCommand::Slider { id, current_value, min, max }
            }
            UiCommand::DragValue { id, current_value } => {
                crate::scripting::rune::api::ui::UiCommand::DragValue { id, current_value }
            }
            UiCommand::Checkbox { id, current_value, label } => {
                crate::scripting::rune::api::ui::UiCommand::Checkbox { id, current_value, label }
            }
            UiCommand::ColorEdit { id, r, g, b } => {
                crate::scripting::rune::api::ui::UiCommand::ColorEdit { id, r, g, b }
            }
        }
    }
}

/// Convert Rune UiResponse to Lua UiResponse.
impl From<crate::scripting::rune::api::ui::UiResponse> for UiResponse {
    fn from(resp: crate::scripting::rune::api::ui::UiResponse) -> Self {
        UiResponse {
            clicked: resp.clicked,
            hovered: resp.hovered,
            changed: resp.changed,
            text_value: resp.text_value,
            float_value: resp.float_value,
            bool_value: resp.bool_value,
            color_value: resp.color_value,
        }
    }
}
