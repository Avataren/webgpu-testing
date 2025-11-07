use std::path::PathBuf;

use crate::app::{AppBuilder, Plugin, StartupContext};

/// Plugin that configures Lua scripting for the application.
#[derive(Default)]
pub struct LuaScriptingPlugin {
    script_root: Option<PathBuf>,
}

impl LuaScriptingPlugin {
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

impl Plugin for LuaScriptingPlugin {
    fn build(&self, app: &mut AppBuilder) {
        let script_root = self.script_root.clone();
        if let Some(root) = script_root {
            app.add_startup_system(move |_ctx: &mut StartupContext<'_>| {
                // TODO: Once we have lua_scripting_mut() on Scene, uncomment this:
                // ctx.scene
                //     .lua_scripting_mut()
                //     .runtime_mut()
                //     .set_script_root(root.clone());
                log::info!("Lua scripting plugin initialized with root: {:?}", root);
            });
        }
    }
}
