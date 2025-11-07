# Rune Scripting Reference

Complete guide to the Rune scripting system for extending the engine with custom gameplay logic and editor tools.

official rune integration examples can be found at https://github.com/rune-rs/rune/tree/main/examples/examples

## Table of Contents

- [Overview](#overview)
- [Quick Start](#quick-start)
- [Core Concepts](#core-concepts)
  - [Script Lifecycle](#script-lifecycle)
  - [State Management](#state-management)
  - [Entity Handles](#entity-handles)
- [UI Plugins](#ui-plugins)
- [Complete API Reference](#complete-api-reference)
- [Advanced Topics](#advanced-topics)
- [Known Limitations](#known-limitations)
- [Best Practices](#best-practices)

---

## Overview

The engine uses [Rune](https://rune-rs.github.io/), a dynamic scripting language with Rust-like syntax, to provide:

- **Gameplay Scripts** - Entity behaviors, game logic, custom components
- **Editor Tools** - Custom panels, inspectors, and workflows
- **Hot Reload** - Changes apply instantly without recompiling
- **Sandboxed Execution** - Script errors don't crash the engine

### Key Features

✅ Entity lifecycle hooks (`on_created`, `update`, `on_ui`)
✅ Per-entity state management
✅ Entity queries and spatial queries
✅ Component read/write access
✅ Input handling (keyboard, mouse)
✅ Event system for inter-script communication
✅ UI scripting for editor tools
✅ File I/O operations
✅ Transform and hierarchy manipulation

---

## Quick Start

### Hello World Script

Create `examples/scripts/hello_world.rn`:

```rune
// @tool
// Hello World Plugin
//
// Your first Rune script

pub fn on_created(self_entity) {
    log_info("Hello from Rune!");
    set_state("message", "Hello World");
}

pub fn on_ui(ui) {
    ui.heading("Hello World");

    let message = get_state("message", "Hello");
    ui.label(message);

    if ui.button("Say Hello") {
        log_info("Button clicked!");
    }
}
```

### Load the Script

1. Start the editor
2. Add a `RuneScript` component to any entity
3. Set the script path to your `.rn` file
4. See your UI panel appear!

---

## Core Concepts

### Script Lifecycle

Scripts have several lifecycle hooks called by the engine:

#### `on_created(self_entity)`

Called once when the script is first loaded.

```rune
pub fn on_created(self_entity) {
    log_info("Script initialized");

    // Initialize state
    set_state("health", 100.0);
    set_state("name", "Player");

    // Spawn child entities
    let weapon = spawn_entity(Some("PlayerWeapon"));
    set_parent(weapon, Some(self_entity));
}
```

#### `update(self_entity, dt)`

Called every frame with delta time in seconds.

```rune
pub fn update(self_entity, dt) {
    // Rotate over time
    let speed = 1.0;
    rotate(self_entity, 0.0, 1.0, 0.0, speed * dt);

    // Handle input
    if is_key_pressed("W") {
        translate(self_entity, 0.0, 0.0, -5.0 * dt);
    }
}
```

#### `on_ui(ui)`

Called every frame to build UI (for plugins marked with `// @tool` or `// @editor`).

```rune
pub fn on_ui(ui) {
    ui.heading("My Tool");

    let value = get_state("value", "default");
    let new_value = ui.text_edit("input", value);

    if new_value != value {
        set_state("value", new_value);
    }
}
```

#### `on_destroyed(self_entity)`

Called when the script is removed or entity is destroyed.

```rune
pub fn on_destroyed(self_entity) {
    log_info("Cleanup complete");
}
```

### State Management

Each script instance has its own persistent state storage.

#### Basic State Functions

```rune
// Store any value (String, number, bool, etc.)
set_state("key", value);

// Get value with default (recommended)
let value = get_state("key", "default");

// Get value or panic if not found
let value = get_state("key");

// Try get value (returns unit () if not found)
let value = try_get_state("key");
```

#### Type-Specific Functions

For performance-critical numeric state:

```rune
// Float state
set_f64("speed", 5.0);
let speed = get_f64("speed"); // Panics if not found

// Boolean state
set_bool("is_active", true);
let active = get_bool("is_active", false); // With default

// String state
set_string("name", "Player");
let name = get_string("name", "Unknown"); // With default
```

#### State for Other Entities

```rune
// Set state on another entity
set_state_for(other_entity, "key", value);

// Get state from another entity
let value = get_state_for(other_entity, "key", default);
```

**Important:** State values are `Value` type. UI functions and comparisons work with `Value` directly without conversion.

### Entity Handles

Entities are represented as `i64` handles. All entity functions accept these handles.

```rune
pub fn on_created(self_entity) {
    // self_entity is your entity's handle

    // Create new entities
    let child = spawn_entity(Some("ChildEntity"));

    // Find entities
    let player = find_entity_by_name("Player");
    if player.is_some() {
        log_info(`Found player: ${player}`);
    }

    // Query entities by component
    let all_cameras = query_entities_with_component("CameraComponent");

    // Spatial queries
    let nearby = get_entities_in_radius(0.0, 0.0, 0.0, 10.0);
}
```

---

## UI Plugins

Scripts marked with `// @tool` or `// @editor` can create custom editor panels.

### Plugin Annotations

```rune
// @tool - Editor tool (always visible)
// @editor - Editor panel
//
// Tool Name
//
// Description of what this tool does
```

### UI API

All UI functions work with `Value` type for consistency with state management.

#### Layouts

```rune
ui.heading("Section Title");
ui.label("Text label");
ui.separator();
```

#### Input Widgets

```rune
// Text input (single line) - works with strings
let text = get_string("input", "");
let new_text = ui.text_edit("input_id", text);
set_string("input", new_text);

// Multiline text editor - works with strings
let content = get_string("content", "");
let new_content = ui.text_edit_multiline("editor", content, None, Some(400.0));
set_string("content", new_content);

// Button
if ui.button("Click Me") {
    log_info("Button clicked!");
}

// Checkbox
let enabled = get_bool("enabled", false);
let new_enabled = ui.checkbox("check", enabled, "Enable Feature");
set_bool("enabled", new_enabled);

// Slider
let value = get_f64("slider");
let new_value = ui.slider("slider_id", value, 0.0, 100.0);
if new_value != value {
    set_f64("slider", new_value);
}

// Drag value
let amount = get_f64("amount");
let new_amount = ui.drag_value("drag", amount);
if new_amount != amount {
    set_f64("amount", new_amount);
}

// Color picker
let (r, g, b) = (1.0, 0.5, 0.0);
let (new_r, new_g, new_b) = ui.color_edit("color", r, g, b);
```

### Example: Text Editor Plugin

```rune
// @editor
// Simple Text Editor
//
// Edit and save text files

pub fn on_created(self_entity) {
    set_string("file_path", "notes.txt");
    set_string("content", "");
    set_string("status", "Ready");
}

pub fn on_ui(ui) {
    ui.heading("Text Editor");

    // File path input
    let path = get_string("file_path", "notes.txt");
    let new_path = ui.text_edit("path", path);
    set_string("file_path", new_path);

    ui.separator();

    // Load/Save buttons
    if ui.button("Load") {
        let path = get_string("file_path", "");
        let content = read_file(path);
        set_string("content", content);
        let path_msg = get_string("file_path", "");
        set_string("status", `Loaded ${path_msg}`);
    }

    if ui.button("Save") {
        let path = get_string("file_path", "");
        let content = get_string("content", "");
        let error = write_file(path, content);
        if error == "" {
            let path_msg = get_string("file_path", "");
            set_string("status", `Saved ${path_msg}`);
        } else {
            set_string("status", error);
        }
    }

    ui.separator();

    // Status
    let status = get_string("status", "Ready");
    ui.label(status);

    ui.separator();

    // Content editor
    let content = get_string("content", "");
    let new_content = ui.text_edit_multiline("content", content, None, Some(400.0));
    set_string("content", new_content);
}
```

---

## Complete API Reference

### Logging

```rune
log_debug(message: String)
log_info(message: String)
log_warn(message: String)
log_error(message: String)
```

### Entity Management

```rune
// Create entities
spawn_entity(name: Option<String>) -> i64
set_name(handle: i64, name: String)

// Find entities
find_entity_by_name(name: String) -> Option<i64>

// Entity hierarchy
set_parent(entity: i64, parent: Option<i64>)
get_parent(entity: i64) -> Option<i64>
get_children(entity: i64) -> Option<Vec<i64>>

// Attach scripts
attach_inline_script(entity: i64, name: String, source: String)
attach_script_file(entity: i64, path: String)

// Import models
import_gltf(entity: i64, path: String, scale: f64)
```

### Transform Operations

```rune
// Position
translate(entity: i64, x: f64, y: f64, z: f64)
set_translation(entity: i64, x: f64, y: f64, z: f64)
get_world_translation(entity: i64) -> Option<Vec<f64>>

// Rotation
rotate(entity: i64, axis_x: f64, axis_y: f64, axis_z: f64, angle_radians: f64)
set_rotation(entity: i64, yaw: f64, pitch: f64, roll: f64)
get_world_rotation(entity: i64) -> Option<Vec<f64>>
look_at(entity: i64, target_x: f64, target_y: f64, target_z: f64)

// Scale
set_scale(entity: i64, x: f64, y: f64, z: f64)
```

### Component System

```rune
// Check for components
has_component(entity: i64, component_name: String) -> bool

// Get component data
get_component(entity: i64, component_name: String) -> Option<Value>

// Modify components
set_component(entity: i64, component_name: String, value: Value)
add_component(entity: i64, component_name: String, value: Value)
remove_component(entity: i64, component_name: String)
```

Available component names:
- `CameraComponent`
- `MeshComponent`
- `MaterialComponent`
- `PointLight`
- `DirectionalLight`
- `SpotLight`
- `Visible`
- `Billboard`
- And more...

### Entity Queries

```rune
// Query by component
query_entities_with_component(component_name: String) -> Vec<i64>

// Spatial queries
get_entities_in_radius(x: f64, y: f64, z: f64, radius: f64) -> Vec<i64>
get_entities_in_box(min_x: f64, min_y: f64, min_z: f64,
                    max_x: f64, max_y: f64, max_z: f64) -> Vec<i64>

get_nearest_entity(x: f64, y: f64, z: f64) -> Option<i64>
get_nearest_entity_with_component(x: f64, y: f64, z: f64,
                                   component_name: String) -> Option<i64>
```

### Input

```rune
// Keyboard
is_key_pressed(key: String) -> bool
is_key_just_pressed(key: String) -> bool
is_key_just_released(key: String) -> bool

// Key names: "W", "A", "S", "D", "Space", "Escape", "LeftShift", etc.

// Mouse buttons (0=Left, 1=Right, 2=Middle)
is_mouse_button_pressed(button: i64) -> bool
is_mouse_button_just_pressed(button: i64) -> bool
is_mouse_button_just_released(button: i64) -> bool

// Mouse position and movement
get_mouse_position() -> Vec<f64>  // [x, y] in screen coordinates
get_mouse_delta() -> Vec<f64>      // [dx, dy] since last frame
get_mouse_scroll_delta() -> Vec<f64>  // [dx, dy] scroll wheel
```

### Event System

```rune
// Emit an event
emit_event(event_name: String, data: Value)

// Subscribe to events
subscribe_event(event_name: String, callback_name: String)

// Unsubscribe
unsubscribe_event(event_name: String)

// Example callback
pub fn on_player_scored(data) {
    let score = data.score;
    log_info(`Player scored ${score} points!`);
}

pub fn on_created(self_entity) {
    subscribe_event("player_scored", "on_player_scored");
}
```

### File I/O

```rune
// Read entire file as string
read_file(path: String) -> String  // Returns error message if failed

// Write string to file
write_file(path: String, content: String) -> String  // Returns "" on success

// Check if file exists
file_exists(path: String) -> bool

// List files in directory
list_files(path: String) -> Vec<String>  // Returns file names
```

### Script Access (Metaprogramming)

Query information about other scripts in the scene. Useful for building script editors and debugging tools.

```rune
// Get count of entities with scripts
get_script_count() -> i64

// Get entity ID for script at index (0-based)
get_script_entity(index: i64) -> u64

// Get source type: "file" or "inline"
get_script_source_type(index: i64) -> String

// Get source path (file path or inline name)
get_script_source_path(index: i64) -> String

// Get entity name for display
get_entity_name(entity: u64) -> String
```

**Example: List all scripts in scene**
```rune
pub fn on_ui(ui) {
    ui.heading("Script Inspector");

    let count = get_script_count();
    ui.label(`Found ${count} script(s)`);

    ui.separator();

    let i = 0;
    while i < count {
        let entity_id = get_script_entity(i);
        let entity_name = get_entity_name(entity_id);
        let source_type = get_script_source_type(i);
        let source_path = get_script_source_path(i);

        ui.label(`${entity_name}: ${source_type} - ${source_path}`);
        i = i + 1;
    }
}
```

**Note**: Script data is cached when you call `get_script_count()`. All subsequent calls to `get_script_entity()`, `get_script_source_type()`, etc. use this cached data for the current frame.

### UI Functions

See [UI Plugins](#ui-plugins) section for complete UI API reference.

---

## Advanced Topics

### Component Manipulation Example

```rune
pub fn on_created(self_entity) {
    // Add a point light
    let light_data = #{
        color: [1.0, 0.8, 0.6],
        intensity: 5.0,
        range: 10.0,
    };
    add_component(self_entity, "PointLight", light_data);

    // Modify the light
    if let Some(light) = get_component(self_entity, "PointLight") {
        light.intensity = 10.0;
        set_component(self_entity, "PointLight", light);
    }
}
```

### Inter-Script Communication

```rune
// Script A - Emitter
pub fn update(self_entity, dt) {
    if is_key_just_pressed("Space") {
        emit_event("player_jumped", #{
            entity: self_entity,
            height: 2.0,
        });
    }
}

// Script B - Listener
pub fn on_player_jumped(data) {
    log_info(`Entity ${data.entity} jumped ${data.height} units!`);
}

pub fn on_created(self_entity) {
    subscribe_event("player_jumped", "on_player_jumped");
}
```

### Building a Character Controller

```rune
pub fn on_created(self_entity) {
    set_f64("move_speed", 5.0);
    set_f64("rotation_speed", 3.0);
}

pub fn update(self_entity, dt) {
    let speed = get_f64("move_speed");
    let rot_speed = get_f64("rotation_speed");

    // Movement
    if is_key_pressed("W") {
        translate(self_entity, 0.0, 0.0, -speed * dt);
    }
    if is_key_pressed("S") {
        translate(self_entity, 0.0, 0.0, speed * dt);
    }
    if is_key_pressed("A") {
        translate(self_entity, -speed * dt, 0.0, 0.0);
    }
    if is_key_pressed("D") {
        translate(self_entity, speed * dt, 0.0, 0.0);
    }

    // Rotation
    if is_key_pressed("Q") {
        rotate(self_entity, 0.0, 1.0, 0.0, rot_speed * dt);
    }
    if is_key_pressed("E") {
        rotate(self_entity, 0.0, 1.0, 0.0, -rot_speed * dt);
    }
}
```

---

## Known Limitations

### Language Syntax

#### No `mut` Keyword

Rune does not support the `mut` keyword. All `let` bindings are mutable by default.

```rune
// ❌ Error
let mut x = 5;

// ✅ Correct
let x = 5;
x = 10;  // Works fine
```

#### No Type Annotations on Struct Fields

```rune
// ❌ Error
struct Player {
    health: i64,
    name: String,
}

// ✅ Correct
struct Player {
    health,
    name,
}
```

#### No Generics

Rune does not support generic type parameters.

```rune
// ❌ Not supported
struct Container<T> {
    value,
}

// ✅ Use dynamic typing instead
struct Container {
    value,  // Can hold any type
}
```

### Value Ownership Issues ⚠️

**CRITICAL**: Rune has complex Value ownership semantics that cause "Cannot read, value is M-000000" errors in many common UI patterns. These issues are **fundamental to Rune's design** and cannot always be worked around.

#### What Triggers Ownership Errors

1. **Using a value in multiple places:**
```rune
// ❌ DANGER: Value gets "moved" by template string, then can't be used again
let source_type = get_script_source_type(i);
let label = `Type: ${source_type}`;  // source_type is "moved" here
if source_type == "file" {  // ERROR: Cannot read, value is M-000000
    // ...
}
```

2. **Comparing values after passing to functions:**
```rune
// ❌ DANGER: Value moved by ui.text_edit(), comparison fails
let old_text = get_string("text", "");
let new_text = ui.text_edit("id", old_text);  // old_text moved
if new_text != old_text {  // ERROR: Cannot read, value is M-000000
    set_string("text", new_text);
}
```

3. **Iterating over Vec of custom structs:**
```rune
// ❌ DOES NOT WORK: Vec iteration causes ownership issues
let scripts = get_all_script_entities();  // Returns Vec<ScriptInfo>
for script in scripts {
    let entity = script.entity;  // ERROR: Cannot read, value is M-000000
    // ...
}
```

4. **Accessing struct fields multiple times:**
```rune
// ❌ DANGER: Even with getter methods, accessing fields in loops fails
for script in scripts {
    let type = script.source_type();
    let path = script.source_path();  // ERROR: Cannot read, value is M-000000
    // ...
}
```

#### Workarounds That Work

**1. Always use type-specific state functions**

Use `get_string()`, `get_f64()`, `get_bool()` instead of generic `get_state()`:

```rune
// ✅ SAFE - type-specific functions return Rust types, not Value
let text = get_string("text", "");
let new_text = ui.text_edit("id", text);
set_string("text", new_text);  // Always update, don't compare

// ✅ SAFE - numeric state
let speed = get_f64("speed");
set_f64("speed", speed * 1.1);

// ✅ SAFE - boolean state
let enabled = get_bool("enabled", false);
set_bool("enabled", !enabled);
```

**2. Refetch values instead of reusing them**

```rune
// ✅ SAFE - fetch fresh values each time
let label = `Type: ${get_script_source_type(i)}`;
if get_script_source_type(i) == "file" {  // Fetch again
    // Use get_script_source_path(i) directly, don't store
    set_string("current_path", get_script_source_path(i));
}
```

**3. Use index-based access instead of iteration**

```rune
// ✅ SAFE - index-based access
let count = get_script_count();
let i = 0;
while i < count {
    // Call getter functions each time, don't store in variables
    let label = `${get_entity_name(get_script_entity(i))}`;
    if ui.button(label) {
        // Fetch fresh values in handler
        let path = get_script_source_path(i);
        set_string("path", path);
    }
    i = i + 1;
}

// ❌ BROKEN - for loop over Vec
for script in get_all_scripts() {  // This pattern doesn't work reliably
    // ...
}
```

**4. Avoid comparisons, always update**

```rune
// ✅ SAFE - no comparison, just update
let text = get_string("text", "");
let new_text = ui.text_edit("id", text);
set_string("text", new_text);  // Always set, don't check if changed

// ❌ BROKEN - comparison uses moved value
if new_text != text {  // Error!
    set_string("text", new_text);
}
```

#### Patterns That Don't Work

Despite multiple attempts and workarounds, these patterns are **not reliable** in Rune:

1. ❌ Returning `Vec<CustomStruct>` from Rust to Rune
2. ❌ Iterating over complex objects with `for` loops
3. ❌ Accessing struct fields multiple times (even with `#[rune(get)]`)
4. ❌ Comparing values after passing to functions
5. ❌ Using values in template strings then using them again

**Recommendation**: If your UI plugin needs complex data iteration (like a list of items with multiple fields), consider:
- Using simpler data structures (separate arrays)
- Using index-based access instead of iteration
- Storing data in state and accessing by key
- Or consider if Rune is the right tool for your use case

### Value Handling Best Practices

**ALWAYS:**
- Use type-specific state functions (`get_string`, `get_f64`, `get_bool`)
- Call getter functions multiple times instead of storing values
- Update state unconditionally instead of comparing first
- Use `while` loops with indices instead of `for` loops over complex data

**NEVER:**
- Store values in variables if you'll use them multiple times
- Use values in template strings and then use them again
- Compare values after passing to functions
- Iterate over `Vec` of custom structs with `for` loops

**Important**: Always use matching get/set functions:
- `set_string()` with `get_string()` for strings
- `set_f64()` with `get_f64()` for numbers
- `set_bool()` with `get_bool()` for booleans

### File I/O

File I/O functions return error messages as strings instead of Result types:

```rune
let content = read_file("file.txt");
if content.starts_with("ERROR:") {
    log_error(content);
} else {
    // Use content
}

let error = write_file("file.txt", "data");
if error != "" {
    log_error(error);
}
```

---

## Best Practices

### 1. Initialize State in `on_created`

```rune
pub fn on_created(self_entity) {
    // Set all default values
    set_state("health", 100.0);
    set_state("name", "Player");
    set_state("is_active", true);
}
```

### 2. Use Type-Specific State for Performance

```rune
// ✅ Fast - uses f64 directly
set_f64("speed", 5.0);
let speed = get_f64("speed");

// ⚠️ Slower - uses Value wrapper
set_state("speed", 5.0);
let speed = get_state("speed", 5.0);
```

### 3. Cache Expensive Queries

```rune
pub fn on_created(self_entity) {
    let cameras = query_entities_with_component("CameraComponent");
    set_state("main_camera", cameras[0]);
}

pub fn update(self_entity, dt) {
    // Use cached value instead of querying every frame
    let camera = get_state("main_camera", 0);
}
```

### 4. Clean Up in `on_destroyed`

```rune
pub fn on_destroyed(self_entity) {
    // Unsubscribe from events
    unsubscribe_event("player_scored");

    // Clean up spawned entities if needed
    let spawned_entities = get_state("spawned", []);
    for entity in spawned_entities {
        // Cleanup logic
    }
}
```

### 5. Use Events for Decoupled Communication

Instead of direct script-to-script calls, use events:

```rune
// ✅ Good - decoupled
emit_event("enemy_died", #{ entity: enemy_id });

// ❌ Bad - tightly coupled
// (No direct script method calls exist anyway)
```

### 6. Organize UI Code

```rune
pub fn on_ui(ui) {
    show_header(ui);
    ui.separator();
    show_controls(ui);
    ui.separator();
    show_status(ui);
}

fn show_header(ui) {
    ui.heading("My Tool");
}

fn show_controls(ui) {
    if ui.button("Action") {
        perform_action();
    }
}

fn show_status(ui) {
    let status = get_state("status", "Ready");
    ui.label(status);
}

fn perform_action() {
    set_state("status", "Action performed!");
}
```

---

## Example Scripts

### Orbit Camera

```rune
pub fn on_created(self_entity) {
    set_f64("orbit_speed", 1.0);
    set_f64("orbit_radius", 5.0);
    set_f64("angle", 0.0);
}

pub fn update(self_entity, dt) {
    let speed = get_f64("orbit_speed");
    let radius = get_f64("orbit_radius");
    let angle = get_f64("angle");

    // Update angle
    let new_angle = angle + speed * dt;
    set_f64("angle", new_angle);

    // Calculate position
    let x = radius * new_angle.cos();
    let z = radius * new_angle.sin();

    set_translation(self_entity, x, 2.0, z);
    look_at(self_entity, 0.0, 0.0, 0.0);
}
```

### Spawner Tool

```rune
// @tool
// Entity Spawner
//
// Spawn entities at mouse position

pub fn on_created(self_entity) {
    set_state("entity_name", "SpawnedEntity");
    set_state("spawn_count", 0.0);
}

pub fn on_ui(ui) {
    ui.heading("Entity Spawner");

    let name = get_string("entity_name", "SpawnedEntity");
    let new_name = ui.text_edit("name", name);
    set_string("entity_name", new_name);

    ui.separator();

    if ui.button("Spawn Entity") {
        let entity = spawn_entity(Some(new_name));

        let count = get_state("spawn_count", 0.0);
        set_state("spawn_count", count + 1.0);

        log_info(`Spawned ${new_name} (total: ${count + 1.0})`);
    }

    let count = get_state("spawn_count", 0.0);
    ui.label(`Total spawned: ${count}`);
}
```

---

## Troubleshooting

### "Cannot read, value is M-000000" Error

**This is the most common Rune error.** It means you're trying to use a Value that has been "moved" (ownership transferred).

**Common causes:**

1. **Using a variable after passing it to a function:**
```rune
let text = get_string("text", "");
let new_text = ui.text_edit("id", text);  // text is moved here
if text != new_text {  // ❌ ERROR: text was moved
    // ...
}
```

**Fix:** Don't reuse variables. Fetch fresh values:
```rune
let text = get_string("text", "");
let new_text = ui.text_edit("id", text);
set_string("text", new_text);  // ✅ Just update, don't compare
```

2. **Using a variable in a template string then using it again:**
```rune
let name = get_string("name", "");
let label = `Hello ${name}`;  // name is moved into template
log_info(name);  // ❌ ERROR: name was moved
```

**Fix:** Fetch the value again or use it inline:
```rune
let label = `Hello ${get_string("name", "")}`;
log_info(get_string("name", ""));  // ✅ Fetch again
```

3. **Iterating over complex data:**
```rune
for item in get_items() {  // ❌ BROKEN in many cases
    let x = item.field;  // ERROR
}
```

**Fix:** Use index-based access:
```rune
let count = get_item_count();
let i = 0;
while i < count {
    let value = get_item_field(i);  // ✅ Call function directly
    i = i + 1;
}
```

**General solution:** See the [Value Ownership Issues](#value-ownership-issues-️) section above for comprehensive workarounds.

### Script Not Loading

1. **Check the console** for compilation errors
   - Look for syntax errors, undefined functions, type mismatches
2. **Verify the script path** is correct relative to the working directory
3. **Ensure the file has a `.rn` extension**
4. **Check for Rune syntax issues:**
   - No `mut` keyword (use `let` only)
   - No type annotations on struct fields
   - Functions need `pub fn` for lifecycle hooks

### UI Not Showing

1. **Ensure script has `// @tool` or `// @editor` annotation** at the top
2. **Implement the `on_ui(ui)` function**
3. **Check console for errors** - UI errors are logged as warnings
4. **Verify the plugin is registered** in `ui_plugins.toml` (for file-based plugins)
5. **Check plugin is enabled** in the manifest (`enabled = true`)

### State Not Persisting

State is per-entity instance. State is lost when:
- The entity is destroyed
- The script component is removed
- The application is restarted

For persistent data across sessions, use file I/O:
```rune
// Save state to file
let data = get_string("important_data", "");
write_file("save_data.txt", data);

// Load state from file
let data = read_file("save_data.txt");
if !data.starts_with("ERROR") {
    set_string("important_data", data);
}
```

### Performance Issues

If your script causes lag:

1. **Cache expensive queries:**
```rune
// ❌ BAD - queries every frame
pub fn update(self_entity, dt) {
    let cameras = query_entities_with_component("CameraComponent");
}

// ✅ GOOD - query once
pub fn on_created(self_entity) {
    let cameras = query_entities_with_component("CameraComponent");
    set_state("camera", cameras[0]);
}
```

2. **Use type-specific state functions** (`get_f64`, `get_string`, `get_bool`) instead of generic `get_state()`
3. **Avoid complex calculations in `on_ui()`** - UI is called every frame
4. **Consider moving heavy work to `update()` and caching results**

### Debugging Tips

1. **Use logging liberally:**
```rune
log_info(`Value of x: ${x}`);
log_debug(`Entering function with entity: ${self_entity}`);
```

2. **Check types with template strings:**
```rune
let value = get_state("key", 0);
log_info(`Type check: ${value}`);  // See what you actually got
```

3. **Test incrementally** - add features one at a time
4. **Consult official examples** at https://github.com/rune-rs/rune/tree/main/examples/examples

---

## Additional Resources

- [Rune Language Documentation](https://rune-rs.github.io/)
- Example scripts in `examples/scripts/`
- Engine source: `src/scripting/rune/`

---

**Last Updated:** 2025-11-07 (Updated with critical Value ownership information)
**Engine Version:** Compatible with latest main branch

**⚠️ Important Note on Rune Limitations:**

This documentation reflects both the capabilities and significant limitations discovered through extensive testing. The Value ownership issues described in the "Known Limitations" section are fundamental to Rune's design and may make it unsuitable for certain types of UI scripting, particularly those involving:
- Complex data iteration and display
- Lists of items with multiple fields
- Scenarios requiring value reuse across multiple operations

Consider these limitations when deciding whether Rune is appropriate for your scripting needs. For simple scripts (entity behaviors, basic UI panels, straightforward tools), Rune works well. For complex UI editors with dynamic data, you may encounter fundamental limitations.
