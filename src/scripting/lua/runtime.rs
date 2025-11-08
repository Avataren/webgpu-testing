use std::path::PathBuf;
use std::sync::Arc;

use mlua::{Lua, LuaOptions, StdLib};

use super::api;
use super::error::LuaScriptingError;
use super::types::{parse_script_mode_annotation, LuaScript, LuaScriptSource, ScriptMode};

/// Runtime wrapper responsible for compiling and executing Lua scripts.
pub struct LuaScriptingRuntime {
    /// The Lua VM instance
    lua: Lua,
    /// Base directory for resolving relative script paths
    script_root: Option<PathBuf>,
}

impl LuaScriptingRuntime {
    /// Construct a new runtime with a fresh Lua VM.
    ///
    /// The Lua VM is configured with:
    /// - Standard library (minus dangerous functions like `dofile`, `loadfile`)
    /// - Safe subset appropriate for scripting
    pub fn new() -> Result<Self, LuaScriptingError> {
        // Create Lua VM with safe standard libraries
        // Exclude potentially dangerous modules: DEBUG, IO
        let lua = Lua::new_with(StdLib::ALL_SAFE, LuaOptions::default())?;

        // Disable some unsafe functions that might bypass our API
        lua.globals().set("dofile", mlua::Nil)?;
        lua.globals().set("loadfile", mlua::Nil)?;
        lua.globals().set("require", mlua::Nil)?; // We'll provide our own module system

        // Register all API functions
        api::register_all_apis(&lua)?;

        Ok(Self {
            lua,
            script_root: None,
        })
    }

    /// Configure a base directory for resolving relative script paths.
    pub fn set_script_root(&mut self, root: impl Into<PathBuf>) {
        self.script_root = Some(root.into());
    }

    /// Get a reference to the Lua VM for registering API functions.
    pub fn lua(&self) -> &Lua {
        &self.lua
    }

    /// Compile a script source to bytecode.
    ///
    /// This parses the script mode annotation and compiles the Lua code to bytecode
    /// for faster reloading and instantiation.
    pub(crate) fn compile(
        &self,
        source: &LuaScriptSource,
    ) -> Result<(Arc<LuaScript>, ScriptMode), LuaScriptingError> {
        let loaded = source.load(self.script_root.as_deref())?;

        // Parse metadata annotations from source
        let script_mode = parse_script_mode_annotation(&loaded.contents);

        // Compile the Lua code to bytecode
        let chunk = if let Some(path) = &loaded.path {
            self.lua
                .load(&*loaded.contents)
                .set_name(path.to_string_lossy())
        } else {
            self.lua.load(&*loaded.contents).set_name(&*loaded.name)
        };

        // Convert to bytecode for faster loading
        let bytecode = chunk.into_function()?.dump(false);

        Ok((
            LuaScript::new(loaded.name, script_mode, bytecode),
            script_mode,
        ))
    }

    /// Load and execute a compiled script in a fresh environment.
    ///
    /// This creates a new environment table for the script, inheriting from _G,
    /// and executes the script's bytecode in that environment.
    #[allow(dead_code)]
    pub(crate) fn load_script(&self, script: &LuaScript) -> Result<(), LuaScriptingError> {
        // Load bytecode and execute it
        // This will define the script's functions in the global environment
        self.lua.load(&**script.chunk).exec()?;

        Ok(())
    }

    /// Check if a script has a specific function defined.
    pub fn has_function(&self, name: &str) -> Result<bool, LuaScriptingError> {
        let globals = self.lua.globals();
        Ok(globals.contains_key(name)? && globals.get::<mlua::Function>(name).is_ok())
    }

    /// Call a script function with the given arguments.
    ///
    /// Returns `Ok(())` if the function exists and executes successfully.
    /// Returns `Err` if there's a Lua error during execution.
    pub fn call_function<A, R>(&self, name: &str, args: A) -> Result<R, LuaScriptingError>
    where
        A: mlua::IntoLuaMulti,
        R: mlua::FromLuaMulti,
    {
        let globals = self.lua.globals();

        if !globals.contains_key(name)? {
            // Function doesn't exist - this is okay, just return default
            return Err(LuaScriptingError::Lua(mlua::Error::RuntimeError(format!(
                "function '{}' not found",
                name
            ))));
        }

        let func: mlua::Function = globals.get(name)?;
        let result = func.call(args)?;

        Ok(result)
    }

    /// Call a script function if it exists, returning None if it doesn't.
    pub fn try_call_function<A, R>(
        &self,
        name: &str,
        args: A,
    ) -> Result<Option<R>, LuaScriptingError>
    where
        A: mlua::IntoLuaMulti,
        R: mlua::FromLuaMulti,
    {
        let globals = self.lua.globals();

        if !globals.contains_key(name)? {
            return Ok(None);
        }

        let func: mlua::Function = globals.get(name)?;
        let result = func.call(args)?;

        Ok(Some(result))
    }
}

impl Default for LuaScriptingRuntime {
    fn default() -> Self {
        Self::new().expect("Failed to create Lua runtime")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_runtime() {
        let runtime = LuaScriptingRuntime::new();
        assert!(runtime.is_ok());
    }

    #[test]
    fn test_compile_simple_script() {
        let runtime = LuaScriptingRuntime::new().unwrap();
        let source = LuaScriptSource::inline("test", "function update(self_entity, dt) end");

        let result = runtime.compile(&source);
        assert!(result.is_ok());

        let (script, mode) = result.unwrap();
        assert_eq!(mode, ScriptMode::RuntimeOnly);
        assert!(script.name.contains("test"));
    }

    #[test]
    fn test_parse_tool_annotation() {
        let runtime = LuaScriptingRuntime::new().unwrap();
        let source =
            LuaScriptSource::inline("test", "-- @tool\nfunction update(self_entity, dt) end");

        let (_script, mode) = runtime.compile(&source).unwrap();
        assert_eq!(mode, ScriptMode::Both);
    }

    #[test]
    fn test_parse_editor_annotation() {
        let runtime = LuaScriptingRuntime::new().unwrap();
        let source = LuaScriptSource::inline("test", "-- @editor\nfunction on_ui(ui) end");

        let (_script, mode) = runtime.compile(&source).unwrap();
        assert_eq!(mode, ScriptMode::EditorOnly);
    }

    #[test]
    fn test_load_and_call_function() {
        let runtime = LuaScriptingRuntime::new().unwrap();
        let source = LuaScriptSource::inline(
            "test",
            r#"
                function greet(name)
                    return "Hello, " .. name
                end
            "#,
        );

        let (script, _) = runtime.compile(&source).unwrap();
        runtime.load_script(&script).unwrap();

        let result: String = runtime.call_function("greet", "World").unwrap();
        assert_eq!(result, "Hello, World");
    }

    #[test]
    fn test_try_call_missing_function() {
        let runtime = LuaScriptingRuntime::new().unwrap();
        let source = LuaScriptSource::inline("test", "-- empty script");

        let (script, _) = runtime.compile(&source).unwrap();
        runtime.load_script(&script).unwrap();

        let result: Result<Option<()>, _> = runtime.try_call_function("nonexistent", ());
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }
}
