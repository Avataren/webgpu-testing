# Lua Scripting Guide

This document describes the Lua scripting integration and provides examples of common patterns.

## Overview

The Lua scripting system provides the same functionality as Rune scripts with a more familiar, flexible syntax. All Rune API functions have been ported to Lua with equivalent behavior.

## Key Differences from Rune

### 1. **Array Indexing**
- **Rune**: 0-indexed arrays `array[0]`
- **Lua**: 1-indexed arrays `array[1]`

```lua
-- Rune
let axis = get_state("axis", [0.0, 1.0, 0.0]);
let x = axis[0];  -- First element

-- Lua equivalent
local axis = get_state("axis", {0.0, 1.0, 0.0})
local x = axis[1]  -- First element (1-indexed!)
```

### 2. **Data Structures**
- **Rune**: Uses structs `struct Point { x, y }`
- **Lua**: Uses tables `{ x = 0, y = 0 }`

```lua
-- Rune
struct CubeState {
    angle,
    frameCount,
}
set_state("cube_state", CubeState { angle: 0.0, frameCount: 0.0 });

-- Lua equivalent
local cube_state = {
    angle = 0.0,
    frameCount = 0.0
}
set_state("cube_state", cube_state)
```

### 3. **Function Declarations**
- **Rune**: Uses `pub fn` keyword
- **Lua**: Uses `function` keyword

```lua
-- Rune
pub fn update(self_entity, dt) {
    // ...
}

-- Lua equivalent
function update(self_entity, dt)
    -- ...
end
```

### 4. **String Formatting**
- **Rune**: Uses string interpolation `\`Value: ${value}\``
- **Lua**: Uses `string.format()`

```lua
-- Rune
log_info(`Position: ${x}, ${y}, ${z}`);

-- Lua equivalent
log_info(string.format("Position: %f, %f, %f", x, y, z))
```

### 5. **Comments**
- **Rune**: Uses `//` for single-line comments
- **Lua**: Uses `--` for single-line comments

## Important Conventions

### Rotation Order (Euler Angles)

The `set_rotation(entity, yaw, pitch, roll)` function uses **YXZ** Euler rotation order:

1. **Yaw** - Rotation around Y axis (turning left/right)
2. **Pitch** - Rotation around X axis (looking up/down)
3. **Roll** - Rotation around Z axis (tilting/rolling)

**All angles must be in radians:**
- 90 degrees = π/2 ≈ 1.5708 radians
- 180 degrees = π ≈ 3.1416 radians
- 360 degrees = 2π ≈ 6.2832 radians

```lua
-- Rotate 90 degrees around Y axis (yaw)
set_rotation(entity, 1.5708, 0, 0)

-- Rotate 45 degrees pitch
set_rotation(entity, 0, 0.7854, 0)

-- Helper function to convert degrees to radians
function deg_to_rad(degrees)
    return degrees * (3.14159265359 / 180)
end

set_rotation(entity, deg_to_rad(90), 0, 0)
```

### Coordinate System

The engine uses a **right-handed 3D coordinate system**:

- **X axis**: Right (positive = right, negative = left)
- **Y axis**: Up (positive = up, negative = down)
- **Z axis**: Back (positive = toward camera, negative = away from camera)

```lua
-- Move entity to the right
translate(entity, 5, 0, 0)

-- Move entity up
translate(entity, 0, 3, 0)

-- Move entity forward (away from camera)
translate(entity, 0, 0, -10)
```

### Array Indexing

**Lua uses 1-based indexing** - the first element of a table is at index 1, not 0.

All API functions that return arrays use this convention:

```lua
-- Get children of an entity
local children = get_children(parent_entity)
if children then
    log_info("Child count: " .. #children)

    -- Iterate using 1-based indices
    for i = 1, #children do
        local child = children[i]
        log_info("Child " .. i .. ": " .. child)
    end
end

-- Query entities
local enemies = query_entities_with_component("Enemy")
for i = 1, #enemies do
    local enemy = enemies[i]
    -- Do something with enemy
end
```

### Entity Handles

Entities are represented as **64-bit integers (i64)** in Lua. You can:
- Store them in variables
- Pass them to functions
- Store them in tables or state
- Compare them with `==`

```lua
-- Store entity reference
local player = spawn_entity("Player")
set_state("player_ref", player)

-- Retrieve and use it later
local player = get_state("player_ref", 0)
if player ~= 0 then
    set_translation(player, 0, 10, 0)
end
```

### World vs Local Transforms

- **Getter functions** (`get_world_translation`, `get_world_rotation`) return transforms in **world space** (accounting for parent hierarchy)
- **Setter functions** (`set_translation`, `set_rotation`, `set_scale`) set **local transforms** relative to the parent

```lua
-- Set local position (relative to parent)
set_translation(child, 5, 0, 0)

-- Get world position (absolute in scene)
local world_pos = get_world_translation(child)
if world_pos then
    log_info(string.format("World position: %.2f, %.2f, %.2f",
        world_pos.x, world_pos.y, world_pos.z))
end
```

## Available API Functions

### Logging
- `log_debug(message)` - Log debug message
- `log_info(message)` - Log info message
- `log_warn(message)` - Log warning message
- `log_error(message)` - Log error message

### State Management
- `set_state(key, value)` - Store any value
- `get_state(key, default)` - Retrieve value with default
- `try_get_state(key)` - Retrieve value or nil
- `set_f64(key, number)` - Store number
- `get_f64(key)` - Retrieve number (panics if not found)
- `set_bool(key, boolean)` - Store boolean
- `get_bool(key, default)` - Retrieve boolean with default
- `set_string(key, string)` - Store string
- `get_string(key, default)` - Retrieve string with default
- For-entity variants: `set_state_for`, `get_state_for`, etc.

### Entity Management
- `spawn_entity(name)` - Create new entity
- `set_name(entity, name)` - Set entity name
- `find_entity_by_name(name)` - Find entity by name
- `attach_inline_script(entity, name, source)` - Attach inline Lua script
- `attach_script(entity, path)` - Attach script from file
- `import_gltf(entity, path, scale)` - Import GLTF model

### Transform API
- `translate(entity, x, y, z)` - Move entity by delta
- `set_translation(entity, x, y, z)` - Set entity position
- `get_world_translation(entity)` - Get entity position (returns table with x, y, z)
- `rotate(entity, axis_x, axis_y, axis_z, angle)` - Rotate around axis
- `set_rotation(entity, yaw, pitch, roll)` - Set rotation (Euler angles in radians)
- `get_world_rotation(entity)` - Get rotation (returns table with yaw, pitch, roll)
- `set_scale(entity, x, y, z)` - Set entity scale
- `look_at(entity, target_x, target_y, target_z)` - Orient towards target

### Hierarchy API
- `set_parent(entity, parent)` - Set parent entity (or nil to unparent)
- `get_parent(entity)` - Get parent entity (returns number or nil)
- `get_children(entity)` - Get child entities (returns table array)

### Input API
- `is_key_pressed(key)` - Check if key is held down
- `is_key_just_pressed(key)` - Check if key was just pressed this frame
- `is_key_just_released(key)` - Check if key was just released this frame
- `is_mouse_button_pressed(button)` - Check if mouse button is held (0=left, 1=right, 2=middle)
- `is_mouse_button_just_pressed(button)` - Check if mouse button was just pressed
- `is_mouse_button_just_released(button)` - Check if mouse button was just released
- `get_mouse_position()` - Get mouse position (returns table with x, y)
- `get_mouse_delta()` - Get mouse movement delta (returns table with x, y)
- `get_mouse_scroll_delta()` - Get scroll wheel delta (returns table with x, y)

### Component API
- `has_component(entity, component_name)` - Check if entity has component
- `get_component(entity, component_name)` - Get component data (currently stubbed)
- `set_component(entity, component_name, value)` - Set component data
- `add_component(entity, component_name, value)` - Add component to entity
- `remove_component(entity, component_name)` - Remove component from entity

### Query API
- `query_entities_with_component(component_name)` - Find all entities with component
- `get_entities_in_radius(x, y, z, radius)` - Find entities within radius
- `get_nearest_entity(x, y, z)` - Find nearest entity to point
- `get_nearest_entity_with_component(x, y, z, component_name)` - Find nearest entity with component
- `get_entities_in_box(min, max)` - Find entities in axis-aligned box

### Events API
- `emit_event(event_name, data)` - Emit custom event with data
- `subscribe_event(event_name, callback_name)` - Subscribe to event
- `unsubscribe_event(event_name)` - Unsubscribe from event

### File I/O API (Sandboxed)
- `read_file(path)` - Read text file (only from `scripts/` or `examples/scripts/`)
- `write_file(path, contents)` - Write text file (only to allowed directories)
- `file_exists(path)` - Check if file exists
- `list_files(dir_path)` - List files in directory

### Clipboard API
- `get_clipboard()` - Get clipboard text (always returns empty string for security)
- `set_clipboard(text)` - Copy text to clipboard

### UI API (Editor Only)

The UI API provides egui widgets for creating editor interfaces. UI is only available in `on_ui(self_entity, ui)` hook.

**Note**: All UI widgets are methods called on the `ui` parameter passed to `on_ui()`.

#### Basic Widgets

- `ui:label(text)` - Display static text label
- `ui:heading(text)` - Display heading text (larger/bold)
- `ui:separator()` - Display horizontal separator line

#### Interactive Widgets

- `ui:button(text)` → `boolean` - Display button, returns true if clicked
- `ui:checkbox(id, current_value, label)` → `boolean` - Display checkbox, returns new value
- `ui:text_edit(id, current_value)` → `string` - Single-line text input, returns new value
- `ui:text_edit_multiline(id, current_value, width, height)` → `string` - Multi-line text editor, returns new value
- `ui:slider(id, current_value, min, max)` → `number` - Horizontal slider, returns new value
- `ui:drag_value(id, current_value)` → `number` - Draggable number widget, returns new value
- `ui:color_edit(id, r, g, b)` → `table{r, g, b}` - Color picker, returns table with RGB values (0.0-1.0)

#### Widget ID Requirements

Most interactive widgets require a unique `id` parameter. This ID is used to:
- Track widget state between frames
- Store responses from user interactions
- Prevent conflicts between widgets

**Best Practice**: Use descriptive IDs that reflect the widget's purpose.

```lua
-- Good IDs
local new_value = ui:slider("player_health", health, 0, 100)
local new_text = ui:text_edit("player_name", name)

-- Avoid generic IDs that might conflict
local value1 = ui:slider("slider1", val1, 0, 100)  -- Could conflict!
local value2 = ui:slider("slider2", val2, 0, 100)
```

#### UI Example

```lua
-- @editor
function on_created(self_entity)
    set_state("counter", 0)
    set_state("text", "Hello!")
    set_state("enabled", true)
    set_state("slider_val", 0.5)
end

function on_ui(self_entity, ui)
    ui:heading("My UI Panel")
    ui:label("Control panel for entity")
    ui:separator()

    -- Button
    local count = get_state("counter", 0)
    if ui:button("Increment") then
        count = count + 1
        set_state("counter", count)
    end
    ui:label(string.format("Count: %d", count))

    -- Checkbox
    local enabled = get_state("enabled", true)
    enabled = ui:checkbox("enable_id", enabled, "Enable feature")
    set_state("enabled", enabled)

    -- Text edit
    local text = get_state("text", "")
    text = ui:text_edit("text_id", text)
    set_state("text", text)

    -- Slider
    local value = get_state("slider_val", 0.5)
    value = ui:slider("slider_id", value, 0.0, 1.0)
    set_state("slider_val", value)
    ui:label(string.format("Value: %.2f", value))
end
```

See `ui_example_simple.lua`, `ui_example_comprehensive.lua`, and `ui_transform_controller.lua` for more examples.

## Example Scripts

### Basic Cube Rotation
See `editor_cube_spin.lua` for a simple rotation example with state management.

### Fractal Cube Animation
See `fractal_cube_rotate.lua` for complex rotation with axis control.

### Event System
- `event_emitter.lua` - Demonstrates event emission
- `event_listener.lua` - Demonstrates event subscription and handling

### Comprehensive API Test
See `lua_api_test.lua` for a comprehensive test of all API categories.

## Lifecycle Hooks

Scripts can implement these lifecycle functions:

```lua
function on_created(self_entity)
    -- Called once when entity is created
end

function update(self_entity, dt)
    -- Called every frame
    -- dt is delta time in seconds
end

function on_ui(self_entity, ui)
    -- Called for editor UI rendering (if script is @editor or @tool)
end
```

## Script Modes

Scripts can declare their mode using annotations:

```lua
-- @editor
-- This script only runs in the editor

-- OR

-- @tool
-- This script only runs in the editor (same as @editor)

-- OR (default)
-- Scripts without annotations run in both editor and runtime
```

## Known Limitations

1. **get_component()** is currently stubbed and returns `nil`. This will be implemented when rune::Value → serde_json::Value conversion is added to the ComponentRegistry.

2. **Script Access API** (metaprogramming) is not yet implemented.

## Best Practices

1. **Always provide default values** when using `get_state()` to handle first-time access
2. **Use typed state functions** (`set_f64`, `set_bool`, etc.) for better performance with simple types
3. **Clean up subscriptions** using `unsubscribe_event()` when no longer needed
4. **Test with `lua_api_test.lua`** to verify API functionality
5. **Check for nil returns** when using functions that may fail (e.g., `find_entity_by_name`)

## Migration from Rune

To migrate existing Rune scripts to Lua:

1. Change file extension from `.rn` to `.lua`
2. Replace `pub fn` with `function`
3. Change `//` comments to `--`
4. Convert structs to tables
5. Adjust array indices from 0-based to 1-based
6. Replace string interpolation with `string.format()`
7. Update variable declarations from `let` to `local`
8. Update UI widget calls: change `.method()` to `:method()` syntax

### UI Migration

Rune and Lua UI APIs are functionally identical but use different calling conventions:

```lua
-- Rune
pub fn on_ui(self_entity, ui) {
    ui.label("Hello");
    if ui.button("Click") {
        log_info("Clicked!");
    }
    let value = ui.slider("slider_id", 0.5, 0.0, 1.0);
}

-- Lua (use colon syntax for method calls)
function on_ui(self_entity, ui)
    ui:label("Hello")
    if ui:button("Click") then
        log_info("Clicked!")
    end
    local value = ui:slider("slider_id", 0.5, 0.0, 1.0)
end
```

Example migration:

```lua
-- Rune
pub fn update(self_entity, dt) {
    let state = get_state("data", Data { value: 0.0 });
    let new_value = state.value + dt;
    set_state("data", Data { value: new_value });
    log_info(`Value: ${new_value}`);
}

-- Lua
function update(self_entity, dt)
    local state = get_state("data", { value = 0.0 })
    local new_value = state.value + dt
    set_state("data", { value = new_value })
    log_info(string.format("Value: %f", new_value))
end
```
