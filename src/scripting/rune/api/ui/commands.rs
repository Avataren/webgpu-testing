/// UI commands that are recorded during script execution and replayed with real egui::Ui.
///
/// This pattern solves the lifetime issues with egui::Ui by deferring actual rendering
/// until after the script VM has completed execution.

#[derive(Debug, Clone)]
pub enum UiCommand {
    Label { text: String },
    Button { text: String },
    Heading { text: String },
    Separator,
}

impl UiCommand {
    /// Render this command using a real egui::Ui context.
    #[cfg(feature = "egui")]
    pub fn render(&self, ui: &mut crate::ui::egui_integration::egui::Ui) -> Option<UiResponse> {
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
        }
    }
}

/// Response from a UI widget.
#[derive(Debug, Clone, Copy, Default)]
pub struct UiResponse {
    pub clicked: bool,
    pub hovered: bool,
}
