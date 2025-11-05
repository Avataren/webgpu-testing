use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use hecs::Entity;
use rune::Value;

use super::error::RuneScriptingError;

/// Host-facing representation of a script source.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RuneScriptSource {
    /// Inline source code bundled with an entity.
    Inline { name: Arc<str>, source: Arc<str> },
    /// External file that should be loaded at runtime.
    File { path: PathBuf },
}

impl RuneScriptSource {
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

    pub(crate) fn load(&self, script_root: Option<&Path>) -> Result<LoadedScript, RuneScriptingError> {
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

                let contents = std::fs::read_to_string(&absolute).map_err(|source| {
                    RuneScriptingError::Io {
                        path: absolute.clone(),
                        source,
                    }
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

/// Compiled Rune script.
#[derive(Debug)]
pub(crate) struct RuneScript {
    pub _name: Arc<str>,
    pub unit: Arc<rune::Unit>,
}

impl RuneScript {
    pub fn new(name: Arc<str>, unit: rune::Unit) -> Arc<Self> {
        Arc::new(Self {
            _name: name,
            unit: Arc::new(unit),
        })
    }
}

pub(crate) type ScriptStateMap = HashMap<(i64, String), Value>;

// ============================================================================
// EVENT SYSTEM DATA STRUCTURES
// ============================================================================

/// An event that can be emitted by scripts.
#[derive(Debug, Clone)]
pub struct ScriptEvent {
    pub name: String,
    pub data: Value,
}

/// A subscription to an event by a specific entity.
#[derive(Debug, Clone)]
pub(crate) struct EventSubscription {
    pub entity_id: Entity,
    pub callback_name: String,
}

/// Map of event names to their subscribers.
pub(crate) type EventSubscriptions = HashMap<String, Vec<EventSubscription>>;
