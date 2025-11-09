# In-Game UI Viewport Anchoring Plan

## Problem Statement

**Current Issue**: Lua-created UI is completely detached from the game viewport, rendering in editor panels without any spatial relationship to the game view. This makes it impossible to create proper in-game UI overlays (HUD, health bars, menus, etc.).

**Why This Matters**:
- Cannot create HUD elements anchored to viewport corners/edges
- Cannot overlay UI on the game view
- No coordinate system relating UI to the game space
- UI has no awareness of viewport bounds or aspect ratio

## Current Architecture Analysis

### What Exists
✅ **RenderRegion System** - GPU-level viewport constraints (scissor rect + viewport)
✅ **ViewportState** - Tracks both egui rect (points) and RenderRegion (pixels)
✅ **Command Recording Pattern** - Deferred UI rendering solving Lua/egui lifetime issues
✅ **egui Integration** - Complete widget set with compositing support (LoadOp::Load)
✅ **Viewport Computation** - DPI-aware conversion from egui rect to GPU pixels

### What's Missing
❌ **No Bridge** between RenderRegion and Lua UI rendering
❌ **No Anchoring System** - Cannot position UI relative to viewport edges/corners
❌ **No Viewport Info in Lua** - Scripts don't know viewport dimensions
❌ **No Coordinate System** - No way to specify "10px from top-right corner"
❌ **No Layout Constraints** - Cannot specify relative positioning

---

## Solution: Two-Phase Implementation

We need **BOTH** systems, implemented in phases:

### Phase 1: Anchoring System (MVP)
**Goal**: Enable basic in-game UI positioning relative to viewport

Think of this like Unity's UI anchors or CSS `position: absolute` with anchors:
- Position UI elements relative to viewport corners/edges/center
- Fixed pixel offsets from anchor points
- Simple, predictable, game-focused

**Use Cases**:
- Health bar in top-left corner
- Score display in top-right
- Crosshair in center
- Button 20px from bottom-right

### Phase 2: Flex Layout System (Advanced)
**Goal**: Enable responsive, constraint-based UI layout

Think of this like CSS Flexbox or Unity's layout groups:
- Automatic sizing and spacing
- Responsive to viewport changes
- Nested containers with flow direction
- Min/max sizes, padding, gaps

**Use Cases**:
- Inventory grids that adapt to screen size
- Menu lists that scroll when needed
- Toolbars that reflow on small screens
- Dialogs that center and clamp size

---

## Phase 1 Implementation: Anchoring System

### 1.1 Design: Anchor Points

```rust
// New type in commands.rs
pub enum Anchor {
    TopLeft,
    TopCenter,
    TopRight,
    CenterLeft,
    Center,
    CenterRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

pub enum UiCommand {
    // ... existing commands ...

    // NEW: Anchored panel
    AnchoredPanel {
        anchor: Anchor,
        offset_x: f32,        // Pixels from anchor point
        offset_y: f32,        // Pixels from anchor point
        width: Option<f32>,   // None = auto-size
        height: Option<f32>,  // None = auto-size
        items: Vec<UiCommand>,
    },
}
```

### 1.2 Lua API

```lua
-- @tool  (runs in both editor and play modes)

function on_ui(self_entity, ui)
    -- Health bar in top-left
    ui:anchored_panel("top_left", 10, 10, 200, 50, function(panel)
        panel:label("Health: " .. health)
        panel:progressbar("health", health, 0, 100)
    end)

    -- Score in top-right
    ui:anchored_panel("top_right", -10, 10, nil, nil, function(panel)
        panel:label("Score: " .. score)
    end)

    -- Centered pause menu
    ui:anchored_panel("center", 0, 0, 400, 300, function(panel)
        if panel:button("Resume") then
            resume_game()
        end
        if panel:button("Quit") then
            quit_game()
        end
    end)
end
```

### 1.3 Rendering Strategy

**Option A: egui Area Widget** (Recommended)
Use egui's built-in `Area` widget which supports absolute positioning:

```rust
// In commands.rs render_and_collect()
UiCommand::AnchoredPanel { anchor, offset_x, offset_y, width, height, items } => {
    // 1. Get viewport dimensions from context
    let viewport_rect = ui.ctx().available_rect(); // or pass from RenderRegion

    // 2. Calculate anchor position
    let anchor_pos = match anchor {
        Anchor::TopLeft => viewport_rect.min,
        Anchor::TopRight => viewport_rect.right_top(),
        Anchor::Center => viewport_rect.center(),
        // ... other anchors
    };

    // 3. Apply offsets (with right/bottom anchors, negative offset moves inward)
    let final_pos = anchor_pos + egui::vec2(offset_x, offset_y);

    // 4. Create Area at position
    let area = egui::Area::new("anchored_panel")
        .fixed_pos(final_pos)
        .constrain(true); // Keep within viewport

    // 5. Optionally set size
    area.show(ui.ctx(), |ui| {
        if let Some(w) = width {
            ui.set_width(w);
        }
        if let Some(h) = height {
            ui.set_height(h);
        }

        // Render child commands
        for item in items {
            item.render_and_collect(ui);
        }
    });
}
```

**Option B: Custom egui::Ui Region** (More control)
Create a child `Ui` with custom clip rect:

```rust
// Manually construct Ui with specific rect
let child_rect = egui::Rect::from_min_size(final_pos, size);
let mut child_ui = ui.child_ui(child_rect, egui::Layout::top_down(egui::Align::Min));

// Apply RenderRegion scissor rect (if needed)
// This would require deeper integration with the render pass
```

### 1.4 Viewport Info Exposure

**Add to UiContext** (context.rs):

```rust
impl UiContext {
    // NEW: Get viewport dimensions
    pub fn get_viewport_size(&self) -> (f32, f32) {
        // This needs to be passed from ViewportState
        // Store in UiContext during creation
        (self.viewport_width, self.viewport_height)
    }

    pub fn get_pixels_per_point(&self) -> f32 {
        self.pixels_per_point
    }
}
```

**Lua API**:
```lua
local width, height = ui:get_viewport_size()
local dpi_scale = ui:get_pixels_per_point()
```

### 1.5 Integration Points

**Changes needed**:

1. **UiContext Creation** (application/ui_plugin_manager.rs or wherever UI scripts run)
   - Pass viewport dimensions from `ViewportState`
   - Store in `UiContext` for Lua access

2. **RenderRegion Application** (NEW)
   - When rendering UI commands for `@tool` scripts
   - Render within game viewport in play mode
   - Render as overlay in editor mode (for WYSIWYG)
   - Ensures UI is clipped to game view

3. **Script Mode Detection** (lua/api/ui/mod.rs)
   - Use existing `@tool` annotation (runs in both editor and play)
   - `@tool` scripts render anchored UI within game viewport
   - In editor: show with WYSIWYG editing capabilities
   - In play: show as actual in-game UI

**Example Flow**:
```rust
// In EditorApplication::render() or similar
for (entity, script) in tool_ui_scripts {
    // Get game viewport region
    let viewport_region = self.viewport_system.game_viewport.region();
    let viewport_rect = self.viewport_system.game_viewport.rect();

    // Create UiContext with viewport info
    let ui_ctx = UiContext::new_with_viewport(
        viewport_rect.width(),
        viewport_rect.height(),
        egui_ctx.pixels_per_point(),
        self.is_editor_mode // For WYSIWYG features
    );

    // Execute script: on_ui(entity, ui_ctx)
    // ... collect commands ...

    // Render commands in game viewport area
    egui::Area::new("game_ui_overlay")
        .fixed_pos(viewport_rect.min)
        .show(egui_ctx, |ui| {
            // Apply RenderRegion clipping (if supported by egui)
            // Or rely on egui's clipping

            for cmd in commands {
                cmd.render_and_collect(ui);
            }
        });
}
```

### 1.6 Anchor Offset Semantics

**Positive offsets** move away from edge:
- TopLeft: +X right, +Y down
- TopRight: +X right (away from edge = left), +Y down
- BottomRight: +X right (left), +Y up (up from edge)

**Better: Sign-agnostic offsets**:
- `offset_x`, `offset_y` always measure **inward** from anchor
- TopRight with offset (10, 10) = "10px left of right edge, 10px down from top"
- BottomLeft with offset (10, 10) = "10px right of left edge, 10px up from bottom"

```rust
let final_pos = match anchor {
    Anchor::TopLeft => viewport_rect.min + vec2(offset_x, offset_y),
    Anchor::TopRight => viewport_rect.right_top() + vec2(-offset_x, offset_y),
    Anchor::BottomLeft => viewport_rect.left_bottom() + vec2(offset_x, -offset_y),
    Anchor::BottomRight => viewport_rect.right_bottom() + vec2(-offset_x, -offset_y),
    Anchor::Center => viewport_rect.center() + vec2(offset_x, offset_y),
    // ... etc
};
```

### 1.7 WYSIWYG Editing in Editor Mode

**Goal**: Allow visual positioning of anchored panels in the editor without editing Lua code.

**Design Principles**:
- Make anchored panels draggable in editor mode
- Show visual guides (anchor point, offset lines, measurements)
- Save positions back to entity properties or script state
- Toggle between "Design Mode" (draggable) and "Preview Mode" (interactive)

#### 1.7.1 Visual Guides

When rendering anchored panels in editor mode, add visual overlays:

```rust
// In AnchoredPanel::render_and_collect() - editor mode only
if is_editor_mode {
    // 1. Draw anchor point indicator
    ui.painter().circle_filled(
        anchor_pos,
        4.0,
        egui::Color32::from_rgb(255, 150, 0)
    );

    // 2. Draw offset lines
    ui.painter().line_segment(
        [anchor_pos, final_pos],
        egui::Stroke::new(1.0, egui::Color32::from_rgb(255, 150, 0))
    );

    // 3. Show measurements
    ui.painter().text(
        anchor_pos + vec2(5.0, 5.0),
        egui::Align2::LEFT_TOP,
        format!("offset: ({:.0}, {:.0})", offset_x, offset_y),
        egui::FontId::monospace(10.0),
        egui::Color32::WHITE
    );

    // 4. Highlight panel border
    ui.painter().rect_stroke(
        panel_rect,
        0.0,
        egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 150, 0))
    );
}
```

#### 1.7.2 Draggable Panels

Make panels interactable in design mode:

```rust
// Add ID to AnchoredPanel for tracking
pub struct AnchoredPanel {
    id: String,           // Unique ID for this panel
    anchor: Anchor,
    offset_x: f32,
    offset_y: f32,
    // ... rest
}

// In render_and_collect()
if is_design_mode {
    // Create invisible sense region for dragging
    let sense_response = ui.allocate_rect(panel_rect, egui::Sense::drag());

    if sense_response.dragged() {
        let delta = sense_response.drag_delta();

        // Convert delta to new offsets based on anchor
        let (new_offset_x, new_offset_y) = calculate_new_offsets(
            anchor,
            offset_x,
            offset_y,
            delta
        );

        // Emit event or update command state
        emit_panel_moved_event(id, new_offset_x, new_offset_y);
    }

    // Visual feedback while dragging
    if sense_response.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    } else if sense_response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
}
```

#### 1.7.3 Saving Edited Positions

**Option A: Save to Entity Components** (Recommended)
Store anchor positions as entity components that override script defaults:

```rust
// New component
#[derive(Component)]
pub struct UiPanelOverride {
    pub panel_id: String,
    pub anchor: Anchor,
    pub offset_x: f32,
    pub offset_y: f32,
}

// When creating UiContext for script
let overrides = entity.get::<UiPanelOverride>();
ui_ctx.set_panel_overrides(overrides);

// In Lua script
ui:anchored_panel("health_bar", "top_left", 10, 10, 200, 50, function(panel)
    -- Default position (10, 10) from top_left
    -- Editor override can change this without touching script
end)
```

**Option B: Save to Script Properties**
Store in entity's script component properties (if that exists):

```rust
// In script component
pub struct ScriptComponent {
    // ... existing fields ...
    pub ui_panel_positions: HashMap<String, (Anchor, f32, f32)>,
}
```

**Option C: Save to Separate UI Layout File**
Store UI layouts separately from scripts:

```lua
-- ui_layouts/hud_layout.lua
return {
    health_bar = { anchor = "top_left", offset_x = 15, offset_y = 12 },
    score = { anchor = "top_right", offset_x = 15, offset_y = 12 },
}
```

#### 1.7.4 Design Mode Toggle

Add UI controls in editor for design mode:

```rust
// In editor menu bar or toolbar
if ui.button("🎨 Design Mode").clicked() {
    self.ui_design_mode = !self.ui_design_mode;
}

// Pass to UiContext
ui_ctx.set_design_mode(self.ui_design_mode);

// Visual indicator
if self.ui_design_mode {
    // Show overlay in game viewport
    egui::Area::new("design_mode_indicator")
        .fixed_pos(viewport_rect.left_top() + vec2(5.0, 5.0))
        .show(ctx, |ui| {
            ui.colored_label(
                egui::Color32::from_rgb(255, 150, 0),
                "🎨 DESIGN MODE - Drag panels to reposition"
            );
        });
}
```

#### 1.7.5 Anchor Snapping

Add snap-to-anchor functionality for easier positioning:

```rust
// When dragging, allow switching anchor points
if sense_response.drag_released() {
    // Check if panel center is closer to a different anchor
    let panel_center = panel_rect.center();
    let new_anchor = find_closest_anchor(viewport_rect, panel_center);

    if new_anchor != current_anchor {
        // Recalculate offsets for new anchor
        let new_offsets = calculate_offsets_from_position(
            new_anchor,
            viewport_rect,
            panel_center
        );

        emit_panel_anchor_changed(id, new_anchor, new_offsets);
    }
}

// Visual feedback: show all anchor points
if is_design_mode {
    for anchor in Anchor::ALL {
        let pos = get_anchor_position(viewport_rect, anchor);
        ui.painter().circle_stroke(
            pos,
            8.0,
            egui::Stroke::new(1.0, egui::Color32::from_rgb(100, 100, 100))
        );
    }
}
```

#### 1.7.6 WYSIWYG Workflow

**Typical usage**:

1. **Create script** with anchored panels using default positions
   ```lua
   -- @tool
   function on_ui(self_entity, ui)
       ui:anchored_panel("health_bar", "top_left", 10, 10, 200, 50, function(panel)
           panel:label("Health: " .. get_health())
       end)
   end
   ```

2. **Enable Design Mode** in editor (button in toolbar)

3. **Drag panels** to desired positions
   - Visual guides show anchor point and offsets
   - All 9 anchor points visible for reference
   - Measurements update in real-time

4. **Release panel** to commit position
   - Offsets automatically calculated
   - Saved to entity component (overrides script defaults)

5. **Toggle Preview Mode** to test interactivity
   - Buttons/sliders work normally
   - No dragging or visual guides

6. **Play mode** uses exact positions from editor
   - Overrides applied to anchored panels
   - Positions consistent between editor and play

### 1.8 Phase 1 Deliverables

**Core Features**:
- ✅ `Anchor` enum with 9 positions
- ✅ `AnchoredPanel` command
- ✅ Lua API: `ui:anchored_panel(id, anchor, x, y, w, h, callback)`
- ✅ Viewport dimensions exposed to Lua
- ✅ `@tool` script support (runs in editor + play)
- ✅ Rendering integration with game viewport

**WYSIWYG Features**:
- ✅ Visual guides (anchor points, offset lines, measurements)
- ✅ Draggable panels in design mode
- ✅ Design mode toggle in editor
- ✅ Position overrides saved to entity components
- ✅ Anchor snapping (optional but recommended)
- ✅ Real-time offset display

**Examples**:
- ✅ Example script: HUD with health/score/crosshair
- ✅ WYSIWYG tutorial video/documentation

---

## Phase 2 Implementation: Flex Layout System

### 2.1 Design: Layout Containers

```rust
pub enum LayoutDirection {
    Horizontal,
    Vertical,
}

pub enum Align {
    Start,    // Left/Top
    Center,
    End,      // Right/Bottom
    Stretch,  // Fill available space
}

pub enum Justify {
    Start,
    Center,
    End,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

pub struct LayoutConstraints {
    min_width: Option<f32>,
    max_width: Option<f32>,
    min_height: Option<f32>,
    max_height: Option<f32>,
    padding: f32,
    gap: f32,  // Spacing between children
}

pub enum UiCommand {
    // ... existing + anchored panel ...

    FlexContainer {
        direction: LayoutDirection,
        align: Align,
        justify: Justify,
        constraints: LayoutConstraints,
        items: Vec<UiCommand>,
    },

    FlexItem {
        grow: f32,     // Flex grow factor (0 = fixed size)
        shrink: f32,   // Flex shrink factor
        basis: Option<f32>,  // Initial size before grow/shrink
        item: Box<UiCommand>,
    },
}
```

### 2.2 Lua API

```lua
-- Inventory grid with auto-layout
ui:anchored_panel("bottom_right", 20, 20, 400, 300, function(panel)
    panel:flex_container("vertical", "stretch", "start", function(container)
        container:gap(10)
        container:padding(15)

        -- Header (fixed size)
        container:flex_item(0, 0, 40, function(item)
            item:heading("Inventory")
        end)

        -- Scrollable item grid (grows to fill space)
        container:flex_item(1, 0, nil, function(item)
            item:flex_container("horizontal", "start", "start", function(grid)
                grid:gap(5)
                grid:wrap(true)  -- Wrap to next row

                for i, item in ipairs(inventory_items) do
                    grid:item_slot(item)
                end
            end)
        end)

        -- Button bar (fixed size)
        container:flex_item(0, 0, 50, function(item)
            item:flex_container("horizontal", "center", "space_evenly", function(buttons)
                if buttons:button("Drop") then drop_item() end
                if buttons:button("Use") then use_item() end
                if buttons:button("Close") then close_inventory() end
            end)
        end)
    end)
end)
```

### 2.3 Layout Algorithm

**Two-pass layout** (like CSS Flexbox):

1. **Pass 1: Measure**
   - Calculate intrinsic sizes of all children
   - Determine available space
   - Calculate flex basis for each item

2. **Pass 2: Position**
   - Apply grow/shrink factors
   - Distribute remaining space
   - Calculate final positions
   - Apply alignment and justification

**Implementation**:
```rust
// In render_and_collect for FlexContainer
fn layout_flex(
    ui: &mut egui::Ui,
    direction: LayoutDirection,
    align: Align,
    justify: Justify,
    items: &[UiCommand],
) {
    let available_size = ui.available_size();

    // Pass 1: Measure children
    let mut child_sizes = Vec::new();
    for item in items {
        let size = measure_item(ui, item);
        child_sizes.push(size);
    }

    // Calculate total size and remaining space
    let total_size: f32 = child_sizes.iter().sum();
    let remaining_space = match direction {
        Horizontal => available_size.x - total_size,
        Vertical => available_size.y - total_size,
    };

    // Apply flex grow/shrink
    let final_sizes = apply_flex(child_sizes, items, remaining_space);

    // Pass 2: Position and render
    let mut cursor = ui.cursor().min;
    for (item, size) in items.iter().zip(final_sizes) {
        let rect = match direction {
            Horizontal => {
                let r = Rect::from_min_size(cursor, vec2(size, available_size.y));
                cursor.x += size + gap;
                r
            }
            Vertical => {
                let r = Rect::from_min_size(cursor, vec2(available_size.x, size));
                cursor.y += size + gap;
                r
            }
        };

        // Render child in calculated rect
        let mut child_ui = ui.child_ui(rect, Layout::default());
        item.render_and_collect(&mut child_ui);
    }
}
```

### 2.4 Phase 2 Deliverables

- ✅ `FlexContainer` and `FlexItem` commands
- ✅ Two-pass layout algorithm
- ✅ Lua API for flex layouts
- ✅ Wrapping support for grids
- ✅ Scrolling containers
- ✅ Min/max size constraints
- ✅ Padding and gap support
- ✅ Example: Responsive inventory UI

---

## Implementation Roadmap

### Milestone 1: Viewport Integration (1-2 days)
**Goal**: Connect RenderRegion to Lua UI system

- [ ] Pass viewport dimensions to UiContext creation
- [ ] Detect `@tool` scripts for in-game UI rendering
- [ ] Add `ui:get_viewport_size()` Lua API
- [ ] Add `ui:get_pixels_per_point()` Lua API
- [ ] Render `@tool` scripts within game viewport Area
- [ ] Test: Simple label in game viewport
- [ ] **Validation**: Lua script can query viewport size and UI appears in game view

### Milestone 2: Basic Anchoring (2-3 days)
**Goal**: Enable corner/edge positioning

- [ ] Add `Anchor` enum (9 positions)
- [ ] Add `AnchoredPanel` command with ID parameter
- [ ] Implement anchor position calculation
- [ ] Add `ui:anchored_panel(id, anchor, x, y, w, h, callback)` Lua API
- [ ] Test: HUD elements in all 9 positions
- [ ] **Validation**: Health bar in top-left, score in top-right, both stay in position

### Milestone 3: Offset and Sizing (1 day)
**Goal**: Precise positioning with offsets

- [ ] Implement offset calculation (inward from anchor)
- [ ] Add optional width/height sizing
- [ ] Handle auto-sizing (nil width/height)
- [ ] Add viewport edge clamping
- [ ] Test: UI at various offsets, ensure clamping works
- [ ] **Validation**: UI positioned 10px from edges, doesn't overflow viewport

### Milestone 4: WYSIWYG Visual Guides (2 days)
**Goal**: Show visual feedback for anchored panels in editor

- [ ] Add editor mode detection to UiContext
- [ ] Draw anchor point indicators (orange circles)
- [ ] Draw offset lines from anchor to panel
- [ ] Show offset measurements as text
- [ ] Highlight panel borders in design mode
- [ ] Test: Visual guides appear only in editor mode
- [ ] **Validation**: Can see anchor points and offset lines for all panels

### Milestone 5: WYSIWYG Dragging (2-3 days)
**Goal**: Make panels draggable in editor

- [ ] Add `UiPanelOverride` component for storing positions
- [ ] Implement drag sensing for anchored panels
- [ ] Calculate new offsets from drag delta
- [ ] Update component on drag release
- [ ] Apply overrides when creating UiContext
- [ ] Visual feedback (cursor changes, highlight while dragging)
- [ ] Test: Drag panels, verify positions persist
- [ ] **Validation**: Can drag panel to new position and it stays there

### Milestone 6: Design Mode Toggle (1 day)
**Goal**: Add UI controls for switching modes

- [ ] Add "Design Mode" button to editor toolbar
- [ ] Pass design mode flag to UiContext
- [ ] Show design mode indicator overlay
- [ ] Toggle between draggable and interactive modes
- [ ] Test: Design mode enables dragging, preview mode enables interaction
- [ ] **Validation**: Can toggle modes and behavior changes correctly

### Milestone 7: Anchor Snapping (1-2 days)
**Goal**: Auto-switch anchor when dragging to different area (Optional but recommended)

- [ ] Implement `find_closest_anchor()` for panel position
- [ ] Recalculate offsets when anchor changes
- [ ] Update override component with new anchor
- [ ] Show all 9 anchor points in design mode
- [ ] Visual feedback when anchor would change
- [ ] Test: Drag from top-left to bottom-right, anchor switches
- [ ] **Validation**: Panel stays in visual position when anchor switches

### Milestone 8: Example and Documentation (1 day)
**Goal**: Demonstrate complete anchoring + WYSIWYG system

- [ ] Create `examples/scripts/ingame_hud.lua`
  - Health bar (top-left)
  - Score (top-right)
  - Crosshair (center)
  - Mini-map (bottom-right)
- [ ] Write WYSIWYG workflow documentation
- [ ] Document anchoring API
- [ ] Create video/GIF of WYSIWYG editing
- [ ] **Validation**: Example runs, can be edited visually, works in play mode

**Phase 1 Complete**: ~11-14 days (with WYSIWYG)

---

### Milestone 9: Flex Container (3-4 days)
**Goal**: Basic flex layout

- [ ] Add `FlexContainer` and `FlexItem` commands
- [ ] Implement horizontal/vertical layout
- [ ] Add gap and padding support
- [ ] Add `ui:flex_container()` Lua API
- [ ] Test: Horizontal button bar, vertical list
- [ ] **Validation**: Buttons evenly spaced, list items stack vertically

### Milestone 10: Flex Sizing (2-3 days)
**Goal**: Grow/shrink and constraints

- [ ] Implement flex grow/shrink algorithm
- [ ] Add min/max size constraints
- [ ] Handle fixed vs flexible items
- [ ] Test: Mixed fixed and flexible items
- [ ] **Validation**: Flexible items expand to fill space, fixed items stay sized

### Milestone 11: Advanced Layout (2-3 days)
**Goal**: Alignment, justification, wrapping

- [ ] Implement align (start, center, end, stretch)
- [ ] Implement justify (space-between, space-around, etc.)
- [ ] Add wrapping for grid layouts
- [ ] Test: Complex layouts with multiple containers
- [ ] **Validation**: Items aligned and justified as specified

### Milestone 12: Example and Polish (2 days)
**Goal**: Production-ready flex layouts

- [ ] Create `examples/scripts/inventory_flex.lua`
- [ ] Create `examples/scripts/pause_menu_flex.lua`
- [ ] Performance testing with many items
- [ ] Document flex layout API
- [ ] **Validation**: Complex UIs render at 60fps, look professional

**Phase 2 Complete**: ~9-12 days

**Total Estimated Time**: ~20-26 days (Phase 1 + Phase 2 with WYSIWYG)

---

## Technical Considerations

### Performance

**Anchoring**:
- ✅ Minimal overhead (simple position calculation)
- ✅ No layout algorithm needed
- ✅ Scales to hundreds of anchored panels

**Flex Layout**:
- ⚠️ Two-pass layout can be expensive with deep nesting
- ⚠️ Consider caching layout results (if items don't change)
- ✅ egui's immediate mode helps (layout happens every frame anyway)

### egui Integration

**egui's Built-in Features We Can Use**:
- `egui::Area` - Absolute positioning (perfect for anchoring)
- `egui::Layout` - Directional layouts (use for flex direction)
- `ui.horizontal()` / `ui.vertical()` - Basic flex-like layout
- `ui.add_space()` - Gap implementation
- `ui.available_size()` - For flex calculations
- `ui.clip_rect()` - Viewport clipping

**Custom Extensions Needed**:
- Flex grow/shrink calculations (egui doesn't do this)
- Justify content (space-between, etc.)
- Explicit min/max constraints

### RenderRegion vs egui Clipping

**Two Clipping Systems**:

1. **RenderRegion (GPU-level)**:
   - Sets scissor rect on wgpu::RenderPass
   - Hard clip at GPU level
   - Used for game viewport rendering

2. **egui Clipping (Software-level)**:
   - Clips during tessellation
   - Set via `ui.set_clip_rect()`
   - Used for all egui widgets

**For Lua UI**:
- Use **egui clipping** (easier integration)
- Set clip rect to game viewport bounds
- egui will handle tessellation with clip

**Code**:
```rust
// When rendering @ingame UI
egui::Area::new("game_ui")
    .fixed_pos(viewport_rect.min)
    .show(ctx, |ui| {
        // Clip to viewport bounds
        ui.set_clip_rect(viewport_rect);

        // Render Lua UI commands
        for cmd in commands {
            cmd.render_and_collect(ui);
        }
    });
```

### Script Annotations

**Three Modes**:

1. **@editor** (existing)
   - Renders in editor panels only
   - Has access to all editor state
   - Not constrained to viewport
   - For editor tools and plugins

2. **@tool** (existing - now used for in-game UI)
   - Runs in both editor and play modes
   - Renders within game viewport (with anchoring)
   - In editor: WYSIWYG editing with visual guides
   - In play: Actual in-game UI (HUD, menus, overlays)

3. **(no annotation)** (existing)
   - Runs in play mode only
   - No UI access
   - Game logic only

### DPI Scaling

**Already Handled**:
- `ViewportState` stores both egui::Rect (points) and RenderRegion (pixels)
- `compute_viewport_region()` applies `pixels_per_point` scaling
- Anchor offsets should be in **logical pixels (points)**, egui handles DPI

**Lua API**:
```lua
-- Offsets in points (DPI-independent)
ui:anchored_panel("top_left", 10, 10, 200, 50, function(panel)
    -- 10 points = 10 pixels on 1x display, 20 pixels on 2x display
end)

-- For DPI-aware custom calculations
local scale = ui:get_pixels_per_point()
```

---

## Alternative Approaches Considered

### Alternative 1: World-Space UI (Not Recommended)

Position UI in 3D world space, project to screen:

**Pros**:
- UI can follow 3D objects (nameplates, health bars above enemies)
- Automatic perspective and depth

**Cons**:
- Complex camera matrix integration
- UI size varies with distance (usually undesired for HUD)
- More appropriate for in-world UI, not HUD

**Verdict**: Keep for Phase 3 (3D UI), not needed for HUD

### Alternative 2: CSS-like Absolute Positioning (Partial)

Use full CSS-style positioning with top/left/right/bottom:

```lua
ui:positioned_panel({
    position = "absolute",
    top = 10,
    right = 10,
    width = 200,
    -- If both left and right set, width is calculated
})
```

**Pros**:
- Familiar to web developers
- Very flexible

**Cons**:
- More complex API
- Anchors + offsets are simpler for game UI
- Can still add this later if needed

**Verdict**: Anchors first, CSS-style later if needed

### Alternative 3: Constraint Solver (Overkill)

Full constraint-based layout like iOS Auto Layout:

**Pros**:
- Ultimate flexibility
- Complex responsive layouts

**Cons**:
- Massive implementation effort
- Performance concerns
- Overkill for game UI

**Verdict**: Not needed for game development

---

## Success Criteria

### Phase 1 Success
- [ ] Lua script can create HUD with health bar in top-left corner
- [ ] HUD stays in correct position when window resizes
- [ ] Multiple anchored panels can coexist without overlap issues
- [ ] UI is clipped to game viewport (doesn't render outside)
- [ ] Performance: 60fps with 10+ anchored panels

### Phase 2 Success
- [ ] Lua script can create inventory with flex layout
- [ ] Inventory adapts to different viewport sizes
- [ ] Buttons in flex container have even spacing
- [ ] Nested flex containers work correctly
- [ ] Performance: 60fps with complex nested layouts (100+ items)

---

## Next Steps

1. **Review this plan** with team
2. **Decide on Phase 1 start date**
3. **Create issues/tasks** for each milestone
4. **Begin Milestone 1**: Viewport integration

Questions to resolve:
- Should `@ingame` scripts run in editor mode, play mode, or both?
- Do we want a preview mode to see in-game UI in editor viewport?
- Should anchored panels be draggable in editor for WYSIWYG positioning?
