// Lua scripting integration module
//
// This module provides a Lua-based scripting system as an alternative to Rune.
// It offers:
// - Full ECS integration with component access
// - Lifecycle hooks (on_created, update, on_ui, on_destroyed)
// - State persistence across script reloads
// - Event system for inter-script communication
// - Command buffer pattern for deferred operations
//
// Key advantages over Rune:
// - No complex value ownership semantics
// - Natural iteration over tables and arrays
// - Mature ecosystem with extensive documentation
// - Better string handling without move semantics

pub mod api;
pub mod commands;
pub mod component;
pub mod entity_registry;
pub mod error;
pub mod guards;
pub mod plugin;
pub mod runtime;
pub mod state;
pub mod types;

// Public exports
pub use component::{LuaScriptComponent, LuaScriptInstance, FunctionCallOutcome};
pub use error::LuaScriptingError;
pub use plugin::LuaScriptingPlugin;
pub use runtime::LuaScriptingRuntime;
pub use state::ScriptingState;
pub use types::{LuaScriptSource, ScriptMode};
