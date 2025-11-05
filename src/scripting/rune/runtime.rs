use std::path::PathBuf;
use std::sync::Arc;

use rune::runtime::RuntimeContext;
use rune::{Context, Diagnostics, Source, Sources};

use super::api::script_module;
use super::component::RuneScriptInstance;
use super::error::RuneScriptingError;
use super::types::{RuneScript, RuneScriptSource};

/// Runtime wrapper responsible for compiling and executing scripts.
pub struct RuneScriptingRuntime {
    context: Context,
    runtime: Arc<RuntimeContext>,
    script_root: Option<PathBuf>,
}

impl RuneScriptingRuntime {
    /// Construct a new runtime with the default Rune modules installed.
    pub fn new() -> Result<Self, RuneScriptingError> {
        let mut context = Context::with_default_modules()?;
        context.install(&script_module()?)?;
        let runtime = Arc::new(context.runtime()?);
        Ok(Self {
            context,
            runtime,
            script_root: None,
        })
    }

    /// Configure a base directory for resolving relative script paths.
    pub fn set_script_root(&mut self, root: impl Into<PathBuf>) {
        self.script_root = Some(root.into());
    }

    pub(crate) fn compile(
        &mut self,
        source: &RuneScriptSource,
    ) -> Result<Arc<RuneScript>, RuneScriptingError> {
        let loaded = source.load(self.script_root.as_deref())?;

        let mut sources = Sources::new();
        let source = if let Some(path) = &loaded.path {
            Source::with_path(loaded.name.as_ref(), loaded.contents.as_ref(), path)?
        } else {
            Source::new(loaded.name.as_ref(), loaded.contents.as_ref())?
        };
        sources.insert(source)?;

        let mut diagnostics = Diagnostics::new();
        let result = rune::prepare(&mut sources)
            .with_context(&self.context)
            .with_diagnostics(&mut diagnostics)
            .build();

        if diagnostics.has_error() {
            return Err(RuneScriptingError::Compile {
                name: loaded.name.clone(),
                message: format!("{diagnostics:?}"),
            });
        }

        let unit = result.map_err(|error| RuneScriptingError::Compile {
            name: loaded.name.clone(),
            message: error.to_string(),
        })?;

        Ok(RuneScript::new(loaded.name, unit))
    }

    pub(crate) fn instantiate(&self, script: Arc<RuneScript>, source: RuneScriptSource) -> RuneScriptInstance {
        RuneScriptInstance::new(self.runtime.clone(), script, source)
    }
}
