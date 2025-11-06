# Engine UI Plugin Quick Start

## Create Your First Plugin in 5 Minutes

### Step 1: Create a New Script File

Create a file in `examples/scripts/` or your project's scripts directory:

**`my_first_plugin.rn`**
```rune
// @tool
// My First Plugin
//
// A simple counter plugin to demonstrate the basics

pub fn on_created(self_entity) {
    log_info("My First Plugin loaded!");
    set_f64("counter", 0.0);
}

pub fn on_ui(self_entity, ui) {
    ui.heading("My First Plugin");
    ui.separator();

    let count = get_f64("counter");
    ui.label(`Count: ${count}`);

    if ui.button("Increment") {
        set_f64("counter", count + 1.0);
        log_info(`Count increased to ${count + 1.0}`);
    }

    if ui.button("Reset") {
        set_f64("counter", 0.0);
        log_info("Counter reset");
    }
}
```

### Step 2: Load in Editor

1. Start the editor
2. Attach a `RuneScript` component to any entity
3. Set the script path to your `.rn` file
4. The plugin window will appear automatically!

### Step 3: See It Work

- Click "Increment" to increase the counter
- Click "Reset" to set it back to 0
- Check the console for log messages

## Key Concepts

### 1. The @tool Annotation
```rune
// @tool  ← This makes it an editor tool
```
Without this, the script runs in play mode only.

### 2. Lifecycle Functions

```rune
pub fn on_created(self_entity) {
    // Called once when script loads
    // Initialize your state here
}

pub fn on_ui(self_entity, ui) {
    // Called every frame
    // Build your UI here
}

pub fn update(self_entity, dt) {
    // Optional: called every frame
    // dt == 0 in editor mode
}
```

### 3. State Storage

```rune
// Save data
set_f64("my_number", 42.0)
set_state("my_text", "hello")

// Load data
let number = get_f64("my_number")
let text = get_state("my_text")

// Safe load (returns () if not found)
let maybe_value = try_get_state("key")
```

### 4. Common Widgets

```rune
// Display text
ui.label("Hello World")
ui.heading("Section Title")
ui.separator()

// Button (returns true when clicked)
if ui.button("Click Me") {
    log_info("Clicked!");
}

// Text input
let name = get_state("name");
let new_name = ui.text_edit("name_input", name);
if new_name != name {
    set_state("name", new_name);
}

// Slider
let volume = get_f64("volume");
let new_volume = ui.slider("volume_slider", volume, 0.0, 1.0);
if new_volume != volume {
    set_f64("volume", new_volume);
}

// Checkbox
let enabled = try_get_state("enabled");
let is_enabled = if enabled is () { false } else { enabled };
let new_enabled = ui.checkbox("enabled_check", is_enabled, "Enable Feature");
if new_enabled != is_enabled {
    set_state("enabled", new_enabled);
}
```

## Common Patterns

### Pattern 1: Settings Panel
```rune
pub fn on_created(self_entity) {
    set_f64("music_volume", 0.8);
    set_f64("sfx_volume", 0.8);
    set_state("fullscreen", true);
}

pub fn on_ui(self_entity, ui) {
    ui.heading("Game Settings");
    ui.separator();

    let music = get_f64("music_volume");
    let new_music = ui.slider("music", music, 0.0, 1.0);
    if new_music != music {
        set_f64("music_volume", new_music);
    }
    ui.label(`Music: ${new_music}`);

    let sfx = get_f64("sfx_volume");
    let new_sfx = ui.slider("sfx", sfx, 0.0, 1.0);
    if new_sfx != sfx {
        set_f64("sfx_volume", new_sfx);
    }
    ui.label(`SFX: ${new_sfx}`);

    let fullscreen = try_get_state("fullscreen");
    let is_full = if fullscreen is () { true } else { fullscreen };
    let new_full = ui.checkbox("fullscreen", is_full, "Fullscreen");
    if new_full != is_full {
        set_state("fullscreen", new_full);
    }

    if ui.button("Apply") {
        log_info("Settings applied!");
    }
}
```

### Pattern 2: Entity Spawner
```rune
pub fn on_created(self_entity) {
    set_state("entity_name", "NewEntity");
    set_f64("x", 0.0);
    set_f64("y", 0.0);
    set_f64("z", 0.0);
}

pub fn on_ui(self_entity, ui) {
    ui.heading("Entity Spawner");
    ui.separator();

    let name = get_state("entity_name");
    let new_name = ui.text_edit("name", name);
    if new_name != name {
        set_state("entity_name", new_name);
    }

    let x = get_f64("x");
    let new_x = ui.drag_value("x", x);
    if new_x != x {
        set_f64("x", new_x);
    }
    ui.label(`X: ${new_x}`);

    let y = get_f64("y");
    let new_y = ui.drag_value("y", y);
    if new_y != y {
        set_f64("y", new_y);
    }
    ui.label(`Y: ${new_y}`);

    let z = get_f64("z");
    let new_z = ui.drag_value("z", z);
    if new_z != z {
        set_f64("z", new_z);
    }
    ui.label(`Z: ${new_z}`);

    if ui.button("Spawn") {
        let entity = spawn_entity(new_name);
        set_translation(entity, new_x, new_y, new_z);
        log_info(`Spawned ${new_name} at (${new_x}, ${new_y}, ${new_z})`);
    }
}
```

### Pattern 3: Info Display with Refresh
```rune
pub fn on_created(self_entity) {
    set_f64("refresh_time", 0.0);
    set_f64("entity_count", 0.0);
}

pub fn update(self_entity, dt) {
    // Refresh stats every second
    let time = get_f64("refresh_time") + dt;
    if time >= 1.0 {
        let entities = query_entities_with_component("Transform");
        set_f64("entity_count", entities.len());
        set_f64("refresh_time", 0.0);
    } else {
        set_f64("refresh_time", time);
    }
}

pub fn on_ui(self_entity, ui) {
    ui.heading("Scene Info");
    ui.separator();

    let count = get_f64("entity_count");
    ui.label(`Total Entities: ${count}`);

    if ui.button("Refresh Now") {
        set_f64("refresh_time", 999.0);  // Force refresh
    }
}
```

## Next Steps

1. **Explore Examples** - Check out the example plugins in `/examples/scripts/`:
   - `scene_stats_panel.rn` - Scene statistics
   - `quick_actions_panel.rn` - Quick actions
   - `debug_console_plugin.rn` - Debug console

2. **Read Full Documentation** - See `ENGINE_UI_PLUGINS.md` for:
   - Complete widget reference
   - Scene interaction API
   - Best practices
   - Advanced patterns

3. **Create Plugin Manifest** - Add your plugin to `ui_plugins.toml`:
   ```toml
   [[plugin]]
   name = "My First Plugin"
   script = "my_first_plugin.rn"
   description = "A simple counter plugin"
   category = "Tutorial"
   enabled = true
   default_visible = true
   ```

4. **Build Something Useful** - Ideas for your next plugin:
   - Level editor tools
   - Asset batch processors
   - Custom inspectors
   - Debug visualizers
   - Project-specific utilities

## Troubleshooting

**Plugin not showing up?**
- Check that `@tool` annotation is at the top
- Verify the script is attached to an entity
- Look for errors in the console

**UI not updating?**
- Make sure `on_ui()` is public: `pub fn on_ui`
- Check that you're calling `set_state()` to save changes

**Need more widgets?**
- See `ENGINE_UI_PLUGINS.md` for the complete widget API
- Currently available: label, button, text_edit, slider, drag_value, checkbox, color_edit

## Tips

1. **Widget IDs must be unique** - Use descriptive IDs like `"player_health"` not `"slider1"`
2. **Always check for changes** - Compare old and new values before saving state
3. **Use try_get_state()** - For optional values to avoid panics
4. **Log liberally** - Use `log_info()`, `log_warn()`, `log_error()` for debugging
5. **Start simple** - Begin with basic widgets, add complexity gradually

Happy plugin development! 🎮
