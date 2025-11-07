# Hot Reload Design Document

> Sprint 1 - Script Hot Reload Implementation
> Created: 2025-11-06

## Overview

Enable automatic reloading of RuneScript plugins when their source files change, with state preservation and error handling.

## Goals

1. **Fast Iteration**: < 3 seconds from save to reload
2. **State Preservation**: 80%+ of state preserved across reloads
3. **Error Handling**: Show compilation errors in UI, keep old version on error
4. **Zero Crashes**: Plugin errors don't crash editor

## Architecture

### Components

```
┌─────────────────────────────────────────────────────────┐
│ ScriptWatcher (new)                                     │
│ - Monitors examples/scripts/*.rn                        │
│ - Detects file modifications                            │
│ - Returns changed paths per frame                       │
└─────────────────────────────────────────────────────────┘
                          ↓
┌─────────────────────────────────────────────────────────┐
│ EditorApplication                                       │
│ - Polls ScriptWatcher each frame                        │
│ - Maps file path → Entity                               │
│ - Emits ReloadPlugin command                            │
└─────────────────────────────────────────────────────────┘
                          ↓
┌─────────────────────────────────────────────────────────┐
│ UiPluginManager                                         │
│ - Receives ReloadPlugin command                         │
│ - Extracts current state                                │
│ - Reloads script source                                 │
│ - Restores state + re-init                              │
└─────────────────────────────────────────────────────────┘
```

### Data Flow

**Frame N:**
1. `ScriptWatcher::poll()` → Detects `script_editor_plugin.rn` changed
2. `EditorApplication` → Looks up entity for that path
3. Emits `EditorCommand::ReloadPlugin { entity, path }`

**Frame N+1:**
4. Command processed in update phase
5. `UiPluginManager::reload_plugin(entity, path)`
   - Serialize state: `old_state = extract_state(entity)`
   - Read new source: `new_source = read_file(path)`
   - Recompile: `scene.reload_script(entity, new_source)`
   - Restore state: `restore_state(entity, old_state)`
   - Re-init: `scene.update(0.0)` to trigger `on_created()`
6. Show notification: "Plugin reloaded: script_editor_plugin"

## State Preservation Strategy

### What Gets Preserved

```rust
pub struct PluginStateSnapshot {
    pub entity: Entity,
    pub state_map: ScriptStateMap,  // All script state
    pub plugin_visible: bool,        // Window visibility
    pub plugin_metadata: PluginMetadata, // Config from TOML
}
```

**Preserved:**
- ✅ All `get_state()` / `set_state()` data
- ✅ Window visibility (open/closed)
- ✅ Plugin metadata

**Not Preserved (Resets):**
- ❌ Event subscriptions (re-registered in `on_created()`)
- ❌ Pending commands (cleared on reload)
- ❌ UI responses (one-frame buffer)

### State Serialization

```rust
impl UiPluginManager {
    fn extract_state(&self, entity: Entity, scene: &Scene) -> Option<PluginStateSnapshot> {
        let world = scene.main_world();
        let component = world.get::<&RuneScriptComponent>(entity).ok()?;

        // Clone the state map
        let state_map = component.instance()?.state_store.borrow().clone();

        let plugin = self.plugins.iter().find(|p| p.entity == entity)?;

        Some(PluginStateSnapshot {
            entity,
            state_map,
            plugin_visible: plugin.visible,
            plugin_metadata: plugin.metadata.clone(),
        })
    }

    fn restore_state(&mut self, snapshot: PluginStateSnapshot, scene: &mut Scene) -> Result<()> {
        let world = scene.main_world_mut();
        let mut component = world.get::<&mut RuneScriptComponent>(snapshot.entity)?;

        // Restore state map
        let instance = component.instance_mut()?;
        *instance.state_store.borrow_mut() = snapshot.state_map;

        // Restore visibility
        if let Some(plugin) = self.plugins.iter_mut().find(|p| p.entity == snapshot.entity) {
            plugin.visible = snapshot.plugin_visible;
        }

        Ok(())
    }
}
```

## Error Handling

### Compilation Errors

```rust
match scene.reload_script(entity, new_source) {
    Ok(_) => {
        // Success - restore state and re-init
        self.restore_state(snapshot)?;
        scene.update(0.0); // Trigger on_created()

        self.show_notification(format!("✅ Reloaded: {}", name));
    }
    Err(compile_error) => {
        // Keep old version, show error
        self.show_error(format!("❌ Compilation failed: {}", compile_error));

        // Old script still running, state unchanged
        return Err(ReloadError::CompileFailed(compile_error));
    }
}
```

### Runtime Errors During on_created()

```rust
// Re-init with new script
match scene.call_on_created(entity) {
    Ok(_) => {
        self.show_notification(format!("✅ Reloaded: {}", name));
    }
    Err(runtime_error) => {
        // Script reloaded but on_created() failed
        self.show_error(format!("⚠️ Reload succeeded but init failed: {}", runtime_error));

        // Script is loaded but may be in invalid state
        // User can fix and reload again
    }
}
```

### File System Errors

```rust
let new_source = match read_file(path) {
    Ok(source) => source,
    Err(io_error) => {
        self.show_error(format!("❌ Failed to read {}: {}", path.display(), io_error));
        return Err(ReloadError::IoError(io_error));
    }
};
```

## UI Integration

### Reload Notification

```rust
// In EditorSharedState
pub struct ReloadNotification {
    pub message: String,
    pub severity: NotificationSeverity,
    pub timestamp: Instant,
}

pub enum NotificationSeverity {
    Success,  // Green
    Warning,  // Yellow
    Error,    // Red
}

// Display as toast in bottom-right corner
// Auto-dismiss after 3 seconds
// Click to dismiss immediately
```

### Manual Reload Button

```rust
// In plugin window title bar
egui::Window::new(&plugin.metadata.name)
    .show(ctx, |ui| {
        ui.horizontal(|ui| {
            if ui.button("🔄 Reload").clicked() {
                command_queue.push(EditorCommand::ReloadPlugin {
                    entity: plugin.entity,
                    path: plugin.script_path.clone(),
                });
            }

            // Rest of plugin UI...
        });
    });
```

### Keyboard Shortcut

```rust
// In plugin window
if ctx.input(|i| {
    i.key_pressed(egui::Key::R) && i.modifiers.ctrl
}) {
    // Ctrl+R to reload current plugin
    if let Some(focused_plugin) = self.get_focused_plugin_entity() {
        self.reload_plugin(focused_plugin);
    }
}
```

## Implementation Phases

### Phase 1: File Watching (This Sprint)
- [x] Create `ScriptWatcher` based on `ShaderWatcher`
- [ ] Add `script_watcher` field to `EditorSharedState`
- [ ] Poll watcher each frame in `gpu_update()`
- [ ] Map file paths to entities via `UiPluginManager`
- [ ] Emit `ReloadPlugin` command

### Phase 2: Basic Reload (This Sprint)
- [ ] Add `ReloadPlugin` to `EditorCommand`
- [ ] Implement `Scene::reload_script(entity, source)`
- [ ] Basic reload without state preservation
- [ ] Test with simple plugin (test_minimal_ui.rn)

### Phase 3: State Preservation (This Sprint)
- [ ] Implement `extract_state()`
- [ ] Implement `restore_state()`
- [ ] Test state preservation with complex plugin (script_editor_plugin.rn)

### Phase 4: Error Handling (This Sprint)
- [ ] Handle compilation errors gracefully
- [ ] Handle runtime errors during on_created()
- [ ] Show errors in UI (toast notifications)
- [ ] Add manual retry mechanism

### Phase 5: UI Polish (This Sprint)
- [ ] Add reload button to plugin windows
- [ ] Add Ctrl+R keyboard shortcut
- [ ] Add reload notification toasts
- [ ] Visual indicator during reload (spinner)

## Testing Strategy

### Manual Testing
1. **Simple reload test**:
   - Open test_minimal_ui.rn
   - Change button text
   - Save → Should auto-reload in < 1 second
   - Verify new text appears

2. **State preservation test**:
   - Open script_editor_plugin.rn
   - Load a script
   - Modify some UI state
   - Save script_editor_plugin.rn
   - Verify editor state preserved

3. **Error handling test**:
   - Introduce syntax error
   - Save → Should show compilation error
   - Keep old version running
   - Fix error → Should reload successfully

4. **Performance test**:
   - Modify script 100 times
   - Check for memory leaks (via Activity Monitor)
   - Verify reload time stays < 1 second

### Automated Testing
```rust
#[cfg(test)]
mod tests {
    #[test]
    fn test_script_watcher_detects_changes() {
        // Write to temp file
        // Verify watcher detects it
    }

    #[test]
    fn test_state_extraction_and_restoration() {
        // Create plugin with state
        // Extract state
        // Reload plugin
        // Restore state
        // Verify state matches
    }

    #[test]
    fn test_reload_with_compilation_error() {
        // Load valid script
        // Reload with invalid script
        // Verify old script still running
        // Verify error shown
    }
}
```

## Performance Targets

- **Reload latency**: < 500ms from file save to reload complete
- **State extraction**: < 10ms
- **Recompilation**: < 100ms for typical script
- **State restoration**: < 10ms
- **Total**: < 1 second end-to-end

## Risks & Mitigations

### Risk 1: State Corruption
**Problem**: State might not be compatible with new script version

**Mitigation**:
- Validate state keys after restore
- Log warnings for unknown keys
- Allow scripts to handle migration in `on_created()`

```rust
// In on_created()
pub fn on_created(self_entity) {
    let version = get_f64("state_version");
    if version < 2.0 {
        // Migrate old state
        migrate_state_v1_to_v2();
        set_f64("state_version", 2.0);
    }
}
```

### Risk 2: Memory Leaks
**Problem**: Old script instances not cleaned up properly

**Mitigation**:
- Proper Drop implementations
- Memory profiling during testing
- Explicit cleanup in reload path

### Risk 3: Infinite Reload Loop
**Problem**: Script saves itself → triggers reload → saves again

**Mitigation**:
- Debounce file changes (500ms)
- Ignore file changes during reload
- Add reload cooldown per plugin

```rust
pub struct ReloadCooldown {
    last_reload: Instant,
    cooldown_ms: u64, // 500ms default
}

impl ReloadCooldown {
    fn can_reload(&self) -> bool {
        self.last_reload.elapsed().as_millis() > self.cooldown_ms as u128
    }
}
```

## Success Criteria

- [x] Design document complete ✅
- [ ] File changes detected within 500ms
- [ ] Reload completes in < 1 second
- [ ] State preserved in 80%+ of test cases
- [ ] Compilation errors shown in UI
- [ ] Zero crashes after 100 reloads
- [ ] Works with all 11 example plugins
- [ ] Memory stable after 100 reload cycles

## Future Enhancements (Post-Sprint 1)

1. **Dependency Tracking**
   - Reload dependent plugins when library changes
   - Topological reload order

2. **Reload History**
   - Undo/redo reloads
   - Rollback to previous version

3. **Live Code Migration**
   - Patch running code without full reload
   - Requires VM support

4. **Multi-File Editing**
   - Edit multiple scripts simultaneously
   - Batch reload

5. **Reload Hooks**
   ```rune
   pub fn on_before_reload() -> State {
       // Save custom state
   }

   pub fn on_after_reload(old_state: State) {
       // Migrate state
   }
   ```

## References

- Existing implementation: `shader_watcher.rs`
- notify crate docs: https://docs.rs/notify/latest/notify/
- Related issue: Script Editor Arguments Bug (#401)
- Improvement plan: `PLUGIN_ARCHITECTURE_IMPROVEMENT_PLAN.md`

---

*This is a living document. Update as implementation progresses.*
