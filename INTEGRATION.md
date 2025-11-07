# Lua Scripting Integration Architecture

## Overview

The Lua scripting system has been successfully integrated alongside the existing Rune scripting system. Both scripting languages now run concurrently in the engine, allowing developers to choose the best tool for their needs.

## Architecture

### Core Components

The integration follows a dual-runtime architecture:

```
Scene
  └─ SceneRuntimeController
      └─ SceneRuntime
          ├─ scripting: ScriptingState (Rune)
          └─ lua_scripting: lua::state::ScriptingState (Lua)
```

### Integration Points

1. **SceneRuntime** (`src/scene/runtime_state.rs`)
   - Added `lua_scripting` field alongside existing `scripting` field
   - Both runtimes initialized in `new()`
   - Both runtimes updated in `run_scripts()`
   - Both runtimes reset in `reset_script_runtime()`

2. **SceneRuntimeController** (`src/scene/runtime_control.rs`)
   - Added `lua_scripting()` and `lua_scripting_mut()` accessor methods
   - Delegates to `SceneRuntime` methods

3. **Scene** (`src/scene/scene.rs`)
   - Added public `lua_scripting()` and `lua_scripting_mut()` accessor methods
   - Exposes Lua runtime to application code

4. **LuaScriptingPlugin** (`src/scripting/lua/plugin.rs`)
   - Now properly integrates with Scene via `lua_scripting_mut()`
   - Sets script root directory during startup
   - Follows same pattern as RuneScriptingPlugin

5. **Editor Application** (`src/bin/editor/application/mod.rs`)
   - Registers both RuneScriptingPlugin and LuaScriptingPlugin
   - Both scripting systems available in editor

## Script Execution Flow

### Runtime Mode
1. Scene calls `SceneRuntime::run_scripts(world, dt, editor_mode)`
2. Rune scripts execute via `scripting.update_scripts()`
3. Lua scripts execute via `lua_scripting.process_scripts()`
4. Both operate on the same ECS World concurrently

### Script Reset/Hot Reload
1. Scene calls `SceneRuntime::reset_script_runtime(world)`
2. State extracted from both runtimes
3. Script instances cleared
4. Runtimes reset
5. State restored after `on_created()` calls

## API Feature Parity

Both Rune and Lua support:

- **Logging**: `log_debug`, `log_info`, `log_warn`, `log_error`
- **State Management**: `get_state`, `set_state`, `get_f64`, `set_f64`, `get_bool`, `set_bool`, `get_string`, `set_string`
- **Entity Management**: `spawn_entity`, `despawn_entity`, `set_name`, `get_name`
- **Transform API**: `set_translation`, `get_world_translation`, `translate`, `set_rotation`, `get_world_rotation`, `set_scale`
- **Hierarchy API**: `set_parent`, `get_parent`, `get_children`
- **Input API**: `is_key_pressed`, `is_key_just_pressed`, `get_mouse_position`
- **Query API**: `get_entities_in_radius`, `raycast`
- **Component API**: `has_component`, `get_component`, `set_component`, `remove_component`, `add_component`
- **Event API**: `emit_event`, `subscribe_event`, `unsubscribe_event`
- **File I/O API**: `read_file`, `write_file`, `file_exists`, `delete_file`
- **Clipboard API**: `get_clipboard`, `set_clipboard`

## Script Isolation

### Rune
- Each script instance has its own context
- State isolated per entity via key-value storage

### Lua
- Each script instance has its own environment table
- Environment has `__index` metatable pointing to global Lua state
- Prevents function name collisions between scripts
- State isolated per entity via key-value storage

## Script Modes

Both systems support:
- `@tool` / `EditorOnly` - Script runs only in editor
- Runtime mode (default) - Script runs only in runtime
- `@editor` / `Both` - Script runs in both modes

## Performance Considerations

- Both runtimes execute sequentially (Rune first, then Lua)
- Command buffers used for deferred ECS operations
- Bytecode compilation for faster script loading
- Per-script environments add minimal overhead

## Example Usage

### Lua Script
```lua
-- scripts/my_script.lua
function on_created(self_entity)
    set_state("counter", 0)
    log_info("Lua script initialized")
end

function update(self_entity, dt)
    local counter = get_state("counter", 0)
    counter = counter + 1
    set_state("counter", counter)

    set_rotation(self_entity, counter * dt, 0.0, 0.0)
end
```

### Rune Script
```rune
// scripts/my_script.rn
pub fn on_created(self_entity) {
    set_state(self_entity, "counter", 0);
    log_info("Rune script initialized");
}

pub fn update(self_entity, dt) {
    let counter = get_state(self_entity, "counter", 0);
    set_state(self_entity, "counter", counter + 1);

    set_rotation(self_entity, counter * dt, 0.0, 0.0);
}
```

## Migration from Rune to Lua

See `scripts/LUA_SCRIPTING_README.md` for detailed migration guide covering:
- Array indexing differences (0-indexed vs 1-indexed)
- Data structure differences (structs vs tables)
- String formatting differences
- Function signature differences

## Future Enhancements

Potential improvements:
- UI system integration for Lua (currently Rune-only)
- Parallel script execution
- Script profiling and performance metrics
- Visual scripting integration
- Script debugging support
