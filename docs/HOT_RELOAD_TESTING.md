# Hot Reload Testing Guide

> Manual testing procedures for script hot reload functionality

## Test Environment Setup

1. Build the editor:
   ```bash
   cargo build --bin editor --features egui
   ```

2. Run the editor:
   ```bash
   cargo run --bin editor --features egui
   ```

## Test 1: Successful Reload (Basic)

**Objective**: Verify that a simple script change triggers a reload with success notification.

**Steps**:
1. Open the editor
2. Ensure `test_minimal_ui.rn` plugin is visible (check @tool plugins)
3. Open `examples/scripts/test_minimal_ui.rn` in your text editor
4. Change line 10 from:
   ```rune
   ui.label("Hello World");
   ```
   to:
   ```rune
   ui.label("Hello World - RELOADED!");
   ```
5. Save the file

**Expected Results**:
- Within 1 second, a green toast notification appears in bottom-right corner
- Message: "✅ Reloaded: test_minimal_ui"
- The plugin UI updates to show "Hello World - RELOADED!"
- Notification auto-dismisses after 5 seconds
- No errors in console

**Success Criteria**: ✅ Pass if notification appears and UI updates

---

## Test 2: Failed Reload (Syntax Error)

**Objective**: Verify that compilation errors are caught and the old script keeps running.

**Steps**:
1. With the editor still running and `test_minimal_ui.rn` visible
2. Open `examples/scripts/test_minimal_ui.rn`
3. Introduce a syntax error on line 12:
   ```rune
   if ui.button("Test Button") {    // Remove closing brace
       log_info("Button clicked!");
   // Missing closing brace here!
   ```
4. Save the file

**Expected Results**:
- Within 1 second, a red toast notification appears
- Message: "❌ Failed to reload test_minimal_ui: [compilation error details]"
- The plugin continues to show the OLD version ("Hello World - RELOADED!")
- Plugin remains functional
- Notification auto-dismisses after 5 seconds

**Success Criteria**: ✅ Pass if old version keeps running and error is shown

---

## Test 3: Error Recovery

**Objective**: Verify that fixing an error allows successful reload.

**Steps**:
1. With the syntax error still present (from Test 2)
2. Fix the syntax error by adding the missing closing brace:
   ```rune
   if ui.button("Test Button") {
       log_info("Button clicked!");
   }
   ```
3. Save the file

**Expected Results**:
- Green success notification appears
- Message: "✅ Reloaded: test_minimal_ui"
- Plugin UI updates with the fixed code
- No errors in console

**Success Criteria**: ✅ Pass if reload succeeds after error is fixed

---

## Test 4: State Preservation

**Objective**: Verify that plugin state is preserved across reloads.

**Setup**: We'll use `script_editor_plugin.rn` which has more state.

**Steps**:
1. In the editor, open the Script Editor plugin (@editor mode)
2. Load a script file into the editor (e.g., `test_minimal_ui.rn`)
3. Make some edits to the text in the Script Editor
4. Note the current cursor position and any unsaved changes
5. Open `examples/scripts/script_editor_plugin.rn` in external editor
6. Make a small cosmetic change (e.g., change a label text)
7. Save the file

**Expected Results**:
- Green success notification appears
- The Script Editor plugin reloads
- The text you were editing is still there (state preserved)
- Cursor position approximately preserved
- Any unsaved edits remain

**Success Criteria**: ✅ Pass if editor content and state remain intact

---

## Test 5: Multiple Rapid Changes (Debouncing)

**Objective**: Verify that rapid saves don't cause multiple reloads.

**Steps**:
1. Open `examples/scripts/test_minimal_ui.rn`
2. Make a small change and save
3. Immediately make another change and save (within 500ms)
4. Repeat 3-4 times rapidly

**Expected Results**:
- Only one or two reload notifications appear (not 3-4)
- The 500ms debounce prevents reload spam
- Final version reflects the last save
- Editor remains responsive

**Success Criteria**: ✅ Pass if fewer notifications than saves (debouncing working)

---

## Test 6: File Watcher Directories

**Objective**: Verify that the watcher monitors the correct directories.

**Steps**:
1. Check console output on editor startup
2. Look for log messages about watched directories

**Expected Output**:
```
[INFO] Watching script directories: ["examples/scripts"]
```
or
```
[INFO] Watching script directories: ["examples/scripts", "scripts"]
```

**Success Criteria**: ✅ Pass if directories are being watched

---

## Test 7: Missing File Handling

**Objective**: Verify graceful handling of file deletion during development.

**Steps**:
1. Create a backup of `test_minimal_ui.rn`
2. With editor running, temporarily rename or delete the file
3. Restore the file with modifications
4. Save

**Expected Results**:
- No crashes during deletion
- Reload succeeds after restoration
- Appropriate error message if reload attempted while missing

**Success Criteria**: ✅ Pass if editor handles file disappearance gracefully

---

## Test 8: Performance Test

**Objective**: Verify no memory leaks or performance degradation over multiple reloads.

**Steps**:
1. Open Activity Monitor / Task Manager
2. Note initial memory usage of editor process
3. Make and save small changes to `test_minimal_ui.rn` 20 times
4. Check memory usage again
5. Continue for 100 total reloads

**Expected Results**:
- Memory usage remains stable (< 10MB growth over 100 reloads)
- Reload time stays < 1 second throughout
- No slowdown in editor responsiveness
- No console warnings about memory

**Success Criteria**: ✅ Pass if memory stable and performance consistent

---

## Test 9: Complex Plugin Reload

**Objective**: Test reloading of a more complex plugin with event subscriptions.

**Steps**:
1. Open a complex plugin like `debug_console_plugin.rn`
2. Interact with it (execute some commands)
3. Modify the script source
4. Save and verify reload

**Expected Results**:
- Plugin reloads successfully
- Event subscriptions re-register
- Plugin functionality intact
- No errors related to stale event handlers

**Success Criteria**: ✅ Pass if complex plugin reloads without issues

---

## Test 10: Notification UI

**Objective**: Verify toast notification appearance and behavior.

**Steps**:
1. Trigger a successful reload
2. Trigger a failed reload (syntax error)
3. Trigger multiple reloads in quick succession

**Expected Results**:
- Success toasts are green with white text
- Error toasts are red with white text
- Multiple toasts stack vertically with 10px spacing
- Toasts show message and age ("2s ago")
- Toasts auto-dismiss after 5 seconds
- Toasts appear in bottom-right corner with margin

**Success Criteria**: ✅ Pass if notifications are visible and properly styled

---

## Debugging Tips

### No Reload Happening

**Check**:
1. Is the file being watched? Check console for "Watching script directories"
2. Is the file extension `.rn`? Watcher only monitors .rn files
3. Is debounce preventing reload? Wait 1 second between saves
4. Check console for file watcher errors

### Old Version Still Showing

**Check**:
1. Did you save the file? (Ctrl+S)
2. Is there a compilation error? Check for red error toast
3. Is the correct file being edited? Verify file path matches plugin

### Notification Not Appearing

**Check**:
1. Is notification rendering called? Check `render_reload_notifications()` in mod.rs:479
2. Is notification being added? Check `process_pending_plugin_reloads()` logic
3. Check console for rendering errors

### State Not Preserved

**Check**:
1. Is `extract_all_state()` being called before reload?
2. Is `restore_all_state()` being called after reload?
3. Check console for state preservation errors
4. Verify `on_created()` doesn't overwrite restored state

---

## Code Verification Checklist

Before manual testing, verify these integration points:

- [x] `ScriptWatcher` created in `EditorSharedState::new()` (core.rs:191)
- [x] `process_script_file_changes()` called in main loop (mod.rs:155)
- [x] `ReloadPlugin` command defined (system.rs:39)
- [x] `ReloadPlugin` processed in `drain_update_commands()` (core.rs:811)
- [x] `process_pending_plugin_reloads()` handles reload logic (core.rs:921)
- [x] `reload_plugin()` updates component (ui_plugin_manager.rs:175)
- [x] Notifications added on success/error (core.rs:942, 958)
- [x] `render_reload_notifications()` renders toasts (mod.rs:631)
- [x] State extraction in `reset_script_runtime()` (runtime_state.rs:60)
- [x] State restoration after `on_created()` (state.rs:212)

---

## Known Limitations

1. **WASM builds**: Hot reload disabled on WebAssembly (watcher stubbed out)
2. **Non-recursive watching**: Only watches root script directories, not subdirectories
3. **Debounce time**: 500ms minimum between reloads per file
4. **File extensions**: Only `.rn` files trigger reloads
5. **Manual reload**: No Ctrl+R shortcut or reload button yet (future enhancement)

---

## Success Metrics

Hot reload is considered fully functional if:

- ✅ File changes detected within 500ms
- ✅ Reload completes in < 1 second
- ✅ State preserved in 80%+ of test cases
- ✅ Compilation errors shown in UI
- ✅ Zero crashes after 100 reloads
- ✅ Works with all 11 example plugins
- ✅ Memory stable after 100 reload cycles

---

## Next Steps After Testing

Once manual testing confirms all tests pass:

1. Update `docs/HOT_RELOAD_DESIGN.md` Phase 1-4 checkboxes
2. Consider Phase 5: UI Polish
   - Add manual reload button to plugin windows
   - Add Ctrl+R keyboard shortcut
   - Add visual indicator during reload (spinner)
3. Consider future enhancements:
   - Dependency tracking
   - Reload history/undo
   - Custom reload hooks in scripts
