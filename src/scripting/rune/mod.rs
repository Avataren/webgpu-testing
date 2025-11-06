// Modular Rune scripting system

pub mod api;
mod commands;
mod component;
mod entity_registry;
mod error;
mod guards;
mod plugin;
mod runtime;
mod state;
mod types;

// Public re-exports
pub use commands::{PendingEventSubscription, PendingEventUnsubscription, PendingGltfImport};
pub use component::RuneScriptComponent;
pub use error::RuneScriptingError;
pub use plugin::RuneScriptingPlugin;
pub use runtime::RuneScriptingRuntime;
pub use state::ScriptingState;
pub use types::{RuneScriptSource, ScriptEvent};
