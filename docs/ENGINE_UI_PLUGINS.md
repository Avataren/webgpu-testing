# Engine UI Plugins

## Overview

Engine UI Plugins are Rune scripts that extend the editor with custom panels, tools, and workflows. Built on top of the UI Scripting Integration system, they provide a powerful way to customize and enhance the editor without modifying core engine code.

## Architecture

### Two-Tier Plugin System

**Tier 1: Native Rust Plugins (Core Engine)**
- Performance-critical features (scene hierarchy, viewport, asset browser)
- Deep engine integration
- Compiled into the editor binary

**Tier 2: Scripted UI Plugins (User-Extensible)**
- Editor tools and utilities
- Custom inspectors and editors
- Project-specific workflows
- Hot-reloadable during development

### Why Scripted Plugins?

1. **Rapid Development** - No compilation, instant feedback
2. **User Extensibility** - Users can create plugins without Rust knowledge
3. **Hot Reload** - Changes apply immediately in editor
4. **Sandboxed** - Plugin errors don't crash the editor
5. **Shareable** - Distribute as `.rn` files
6. **Project-Specific** - Custom tools for specific game genres

## Creating a Plugin

### Basic Structure

```rune
// @tool
// Plugin Name
//
// Description of what this plugin does

pub fn on_created(self_entity) {
    // Initialize plugin state
    log_info("Plugin initialized");

    // Set default values
    set_state("my_value", "default");
    set_f64("counter", 0.0);
}

pub fn update(self_entity, dt) {
    // Called every frame (dt == 0 in editor mode)
    // Use for background tasks, polling, updates
}

pub fn on_ui(self_entity, ui) {
    // Called every frame to render UI
    ui.heading("My Plugin");
    ui.separator();

    // Add your widgets here
    if ui.button("Click Me") {
        log_info("Button clicked!");
    }
}
```

### Key Requirements

1. **@tool Annotation** - Marks script as editor tool (runs with dt=0)
2. **on_created()** - Initialize state once when plugin loads
3. **update()** - Optional, for background processing
4. **on_ui()** - Required, renders plugin UI each frame

## Available Widgets

### Display Widgets
```rune
ui.label("Text to display")
ui.heading("Section Heading")
ui.separator()  // Horizontal line
```

### Interactive Widgets
```rune
// Button (returns true when clicked)
if ui.button("Click Me") {
    // Handle click
}

// Text input (returns current value)
let name = get_state("name");
let new_name = ui.text_edit("name", name);
if new_name != name {
    set_state("name", new_name);
}

// Slider (returns current value)
let value = get_f64("slider");
let new_value = ui.slider("slider", value, 0.0, 100.0);
if new_value != value {
    set_f64("slider", new_value);
}

// Drag value (returns current value)
let speed = get_f64("speed");
let new_speed = ui.drag_value("speed", speed);
if new_speed != speed {
    set_f64("speed", new_speed);
}

// Checkbox (returns current state)
let enabled = try_get_state("enabled");
let is_enabled = if enabled is () { false } else { enabled };
let new_enabled = ui.checkbox("enabled", is_enabled, "Enable Feature");
if new_enabled != is_enabled {
    set_state("enabled", new_enabled);
}

// Color picker (returns RGB tuple)
let r = get_f64("color_r");
let g = get_f64("color_g");
let b = get_f64("color_b");
let (new_r, new_g, new_b) = ui.color_edit("color", r, g, b);
if new_r != r || new_g != g || new_b != b {
    set_f64("color_r", new_r);
    set_f64("color_g", new_g);
    set_f64("color_b", new_b);
}
```

## State Management

### Per-Plugin State Storage

Each plugin instance has isolated state storage:

```rune
// String/generic values
set_state("key", value)
let value = get_state("key")
let optional_value = try_get_state("key")  // Returns () if not found

// Numeric values (type-safe helpers)
set_f64("number_key", 42.0)
let number = get_f64("number_key")
```

### Widget Response Pattern

**Important:** Widget IDs must be unique within a plugin!

```rune
// 1. Get current value from state
let value = get_f64("my_slider");

// 2. Render widget and get new value
let new_value = ui.slider("my_slider", value, 0.0, 100.0);

// 3. Detect change and update state
if new_value != value {
    set_f64("my_slider", new_value);
    log_info(`Value changed to ${new_value}`);
}
```

## Scene Interaction

### Entity Management
```rune
// Create entities
let entity = spawn_entity("EntityName");

// Transform entities
set_translation(entity, x, y, z);
set_rotation(entity, x, y, z);
set_scale(entity, x, y, z);
translate(entity, dx, dy, dz);
rotate(entity, rx, ry, rz);

// Query entities
let all_entities = query_entities_with_component("Transform");
let meshes = query_entities_with_component("MeshRenderer");
let nearby = get_entities_in_radius(x, y, z, radius);

// Hierarchy
set_parent(child, parent);
let parent = get_parent(entity);
let children = get_children(entity);
let found = find_entity_by_name("EntityName");
```

### Component Operations
```rune
// Check components
if has_component(entity, "MeshRenderer") {
    // Entity has mesh renderer
}

// Get/set component data (if implemented)
let component_data = get_component(entity, "Transform");
set_component(entity, "Transform", data);

// Add/remove components
add_component(entity, "Light");
remove_component(entity, "MeshRenderer");
```

### Input Queries
```rune
// Keyboard
if is_key_pressed("Space") {
    // Handle spacebar
}

// Mouse
if is_mouse_button_pressed("Left") {
    let (x, y) = get_mouse_position();
    log_info(`Clicked at ${x}, ${y}`);
}

let (dx, dy) = get_mouse_scroll_delta();
```

## Example Plugins

### 1. Scene Statistics Panel
**Purpose:** Display real-time scene statistics

```rune
// @tool
pub fn on_created(self_entity) {
    set_f64("refresh_interval", 0.5);
    set_f64("time_since_refresh", 0.0);
}

pub fn update(self_entity, dt) {
    let time = get_f64("time_since_refresh") + dt;
    let interval = get_f64("refresh_interval");

    if time >= interval {
        let entities = query_entities_with_component("Transform");
        set_f64("entity_count", entities.len());
        set_f64("time_since_refresh", 0.0);
    } else {
        set_f64("time_since_refresh", time);
    }
}

pub fn on_ui(self_entity, ui) {
    ui.heading("Scene Statistics");
    ui.separator();

    let count = get_f64("entity_count");
    ui.label(`Total Entities: ${count}`);
}
```

### 2. Quick Actions Panel
**Purpose:** Shortcuts for common operations

```rune
// @tool
pub fn on_created(self_entity) {
    set_f64("spawn_x", 0.0);
    set_f64("spawn_y", 0.0);
    set_f64("spawn_z", 0.0);
}

pub fn on_ui(self_entity, ui) {
    ui.heading("Quick Actions");
    ui.separator();

    let x = get_f64("spawn_x");
    let new_x = ui.drag_value("spawn_x", x);
    if new_x != x {
        set_f64("spawn_x", new_x);
    }
    ui.label(`X: ${new_x}`);

    if ui.button("Spawn Entity") {
        let entity = spawn_entity("NewEntity");
        set_translation(entity, new_x, 0.0, 0.0);
        log_info("Entity spawned!");
    }
}
```

### 3. Debug Console
**Purpose:** Interactive command execution

```rune
// @tool
pub fn on_created(self_entity) {
    set_state("command", "");
}

pub fn on_ui(self_entity, ui) {
    ui.heading("Debug Console");
    ui.separator();

    let cmd = get_state("command");
    let new_cmd = ui.text_edit("command", cmd);
    if new_cmd != cmd {
        set_state("command", new_cmd);
    }

    if ui.button("Execute") {
        execute_command(new_cmd);
        set_state("command", "");
    }
}

fn execute_command(cmd) {
    if cmd == "stats" {
        let entities = query_entities_with_component("Transform");
        log_info(`Total entities: ${entities.len()}`);
    } else {
        log_warn(`Unknown command: ${cmd}`);
    }
}
```

## Plugin Manifest

### ui_plugins.toml

Create a manifest file to declare available plugins:

```toml
[[plugin]]
name = "Scene Statistics"
script = "scene_stats_panel.rn"
description = "Real-time scene statistics"
category = "Debug"
enabled = true
default_visible = true

[[plugin]]
name = "Quick Actions"
script = "quick_actions_panel.rn"
description = "Common editor operations"
category = "Workflow"
enabled = true
default_visible = true
```

### Manifest Fields

- **name** - Display name in editor
- **script** - Filename (relative to scripts directory)
- **description** - What the plugin does
- **category** - Grouping (Debug, Workflow, Assets, etc.)
- **enabled** - Whether to load the plugin
- **default_visible** - Show window by default

## Best Practices

### 1. State Management
```rune
// ✅ DO: Use descriptive state keys
set_state("spawn_position_x", 0.0)

// ❌ DON'T: Use cryptic keys
set_state("x", 0.0)
```

### 2. Widget IDs
```rune
// ✅ DO: Use unique, descriptive IDs
ui.slider("player_health", health, 0.0, 100.0)

// ❌ DON'T: Use generic IDs (risk conflicts)
ui.slider("slider1", value, 0.0, 100.0)
```

### 3. Performance
```rune
// ✅ DO: Throttle expensive operations
if time >= refresh_interval {
    let entities = query_entities_with_component("Transform");
    set_f64("cached_count", entities.len());
}

// ❌ DON'T: Query every frame unnecessarily
pub fn on_ui(self_entity, ui) {
    let entities = query_entities_with_component("Transform");  // Every frame!
    ui.label(`Count: ${entities.len()}`);
}
```

### 4. Error Handling
```rune
// ✅ DO: Handle optional values
let value = try_get_state("key");
let safe_value = if value is () { 0.0 } else { value };

// ❌ DON'T: Assume state exists
let value = get_state("key");  // Panics if not set!
```

### 5. Logging
```rune
// ✅ DO: Use appropriate log levels
log_info("Plugin initialized");
log_warn("Unexpected value, using default");
log_error("Failed to load resource");

// ❌ DON'T: Spam logs
pub fn update(self_entity, dt) {
    log_info("Update called");  // Every frame!
}
```

## Plugin Categories

### Debug
- Performance profilers
- Statistics panels
- Console tools
- Debug visualizers

### Workflow
- Quick action panels
- Batch operations
- Shortcuts and macros
- Project management

### Assets
- Material editors
- Texture tools
- Model importers
- Audio utilities

### Animation
- Timeline editors
- Keyframe tools
- Curve editors
- Animation previews

### Level Design
- Placement tools
- Terrain editors
- Lighting helpers
- Prefab managers

## Advanced Patterns

### Tabbed Interface
```rune
pub fn on_created(self_entity) {
    set_f64("active_tab", 0.0);
}

pub fn on_ui(self_entity, ui) {
    ui.heading("Multi-Tab Plugin");

    if ui.button("Tab 1") { set_f64("active_tab", 0.0); }
    if ui.button("Tab 2") { set_f64("active_tab", 1.0); }
    if ui.button("Tab 3") { set_f64("active_tab", 2.0); }

    ui.separator();

    let tab = get_f64("active_tab");
    if tab == 0.0 {
        render_tab1(self_entity, ui);
    } else if tab == 1.0 {
        render_tab2(self_entity, ui);
    } else {
        render_tab3(self_entity, ui);
    }
}
```

### Collapsible Sections
```rune
pub fn on_created(self_entity) {
    set_state("show_advanced", false);
}

pub fn on_ui(self_entity, ui) {
    ui.heading("Settings");

    // Basic settings always visible
    ui.label("Basic setting");

    // Advanced settings toggle
    let show = try_get_state("show_advanced");
    let is_visible = if show is () { false } else { show };
    let new_visible = ui.checkbox("show_advanced", is_visible, "Show Advanced");
    if new_visible != is_visible {
        set_state("show_advanced", new_visible);
    }

    if new_visible {
        ui.separator();
        ui.label("Advanced settings here");
    }
}
```

### Command Pattern
```rune
fn execute_command(cmd) {
    log_info(`> ${cmd}`);

    if cmd == "help" {
        show_help();
    } else if cmd.starts_with("spawn ") {
        let parts = cmd.split(" ");
        if parts.len() >= 2 {
            spawn_entity(parts[1]);
        }
    } else {
        log_warn(`Unknown command: ${cmd}`);
    }
}
```

## Future Enhancements

### Planned Features (Not Yet Implemented)

1. **Layout Control**
   - Horizontal/vertical layouts
   - Grouping and panels
   - Collapsing headers
   - Scroll areas

2. **More Widgets**
   - Multi-line text edit
   - Combo boxes / dropdowns
   - Radio buttons
   - Progress bars
   - Images

3. **Styling**
   - Custom colors and themes
   - Font sizes
   - Spacing control

4. **Panel Integration**
   - Dock into main editor
   - Save/restore layouts
   - Keyboard shortcuts

5. **Plugin Communication**
   - Inter-plugin events
   - Shared state
   - Plugin dependencies

## Troubleshooting

### Plugin Not Loading
- Verify `@tool` annotation at top of file
- Check file is in correct scripts directory
- Look for syntax errors in console

### UI Not Updating
- Ensure `on_ui()` function is public: `pub fn on_ui`
- Verify state changes are being saved
- Check widget IDs are unique

### Performance Issues
- Move expensive queries to `update()` with throttling
- Cache query results instead of querying every frame
- Use appropriate refresh intervals

### State Not Persisting
- Call `set_state()` or `set_f64()` to save changes
- Use `try_get_state()` to handle missing values
- Initialize state in `on_created()`

## Examples Location

See `/examples/scripts/` for working plugin examples:
- `scene_stats_panel.rn` - Scene statistics display
- `quick_actions_panel.rn` - Quick action shortcuts
- `debug_console_plugin.rn` - Interactive debug console
- `character_editor_tool.rn` - Character property editor
- `advanced_widgets_example.rn` - All widget types demo

## Conclusion

Engine UI Plugins provide a powerful, flexible way to extend the editor with custom tools and workflows. By leveraging the UI Scripting Integration system, you can create sophisticated editor extensions without touching the core engine code, enabling rapid development and community contributions.

Start simple with a basic panel, then expand with more complex interactions as you become familiar with the API. The examples provide a solid foundation for building your own custom editor tools.

Happy plugin development!
