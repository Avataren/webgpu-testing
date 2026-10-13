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

/// Optional size and color for a text widget; unset fields keep the theme's look.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TextStyle {
    /// Font size in points.
    pub size: Option<f32>,
    /// RGBA, not premultiplied.
    pub color: Option<[u8; 4]>,
}

#[cfg(feature = "egui")]
impl TextStyle {
    fn rich_text(&self, text: &str) -> egui::RichText {
        let mut rich = egui::RichText::new(text);
        if let Some(size) = self.size {
            rich = rich.size(size);
        }
        if let Some([r, g, b, a]) = self.color {
            rich = rich.color(egui::Color32::from_rgba_unmultiplied(r, g, b, a));
        }
        rich
    }
}

/// UI commands that are recorded during Lua script execution and replayed with real egui::Ui.
#[derive(Debug, Clone)]
pub enum UiCommand {
    Label {
        text: String,
        style: TextStyle,
    },
    Button {
        id: String,
        text: String,
    },
    Heading {
        text: String,
        style: TextStyle,
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
    /// Text drawn over the game view at a fixed position (offsets are inset from the
    /// anchored edge, as for anchored panels), without a panel around it.
    ScreenLabel {
        anchor: Anchor,
        offset_x: f32,
        offset_y: f32,
        text: String,
        style: TextStyle,
        backdrop: bool,
    },
    /// Text drawn over the scene at a world position, already projected to the
    /// viewport: `x`/`y` are fractions of its width and height from the top left.
    WorldLabel {
        x: f32,
        y: f32,
        text: String,
        color: [u8; 4],
        size: f32,
        backdrop: bool,
    },
}

impl UiCommand {
    /// Whether the command is drawn over the game view (a HUD element) rather than
    /// inside the script's window.
    pub fn is_overlay(&self) -> bool {
        matches!(
            self,
            UiCommand::AnchoredPanel { .. }
                | UiCommand::ScreenLabel { .. }
                | UiCommand::WorldLabel { .. }
        )
    }

    /// Render this command with a real egui::Ui and record widget responses by id.
    ///
    /// `viewport_rect` is the area anchored panels are positioned in; it defaults to
    /// the available rect of `ui`.
    #[cfg(feature = "egui")]
    pub fn render_and_collect(
        &self,
        ui: &mut egui::Ui,
        responses: &mut std::collections::HashMap<String, UiResponse>,
        viewport_rect: Option<egui::Rect>,
    ) {
        let mut render_children = |ui: &mut egui::Ui, items: &[UiCommand]| {
            for item in items {
                item.render_and_collect(ui, responses, viewport_rect);
            }
        };

        match self {
            UiCommand::MenuBar { items } => {
                egui::MenuBar::new().ui(ui, |ui| render_children(ui, items));
            }
            UiCommand::Menu { text, items } => {
                ui.menu_button(text, |ui| render_children(ui, items));
            }
            UiCommand::Horizontal { items, wrap } => {
                if *wrap {
                    ui.horizontal_wrapped(|ui| render_children(ui, items));
                } else {
                    ui.horizontal(|ui| render_children(ui, items));
                }
            }
            UiCommand::CenteredArea { width, items } => {
                ui.vertical_centered(|ui| {
                    let width = width
                        .unwrap_or_else(|| (ui.available_width() * 0.6_f32).clamp(320.0, 760.0));
                    ui.set_width(width);
                    render_children(ui, items);
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
                        // A backdrop keeps HUD text readable over any scene.
                        egui::Frame::popup(ui.style()).show(ui, |ui| {
                            match width {
                                Some(width) => ui.set_width(*width),
                                // Without a width the panel grows to fit its text.
                                None => ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend),
                            }
                            if let Some(height) = height {
                                ui.set_height(*height);
                            }
                            render_children(ui, items);
                        });
                    });
            }
            UiCommand::ScreenLabel {
                anchor,
                offset_x,
                offset_y,
                text,
                style,
                backdrop,
            } => {
                let base_rect = viewport_rect.unwrap_or_else(|| ui.max_rect());
                let painter = ui
                    .ctx()
                    .layer_painter(egui::LayerId::new(
                        egui::Order::Foreground,
                        egui::Id::new("script_screen_labels"),
                    ))
                    .with_clip_rect(base_rect);
                let color = style.color.map_or(egui::Color32::WHITE, |[r, g, b, a]| {
                    egui::Color32::from_rgba_unmultiplied(r, g, b, a)
                });
                let galley = painter.layout_no_wrap(
                    text.clone(),
                    egui::FontId::proportional(style.size.unwrap_or(16.0)),
                    color,
                );
                let position = anchored_position(
                    base_rect,
                    *anchor,
                    *offset_x,
                    *offset_y,
                    Some(galley.size().x),
                    Some(galley.size().y),
                );
                let text_rect = egui::Rect::from_min_size(position, galley.size());
                if *backdrop {
                    painter.rect_filled(
                        text_rect.expand2(egui::vec2(6.0, 3.0)),
                        4.0,
                        egui::Color32::from_black_alpha(150),
                    );
                }
                painter.galley(text_rect.min, galley, egui::Color32::WHITE);
            }
            UiCommand::WorldLabel {
                x,
                y,
                text,
                color,
                size,
                backdrop,
            } => {
                let base_rect = viewport_rect.unwrap_or_else(|| ui.max_rect());
                let position =
                    base_rect.min + egui::vec2(x * base_rect.width(), y * base_rect.height());
                let painter = ui
                    .ctx()
                    .layer_painter(egui::LayerId::new(
                        egui::Order::Foreground,
                        egui::Id::new("script_world_labels"),
                    ))
                    .with_clip_rect(base_rect);
                let [r, g, b, a] = *color;
                let galley = painter.layout_no_wrap(
                    text.clone(),
                    egui::FontId::proportional(*size),
                    egui::Color32::from_rgba_unmultiplied(r, g, b, a),
                );
                let text_rect = egui::Align2::CENTER_CENTER.anchor_size(position, galley.size());
                if *backdrop {
                    painter.rect_filled(
                        text_rect.expand2(egui::vec2(6.0, 3.0)),
                        4.0,
                        egui::Color32::from_black_alpha(150),
                    );
                }
                painter.galley(text_rect.min, galley, egui::Color32::WHITE);
            }
            leaf => {
                if let Some((id, response)) = leaf.render_leaf(ui) {
                    match responses.get_mut(id) {
                        Some(existing) => existing.merge(response),
                        None => {
                            responses.insert(id.to_owned(), response);
                        }
                    }
                }
            }
        }
    }

    /// Render a non-container command, returning its id and response if it is
    /// interactive. Value fields are only reported when the user changed the value.
    #[cfg(feature = "egui")]
    fn render_leaf(&self, ui: &mut egui::Ui) -> Option<(&str, UiResponse)> {
        fn interaction(response: &egui::Response) -> UiResponse {
            UiResponse {
                clicked: response.clicked(),
                hovered: response.hovered(),
                changed: response.changed(),
                ..Default::default()
            }
        }

        match self {
            UiCommand::Label { text, style } => {
                ui.label(style.rich_text(text));
                None
            }
            UiCommand::Heading { text, style } => {
                let mut rich = style.rich_text(text);
                if style.size.is_none() {
                    rich = rich.heading();
                }
                ui.label(rich);
                None
            }
            UiCommand::Separator => {
                ui.separator();
                None
            }
            UiCommand::Button { id, text } => Some((id, interaction(&ui.button(text)))),
            UiCommand::MenuItem { id, text } => Some((id, interaction(&ui.button(text)))),
            UiCommand::TextEdit { id, current_value } => {
                let mut text = current_value.clone();
                let response = ui.add(egui::TextEdit::singleline(&mut text).id_salt(id));
                let changed = response.changed();
                Some((
                    id,
                    UiResponse {
                        text_value: changed.then_some(text),
                        ..interaction(&response)
                    },
                ))
            }
            UiCommand::TextEditMultiline {
                id,
                current_value,
                width,
                height,
            } => {
                let mut text = current_value.clone();
                fn editor<'t>(text: &'t mut String, id: &str) -> egui::TextEdit<'t> {
                    egui::TextEdit::multiline(text).id_salt(id).code_editor()
                }

                let response = if let Some(target_height) = *height {
                    let target_width = width.unwrap_or_else(|| ui.available_width());
                    egui::ScrollArea::both()
                        .id_salt(id.as_str())
                        .auto_shrink([false, false])
                        .max_width(target_width)
                        .max_height(target_height)
                        .show(ui, |ui| {
                            let size = egui::vec2(target_width, target_height);
                            ui.set_min_size(size);
                            ui.add_sized(size, editor(&mut text, id))
                        })
                        .inner
                } else {
                    let size = egui::vec2(
                        width.unwrap_or_else(|| ui.available_width()),
                        ui.available_height(),
                    );
                    ui.add_sized(size, editor(&mut text, id))
                };

                let changed = response.changed();
                Some((
                    id,
                    UiResponse {
                        text_value: changed.then_some(text),
                        ..interaction(&response)
                    },
                ))
            }
            UiCommand::Slider {
                id,
                current_value,
                min,
                max,
            } => {
                let mut value = *current_value;
                let response = ui.add(egui::Slider::new(&mut value, *min..=*max));
                let changed = response.changed();
                Some((
                    id,
                    UiResponse {
                        float_value: changed.then_some(value),
                        ..interaction(&response)
                    },
                ))
            }
            UiCommand::DragValue { id, current_value } => {
                let mut value = *current_value;
                let response = ui.add(egui::DragValue::new(&mut value));
                let changed = response.changed();
                Some((
                    id,
                    UiResponse {
                        float_value: changed.then_some(value),
                        ..interaction(&response)
                    },
                ))
            }
            UiCommand::Checkbox {
                id,
                current_value,
                label,
            } => {
                let mut checked = *current_value;
                let response = ui.checkbox(&mut checked, label);
                let changed = response.changed();
                Some((
                    id,
                    UiResponse {
                        bool_value: changed.then_some(checked),
                        ..interaction(&response)
                    },
                ))
            }
            UiCommand::ColorEdit { id, r, g, b } => {
                let mut color = [*r, *g, *b];
                let response = ui.color_edit_button_rgb(&mut color);
                let changed = response.changed();
                Some((
                    id,
                    UiResponse {
                        color_value: changed.then_some((color[0], color[1], color[2])),
                        ..interaction(&response)
                    },
                ))
            }
            UiCommand::MenuBar { .. }
            | UiCommand::Menu { .. }
            | UiCommand::Horizontal { .. }
            | UiCommand::CenteredArea { .. }
            | UiCommand::AnchoredPanel { .. }
            | UiCommand::ScreenLabel { .. }
            | UiCommand::WorldLabel { .. } => None,
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
///
/// Value fields (`text_value`, `float_value`, ...) are only set when `changed` is true.
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

impl UiResponse {
    /// Combine with a response for the same widget from a later UI pass in the same
    /// frame. Clicks and changes from any pass are kept; values come from the latest
    /// pass that changed them.
    pub fn merge(&mut self, later: UiResponse) {
        self.clicked |= later.clicked;
        self.hovered = later.hovered;
        if later.changed {
            self.changed = true;
            self.text_value = later.text_value;
            self.float_value = later.float_value;
            self.bool_value = later.bool_value;
            self.color_value = later.color_value;
        }
    }
}

/// Merge per-entity widget responses from a later UI pass into `target`.
///
/// egui may run the UI more than once per frame and only the first pass sees input
/// events, so overwriting responses from a later pass would lose clicks.
pub fn merge_ui_responses<K: Eq + std::hash::Hash>(
    target: &mut std::collections::HashMap<K, std::collections::HashMap<String, UiResponse>>,
    later: std::collections::HashMap<K, std::collections::HashMap<String, UiResponse>>,
) {
    for (key, responses) in later {
        let entry = target.entry(key).or_default();
        for (id, response) in responses {
            match entry.get_mut(&id) {
                Some(existing) => existing.merge(response),
                None => {
                    entry.insert(id, response);
                }
            }
        }
    }
}

#[cfg(test)]
mod response_tests {
    use super::{merge_ui_responses, UiResponse};
    use std::collections::HashMap;

    #[test]
    fn later_pass_does_not_erase_click() {
        let mut target: HashMap<u32, HashMap<String, UiResponse>> = [(
            1,
            [(
                "button:Go".to_owned(),
                UiResponse {
                    clicked: true,
                    ..Default::default()
                },
            )]
            .into(),
        )]
        .into();
        let later = [(1, [("button:Go".to_owned(), UiResponse::default())].into())].into();

        merge_ui_responses(&mut target, later);

        assert!(target[&1]["button:Go"].clicked);
    }
}
