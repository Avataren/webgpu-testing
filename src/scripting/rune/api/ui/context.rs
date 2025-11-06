use super::commands::{UiCommand, UiResponse};
use rune::Any;
use std::cell::RefCell;
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
    responses: Rc<RefCell<Vec<Option<UiResponse>>>>,
}

impl UiContext {
    /// Create a new UI context.
    pub fn new() -> Self {
        Self {
            commands: Rc::new(RefCell::new(Vec::new())),
            responses: Rc::new(RefCell::new(Vec::new())),
        }
    }

    /// Get the recorded commands for rendering.
    pub fn take_commands(&self) -> Vec<UiCommand> {
        self.commands.borrow_mut().drain(..).collect()
    }

    /// Set responses from rendering.
    pub fn set_responses(&self, responses: Vec<Option<UiResponse>>) {
        *self.responses.borrow_mut() = responses;
    }

    /// Display a text label.
    #[rune::function(instance)]
    pub fn label(&self, text: String) {
        self.commands.borrow_mut().push(UiCommand::Label { text });
        self.responses.borrow_mut().push(None);
    }

    /// Display a button and return whether it was clicked.
    #[rune::function(instance)]
    pub fn button(&self, text: String) -> bool {
        let index = self.commands.borrow().len();
        self.commands.borrow_mut().push(UiCommand::Button {
            text: text.clone(),
        });
        self.responses.borrow_mut().push(None);

        // Check if we have a response from the previous frame
        let responses = self.responses.borrow();
        if let Some(Some(response)) = responses.get(index) {
            response.clicked
        } else {
            false
        }
    }

    /// Display a heading.
    #[rune::function(instance)]
    pub fn heading(&self, text: String) {
        self.commands.borrow_mut().push(UiCommand::Heading { text });
        self.responses.borrow_mut().push(None);
    }

    /// Display a separator line.
    #[rune::function(instance)]
    pub fn separator(&self) {
        self.commands.borrow_mut().push(UiCommand::Separator);
        self.responses.borrow_mut().push(None);
    }
}

impl Default for UiContext {
    fn default() -> Self {
        Self::new()
    }
}
