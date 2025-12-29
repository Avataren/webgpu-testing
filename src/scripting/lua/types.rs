//! # Lua Scripting Type Definitions
//!
//! This module defines the core types used by the Lua scripting system.
//!
//! ## Key Types
//!
//! - [`LuaScriptSource`] - Represents where script code comes from (inline or file)
//! - [`ScriptMode`] - Execution mode based on annotations (@editor, @tool, etc.)
//! - [`LuaScript`] - Compiled Lua bytecode ready for execution
//! - [`ScriptEvent`] - Event data for inter-script communication
//! - [`ScriptStateMap`] - Persistent state storage for scripts

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use hecs::Entity;
use mlua::Lua;
use serde::{Deserialize, Serialize};

use super::error::LuaScriptingError;

/// Represents the source of a Lua script.
///
/// Scripts can either be embedded inline (useful for small scripts or code generation)
/// or loaded from external files (better for larger scripts and development).
///
/// # Example
///
/// ```rust,ignore
/// // Inline script
/// let inline = LuaScriptSource::inline("rotator", "
///     function update(self_entity, dt)
///         rotate(self_entity, 0, 1, 0, dt)
///     end
/// ");
///
/// // File-based script
/// let file = LuaScriptSource::file("scripts/player_controller.lua");
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LuaScriptSource {
    /// Inline source code bundled with an entity.
    Inline { name: String, source: String },
    /// External file that should be loaded at runtime.
    File { path: PathBuf },
}

impl LuaScriptSource {
    /// Construct an inline script source.
    pub fn inline(name: impl Into<String>, source: impl Into<String>) -> Self {
        Self::Inline {
            name: name.into(),
            source: source.into(),
        }
    }

    /// Construct a file-backed script source.
    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self::File { path: path.into() }
    }

    pub(crate) fn load(
        &self,
        script_root: Option<&Path>,
    ) -> Result<LoadedScript, LuaScriptingError> {
        match self {
            Self::Inline { name, source } => Ok(LoadedScript {
                name: name.clone(),
                contents: source.clone(),
                path: None,
            }),
            Self::File { path } => {
                let absolute = if path.is_absolute() {
                    path.clone()
                } else if let Some(root) = script_root {
                    root.join(path)
                } else {
                    path.clone()
                };

                let contents =
                    std::fs::read_to_string(&absolute).map_err(|source| LuaScriptingError::Io {
                        path: absolute.clone(),
                        source,
                    })?;

                Ok(LoadedScript {
                    name: absolute.to_string_lossy().into_owned(),
                    contents,
                    path: Some(absolute),
                })
            }
        }
    }
}

pub(crate) struct LoadedScript {
    pub name: String,
    pub contents: String,
    pub path: Option<PathBuf>,
}

/// Script execution mode determined by annotations.
///
/// Scripts can declare their intended execution context using comment annotations
/// at the top of the file:
///
/// - No annotation → `RuntimeOnly` (default, for gameplay scripts)
/// - `-- @editor` → `EditorOnly` (for editor tools and utilities)
/// - `-- @tool` → `Both` (for tools that work in editor and runtime)
///
/// # Example
///
/// ```lua
/// -- @editor
/// -- This script only runs in editor mode
///
/// function on_ui(self_entity, ui)
///     ui:heading("Editor Tool")
///     if ui:button("Do Something") then
///         log_info("Button clicked in editor")
///     end
/// end
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScriptMode {
    /// No annotation - runs only in play/runtime mode
    RuntimeOnly,
    /// `@editor` annotation - runs only in editor mode
    EditorOnly,
    /// `@tool` annotation - runs in both editor and play modes
    Both,
}

impl Default for ScriptMode {
    fn default() -> Self {
        Self::RuntimeOnly
    }
}

/// Parse metadata annotations from script source to determine execution mode.
/// Looks for `-- @editor` or `-- @tool` comment annotations.
pub(crate) fn parse_script_mode_annotation(source: &str) -> ScriptMode {
    for line in source.lines() {
        let trimmed = line.trim();
        if let Some(comment) = trimmed.strip_prefix("--") {
            let comment = comment.trim();
            if comment == "@tool" {
                return ScriptMode::Both;
            } else if comment == "@editor" || comment == "@editor_tool" {
                // @editor_tool is legacy, map it to @editor
                return ScriptMode::EditorOnly;
            }
        }
        // Stop at first non-comment, non-empty line (annotations should be at the top)
        if !trimmed.is_empty() && !trimmed.starts_with("--") {
            break;
        }
    }
    ScriptMode::RuntimeOnly
}

/// Compiled Lua script with callable functions.
#[derive(Clone)]
pub(crate) struct LuaScript {
    pub name: String,
    #[allow(dead_code)]
    pub mode: ScriptMode,
    /// Compiled Lua chunk (stored as bytecode for reuse)
    pub chunk: Arc<Vec<u8>>,
}

impl LuaScript {
    pub fn new(name: String, mode: ScriptMode, chunk: Vec<u8>) -> Arc<Self> {
        Arc::new(Self {
            name,
            mode,
            chunk: Arc::new(chunk),
        })
    }

    /// Load this script into a Lua context and return its global environment.
    #[allow(dead_code)]
    pub fn load(&self, lua: &Lua) -> Result<mlua::Table, mlua::Error> {
        // Load the chunk from bytecode
        lua.load(&**self.chunk).exec()?;

        // Return the global table for this script
        Ok(lua.globals())
    }
}

/// State storage for Lua scripts - maps (entity_id, key) to serialized JSON values.
/// We use serde_json::Value instead of LuaValue to avoid lifetime issues.
pub(crate) type ScriptStateMap = HashMap<(i64, String), serde_json::Value>;

// ============================================================================
// EVENT SYSTEM DATA STRUCTURES
// ============================================================================

/// An event that can be emitted by scripts.
#[derive(Debug, Clone)]
pub struct ScriptEvent {
    pub name: String,
    pub data: serde_json::Value,
}

/// A subscription to an event by a specific entity.
#[derive(Debug, Clone)]
pub(crate) struct EventSubscription {
    pub entity_id: Entity,
    #[allow(dead_code)]
    pub callback_name: String,
}

/// Map of event names to their subscribers.
pub(crate) type EventSubscriptions = HashMap<String, Vec<EventSubscription>>;

// ============================================================================
// COROUTINE SYSTEM DATA STRUCTURES
// ============================================================================

/// Unique identifier for a coroutine.
pub(crate) type CoroutineId = i64;

/// Status of a coroutine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoroutineStatus {
    /// Coroutine is currently executing
    Running,
    /// Coroutine is suspended and can be resumed
    Suspended,
    /// Coroutine has completed execution
    Dead,
}

impl CoroutineStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Suspended => "suspended",
            Self::Dead => "dead",
        }
    }
}

/// Represents a wait state for time-based delays.
#[derive(Debug, Clone, Copy)]
pub(crate) enum WaitState {
    /// No wait - coroutine can resume immediately
    None,
    /// Wait for a specified duration in seconds
    Seconds(f64),
    /// Wait for a specified number of frames
    Frames(u32),
}

/// State for a single coroutine.
pub(crate) struct CoroutineState {
    /// The Lua thread (coroutine)
    pub thread: mlua::Thread,
    /// Current status of the coroutine
    pub status: CoroutineStatus,
    /// Wait state for time-based delays
    pub wait_state: WaitState,
    /// Accumulated time for Seconds wait
    pub accumulated_time: f64,
}

impl CoroutineState {
    pub fn new(thread: mlua::Thread) -> Self {
        Self {
            thread,
            status: CoroutineStatus::Suspended,
            wait_state: WaitState::None,
            accumulated_time: 0.0,
        }
    }

    /// Check if this coroutine is ready to resume based on its wait state.
    pub fn is_ready(&self, _dt: f64) -> bool {
        match self.wait_state {
            WaitState::None => true,
            WaitState::Seconds(duration) => self.accumulated_time >= duration,
            WaitState::Frames(count) => count == 0,
        }
    }

    /// Update the wait state with the delta time.
    pub fn update(&mut self, dt: f64) {
        match self.wait_state {
            WaitState::Seconds(_) => {
                self.accumulated_time += dt;
            }
            WaitState::Frames(ref mut count) => {
                if *count > 0 {
                    *count -= 1;
                }
            }
            WaitState::None => {}
        }
    }

    /// Reset the wait state after resuming.
    pub fn reset_wait(&mut self) {
        self.wait_state = WaitState::None;
        self.accumulated_time = 0.0;
    }
}

/// Map of coroutine IDs to their states.
pub(crate) type CoroutineMap = HashMap<CoroutineId, CoroutineState>;
