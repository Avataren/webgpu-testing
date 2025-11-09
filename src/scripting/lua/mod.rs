//! # Lua Scripting Integration
//!
//! This module provides a comprehensive Lua-based scripting system for the game engine,
//! offering an alternative to the Rune scripting language.
//!
//! ## Features
//!
//! - **Full ECS Integration** - Direct access to entities, components, and queries
//! - **Lifecycle Hooks** - `on_created`, `update`, `on_ui`, `on_destroyed` callbacks
//! - **State Persistence** - Script state survives reloads and serialization
//! - **Event System** - Inter-script communication via typed events
//! - **Command Buffer** - Deferred operations for safe mutation
//! - **Editor Integration** - UI tools with egui via `@editor` scripts
//! - **Safe Sandboxing** - Restricted file I/O and disabled dangerous functions
//!
//! ## Key Advantages Over Rune
//!
//! - **No complex ownership** - No move semantics or borrow checker in scripts
//! - **Natural iteration** - Simple loops over tables and arrays
//! - **Mature ecosystem** - Extensive documentation and community resources
//! - **Better string handling** - No lifetime or ownership issues
//! - **Familiar syntax** - Widely known language with abundant learning materials
//!
//! ## Architecture
//!
//! ### Module Organization
//!
//! - [`api`] - All API functions exposed to Lua scripts (logging, state, entities, etc.)
//! - [`component`] - [`LuaScriptComponent`] for attaching scripts to entities
//! - [`runtime`] - [`LuaScriptingRuntime`] manages Lua VM and compilation
//! - [`state`] - [`ScriptingState`] handles script lifecycle and execution
//! - [`types`] - Core types like [`LuaScriptSource`] and [`ScriptMode`]
//! - [`commands`] - Command buffer for deferred operations
//! - [`guards`] - Thread-local guards for safe context access
//! - [`plugin`] - Bevy plugin for integration
//!
//! ## Conventions
//!
//! ### Rotation Order
//!
//! The `set_rotation(entity, yaw, pitch, roll)` function uses **YXZ** Euler rotation order:
//! 1. Yaw - Rotation around Y axis (left/right)
//! 2. Pitch - Rotation around X axis (up/down)
//! 3. Roll - Rotation around Z axis (tilt/roll)
//!
//! All angles are in **radians** (90° = π/2 ≈ 1.5708).
//!
//! ### Coordinate System
//!
//! - Right-handed 3D coordinates
//! - X axis: Right
//! - Y axis: Up
//! - Z axis: Back (toward camera in default view)
//!
//! ### Array Indexing
//!
//! Lua uses **1-based indexing** for tables. All API functions that return arrays
//! (like `get_children()`, `query_entities_with_component()`) use 1-based indices.
//!
//! ### Entity Handles
//!
//! Entities are represented as 64-bit integers (i64) in Lua. These handles can
//! be stored in variables and passed between functions.
//!
//! ## Script Lifecycle
//!
//! Scripts attached to entities go through the following lifecycle:
//!
//! 1. **Creation** - Script is compiled and `on_created(self_entity)` is called once
//! 2. **Update Loop** - `update(self_entity, dt)` called every frame
//! 3. **UI Rendering** - `on_ui(self_entity, ui)` called for @editor/@tool scripts
//! 4. **Destruction** - `on_destroyed(self_entity)` called when entity is removed
//!
//! State is automatically restored after compilation, so `on_created` sees
//! previously saved state values.
//!
//! ## Script Modes
//!
//! Scripts can declare their execution mode via annotations:
//!
//! - **No annotation** - Runs only during gameplay (default)
//! - **`-- @editor`** - Runs only in editor mode, has UI access
//! - **`-- @tool`** - Runs in both editor and play modes
//!
//! ## Quick Start Example
//!
//! ```lua
//! -- Rotating cube script
//!
//! function on_created(self_entity)
//!     log_info("Cube created!")
//!     set_f64("rotation_speed", 2.0)
//! end
//!
//! function update(self_entity, dt)
//!     local speed = get_f64("rotation_speed", 1.0)
//!     rotate(self_entity, 0, 1, 0, dt * speed)
//! end
//! ```
//!
//! ## API Documentation
//!
//! For detailed API documentation, see the [`api`] module and its submodules:
//!
//! - [`api::logging`] - Debug, info, warn, error logging
//! - [`api::state`] - Persistent state storage
//! - [`api::entity`] - Entity spawning and management
//! - [`api::transform`] - Position, rotation, scale
//! - [`api::hierarchy`] - Parent-child relationships
//! - [`api::input`] - Keyboard and mouse input
//! - [`api::component`] - Component manipulation
//! - [`api::query`] - Entity queries
//! - [`api::events`] - Event system
//! - [`api::file_io`] - Sandboxed file operations
//! - [`api::ui`] - Editor UI widgets
//!
//! ## See Also
//!
//! - User guide: `scripts/LUA_SCRIPTING_README.md`
//! - Example scripts in `examples/scripts/`

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

#[cfg(test)]
mod integration_tests;

// Public exports
pub use commands::PendingGltfImport;
pub use component::{FunctionCallOutcome, LuaScriptComponent};
pub use error::LuaScriptingError;
pub use plugin::LuaScriptingPlugin;
pub use runtime::LuaScriptingRuntime;
pub use state::ScriptingState;
pub use types::{LuaScriptSource, ScriptMode};
