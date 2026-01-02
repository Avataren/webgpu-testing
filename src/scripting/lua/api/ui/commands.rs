//! UI commands that are recorded during Lua script execution and replayed with real egui::Ui.
//!
//! This pattern solves the lifetime issues with egui::Ui by deferring actual rendering
//! until after the Lua VM has completed execution.

/// Defines anchor points for viewport-aligned panels.
///
/// # Examples
/// ```
/// use wgpu_cube::scripting::lua::api::ui::Anchor;
///
/// let anchor = Anchor::TopLeft;
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    TopLeft,
    TopCenter,
    TopRight,
    CenterLeft,
    Center,
    CenterRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

/// UI commands that are recorded during Lua script execution and replayed with real egui::Ui.
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
    Horizontal {
        items: Vec<UiCommand>,
        wrap: bool,
    },
    CenteredArea {
        width: Option<f32>,
        items: Vec<UiCommand>,
    },
    AnchoredPanel {
        id: String,
        anchor: Anchor,
        offset_x: f32,
        offset_y: f32,
        width: Option<f32>,
        height: Option<f32>,
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
        viewport_rect: Option<egui::Rect>,
    ) {
        // Special handling for MenuBar and Menu to avoid double rendering
        match self {
            UiCommand::MenuBar { items } => {
                println!("Rendering MenuBar with {} items", items.len());
                egui::MenuBar::new().ui(ui, |ui| {
                    for item in items {
                        item.render_and_collect(ui, responses, viewport_rect);
                    }
                });
            }
            UiCommand::Menu { text, items } => {
                println!("Rendering Menu '{}' with {} items", text, items.len());
                ui.menu_button(text, |ui| {
                    for item in items {
                        item.render_and_collect(ui, responses, viewport_rect);
                    }
                });
            }
            UiCommand::Horizontal { items, wrap } => {
                let render_children = |ui: &mut egui::Ui| {
                    for item in items {
                        item.render_and_collect(ui, responses, viewport_rect);
                    }
                };

                if *wrap {
                    ui.horizontal_wrapped(render_children);
                } else {
                    ui.horizontal(render_children);
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
                        item.render_and_collect(ui, responses, viewport_rect);
                    }
                });
            }
            UiCommand::AnchoredPanel {
                id,
                anchor,
                offset_x,
                offset_y,
                width,
                height,
                items,
            } => {
                let base_rect = viewport_rect.unwrap_or_else(|| ui.max_rect());
                let position =
                    anchored_position(base_rect, *anchor, *offset_x, *offset_y, *width, *height);
                egui::Area::new(egui::Id::new(id))
                    .fixed_pos(position)
                    .constrain_to(base_rect)
                    .interactable(true)
                    .show(ui.ctx(), |ui| {
                        if let Some(width) = width {
                            ui.set_width(*width);
                        }
                        if let Some(height) = height {
                            ui.set_height(*height);
                        }
                        for item in items {
                            item.render_and_collect(ui, responses, viewport_rect);
                        }
                    });
            }
            UiCommand::MenuItem { .. } => {
                // For all other commands, use render_with_id
                if let Some((id, response)) = self.render_with_id(ui, viewport_rect) {
                    responses.insert(id, response);
                }
            }
            _ => {
                // For all other commands, use render_with_id
                if let Some((id, response)) = self.render_with_id(ui, viewport_rect) {
                    responses.insert(id, response);
                } else {
                    let _ = self.render(ui, viewport_rect);
                }
            }
        }
    }

    /// Render this command and return (id, response) if applicable.
    #[cfg(feature = "egui")]
    fn render_with_id(
        &self,
        ui: &mut egui::Ui,
        viewport_rect: Option<egui::Rect>,
    ) -> Option<(String, UiResponse)> {
        match self {
            UiCommand::MenuItem { id, .. } => self
                .render(ui, viewport_rect)
                .map(|response| (id.clone(), response)),
            UiCommand::Button { text } => self
                .render(ui, viewport_rect)
                .map(|response| (format!("button_{}", text), response)),
            UiCommand::TextEdit { id, .. }
            | UiCommand::TextEditMultiline { id, .. }
            | UiCommand::Slider { id, .. }
            | UiCommand::DragValue { id, .. }
            | UiCommand::Checkbox { id, .. }
            | UiCommand::ColorEdit { id, .. } => self
                .render(ui, viewport_rect)
                .map(|response| (id.clone(), response)),
            _ => None,
        }
    }

    /// Render this command using a real egui::Ui context.
    #[cfg(feature = "egui")]
    pub fn render(
        &self,
        ui: &mut egui::Ui,
        viewport_rect: Option<egui::Rect>,
    ) -> Option<UiResponse> {
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
                egui::MenuBar::new().ui(ui, |ui| {
                    for item in items {
                        item.render(ui, viewport_rect);
                    }
                });
                None
            }
            UiCommand::Menu { text, items } => {
                ui.menu_button(text, |ui| {
                    for item in items {
                        item.render(ui, viewport_rect);
                    }
                });
                None
            }
            UiCommand::Horizontal { items, wrap } => {
                let render_children = |ui: &mut egui::Ui| {
                    for item in items {
                        item.render(ui, viewport_rect);
                    }
                };

                if *wrap {
                    ui.horizontal_wrapped(render_children);
                } else {
                    ui.horizontal(render_children);
                }
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
                        item.render(ui, viewport_rect);
                    }
                });
                None
            }
            UiCommand::AnchoredPanel {
                id,
                anchor,
                offset_x,
                offset_y,
                width,
                height,
                items,
            } => {
                let base_rect = viewport_rect.unwrap_or_else(|| ui.max_rect());
                let position =
                    anchored_position(base_rect, *anchor, *offset_x, *offset_y, *width, *height);
                egui::Area::new(egui::Id::new(id))
                    .fixed_pos(position)
                    .constrain_to(base_rect)
                    .interactable(true)
                    .show(ui.ctx(), |ui| {
                        if let Some(width) = width {
                            ui.set_width(*width);
                        }
                        if let Some(height) = height {
                            ui.set_height(*height);
                        }
                        for item in items {
                            item.render(ui, viewport_rect);
                        }
                    });
                None
            }
        }
    }
}

#[cfg(feature = "egui")]
fn anchored_position(
    base_rect: egui::Rect,
    anchor: Anchor,
    offset_x: f32,
    offset_y: f32,
    width: Option<f32>,
    height: Option<f32>,
) -> egui::Pos2 {
    let anchor_pos = match anchor {
        Anchor::TopLeft => base_rect.left_top(),
        Anchor::TopCenter => base_rect.center_top(),
        Anchor::TopRight => base_rect.right_top(),
        Anchor::CenterLeft => base_rect.left_center(),
        Anchor::Center => base_rect.center(),
        Anchor::CenterRight => base_rect.right_center(),
        Anchor::BottomLeft => base_rect.left_bottom(),
        Anchor::BottomCenter => base_rect.center_bottom(),
        Anchor::BottomRight => base_rect.right_bottom(),
    };

    let x_sign = match anchor {
        Anchor::TopRight | Anchor::CenterRight | Anchor::BottomRight => -1.0,
        _ => 1.0,
    };
    let y_sign = match anchor {
        Anchor::BottomLeft | Anchor::BottomCenter | Anchor::BottomRight => -1.0,
        _ => 1.0,
    };

    let mut position = anchor_pos + egui::vec2(offset_x * x_sign, offset_y * y_sign);

    if let Some(width) = width {
        match anchor {
            Anchor::TopRight | Anchor::CenterRight | Anchor::BottomRight => {
                position.x -= width;
            }
            Anchor::TopCenter | Anchor::Center | Anchor::BottomCenter => {
                position.x -= width * 0.5;
            }
            _ => {}
        }
    }

    if let Some(height) = height {
        match anchor {
            Anchor::BottomLeft | Anchor::BottomCenter | Anchor::BottomRight => {
                position.y -= height;
            }
            Anchor::CenterLeft | Anchor::Center | Anchor::CenterRight => {
                position.y -= height * 0.5;
            }
            _ => {}
        }
    }

    position
}

#[cfg(all(test, feature = "egui"))]
mod tests {
    use super::{anchored_position, Anchor};

    #[test]
    fn anchored_position_top_left_applies_offsets() {
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(100.0, 50.0));
        let position = anchored_position(rect, Anchor::TopLeft, 10.0, 12.0, None, None);

        assert_eq!(position, egui::pos2(10.0, 12.0));
    }

    #[test]
    fn anchored_position_bottom_right_insets() {
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(200.0, 100.0));
        let position =
            anchored_position(rect, Anchor::BottomRight, 10.0, 8.0, Some(40.0), Some(20.0));

        assert_eq!(position, egui::pos2(150.0, 72.0));
    }

    #[test]
    fn anchored_position_center_offsets_by_size() {
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 300.0));
        let position = anchored_position(rect, Anchor::Center, 10.0, -5.0, Some(100.0), Some(50.0));

        assert_eq!(position, egui::pos2(110.0, 120.0));
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
