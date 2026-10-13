use crate::project::resolve_project_path;
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
            SerializedLuaScriptSource::File { path } => {
                LuaScriptSource::file(resolve_script_path(path))
            }
        }
    }
}

/// Saved projects store their scripts relative to the project; resolve those against
/// the active project. Older saves may hold paths relative to the working directory,
/// which are kept when the project has no such file.
fn resolve_script_path(path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        return path;
    }
    let resolved = resolve_project_path(&path);
    if resolved.exists() {
        resolved
    } else {
        path
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
