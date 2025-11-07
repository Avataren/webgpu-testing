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
    ) -> Result<(Arc<RuneScript>, ScriptMode), RuneScriptingError> {
        let loaded = source.load(self.script_root.as_deref())?;

        // Parse metadata annotations from source
        let script_mode = parse_script_mode_annotation(&loaded.contents);

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

        Ok((RuneScript::new(loaded.name, unit), script_mode))
    }

    pub(crate) fn instantiate(&self, script: Arc<RuneScript>, source: RuneScriptSource) -> RuneScriptInstance {
        RuneScriptInstance::new(self.runtime.clone(), script, source)
    }
}

/// Script execution mode based on annotations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptMode {
    /// No annotation - runs only in play/runtime mode
    RuntimeOnly,
    /// @editor annotation - runs only in editor mode
    EditorOnly,
    /// @tool annotation - runs in both editor and play modes
    Both,
}

/// Parse metadata annotations from script source to determine execution mode.
/// Looks for `// @editor` or `// @tool` comment annotations.
fn parse_script_mode_annotation(source: &str) -> ScriptMode {
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("//") {
            let comment = trimmed[2..].trim();
            if comment == "@tool" {
                return ScriptMode::Both;
            } else if comment == "@editor" || comment == "@editor_tool" {
                // @editor_tool is legacy, map it to @editor
                return ScriptMode::EditorOnly;
            }
        }
        // Stop at first non-comment, non-empty line (annotations should be at the top)
        if !trimmed.is_empty() && !trimmed.starts_with("//") {
            break;
        }
    }
    ScriptMode::RuntimeOnly
}

