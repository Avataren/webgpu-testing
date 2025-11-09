# Lua UI System Investigation Report

## Executive Summary

The WebGPU testing project has a **command-based immediate-mode UI system** using **egui** with Lua scripting integration. The UI renders within the editor's egui context and currently **fills available panel space** without viewport-constrained rendering. The rendering pipeline uses a `RenderRegion` system for game viewport constraints, but this is NOT currently connected to Lua UI rendering.

---

## 1. UI FRAMEWORK & IMPLEMENTATION

### Framework: egui with Command Recording Pattern

**Key Components:**
- **Framework**: egui (Rust immediate-mode GUI)
- **Language Integration**: mlua (Lua 5.4 bindings)
- **Location**: `/home/user/webgpu-testing/src/scripting/lua/api/ui/`

### Architecture: Command Recording Pattern

The system uses a **deferred rendering approach**:

```
Lua Script Execution (Immediate-Mode) 
    ↓
Record UiCommand (Label, Button, Slider, etc.)
    ↓
Store in Arc<Mutex<Vec<UiCommand>>>
    ↓
Post-Script: Replay Commands with Real egui::Ui
    ↓
Collect UiResponse (clicked, changed, text_value, etc.)
    ↓
Return Responses to Next Frame
```

**File**: `/home/user/webgpu-testing/src/scripting/lua/api/ui/context.rs`

```rust
pub struct UiContext {
    commands: Arc<Mutex<Vec<UiCommand>>>,
    responses: Arc<Mutex<HashMap<String, UiResponse>>>,
}
```

### Why This Pattern?

1. **Lifetime Issues**: egui::Ui has strict lifetime requirements that conflict with Lua VM execution
2. **Async Compatibility**: Uses Arc/Mutex (Send) instead of Rc/RefCell for mlua compatibility
3. **Frame Deferred**: Responses collected AFTER Lua completes, fed back next frame

---

## 2. AVAILABLE UI COMPONENTS

### Complete Widget Set

**File**: `/home/user/webgpu-testing/src/scripting/lua/api/ui/commands.rs`

```rust
pub enum UiCommand {
    Label { text: String },
    Button { text: String },
    Heading { text: String },
    Separator,
    
    // Input Widgets
    TextEdit { id: String, current_value: String },
    TextEditMultiline { id: String, current_value: String, width: Option<f32>, height: Option<f32> },
    Slider { id: String, current_value: f64, min: f64, max: f64 },
    DragValue { id: String, current_value: f64 },
    Checkbox { id: String, current_value: bool, label: String },
    ColorEdit { id: String, r: f32, g: f32, b: f32 },
    
    // Layout
    MenuBar { items: Vec<UiCommand> },
    Menu { text: String, items: Vec<UiCommand> },
    MenuItem { id: String, text: String },
    CenteredArea { width: Option<f32>, items: Vec<UiCommand> },
}
```

### Response Types

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

---

## 3. VIEWPORT/RENDERING SYSTEM

### RenderRegion: GPU-Level Viewport Constraints

**Location**: `/home/user/webgpu-testing/src/renderer/render_region.rs`

The renderer has a **viewport constraint system** that clips rendering to specified regions:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderRegion {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl RenderRegion {
    /// Apply hardware viewport & scissor rect to GPU pass
    pub fn apply_to_pass<'a>(&self, pass: &mut wgpu::RenderPass<'a>) {
        pass.set_viewport(self.x as f32, self.y as f32, self.width as f32, self.height as f32, 0.0, 1.0);
        pass.set_scissor_rect(self.x, self.y, self.width, self.height);
    }
}
```

**Usage Locations**:
- `/home/user/webgpu-testing/src/renderer/frame_graph.rs` - Main render passes
- `/home/user/webgpu-testing/src/renderer/render_context.rs` - Custom render hooks
- `/home/user/webgpu-testing/src/renderer/postprocess/mod.rs` - Post-processing

### Editor Viewport Layout

**Location**: `/home/user/webgpu-testing/src/bin/editor/layout.rs`

The editor uses **egui-tiles** for dockable panels:

```rust
pub struct ViewportState {
    region: Option<RenderRegion>,      // GPU-level constraint
    rect: Option<egui::Rect>,          // egui-level rect (in points, not pixels)
}

pub struct ViewportSystem {
    pub scene_viewport: ViewportState,
    pub game_viewport: ViewportState,
    pub game_view_display: GameViewDisplayMode,
    pub grid_postprocess: Option<ViewportGrid>,
}
```

### Viewport Region Computation

**Location**: `/home/user/webgpu-testing/src/bin/editor/layout.rs:236-249`

```rust
pub fn compute_viewport_region(ctx: &egui::Context, rect: egui::Rect) -> Option<RenderRegion> {
    let pixels_per_point = ctx.pixels_per_point();
    let screen = ctx.viewport_rect();
    let max_width = (screen.width() * pixels_per_point).round().max(0.0) as u32;
    let max_height = (screen.height() * pixels_per_point).round().max(0.0) as u32;

    let min_x = (rect.min.x * pixels_per_point).floor().max(0.0);
    let min_y = (rect.min.y * pixels_per_point).floor().max(0.0);
    let width = (rect.width() * pixels_per_point).round().max(0.0);
    let height = (rect.height() * pixels_per_point).round().max(0.0);

    let region = RenderRegion::new(min_x as u32, min_y as u32, width as u32, height as u32)?;
    region.clamp(max_width, max_height)
}
```

**Key Insight**: 
- Converts egui coordinates (in points) to GPU coordinates (in pixels)
- Accounts for DPI scaling (pixels_per_point)
- Clamps to screen bounds

---

## 4. CURRENT UI POSITIONING & LAYOUT

### How UI Currently Works

1. **Rendered Within Editor Panels**: Lua UI renders inside egui panels using the command system
2. **Fills Available Space**: No viewport constraints - stretches to fill panel dimensions
3. **Layout Built by egui**: Uses egui's vertical/horizontal layout system
4. **Positioned via egui::Rect**: Logical coordinates, not GPU pixels

### Available Layout Features

From `context.rs` (lines 179-197):

```lua
-- Centered area with optional fixed width (currently only layout feature)
ui:centered_area(width, callback)
    → ui.vertical_centered(|ui| { ... })
    → Sets width with ui.set_width()
    → Optionally clamps: (width * 0.6).clamp(320.0, 760.0)
```

### Example: Welcome Screen Panel

**File**: `/home/user/webgpu-testing/examples/scripts/welcome_screen.lua:255-266`

```lua
local PANEL_WIDTH = 640.0

function on_ui(self_entity, ui)
    if not get_bool("welcome_visible", true) then return end
    
    ui:centered_area(PANEL_WIDTH, function(center)
        hero_section(center)
        render_quick_actions(center, show_create)
        -- ... more sections
    end)
end
```

---

## 5. CURRENT LIMITATIONS & ARCHITECTURAL GAPS

### NO Viewport-Constrained Rendering

**Current State**:
- Lua UI renders via egui's immediate-mode API
- egui renders directly to the editor surface
- NO connection to `RenderRegion` system
- NO GPU-level viewport constraints for Lua UI

**Implications**:
- Lua UI cannot be rendered within a specific viewport region
- Cannot overlay game view with UI
- Cannot constrain UI to game view bounds
- Camera aspect ratio not available to UI

### NO Parent-Child Relationships

**Current State**:
- Each widget is independent
- No concept of UI hierarchies or nesting (except layout grouping)
- No anchoring or constraint system

**Available**:
- `CenteredArea` for center alignment only
- Basic egui layout (vertical/horizontal)

### NO Camera/View Integration

**Current State**:
- UI system is completely separate from rendering pipeline
- No access to camera matrices, viewport dimensions
- No world-space to screen-space conversion

**Game View Integration**:
- Game viewport uses `RenderRegion` + `ViewportState`
- Camera is aware of viewport via `EditorCameraController.viewport_rect`
- But this is NOT exposed to Lua UI system

---

## 6. KEY FILES & CODE REFERENCES

### Core UI Implementation

| File | Purpose |
|------|---------|
| `/home/user/webgpu-testing/src/scripting/lua/api/ui/mod.rs` | Register UI API with Lua |
| `/home/user/webgpu-testing/src/scripting/lua/api/ui/context.rs` | UiContext: command recording |
| `/home/user/webgpu-testing/src/scripting/lua/api/ui/commands.rs` | UiCommand enum & rendering |
| `/home/user/webgpu-testing/src/ui/egui_integration.rs` | egui + wgpu integration |

### Viewport & Rendering

| File | Purpose |
|------|---------|
| `/home/user/webgpu-testing/src/renderer/render_region.rs` | RenderRegion GPU constraint |
| `/home/user/webgpu-testing/src/bin/editor/layout.rs` | ViewportState & computation |
| `/home/user/webgpu-testing/src/renderer/frame_graph.rs` | Main render pipeline |
| `/home/user/webgpu-testing/src/renderer/render_context.rs` | Custom render hooks |

### Editor Application

| File | Purpose |
|------|---------|
| `/home/user/webgpu-testing/src/bin/editor/application/core.rs` | EditorApplication & ViewportSystem |
| `/home/user/webgpu-testing/src/bin/editor/camera.rs` | EditorCameraController |
| `/home/user/webgpu-testing/src/bin/editor/application/camera_system.rs` | Camera system integration |

### Example Scripts

| File | Purpose |
|------|---------|
| `/home/user/webgpu-testing/examples/scripts/welcome_screen.lua` | Production UI plugin (640px centered panel) |
| `/home/user/webgpu-testing/scripts/ui_example_comprehensive.lua` | All widget showcase |
| `/home/user/webgpu-testing/examples/scripts/test_minimal_ui.lua` | Minimal example |

---

## 7. ARCHITECTURAL PATTERNS & EXISTING SYSTEMS

### Pattern 1: Command Recording for Deferred Rendering

**Where Used**:
- UI command recording (above)
- Could be extended to viewport constraints

**Advantage**: Solves lifetime issues, defers to proper context

### Pattern 2: RenderRegion + ViewportState Coordination

**Current Flow**:
```
egui::Rect (panel bounds in points)
    ↓ [compute_viewport_region]
RenderRegion (GPU pixels)
    ↓ [apply_to_pass]
wgpu::RenderPass (hardware clipping)
```

**Not Connected To**:
- Lua UI rendering
- UI command system

### Pattern 3: Viewport Aware Camera

**Current Flow**:
```
EditorCameraController.viewport_rect ← ViewportState.rect()
    ↓
Aspect ratio calculation (line 36-38 in camera_system.rs)
    ↓
Ray-casting from UV (line 40)
```

**Not Available To**:
- Lua UI scripts
- UI command rendering

---

## 8. RENDERING PIPELINE INTEGRATION POINTS

### Where RenderRegion is Applied

1. **Frame Graph Main Pass** (frame_graph.rs)
   ```rust
   if let Some(region) = region {
       region.apply_to_pass(&mut pass);
   }
   ```

2. **Render Context Passes** (render_context.rs)
   - Custom render hooks support optional RenderRegion

3. **Post-Processing** (postprocess/mod.rs)
   - Applies region to effects passes

### egui Rendering Path

**Location**: `/home/user/webgpu-testing/src/ui/egui_integration.rs:123-185`

```rust
pub fn render(&mut self, target: &mut EguiRenderTarget<'_>, output: egui::FullOutput) {
    // 1. Tessellate UI shapes
    let primitives = self.ctx.tessellate(output.shapes, output.pixels_per_point);
    
    // 2. Update GPU buffers
    self.renderer.update_buffers(target.device, target.queue, target.encoder, &primitives, &screen_descriptor);
    
    // 3. Begin render pass with LOADOp (not CLEAR)
    let pass = target.encoder.begin_render_pass(&wgpu::RenderPassDescriptor { ... });
    
    // 4. Render egui with the pass
    self.renderer.render(&mut pass_static, &primitives, &screen_descriptor);
}
```

**Key**: Uses `LoadOp::Load` - composites on top of existing content

---

## 9. SCRIPT ANNOTATION SYSTEM

### Script Modes

**Location**: `/home/user/webgpu-testing/src/scripting/lua/api/ui/mod.rs:19-23`

```lua
-- @editor
  -- Only runs in editor mode, has on_ui callback
  
-- @tool
  -- Runs in both editor and play modes (same as @editor currently)
  
-- (no annotation)
  -- Runs in play mode only, no UI access
```

### Lua API Entry Points

```lua
function on_created(self_entity)
    -- Called once when script is loaded
end

function on_update(self_entity, dt)
    -- Called each frame (all modes)
end

function on_ui(self_entity, ui)
    -- Called each frame (editor/@tool only)
    -- UI parameter is UiContext userdata
end
```

---

## 10. CURRENT STATE SUMMARY

### What Works

- ✅ Complete UI widget set (buttons, sliders, text, etc.)
- ✅ Command recording deferred pattern
- ✅ Response feedback to Lua
- ✅ Basic layout (vertical, horizontal, centered)
- ✅ Menu bar and menus
- ✅ Editor integration (renders in UI panels)

### What's Missing for Viewport Constraints

- ❌ No RenderRegion for Lua UI
- ❌ No viewport-constrained rendering
- ❌ No viewport dimensions exposed to Lua
- ❌ No screen-space coordinate system
- ❌ No camera/view matrix access
- ❌ No world-to-screen conversion
- ❌ No anchoring/alignment system beyond centered_area

### Existing Foundation

- ✅ RenderRegion system in renderer
- ✅ ViewportState + ViewportSystem in editor
- ✅ Viewport region computation already done
- ✅ editor camera knows viewport bounds
- ✅ egui has pixel-level positioning

---

## 11. RECOMMENDATIONS FOR VIEWPORT-CONSTRAINED UI

### Minimal Implementation Path

1. **Expose Viewport Info to Lua**
   - Add `ui:get_viewport_rect()` → {x, y, width, height}
   - Add `ui:get_pixels_per_point()` → f32
   
2. **Add Absolute Positioning**
   - `ui:absolute_panel(x, y, width, height, callback)` 
   - Uses viewport-relative coordinates
   
3. **Connect to RenderRegion** (for game view overlay)
   - Pass RenderRegion to UI command rendering
   - Apply scissor rect to egui render pass
   
### Full Implementation Path

1. Add viewport-aware layout system
2. Implement anchoring (top-left, center, etc.)
3. Add constraint solving
4. Expose camera for world-to-screen projection
5. Support nested UI hierarchies

---

## 12. DATA FLOW DIAGRAM

```
Lua Script (on_ui callback)
    ↓
UiContext::button("text") 
    ↓
Push UiCommand::Button to commands vector
    ↓
Return response from previous frame (or default)
    ↓
[Script continues, builds command tree]
    ↓
[Script returns]
    ↓
take_commands() - drain command vector
    ↓
UiCommand::render_and_collect(egui::Ui)
    ↓
[egui renders Button immediately]
    ↓
Collect UiResponse (clicked, etc.)
    ↓
set_responses() - store for next frame
    ↓
egui renders to surface with LoadOp::Load
    ↓
[Next frame: responses fed back to Lua]
```

---

## CONCLUSION

The Lua UI system is **editor-integrated only** with **no current support for viewport-constrained rendering**. The game viewport uses a separate `RenderRegion` system that could be integrated with the UI system to enable:

- Viewport-constrained UI overlays
- Game-view-bound UI panels
- Multiple independent viewport regions

The architectural foundation is solid (command recording pattern, RenderRegion system), but **there is no bridge between them for Lua UI rendering**.
