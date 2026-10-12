use super::commands::{Anchor, UiCommand, UiResponse};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// Lock a mutex, recovering the data if a previous holder panicked.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// UI context that Lua scripts use to build their UI.
///
/// This type uses command recording - all UI operations are recorded as commands
/// and then replayed later with a real egui::Ui context. This solves lifetime
/// issues with egui and allows the UI to be built within the Lua VM.
///
/// Widget interaction is reported back one frame later through [`UiResponse`]s keyed
/// by widget id. Ids are derived from the widget's label or explicit id plus an
/// occurrence counter, so two buttons with the same label stay independent.
///
/// Value widgets (text edits, sliders, checkboxes, ...) return the user's edit only
/// on the frame after it happened; otherwise they return the value the script passed
/// in. Scripts therefore own the value and can change it at any time:
///
/// ```lua
/// name = ui:text_edit("name", name)
/// if ui:button("Clear") then name = "" end
/// ```
///
/// Note: Uses Arc/Mutex instead of Rc/RefCell to support mlua's Send requirement.
#[derive(Clone)]
pub struct UiContext {
    commands: Arc<Mutex<Vec<UiCommand>>>,
    /// Responses from the previous frame, shared with nested layout contexts.
    responses: Arc<Mutex<HashMap<String, UiResponse>>>,
    /// Occurrences of each widget id this frame, shared with nested layout contexts.
    id_counts: Arc<Mutex<HashMap<String, u32>>>,
    /// Viewport `(width, height, pixels_per_point)` for in-game UI positioning.
    viewport: Option<(f32, f32, f32)>,
}

impl UiContext {
    /// Create a new UI context.
    pub fn new() -> Self {
        Self {
            commands: Arc::default(),
            responses: Arc::default(),
            id_counts: Arc::default(),
            viewport: None,
        }
    }

    pub(crate) fn new_with_viewport_info(width: f32, height: f32, pixels_per_point: f32) -> Self {
        let mut context = Self::new();
        context.set_viewport_info(width, height, pixels_per_point);
        context
    }

    /// Set viewport information for in-game UI positioning.
    pub fn set_viewport_info(&mut self, width: f32, height: f32, pixels_per_point: f32) {
        self.viewport = Some((width, height, pixels_per_point));
    }

    /// Get the viewport size (width, height) in logical points.
    /// Returns (0, 0) if viewport info is not set.
    pub fn get_viewport_size(&self) -> (f32, f32) {
        self.viewport
            .map_or((0.0, 0.0), |(width, height, _)| (width, height))
    }

    /// Get the DPI scaling factor (pixels per point).
    /// Returns 1.0 if not set.
    pub fn get_pixels_per_point(&self) -> f32 {
        self.viewport.map_or(1.0, |(_, _, ppp)| ppp)
    }

    /// Get the recorded commands for rendering.
    pub fn take_commands(&self) -> Vec<UiCommand> {
        std::mem::take(&mut *lock(&self.commands))
    }

    /// Set responses from rendering.
    pub fn set_responses(&self, responses: HashMap<String, UiResponse>) {
        *lock(&self.responses) = responses;
    }

    fn push(&self, command: UiCommand) {
        lock(&self.commands).push(command);
    }

    /// Allocate the id for a widget of `kind` keyed by `key` (a label or explicit id).
    ///
    /// Repeated keys within a frame get an occurrence suffix (`#1`, `#2`, ...), so ids
    /// are unique yet stable as long as the UI is built in the same order.
    fn claim_id(&self, kind: &str, key: &str) -> String {
        let base = format!("{kind}:{key}");
        let mut counts = lock(&self.id_counts);
        let count = counts.entry(base.clone()).or_insert(0);
        let id = match *count {
            0 => base,
            n => format!("{base}#{n}"),
        };
        *count += 1;
        id
    }

    fn was_clicked(&self, id: &str) -> bool {
        lock(&self.responses).get(id).is_some_and(|r| r.clicked)
    }

    /// The previous frame's response for `id`, if the user changed the widget's value.
    fn changed_response(&self, id: &str) -> Option<UiResponse> {
        lock(&self.responses)
            .get(id)
            .filter(|response| response.changed)
            .cloned()
    }

    /// Display a text label.
    pub fn label(&self, text: String) {
        self.push(UiCommand::Label { text });
    }

    /// Display a button and return whether it was clicked.
    ///
    /// The button is identified by `id` if given, otherwise by its label.
    pub fn button(&self, text: String, id: Option<String>) -> bool {
        let id = self.claim_id("button", id.as_deref().unwrap_or(&text));
        let clicked = self.was_clicked(&id);
        self.push(UiCommand::Button { id, text });
        clicked
    }

    /// Display a heading.
    pub fn heading(&self, text: String) {
        self.push(UiCommand::Heading { text });
    }

    /// Display a separator line.
    pub fn separator(&self) {
        self.push(UiCommand::Separator);
    }

    /// Display a text input field and return the new value.
    pub fn text_edit(&self, id: String, current_value: String) -> String {
        let id = self.claim_id("text_edit", &id);
        let value = self
            .changed_response(&id)
            .and_then(|r| r.text_value)
            .unwrap_or(current_value);
        self.push(UiCommand::TextEdit {
            id,
            current_value: value.clone(),
        });
        value
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
        let id = self.claim_id("text_edit", &id);
        let value = self
            .changed_response(&id)
            .and_then(|r| r.text_value)
            .unwrap_or(current_value);
        self.push(UiCommand::TextEditMultiline {
            id,
            current_value: value.clone(),
            width: width.map(|v| v as f32),
            height: height.map(|v| v as f32),
        });
        value
    }

    /// Display a slider and return the new value.
    pub fn slider(&self, id: String, current_value: f64, min: f64, max: f64) -> f64 {
        let id = self.claim_id("slider", &id);
        let value = self
            .changed_response(&id)
            .and_then(|r| r.float_value)
            .unwrap_or(current_value);
        self.push(UiCommand::Slider {
            id,
            current_value: value,
            min,
            max,
        });
        value
    }

    /// Display a drag value widget and return the new value.
    pub fn drag_value(&self, id: String, current_value: f64) -> f64 {
        let id = self.claim_id("drag_value", &id);
        let value = self
            .changed_response(&id)
            .and_then(|r| r.float_value)
            .unwrap_or(current_value);
        self.push(UiCommand::DragValue {
            id,
            current_value: value,
        });
        value
    }

    /// Display a checkbox and return the new value.
    pub fn checkbox(&self, id: String, current_value: bool, label: String) -> bool {
        let id = self.claim_id("checkbox", &id);
        let value = self
            .changed_response(&id)
            .and_then(|r| r.bool_value)
            .unwrap_or(current_value);
        self.push(UiCommand::Checkbox {
            id,
            current_value: value,
            label,
        });
        value
    }

    /// Display a color picker and return the new RGB values.
    pub fn color_edit(&self, id: String, r: f64, g: f64, b: f64) -> (f64, f64, f64) {
        let id = self.claim_id("color_edit", &id);
        let (r, g, b) = self
            .changed_response(&id)
            .and_then(|response| response.color_value)
            .map_or((r, g, b), |(r, g, b)| (r as f64, g as f64, b as f64));
        self.push(UiCommand::ColorEdit {
            id,
            r: r as f32,
            g: g as f32,
            b: b as f32,
        });
        (r, g, b)
    }

    /// Run `callback` with a nested context and return the commands it recorded.
    fn record_children(&self, callback: mlua::Function) -> mlua::Result<Vec<UiCommand>> {
        let child = self.new_child_context();
        callback.call::<()>(child.clone())?;
        Ok(child.take_commands())
    }

    /// Render a block of UI centered horizontally with an optional fixed width.
    pub fn centered_area(&self, width: f64, callback: mlua::Function) -> mlua::Result<()> {
        let items = self.record_children(callback)?;
        self.push(UiCommand::CenteredArea {
            width: (width > 0.0).then_some(width as f32),
            items,
        });
        Ok(())
    }

    /// Render a block of UI anchored to a fixed position in the viewport.
    ///
    /// Offsets are specified in logical points and interpreted relative to the chosen anchor.
    #[allow(clippy::too_many_arguments)]
    pub fn anchored_panel(
        &self,
        id: String,
        anchor: Anchor,
        offset_x: f64,
        offset_y: f64,
        width: Option<f64>,
        height: Option<f64>,
        callback: mlua::Function,
    ) -> mlua::Result<()> {
        let items = self.record_children(callback)?;
        self.push(UiCommand::AnchoredPanel {
            id,
            anchor,
            offset_x: offset_x as f32,
            offset_y: offset_y as f32,
            width: width.map(|value| value as f32),
            height: height.map(|value| value as f32),
            items,
        });
        Ok(())
    }

    /// Layout items horizontally without wrapping.
    pub fn horizontal(&self, callback: mlua::Function) -> mlua::Result<()> {
        let items = self.record_children(callback)?;
        self.push(UiCommand::Horizontal { items, wrap: false });
        Ok(())
    }

    /// Layout items horizontally and allow wrapping onto multiple rows.
    pub fn horizontal_wrapped(&self, callback: mlua::Function) -> mlua::Result<()> {
        let items = self.record_children(callback)?;
        self.push(UiCommand::Horizontal { items, wrap: true });
        Ok(())
    }

    /// Display a menu bar. The callback function will be called to add menu items.
    pub fn menu_bar(&self, callback: mlua::Function) -> mlua::Result<()> {
        let items = self.record_children(callback)?;
        self.push(UiCommand::MenuBar { items });
        Ok(())
    }

    /// Display a menu with a title. The callback function will be called to add menu items.
    pub fn menu(&self, text: String, callback: mlua::Function) -> mlua::Result<()> {
        let items = self.record_children(callback)?;
        self.push(UiCommand::Menu { text, items });
        Ok(())
    }

    /// Display a menu item (like a button in a menu).
    pub fn menu_item(&self, id: String, text: String) -> bool {
        let id = self.claim_id("menu_item", &id);
        let clicked = self.was_clicked(&id);
        self.push(UiCommand::MenuItem { id, text });
        clicked
    }

    /// A context for a nested layout: it records its own commands but shares
    /// responses, id allocation and viewport info with its parent.
    fn new_child_context(&self) -> UiContext {
        UiContext {
            commands: Arc::default(),
            responses: Arc::clone(&self.responses),
            id_counts: Arc::clone(&self.id_counts),
            viewport: self.viewport,
        }
    }
}

impl Default for UiContext {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::UiContext;
    use crate::scripting::lua::api::{Anchor, UiCommand, UiResponse};
    use mlua::Lua;

    #[test]
    fn anchored_panel_records_commands() {
        let lua = Lua::new();
        let ui_context = UiContext::new();
        let callback = lua
            .load(
                r#"
                return function(panel)
                    panel:label("Hello HUD")
                end
                "#,
            )
            .eval::<mlua::Function>()
            .expect("Failed to build Lua callback");

        ui_context
            .anchored_panel(
                "hud_panel".to_string(),
                Anchor::TopLeft,
                12.0,
                24.0,
                Some(200.0),
                Some(50.0),
                callback,
            )
            .expect("Anchored panel failed");

        let commands = ui_context.take_commands();
        assert_eq!(commands.len(), 1);

        match &commands[0] {
            UiCommand::AnchoredPanel {
                id,
                anchor,
                offset_x,
                offset_y,
                width,
                height,
                items,
            } => {
                assert_eq!(id, "hud_panel");
                assert_eq!(*anchor, Anchor::TopLeft);
                assert_eq!(*offset_x, 12.0);
                assert_eq!(*offset_y, 24.0);
                assert_eq!(*width, Some(200.0));
                assert_eq!(*height, Some(50.0));
                assert_eq!(items.len(), 1);
                assert!(matches!(
                    items[0],
                    UiCommand::Label {
                        ref text
                    } if text == "Hello HUD"
                ));
            }
            other => panic!("Expected anchored panel, got {other:?}"),
        }
    }

    #[test]
    fn anchored_panel_copies_viewport_info() {
        let lua = Lua::new();
        let ui_context = UiContext::new_with_viewport_info(320.0, 200.0, 2.0);

        let callback = lua
            .load(
                r#"
                return function(panel)
                    local vp = panel:get_viewport_size()
                    panel:label(string.format("w=%.1f h=%.1f", vp.width, vp.height))
                end
                "#,
            )
            .eval::<mlua::Function>()
            .expect("Failed to build Lua callback");

        ui_context
            .anchored_panel(
                "hud_panel".to_string(),
                Anchor::TopLeft,
                0.0,
                0.0,
                None,
                None,
                callback,
            )
            .expect("Anchored panel failed");

        let commands = ui_context.take_commands();
        assert_eq!(commands.len(), 1);

        match &commands[0] {
            UiCommand::AnchoredPanel { items, .. } => {
                assert!(matches!(
                    items[0],
                    UiCommand::Label {
                        ref text
                    } if text == "w=320.0 h=200.0"
                ));
            }
            other => panic!("Expected anchored panel, got {other:?}"),
        }
    }

    fn clicked() -> UiResponse {
        UiResponse {
            clicked: true,
            ..Default::default()
        }
    }

    fn command_id(command: &UiCommand) -> &str {
        match command {
            UiCommand::Button { id, .. }
            | UiCommand::TextEdit { id, .. }
            | UiCommand::DragValue { id, .. } => id,
            other => panic!("unexpected command {other:?}"),
        }
    }

    #[test]
    fn buttons_with_the_same_label_have_distinct_ids() {
        let first = UiContext::new();
        first.button("Open".into(), None);
        first.button("Open".into(), None);
        let commands = first.take_commands();
        let second_id = command_id(&commands[1]).to_owned();
        assert_ne!(command_id(&commands[0]), second_id);

        // Clicking the second button must not report the first as clicked.
        let next = UiContext::new();
        next.set_responses([(second_id, clicked())].into());
        assert!(!next.button("Open".into(), None));
        assert!(next.button("Open".into(), None));
    }

    #[test]
    fn nested_layouts_share_id_allocation() {
        let lua = Lua::new();
        let ui = UiContext::new();
        ui.button("OK".into(), None);
        let callback = lua
            .load(r#"return function(row) row:button("OK") end"#)
            .eval::<mlua::Function>()
            .unwrap();
        ui.horizontal(callback).unwrap();

        let commands = ui.take_commands();
        let UiCommand::Horizontal { items, .. } = &commands[1] else {
            panic!("expected horizontal layout");
        };
        assert_ne!(command_id(&commands[0]), command_id(&items[0]));
    }

    #[test]
    fn value_widgets_return_script_value_unless_user_changed_it() {
        // An unchanged response must not override a value the script set itself.
        let ui = UiContext::new();
        ui.set_responses(
            [(
                "text_edit:name".to_owned(),
                UiResponse {
                    text_value: Some("stale".into()),
                    ..Default::default()
                },
            )]
            .into(),
        );
        assert_eq!(ui.text_edit("name".into(), String::new()), "");

        let ui = UiContext::new();
        ui.set_responses(
            [(
                "drag_value:x".to_owned(),
                UiResponse {
                    changed: true,
                    float_value: Some(4.0),
                    ..Default::default()
                },
            )]
            .into(),
        );
        assert_eq!(ui.drag_value("x".into(), 0.0), 4.0);
        // The recorded widget shows the edited value, not the stale script value.
        assert!(matches!(
            ui.take_commands()[0],
            UiCommand::DragValue { current_value, .. } if current_value == 4.0
        ));
    }
}
