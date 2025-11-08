use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use hecs::Entity;
use mlua::Lua;
use serde::{Deserialize, Serialize};

use super::error::LuaScriptingError;

/// Host-facing representation of a script source.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LuaScriptSource {
    /// Inline source code bundled with an entity.
    Inline { name: Arc<str>, source: Arc<str> },
    /// External file that should be loaded at runtime.
    File { path: PathBuf },
}

impl LuaScriptSource {
    /// Construct an inline script source.
    pub fn inline(name: impl Into<String>, source: impl Into<String>) -> Self {
        Self::Inline {
            name: Arc::from(name.into().into_boxed_str()),
            source: Arc::from(source.into().into_boxed_str()),
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
                    name: Arc::from(absolute.to_string_lossy().into_owned().into_boxed_str()),
                    contents: Arc::from(contents.into_boxed_str()),
                    path: Some(absolute),
                })
            }
        }
    }
}

pub(crate) struct LoadedScript {
    pub name: Arc<str>,
    pub contents: Arc<str>,
    pub path: Option<PathBuf>,
}

/// Script execution mode based on annotations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScriptMode {
    /// No annotation - runs only in play/runtime mode
    RuntimeOnly,
    /// @editor annotation - runs only in editor mode
    EditorOnly,
    /// @tool annotation - runs in both editor and play modes
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
        if trimmed.starts_with("--") {
            let comment = trimmed[2..].trim();
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
    pub name: Arc<str>,
    #[allow(dead_code)]
    pub mode: ScriptMode,
    /// Compiled Lua chunk (stored as bytecode for reuse)
    pub chunk: Arc<Vec<u8>>,
}

impl LuaScript {
    pub fn new(name: Arc<str>, mode: ScriptMode, chunk: Vec<u8>) -> Arc<Self> {
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
