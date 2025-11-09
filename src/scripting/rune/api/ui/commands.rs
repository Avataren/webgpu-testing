/// UI commands that are recorded during script execution and replayed with real egui::Ui.
///
/// This pattern solves the lifetime issues with egui::Ui by deferring actual rendering
/// until after the script VM has completed execution.

#[derive(Debug, Clone)]
pub enum UiCommand {
    Label {
        text: String,
    },
    Button {
        text: String,
    },
    Heading {
        text: String,
    },
    Separator,
    TextEdit {
        id: String,
        current_value: String,
    },
    TextEditMultiline {
        id: String,
        current_value: String,
        width: Option<f32>,
        height: Option<f32>,
    },
    Slider {
        id: String,
        current_value: f64,
        min: f64,
        max: f64,
    },
    DragValue {
        id: String,
        current_value: f64,
    },
    Checkbox {
        id: String,
        current_value: bool,
        label: String,
    },
    ColorEdit {
        id: String,
        r: f32,
        g: f32,
        b: f32,
    },
    MenuBar {
        items: Vec<UiCommand>,
    },
    Menu {
        text: String,
        items: Vec<UiCommand>,
    },
    MenuItem {
        id: String,
        text: String,
    },
    CenteredArea {
        width: Option<f32>,
        items: Vec<UiCommand>,
    },
}

impl UiCommand {
    /// Render this command using a real egui::Ui context and collect responses.
    #[cfg(feature = "egui")]
    pub fn render_and_collect(
        &self,
        ui: &mut egui::Ui,
        responses: &mut std::collections::HashMap<String, UiResponse>,
    ) {
        // Special handling for MenuBar and Menu to avoid double rendering
        match self {
            UiCommand::MenuBar { items } => {
                egui::menu::bar(ui, |ui| {
                    for item in items {
                        item.render_and_collect(ui, responses);
                    }
                });
            }
            UiCommand::Menu { text, items } => {
                ui.menu_button(text, |ui| {
                    for item in items {
                        item.render_and_collect(ui, responses);
                    }
                });
            }
            UiCommand::CenteredArea { width, items } => {
                ui.vertical_centered(|ui| {
                    if let Some(w) = width {
                        ui.set_width(*w);
                    } else {
                        let target = (ui.available_width() * 0.6_f32).clamp(320.0, 760.0);
                        ui.set_width(target);
                    }
                    for item in items {
                        item.render_and_collect(ui, responses);
                    }
                });
            }
            UiCommand::MenuItem { id, .. } => {
                // For all other commands, use render_with_id
                if let Some((id, response)) = self.render_with_id(ui) {
                    responses.insert(id, response);
                }
            }
            _ => {
                // For all other commands, use render_with_id
                if let Some((id, response)) = self.render_with_id(ui) {
                    responses.insert(id, response);
                } else {
                    let _ = self.render(ui);
                }
            }
        }
    }

    /// Render this command and return (id, response) if applicable.
    #[cfg(feature = "egui")]
    fn render_with_id(&self, ui: &mut egui::Ui) -> Option<(String, UiResponse)> {
        match self {
            UiCommand::MenuItem { id, .. } => {
                self.render(ui).map(|response| (id.clone(), response))
            }
            UiCommand::Button { text } => self
                .render(ui)
                .map(|response| (format!("button_{}", text), response)),
            UiCommand::TextEdit { id, .. }
            | UiCommand::TextEditMultiline { id, .. }
            | UiCommand::Slider { id, .. }
            | UiCommand::DragValue { id, .. }
            | UiCommand::Checkbox { id, .. }
            | UiCommand::ColorEdit { id, .. } => {
                self.render(ui).map(|response| (id.clone(), response))
            }
            _ => None,
        }
    }

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
            UiCommand::TextEdit {
                id: _,
                current_value,
            } => {
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
            UiCommand::TextEditMultiline {
                id: _,
                current_value,
                width,
                height,
            } => {
                let mut text = current_value.clone();
                let text_edit = egui::TextEdit::multiline(&mut text).code_editor();

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
            UiCommand::Slider {
                id: _,
                current_value,
                min,
                max,
            } => {
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
            UiCommand::DragValue {
                id: _,
                current_value,
            } => {
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
            UiCommand::Checkbox {
                id: _,
                current_value,
                label,
            } => {
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
            UiCommand::MenuBar { items } => {
                egui::menu::bar(ui, |ui| {
                    for item in items {
                        item.render(ui);
                    }
                });
                None
            }
            UiCommand::Menu { text, items } => {
                ui.menu_button(text, |ui| {
                    for item in items {
                        item.render(ui);
                    }
                });
                None
            }
            UiCommand::MenuItem { id: _, text } => {
                let clicked = ui.button(text).clicked();
                if clicked {
                    Some(UiResponse {
                        clicked: true,
                        ..Default::default()
                    })
                } else {
                    None
                }
            }
            UiCommand::CenteredArea { width, items } => {
                ui.vertical_centered(|ui| {
                    if let Some(w) = width {
                        ui.set_width(*w);
                    } else {
                        let target = (ui.available_width() * 0.6_f32).clamp(320.0, 760.0);
                        ui.set_width(target);
                    }
                    for item in items {
                        item.render(ui);
                    }
                });
                None
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
