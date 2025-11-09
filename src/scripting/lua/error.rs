use std::path::PathBuf;
use std::sync::Arc;

use hecs::{ComponentError, NoSuchEntity};
use thiserror::Error;

/// Error type produced by the Lua scripting integration.
#[derive(Debug, Error)]
pub enum LuaScriptingError {
    /// Loading a script from disk failed.
    #[error("failed to load script `{path}`: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// Compiling a script failed.
    #[error("failed to compile script `{name}`: {message}")]
    Compile { name: Arc<str>, message: String },
    /// A Lua execution error occurred.
    #[error("Lua error: {0}")]
    Lua(#[from] mlua::Error),
    /// An error occurred while mutating the ECS world.
    #[error("failed to mutate ECS world: {0}")]
    Hecs(#[from] ComponentError),
    /// Tried to access an entity that no longer exists.
    #[error("failed to access entity: {0}")]
    MissingEntity(#[from] NoSuchEntity),
    /// Serialization error when converting Lua values.
    #[error("serialization error: {0}")]
    Serialization(String),
}

impl From<serde_json::Error> for LuaScriptingError {
    fn from(err: serde_json::Error) -> Self {
        Self::Serialization(err.to_string())
    }
}
