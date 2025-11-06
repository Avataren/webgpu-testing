# RuneScript Limitations and Workarounds

This document catalogs known limitations of the RuneScript integration and provides workarounds where available.

## Table of Contents
- [Language Syntax Limitations](#language-syntax-limitations)
- [Type System Limitations](#type-system-limitations)
- [API Limitations](#api-limitations)
- [UI System Limitations](#ui-system-limitations)
- [File I/O Limitations](#file-io-limitations)
- [Performance Considerations](#performance-considerations)

---

## Language Syntax Limitations

### 1. No `mut` Keyword

**Issue:** Rune does not support the `mut` keyword for variable declarations.

**Error:**
```
UnsupportedMut
```

**Incorrect:**
```rune
let mut state = get_state("key", default);
state.value = 10;  // Trying to mutate
```

**Correct:**
```rune
let state = get_state("key", default);
state.value = 10;  // Works - all variables are mutable by default
```

**Explanation:** In Rune, all `let` bindings are mutable by default. There is no immutability concept like Rust's default immutability.

---

### 2. No Type Annotations on Struct Fields

**Issue:** Struct field declarations cannot include type annotations.

**Error:**
```
Static typing on fields is not supported
```

**Incorrect:**
```rune
struct MyState {
    count: i64,
    name: String,
    items: Vec,
}
```

**Correct:**
```rune
struct MyState {
    count,    // Type inferred from initialization
    name,     // Document types in comments
    items,    // Vec - list of items
}
```

**Workaround:** Use comments to document expected types:
```rune
struct MyState {
    count,        // i64 - counter value
    name,         // String - entity name
    items,        // Vec - list of script infos
    is_active,    // bool - whether system is active
}
```

---

## Type System Limitations

### 3. Dynamic Typing Only

**Issue:** Rune uses dynamic typing exclusively. Type errors are runtime errors, not compile-time errors.

**Impact:**
```rune
let value = "hello";
value = 42;  // No error - type can change
let result = value + " world";  // Runtime error: can't add string to number
```

**Best Practice:**
- Be consistent with types in a single variable
- Use descriptive variable names to indicate types
- Test thoroughly to catch type errors

---

### 4. Limited Generic Support

**Issue:** Generic types in function signatures are limited.

**Workaround:** Use dynamic typing and handle multiple types at runtime:
```rune
pub fn process_value(value) {
    if value is String {
        // Handle string
    } else if value is i64 {
        // Handle number
    }
}
```

---

## API Limitations

### 5. Clipboard Read Access Not Available

**Issue:** `get_clipboard()` always returns an empty string due to browser/OS security restrictions.

**Incorrect Expectation:**
```rune
if ui.button("Paste") {
    let text = get_clipboard();  // Always returns ""
    // Cannot programmatically read clipboard
}
```

**Workaround:** Users must paste with Ctrl+V/Cmd+V directly into text fields:
```rune
// Text edit widgets support native paste automatically
let new_text = ui.text_edit_multiline("id", current_text, None, None);
// User presses Ctrl+V → works automatically
```

**What Works:**
- ✅ Ctrl+C, Ctrl+X, Ctrl+V in text fields (handled by egui)
- ✅ `set_clipboard(text)` - programmatic copy
- ❌ `get_clipboard()` - programmatic paste

---

### 6. No Direct Hot-Reload API

**Issue:** Scripts cannot trigger their own hot-reload.

**Current State:**
```rune
pub fn reload_all_scripts() -> String {
    // No API to trigger reload from script
    "NOTE: Script reload must be triggered manually"
}
```

**Workaround:**
- Save changes with `write_file()`
- User manually restarts scripts through editor UI
- Or: Scene change triggers script reload

**Future:** Could add command buffer API for scripts to request reload.

---

### 7. Limited Entity Query API

**Available:**
```rune
// Find entity by name
let entity = find_entity_by_name("Player");

// Get all entities with scripts
let scripts = get_all_script_entities();

// Query entities with specific component (limited)
let entities = query_entities_with_component("Transform");
```

**Not Available:**
- Complex queries (multiple components AND/OR)
- Query by component value (e.g., all entities with speed > 10)
- Spatial queries beyond basic radius/box

**Workaround:** Query all entities and filter in script code.

---

## UI System Limitations

### 8. No Syntax Highlighting in Text Editor

**Issue:** Cannot apply custom text formatting/colors in multiline text editor.

**Limitation:**
```rune
// No way to implement this in RuneScript:
let editor = ui.text_edit_multiline_with_highlighter(
    "code",
    content,
    my_syntax_highlighter  // Not supported
);
```

**Workaround:**
- Use monospace font (automatic in `text_edit_multiline`)
- External syntax highlighting not possible from scripts
- Consider using Rust-based editor for advanced features

---

### 9. Fixed Widget Set

**Available Widgets:**
- `label(text)`
- `heading(text)`
- `button(text)`
- `separator()`
- `text_edit(id, value)`
- `text_edit_multiline(id, value, width, height)`
- `slider(id, value, min, max)`
- `drag_value(id, value)`
- `checkbox(id, value, label)`
- `color_edit(id, r, g, b)`

**Not Available:**
- Tree views
- Tables with sorting
- Custom layouts (grid, flexbox)
- Tabs
- Combo boxes / dropdowns
- Progress bars
- Custom widgets

**Workaround:** Build complex UIs from basic widgets, or request new widget types.

---

### 10. No Window Management

**Issue:** Cannot programmatically create/close windows, change window properties after creation.

**Limitation:**
```rune
// Cannot do this:
ui.create_window("My Window", 400, 300);
ui.close_window("Other Window");
ui.set_window_position(100, 100);
```

**Current:** Each plugin gets one window, managed by the editor. Window title, size, and visibility controlled by plugin metadata.

---

## File I/O Limitations

### 11. Sandboxed File Access

**Issue:** File operations restricted to specific directories for security.

**Allowed Directories:**
- `examples/scripts/`
- `scripts/`

**Blocked:**
```rune
read_file("/etc/passwd")           // ERROR: Access denied
read_file("../../../secrets.txt")  // ERROR: Access denied
read_file("C:\\Windows\\System32") // ERROR: Access denied
```

**Allowed:**
```rune
read_file("examples/scripts/my_script.rn")  // OK
write_file("scripts/config.json", data)      // OK
```

**Workaround:** Store script-related files in allowed directories. Use project system for assets.

---

### 12. No Directory Manipulation

**Not Available:**
```rune
create_directory("scripts/utils")     // No API
delete_file("scripts/old.rn")         // No API
rename_file("old.rn", "new.rn")       // No API
copy_file("a.rn", "b.rn")             // No API
```

**Available:**
```rune
read_file(path)        // Read file contents
write_file(path, data) // Write file contents (creates if doesn't exist)
file_exists(path)      // Check if file exists
list_files(dir)        // List files in directory
```

**Workaround:** Use external tools or request new APIs.

---

### 13. Text-Only File I/O

**Issue:** Can only read/write text files. No binary file support.

**Not Supported:**
```rune
read_binary_file("image.png")    // No API
write_binary_file("data.bin", bytes) // No API
```

**Workaround:** Use asset system for binary files (meshes, textures, etc.).

---

## Performance Considerations

### 14. Script Execution Performance

**Issue:** RuneScript executes in a VM, slower than native Rust.

**Impact:**
- Simple UI updates: Fast (< 1ms)
- Heavy computation in scripts: Can be slow
- Large data processing: Consider native Rust

**Best Practices:**
```rune
// ❌ Bad: Processing large arrays every frame
pub fn update(self_entity, dt) {
    let items = get_all_items();  // 10,000 items
    for item in items {
        process_item(item);  // Heavy computation
    }
}

// ✅ Good: Cache and process incrementally
pub fn update(self_entity, dt) {
    let batch_size = 100;
    let offset = get_state("offset", 0);
    process_batch(offset, batch_size);
    set_state("offset", offset + batch_size);
}
```

---

### 15. No Multi-threading

**Issue:** All scripts run on single thread, sequentially.

**Limitation:**
```rune
// Cannot parallelize work:
for item in large_collection {
    expensive_operation(item);  // Runs sequentially
}
```

**Workaround:**
- Keep per-frame work small
- Distribute work across multiple frames
- Use native Rust for CPU-intensive tasks

---

## Script Mode Limitations

### 16. Mode-Based Execution

**Issue:** Scripts execute based on annotation, not runtime state.

**Annotations:**
```rune
// @editor  - Runs ONLY in editor mode
// @tool    - Runs in BOTH editor and play modes
// (none)   - Runs ONLY in play mode
```

**Limitation:** Cannot dynamically change mode at runtime:
```rune
// Cannot do this:
if some_condition {
    switch_to_play_mode();  // No API
}
```

**Workaround:** Use multiple scripts with different modes if needed.

---

## Script Lifecycle Limitations

### 17. No Cleanup Callback

**Available:**
```rune
pub fn on_created(self_entity) { }  // ✅ Called once on init
pub fn update(self_entity, dt) { }  // ✅ Called every frame (play mode)
pub fn on_ui(self_entity, ui) { }   // ✅ Called every frame (for UI)
```

**Not Available:**
```rune
pub fn on_destroyed(self_entity) { }  // ❌ No cleanup callback
```

**Impact:** Cannot clean up resources when script is removed or reloaded.

**Workaround:** Resources are automatically cleaned up by VM, but external state may persist.

---

### 18. No Component Queries During on_created()

**Issue:** Cannot query for components (like `get_all_script_entities()`) inside `on_created()` callbacks.

**Error:**
```
thread 'main' panicked at archetype.rs:124:13:
RuneScriptComponent already borrowed uniquely
```

**Why It Happens:**
When `on_created()` is called, the engine is iterating over all script components with a mutable borrow. Attempting to query components from within `on_created()` creates a borrow conflict.

**Incorrect:**
```rune
pub fn on_created(self_entity) {
    // ❌ This will panic!
    let scripts = get_all_script_entities();  // Tries to borrow RuneScriptComponent
    // Panic: already borrowed during on_created iteration
}
```

**Correct:**
```rune
struct MyState {
    needs_init,  // bool - whether initialization is needed
    data,        // Vec - data to load
}

pub fn on_created(self_entity) {
    // ✅ Defer component queries to on_ui()
    set_state("my_state", MyState {
        needs_init: true,
        data: [],
    });
}

pub fn on_ui(self_entity, ui) {
    let state = get_state("my_state", MyState {
        needs_init: true,
        data: [],
    });

    if state.needs_init {
        // ✅ Safe to query here - not during component iteration
        let scripts = get_all_script_entities();
        state.data = scripts;
        state.needs_init = false;
        set_state("my_state", state);
    }

    // Use state.data...
}
```

**Rule of Thumb:** Never call component query functions from `on_created()`. Defer to `on_ui()` or `update()`.

---

## State Management Limitations

### 19. No Persistent Storage

**Issue:** Script state is in-memory only, lost on editor restart.

**Example:**
```rune
pub fn on_created(self_entity) {
    set_state("count", 0);  // Lost when editor closes
}
```

**Workaround:**
- Use `write_file()` to persist important state
- Load from file in `on_created()`

```rune
pub fn on_created(self_entity) {
    if file_exists("scripts/my_state.json") {
        let data = read_file("scripts/my_state.json");
        // Parse and restore state
    }
}
```

---

### 20. State Scope Limited to Entity

**Issue:** State is per-entity, not global.

**Limitation:**
```rune
// In script A (entity 1):
set_state("shared", "value");

// In script B (entity 2):
let val = get_state("shared", "default");  // Gets entity 2's state, not entity 1's
```

**Workaround:**
- Use events for inter-script communication
- Store shared state in a dedicated "manager" entity
- Use file I/O for truly global state

---

## Summary Table

| Feature | Status | Workaround Available |
|---------|--------|---------------------|
| `mut` keyword | ❌ Not supported | ✅ All vars mutable by default |
| Type annotations on struct fields | ❌ Not supported | ✅ Use comments |
| Clipboard read | ❌ Not supported | ✅ Native Ctrl+V works |
| Syntax highlighting | ❌ Not supported | ❌ Use external editor |
| Hot-reload trigger | ❌ Not supported | ⚠️ Manual restart |
| Sandboxed file I/O | ⚠️ Limited | ✅ Use allowed dirs |
| Binary file I/O | ❌ Not supported | ⚠️ Use asset system |
| Multi-threading | ❌ Not supported | ⚠️ Batch processing |
| Cleanup callback | ❌ Not supported | ⚠️ Automatic VM cleanup |
| Component queries in on_created() | ❌ Not supported | ✅ Defer to on_ui() |
| Persistent state | ❌ Not supported | ✅ Use file I/O |
| Global state | ❌ Not supported | ✅ Use events/files |

**Legend:**
- ✅ Good workaround available
- ⚠️ Partial workaround
- ❌ No workaround

---

## Getting Help

**Found a limitation not listed here?**
- Check the Rune language documentation
- Review example scripts in `examples/scripts/`
- Ask for new APIs or features

**Want to contribute?**
- Propose new RuneScript APIs
- Extend the widget system
- Improve documentation

---

*Last updated: 2025-11-06*
