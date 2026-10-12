use crate::scripting::{LuaScriptComponent, LuaScriptSource};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SerializedLuaScriptSource {
    Inline { name: String, source: String },
    File { path: PathBuf },
}

impl From<&LuaScriptSource> for SerializedLuaScriptSource {
    fn from(source: &LuaScriptSource) -> Self {
        match source {
            LuaScriptSource::Inline { name, source } => Self::Inline {
                name: name.to_string(),
                source: source.to_string(),
            },
            LuaScriptSource::File { path } => Self::File { path: path.clone() },
        }
    }
}

impl From<SerializedLuaScriptSource> for LuaScriptSource {
    fn from(serialized: SerializedLuaScriptSource) -> Self {
        match serialized {
            SerializedLuaScriptSource::Inline { name, source } => {
                LuaScriptSource::inline(name, source)
            }
            SerializedLuaScriptSource::File { path } => LuaScriptSource::file(path),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedLuaScript {
    pub source: SerializedLuaScriptSource,
    /// Legacy runtime flag from older save files. It is ignored on load: whether
    /// `on_created` has run is runtime state, so a loaded script always starts fresh.
    #[serde(default, skip_serializing)]
    pub created_called: bool,
}

impl From<&LuaScriptComponent> for SerializedLuaScript {
    fn from(component: &LuaScriptComponent) -> Self {
        Self {
            source: SerializedLuaScriptSource::from(component.source()),
            created_called: false,
        }
    }
}

impl SerializedLuaScript {
    pub(crate) fn into_component(self) -> LuaScriptComponent {
        LuaScriptComponent::new(self.source.into())
    }
}

// Legacy type aliases for backward compatibility with old save files
#[deprecated(note = "Use SerializedLuaScriptSource instead")]
#[allow(dead_code)]
pub type SerializedRuneScriptSource = SerializedLuaScriptSource;

#[deprecated(note = "Use SerializedLuaScript instead")]
#[allow(dead_code)]
pub type SerializedRuneScript = SerializedLuaScript;
