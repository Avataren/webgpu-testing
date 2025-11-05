use std::path::PathBuf;

use crate::app::{AppBuilder, Plugin, StartupContext};

/// Plugin that configures Rune scripting for the editor.
#[derive(Default)]
pub struct RuneScriptingPlugin {
    script_root: Option<PathBuf>,
}

impl RuneScriptingPlugin {
    /// Create a new plugin.
    pub fn new() -> Self {
        Self { script_root: None }
    }

    /// Configure the base directory for script loading.
    pub fn with_script_root(mut self, path: impl Into<PathBuf>) -> Self {
        self.script_root = Some(path.into());
        self
    }
}

impl Plugin for RuneScriptingPlugin {
    fn build(&self, app: &mut AppBuilder) {
        let script_root = self.script_root.clone();
        if let Some(root) = script_root {
            app.add_startup_system(move |ctx: &mut StartupContext<'_>| {
                ctx.scene
                    .scripting_mut()
                    .runtime_mut()
                    .set_script_root(root.clone());
            });
        }
    }
}
