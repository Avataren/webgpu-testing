use crate::scripting::{RuneScriptComponent, RuneScriptSource};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SerializedRuneScriptSource {
    Inline { name: String, source: String },
    File { path: PathBuf },
}

impl From<&RuneScriptSource> for SerializedRuneScriptSource {
    fn from(source: &RuneScriptSource) -> Self {
        match source {
            RuneScriptSource::Inline { name, source } => Self::Inline {
                name: name.to_string(),
                source: source.to_string(),
            },
            RuneScriptSource::File { path } => Self::File { path: path.clone() },
        }
    }
}

impl From<SerializedRuneScriptSource> for RuneScriptSource {
    fn from(serialized: SerializedRuneScriptSource) -> Self {
        match serialized {
            SerializedRuneScriptSource::Inline { name, source } => {
                RuneScriptSource::inline(name, source)
            }
            SerializedRuneScriptSource::File { path } => RuneScriptSource::file(path),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedRuneScript {
    pub source: SerializedRuneScriptSource,
    pub created_called: bool,
}

impl From<&RuneScriptComponent> for SerializedRuneScript {
    fn from(component: &RuneScriptComponent) -> Self {
        Self {
            source: SerializedRuneScriptSource::from(component.source()),
            created_called: component.created_called(),
        }
    }
}

impl SerializedRuneScript {
    pub(crate) fn into_component(self) -> RuneScriptComponent {
        let mut component = RuneScriptComponent::new(self.source.into());
        component.set_created_called(self.created_called);
        component
    }
}
