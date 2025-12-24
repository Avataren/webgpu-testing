//! Integration tests for Lua scripting lifecycle and API modules.

#[cfg(test)]
mod tests {
    use crate::scripting::lua::component::LuaScriptComponent;
    use crate::scripting::lua::error::LuaScriptingError;
    use crate::scripting::lua::runtime::LuaScriptingRuntime;
    use crate::scripting::lua::state::ScriptingState;
    use crate::scripting::lua::types::LuaScriptSource;
    use hecs::World;

    /// Test creating a new scripting state.
    #[test]
    fn test_create_scripting_state() -> Result<(), LuaScriptingError> {
        let state = ScriptingState::new()?;
        // State created successfully, runtime exists
        let _runtime = state.runtime();
        Ok(())
    }

    /// Test creating script components.
    #[test]
    fn test_create_script_component() {
        let source = LuaScriptSource::inline("test", "function on_create() end");
        let _component = LuaScriptComponent::new(source.clone());
        // Component created successfully
    }

    /// Test script source creation methods.
    #[test]
    fn test_script_source_inline() {
        let source = LuaScriptSource::inline("test_script", "return 42");
        match source {
            LuaScriptSource::Inline { name, source } => {
                assert_eq!(name.as_str(), "test_script");
                assert_eq!(source.as_str(), "return 42");
            }
            _ => panic!("Expected inline source"),
        }
    }

    #[test]
    fn test_script_source_file() {
        let source = LuaScriptSource::file("scripts/test.lua");
        match source {
            LuaScriptSource::File { path } => {
                assert_eq!(path.to_str().unwrap(), "scripts/test.lua");
            }
            _ => panic!("Expected file source"),
        }
    }

    /// Test runtime creation.
    #[test]
    fn test_create_runtime() -> Result<(), LuaScriptingError> {
        let _runtime = LuaScriptingRuntime::new()?;
        // Runtime created successfully
        Ok(())
    }

    /// Test basic Lua code execution through runtime.
    #[test]
    fn test_execute_lua_code() -> Result<(), Box<dyn std::error::Error>> {
        let runtime = LuaScriptingRuntime::new()?;

        // Execute simple Lua code
        let result = runtime.lua().load("return 1 + 1").eval::<i32>()?;
        assert_eq!(result, 2);

        Ok(())
    }

    /// Test state extraction and restoration.
    #[test]
    fn test_state_extraction() -> Result<(), LuaScriptingError> {
        let state = ScriptingState::new()?;
        let world = World::new();

        // Extract state from empty world
        let extracted = state.extract_state_for_world(&world);
        assert!(extracted.is_empty());

        Ok(())
    }


    /// Test multiple script sources.
    #[test]
    fn test_multiple_script_sources() {
        let inline1 = LuaScriptSource::inline("script1", "function on_create() end");
        let inline2 = LuaScriptSource::inline("script2", "function on_update() end");
        let file1 = LuaScriptSource::file("test1.lua");

        let _comp1 = LuaScriptComponent::new(inline1);
        let _comp2 = LuaScriptComponent::new(inline2);
        let _comp3 = LuaScriptComponent::new(file1);

        // All components created successfully
    }

    /// Test script source equality.
    #[test]
    fn test_script_source_equality() {
        let source1 = LuaScriptSource::inline("test", "code");
        let source2 = LuaScriptSource::inline("test", "code");
        let source3 = LuaScriptSource::inline("test", "different");

        assert_eq!(source1, source2);
        assert_ne!(source1, source3);
    }

    /// Test script source cloning.
    #[test]
    fn test_script_source_clone() {
        let source = LuaScriptSource::inline("test", "code");
        let cloned = source.clone();
        assert_eq!(source, cloned);
    }

    /// Test world with script components.
    #[test]
    fn test_world_with_scripts() {
        let mut world = World::new();

        let script =
            LuaScriptComponent::new(LuaScriptSource::inline("test", "function on_create() end"));

        // Spawn entities with scripts
        let e1 = world.spawn((script.clone(),));
        let e2 = world.spawn((script.clone(),));
        let e3 = world.spawn((script,));

        // Verify entities exist
        assert!(world.contains(e1));
        assert!(world.contains(e2));
        assert!(world.contains(e3));
    }

    /// Test error handling for runtime creation.
    #[test]
    fn test_runtime_error_handling() {
        // Creating a runtime should succeed
        let result = LuaScriptingRuntime::new();
        assert!(result.is_ok());

        // Multiple runtimes can coexist
        let _runtime1 = LuaScriptingRuntime::new().unwrap();
        let _runtime2 = LuaScriptingRuntime::new().unwrap();
    }

    /// Test process_scripts exists and can be called.
    #[test]
    fn test_process_scripts_exists() -> Result<(), LuaScriptingError> {
        let mut state = ScriptingState::new()?;
        let mut world = World::new();

        // Add a simple script
        let script = LuaScriptComponent::new(LuaScriptSource::inline("empty", "-- empty script"));
        world.spawn((script,));

        // Process scripts should not panic
        let _result = state.process_scripts(&mut world, 0.016, false);

        Ok(())
    }

    /// Test that runtime can be accessed.
    #[test]
    fn test_runtime_access() -> Result<(), LuaScriptingError> {
        let state = ScriptingState::new()?;
        let _runtime = state.runtime();
        Ok(())
    }

    /// Test runtime reset.
    #[test]
    fn test_runtime_reset() -> Result<(), LuaScriptingError> {
        let mut state = ScriptingState::new()?;
        state.reset_runtime();
        Ok(())
    }
}
