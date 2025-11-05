use std::path::PathBuf;
use std::sync::Arc;

use hecs::{ComponentError, NoSuchEntity};
use rune::alloc::Error as RuneAllocError;
use rune::ContextError;
use thiserror::Error;

use crate::scripting::component_registry::ComponentRegistryError;

/// Error type produced by the Rune scripting integration.
#[derive(Debug, Error)]
pub enum RuneScriptingError {
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
    /// A virtual machine execution error occurred.
    #[error("{0}")]
    Vm(#[from] rune::runtime::VmError),
    /// Allocation within the Rune runtime failed.
    #[error("failed to allocate Rune resources: {0}")]
    Allocation(#[from] RuneAllocError),
    /// An error occurred while mutating the ECS world.
    #[error("failed to mutate ECS world: {0}")]
    Hecs(#[from] ComponentError),
    /// Tried to access an entity that no longer exists.
    #[error("failed to access entity: {0}")]
    MissingEntity(#[from] NoSuchEntity),
    /// Failed to initialize the Rune runtime context.
    #[error("failed to initialize Rune context: {0}")]
    Context(#[from] ContextError),
    /// Component registry error.
    #[error("component registry error: {0}")]
    ComponentRegistry(#[from] ComponentRegistryError),
}
