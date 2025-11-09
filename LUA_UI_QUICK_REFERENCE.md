# Lua UI System - Quick Reference Guide

## What's the Current State?

**System Type**: Editor-integrated immediate-mode UI (egui) with Lua scripting via mlua

**UI Framework**: egui (Rust), accessed through Lua with command recording pattern

**Viewport Constraints**: NOT IMPLEMENTED - Lua UI currently has no viewport restriction capability

---

## Key Components Quick Lookup

| What | Where | Purpose |
|------|-------|---------|
| UI command recording | `/src/scripting/lua/api/ui/context.rs` | Stores UI operations as commands |
| UI widget types | `/src/scripting/lua/api/ui/commands.rs` | Button, Slider, TextEdit, etc. |
| Viewport constraints | `/src/renderer/render_region.rs` | GPU scissor rect + viewport for game view |
| Editor layout | `/src/bin/editor/layout.rs` | Dockable panels, viewport computation |
| egui rendering | `/src/ui/egui_integration.rs` | Renders UI to wgpu surface |

---

## Data Flow in 30 Seconds

```
Lua Script on_ui() 
  └─> Records UiCommand (immediate-mode API)
      └─> Next: Playback with real egui::Ui
          └─> Collect UiResponse
              └─> Store for next frame
                  └─> Lua accesses responses this frame
```

---

## Current UI Capabilities

### Available Widgets
- Labels, Buttons, Headings
- Text input (single & multi-line)
- Sliders, Drag values
- Checkboxes, Color pickers
- Menus & menu bars
- **Layout**: CenteredArea only (not full constraint system)

### Current Limitations
- No viewport bounds awareness
- No world-to-screen projection
- No camera/view matrix access
- No anchoring system (except centered_area)
- No parent-child UI hierarchies
- Fills entire panel space (no constrained rendering)

---

## Example Script Structure

```lua
-- @editor  [Run in editor mode]

function on_created(self_entity)
    -- Initialize once
    set_state("my_value", 0)
end

function on_ui(self_entity, ui)
    -- Called every frame
    
    local value = get_state("my_value", 0)
    ui:heading("My Panel")
    
    local new_value = ui:slider("my_slider", value, 0, 100)
    if new_value ~= value then
        set_state("my_value", new_value)
    end
end
```

---

## RenderRegion Explained (What's Missing)

**RenderRegion** = GPU pixel bounds for viewport clipping

```rust
RenderRegion {
    x: u32,      // Left edge (pixels)
    y: u32,      // Top edge (pixels)  
    width: u32,  // Width (pixels)
    height: u32  // Height (pixels)
}
```

**Used for**: Game viewport rendering (scissor rect + hardware viewport)

**NOT used for**: Lua UI rendering (the gap!)

---

## File Paths Reference

### Lua UI Implementation
```
/src/scripting/lua/api/ui/
├── mod.rs              [API registration]
├── context.rs          [UiContext - command recording]
└── commands.rs         [UiCommand enum + egui rendering]
```

### Viewport System
```
/src/renderer/
├── render_region.rs    [RenderRegion struct]
├── frame_graph.rs      [Main render pipeline]
└── render_context.rs   [Custom render hooks]

/src/bin/editor/
├── layout.rs           [ViewportState, compute_viewport_region()]
└── application/
    ├── core.rs         [EditorApplication, ViewportSystem]
    └── camera_system.rs [Camera viewport awareness]
```

### Integration
```
/src/ui/egui_integration.rs    [egui + wgpu bridge]
/src/bin/editor/application/ui_plugin_manager.rs [Lua UI plugin loading]
```

### Examples
```
/examples/scripts/
├── welcome_screen.lua          [Full editor UI plugin]
└── test_minimal_ui.lua         [Minimal test]

/scripts/ui_example_comprehensive.lua [Widget showcase]
```

---

## Key Types at a Glance

### UiContext (What Lua scripts call)
```rust
pub struct UiContext {
    commands: Arc<Mutex<Vec<UiCommand>>>,
    responses: Arc<Mutex<HashMap<String, UiResponse>>>,
}

// Available methods (called from Lua as ui:method_name())
impl UiContext {
    pub fn button(&self, text: String) -> bool
    pub fn slider(&self, id: String, value: f64, min: f64, max: f64) -> f64
    pub fn text_edit(&self, id: String, current_value: String) -> String
    pub fn checkbox(&self, id: String, value: bool, label: String) -> bool
    // ... etc
}
```

### UiCommand (What gets recorded and replayed)
```rust
pub enum UiCommand {
    Button { text: String },
    Slider { id: String, current_value: f64, min: f64, max: f64 },
    TextEdit { id: String, current_value: String },
    // ... many more widget types
}

impl UiCommand {
    pub fn render_and_collect(
        &self,
        ui: &mut egui::Ui,
        responses: &mut HashMap<String, UiResponse>,
    ) // Renders with egui, collects response
}
```

### UiResponse (What's returned to scripts next frame)
```rust
pub struct UiResponse {
    pub clicked: bool,
    pub hovered: bool,
    pub changed: bool,
    pub text_value: Option<String>,
    pub float_value: Option<f64>,
    pub bool_value: Option<bool>,
    pub color_value: Option<(f32, f32, f32)>,
}
```

### RenderRegion (GPU-level constraint, NOT used by Lua UI)
```rust
pub struct RenderRegion {
    x: u32,      // Pixel coords
    y: u32,
    width: u32,
    height: u32,
}

impl RenderRegion {
    pub fn apply_to_pass<'a>(&self, pass: &mut wgpu::RenderPass<'a>) {
        pass.set_viewport(...);        // Hardware viewport
        pass.set_scissor_rect(...);    // Hardware clipping
    }
}
```

### ViewportState (Coordination between egui and GPU)
```rust
pub struct ViewportState {
    region: Option<RenderRegion>,   // GPU pixels
    rect: Option<egui::Rect>,       // egui points
}

// Both set together via:
pub fn compute_viewport_region(ctx: &egui::Context, rect: egui::Rect) -> Option<RenderRegion>
```

---

## The Critical Missing Piece

**Problem**: Lua UI commands render via egui without viewport constraints

**Solution Path** (not implemented):
1. Pass `RenderRegion` to `UiCommand::render_and_collect()`
2. Apply region before rendering: `region.apply_to_pass(&mut pass)`
3. Expose viewport info to Lua scripts: `ui:get_viewport_rect()`

**Impact**: Would enable viewport-constrained UI overlays on game view

---

## How to Find Code

### "How do UI commands get rendered?"
→ `/src/scripting/lua/api/ui/commands.rs:147-309` (`UiCommand::render()`)

### "Where does viewport region get computed?"
→ `/src/bin/editor/layout.rs:236-249` (`compute_viewport_region()`)

### "Where does the game viewport get rendered?"
→ `/src/renderer/frame_graph.rs` & `/src/renderer/render_context.rs` (check `apply_to_pass()`)

### "How are Lua scripts loaded as UI plugins?"
→ `/src/bin/editor/application/ui_plugin_manager.rs`

### "Where's the camera viewport tracking?"
→ `/src/bin/editor/application/camera_system.rs:55-67`

### "How does egui integrate with wgpu?"
→ `/src/ui/egui_integration.rs:123-185` (`EguiContext::render()`)

---

## Common Gotchas

1. **Responses are delayed by one frame**
   - Lua reads responses collected from previous frame
   - Allows deferred rendering pattern but feels "laggy"

2. **UI fills entire panel**
   - `centered_area(width)` is only positioning feature
   - No real constraint system yet

3. **No access to viewport dimensions**
   - Camera knows viewport rect via `EditorCameraController.viewport_rect`
   - But Lua scripts cannot access this

4. **RenderRegion is separate system**
   - Used for game viewport rendering only
   - Not connected to UI pipeline

5. **egui renders with LoadOp::Load**
   - Composites on top of existing content
   - egui uses its own clipping for panel bounds

---

## Scripts and Annotations

### Running Modes
```lua
-- @editor
-- Runs only in editor mode, on_ui() is called

-- @tool
-- Runs in both editor and play (currently same as @editor)

-- (no annotation)
-- Runs only in play mode, no on_ui() access
```

### Callback Sequence
```lua
function on_created(self_entity)
    -- Called once when script loads
end

function on_update(self_entity, dt)
    -- Called every frame (all modes)
end

function on_ui(self_entity, ui)
    -- Called every frame (editor/@tool only)
    -- ui is UiContext userdata
end
```

---

## Command Recording vs Direct Rendering

### Why command recording?

**Problem**: egui::Ui has lifetime constraints incompatible with Lua VM

**Solution**: Defer rendering
1. Lua records commands (fast, no lifetime issues)
2. After Lua completes, replay commands with real egui::Ui
3. Collect responses
4. Feed back to next frame

**Benefit**: Clean separation of concerns, solves lifetime issues, defers to proper context

---

## Next Steps for Implementation

If you want to add viewport constraints:

1. **Phase 1: Expose viewport info**
   - Add `ui:get_viewport_rect()` method
   - Add `ui:get_pixels_per_point()` method

2. **Phase 2: Add constrained rendering**
   - Pass RenderRegion through command rendering
   - Apply scissor rect to egui render pass

3. **Phase 3: Add positioning API**
   - `ui:absolute_panel(x, y, width, height, callback)`
   - `ui:anchored_panel(anchor, offset_x, offset_y, callback)`

4. **Phase 4: Full constraint system**
   - Parent-child relationships
   - Anchor system (top-left, center, etc.)
   - Size constraints
   - Aspect ratio preservation

---

## File Sizes (for reference)

```
/src/scripting/lua/api/ui/context.rs     ~253 lines   (core logic)
/src/scripting/lua/api/ui/commands.rs    ~410 lines   (rendering)
/src/bin/editor/layout.rs                ~267 lines   (viewport calc)
/src/renderer/render_region.rs           ~81 lines    (RenderRegion)
/src/ui/egui_integration.rs              ~270 lines   (egui integration)
```

---

**Report Generated**: Investigation of Lua UI system for viewport-constrained rendering planning

**Key Finding**: Two separate rendering systems that need to be bridged
