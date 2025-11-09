# Lua UI System Architecture Diagram

## High-Level Component Relationships

```
┌─────────────────────────────────────────────────────────────────────┐
│                      EDITOR APPLICATION                              │
│  /src/bin/editor/application/core.rs                                │
├─────────────────────────────────────────────────────────────────────┤
│                                                                       │
│  EditorSharedState                                                    │
│  ├─ ViewportSystem                                                    │
│  │  ├─ scene_viewport: ViewportState                                 │
│  │  │  └─ region: Option<RenderRegion>    (GPU pixels)              │
│  │  │  └─ rect: Option<egui::Rect>        (egui points)             │
│  │  └─ game_viewport: ViewportState                                  │
│  ├─ ui_plugin_manager: UiPluginManager                               │
│  ├─ script_ui_commands: HashMap<Entity, Vec<UiCommand>>              │
│  └─ script_ui_responses: HashMap<Entity, HashMap<String, UiResponse>>│
│                                                                       │
└─────────────────────────────────────────────────────────────────────┘
                                   │
                  ┌────────────────┼────────────────┐
                  │                │                │
                  v                v                v
        ┌──────────────────┐  ┌──────────────┐  ┌─────────────────┐
        │  CAMERA SYSTEM   │  │ LAYOUT/VIEWPORT  │  RENDERING      │
        ├──────────────────┤  ├──────────────┤  └─────────────────┘
        │ EditorCamera     │  │ show_viewport()  │
        │ Controller       │  │ compute_viewport │  RenderRegion
        │                  │  │ _region()        │  (scissor rect
        │ viewport_rect ◄──────► rect()          │   + viewport)
        │ (egui::Rect)     │  │                  │
        └──────────────────┘  └──────────────┘
                  │
                  │ Aspect ratio for ray-cast
                  │
                  v
        ┌──────────────────┐
        │  RAY CASTING     │
        │  (gizmo picking) │
        └──────────────────┘


```

## Lua UI Flow (Command Recording Pattern)

```
┌────────────────────────────────────────────────────────────────────────┐
│                        LUA SCRIPT EXECUTION                             │
│                    (on_ui callback in Lua script)                      │
├────────────────────────────────────────────────────────────────────────┤
│                                                                         │
│  Script calls: ui:button("Click Me")                                   │
│        │                                                               │
│        ├─> UiContext::button(text: String) -> bool                    │
│        │   ├─ Push UiCommand::Button { text }                         │
│        │   │  (to commands: Arc<Mutex<Vec<UiCommand>>>)               │
│        │   │                                                           │
│        │   └─ Return: responses.get("button_Click Me").clicked        │
│        │      └─ First frame: false (no prior response)               │
│        │      └─ Later frames: actual value from previous frame        │
│        │                                                               │
│        └─ Script receives: true (if clicked last frame)                │
│                                                                         │
│  More widget calls accumulate commands...                              │
│                                                                         │
└────────────────────────────────────────────────────────────────────────┘
                                  │
                                  │ Script returns
                                  v
┌────────────────────────────────────────────────────────────────────────┐
│                      COMMAND PLAYBACK                                   │
│          (After Lua VM completes, in EditorApplication)               │
├────────────────────────────────────────────────────────────────────────┤
│                                                                         │
│  take_commands() -> Vec<UiCommand>                                     │
│        │                                                               │
│        └─> UiCommand::render_and_collect(&mut egui::Ui)               │
│            └─> egui::Ui::button("Click Me")                           │
│                ├─ Renders button immediately to egui                  │
│                ├─ Returns egui::Response                               │
│                │                                                       │
│                └─> Convert to UiResponse                               │
│                    {                                                   │
│                      clicked: response.clicked(),                      │
│                      hovered: response.hovered(),                      │
│                      changed: response.changed(),                      │
│                      ...                                               │
│                    }                                                   │
│                                                                         │
│  Collect all responses into HashMap<String, UiResponse>                │
│                                                                         │
└────────────────────────────────────────────────────────────────────────┘
                                  │
                                  │ set_responses(responses)
                                  v
┌────────────────────────────────────────────────────────────────────────┐
│                     RESPONSE STORAGE                                    │
│          (Stored in Arc<Mutex<HashMap>> for next frame)               │
├────────────────────────────────────────────────────────────────────────┤
│                                                                         │
│  responses: Arc<Mutex<HashMap<String, UiResponse>>>                   │
│                                                                         │
└────────────────────────────────────────────────────────────────────────┘
                                  │
                                  │ Next frame
                                  v
┌────────────────────────────────────────────────────────────────────────┐
│                      LUA SCRIPT EXECUTION (NEXT FRAME)                 │
│                                                                         │
│  ui:button("Click Me")                                                 │
│      └─ Returns: responses.get("button_Click Me").clicked              │
│         └─ Returns the value from previous frame's response!           │
│                                                                         │
└────────────────────────────────────────────────────────────────────────┘

```

## Viewport Rendering System (Disconnected from Lua UI)

```
┌────────────────────────────────────────────────────────────────────────┐
│                         EDITOR LAYOUT (egui-tiles)                     │
│               /src/bin/editor/layout.rs                                │
├────────────────────────────────────────────────────────────────────────┤
│                                                                         │
│  show_viewport(ui, viewport_state, "Game View", ...)                  │
│      │                                                                 │
│      ├─> ui.allocate_exact_size()  [egui panel bounds in points]      │
│      │                                                                 │
│      ├─> compute_viewport_region(egui_ctx, rect)                      │
│      │   │                                                             │
│      │   ├─> pixels_per_point: f32                                    │
│      │   ├─> egui::Rect -> RenderRegion (DPI scaled)                  │
│      │   │                                                             │
│      │   └─> RenderRegion::new(x, y, width, height)?                  │
│      │       └─> Clamp to screen bounds                                │
│      │                                                                 │
│      └─> viewport_state.set(rect, region)                             │
│                                                                         │
│  ViewportState                                                          │
│  ├─ rect: Option<egui::Rect>      [logical, in points]                │
│  └─ region: Option<RenderRegion>  [physical, in GPU pixels]           │
│                                                                         │
└────────────────────────────────────────────────────────────────────────┘
                                  │
                    ┌─────────────┴──────────────┐
                    │                            │
                    v                            v
         ┌──────────────────────┐    ┌──────────────────────┐
         │  GAME VIEWPORT       │    │  CAMERA SYSTEM       │
         │  Rendering           │    │                      │
         ├──────────────────────┤    ├──────────────────────┤
         │ RenderPass::render   │    │ viewport_rect        │
         │   for viewport       │    │ (aspect ratio calc)  │
         │                      │    │                      │
         │ region.apply_to_pass │    │ Ray casting from UV  │
         │   set_viewport()     │    │ for gizmo picking    │
         │   set_scissor_rect() │    │                      │
         │                      │    │ NOT exposed to       │
         │ Clips GPU rendering  │    │ Lua UI               │
         │ to viewport bounds   │    │                      │
         └──────────────────────┘    └──────────────────────┘

```

## egui Rendering Path (Where Lua UI Actually Renders)

```
┌────────────────────────────────────────────────────────────────────────┐
│                    egui_integration.rs::EguiContext                    │
├────────────────────────────────────────────────────────────────────────┤
│                                                                         │
│  run_ui() callback                                                      │
│    └─> EditorApplication::show_menu_bar(ctx)                           │
│    └─> EditorApplication::show_panels(ctx)  [Layout/viewports]        │
│    └─> UiPluginManager::render(ctx)         [Lua UI scripts]          │
│                                                                         │
│  Lua scripts in plugin manager:                                         │
│    ├─ welcome_screen.lua                                               │
│    ├─ script_editor_plugin.lua                                         │
│    └─ [other @editor scripts]                                          │
│                                                                         │
│  Each script:                                                           │
│    ├─ get_ui_commands() -> Vec<UiCommand>                              │
│    │                                                                   │
│    ├─> render_and_collect(ui, responses)                               │
│    │   └─> Egui renders immediately to egui::Context                   │
│    │                                                                   │
│    └─ set_ui_responses(responses)                                      │
│                                                                         │
│  egui::Context accumulates all shapes/textures                          │
│                                                                         │
└────────────────────────────────────────────────────────────────────────┘
                                  │
                                  │ end_frame()
                                  v
┌────────────────────────────────────────────────────────────────────────┐
│                      EGUI TESSELLATION & RENDERING                     │
│                   egui_integration.rs::render()                        │
├────────────────────────────────────────────────────────────────────────┤
│                                                                         │
│  1. ctx.tessellate(shapes) -> Vec<egui::ClippedPrimitive>              │
│                                                                         │
│  2. renderer.update_buffers() -> Upload to GPU                         │
│                                                                         │
│  3. begin_render_pass()                                                 │
│     LoadOp::Load (compose on existing surface)                         │
│                                                                         │
│  4. renderer.render()                                                   │
│     └─> Draw egui primitives to surface                                │
│                                                                         │
│  ** NO RenderRegion applied here **                                    │
│  ** UI renders full screen (or panel bounds via egui clipping) **      │
│                                                                         │
└────────────────────────────────────────────────────────────────────────┘

```

## File Dependency Graph

```
Scripts/Lua API
  ├─ /src/scripting/lua/api/ui/mod.rs          [Register UI API]
  ├─ /src/scripting/lua/api/ui/context.rs      [UiContext - command recording]
  └─ /src/scripting/lua/api/ui/commands.rs     [UiCommand enum & render]
       │
       └─> Uses: Arc<Mutex> for Send support across Lua VM

Editor Application
  ├─ /src/bin/editor/application/core.rs       [EditorApplication & ViewportSystem]
  ├─ /src/bin/editor/layout.rs                 [Editor layout & viewport computation]
  ├─ /src/bin/editor/application/ui.rs         [Menu bar & panels]
  ├─ /src/bin/editor/application/ui_plugin_manager.rs [Plugin loading]
  └─ /src/bin/editor/application/camera_system.rs    [Camera with viewport awareness]

Rendering System
  ├─ /src/ui/egui_integration.rs               [egui + wgpu integration]
  ├─ /src/renderer/render_region.rs            [RenderRegion viewport constraint]
  ├─ /src/renderer/frame_graph.rs              [Main render pipeline]
  ├─ /src/renderer/render_context.rs           [Custom render hooks]
  └─ /src/renderer/postprocess/mod.rs          [Post-process with viewport]

Camera System
  └─ /src/bin/editor/camera.rs                 [EditorCameraController]
       └─ viewport_rect: Option<egui::Rect>    [Set by camera_system.rs]

Layout & Viewport
  └─ /src/bin/editor/layout.rs                 [ViewportState & compute_viewport_region]
       └─ RenderRegion + egui::Rect coordination

Example Scripts
  ├─ /examples/scripts/welcome_screen.lua      [Full editor UI plugin]
  ├─ /scripts/ui_example_comprehensive.lua     [Widget showcase]
  └─ /examples/scripts/test_minimal_ui.lua     [Minimal test]

```

## Key Data Structures and Types

```
┌─────────────────────────────────────────────────────────────┐
│  RenderRegion (GPU pixel coordinates)                       │
├─────────────────────────────────────────────────────────────┤
│  x: u32          [left edge in pixels]                      │
│  y: u32          [top edge in pixels]                       │
│  width: u32      [width in pixels]                          │
│  height: u32     [height in pixels]                         │
│                                                             │
│  apply_to_pass(&mut RenderPass)                             │
│    ├─> set_viewport(x, y, width, height, 0.0, 1.0)        │
│    └─> set_scissor_rect(x, y, width, height)               │
└─────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────┐
│  ViewportState (egui + GPU coordination)                    │
├─────────────────────────────────────────────────────────────┤
│  region: Option<RenderRegion>  [GPU pixels from above]      │
│  rect: Option<egui::Rect>      [egui points (DPI aware)]    │
│                                                             │
│  Both populated together in compute_viewport_region()       │
└─────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────┐
│  UiCommand (Lua UI action)                                  │
├─────────────────────────────────────────────────────────────┤
│  Button { text }                                            │
│  Slider { id, current_value, min, max }                     │
│  TextEdit { id, current_value }                             │
│  ... (see enum in commands.rs)                              │
│                                                             │
│  render_and_collect(&mut egui::Ui)                          │
│    └─> Calls egui methods, collects responses               │
└─────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────┐
│  UiResponse (Response from egui widget)                     │
├─────────────────────────────────────────────────────────────┤
│  clicked: bool                                              │
│  hovered: bool                                              │
│  changed: bool                                              │
│  text_value: Option<String>                                 │
│  float_value: Option<f64>                                   │
│  bool_value: Option<bool>                                   │
│  color_value: Option<(f32, f32, f32)>                       │
└─────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────┐
│  UiContext (Lua userdata - the 'ui' parameter)              │
├─────────────────────────────────────────────────────────────┤
│  commands: Arc<Mutex<Vec<UiCommand>>>                       │
│  responses: Arc<Mutex<HashMap<String, UiResponse>>>         │
│                                                             │
│  Methods called from Lua:                                   │
│  ├─ label(text)                                             │
│  ├─ button(text) -> bool                                    │
│  ├─ text_edit(id, value) -> String                          │
│  ├─ slider(id, value, min, max) -> f64                      │
│  └─ ... (all widget types)                                  │
│                                                             │
│  take_commands() -> Vec<UiCommand>  [For rendering]         │
│  set_responses(HashMap)             [From responses]        │
└─────────────────────────────────────────────────────────────┘

```

## Critical Gap: No Viewport Connection for Lua UI

```
DESIRED STATE (Not Implemented)
┌──────────────────────────────────────┐
│  RenderRegion from ViewportState     │
│  (game_viewport.region())            │
└──────────────────┬───────────────────┘
                   │
                   v
         ┌────────────────────┐
         │ Pass to UI Renderer │
         └────────┬───────────┘
                  │
                  v
    ┌─────────────────────────────┐
    │ UiCommand::render_and_collect│
    │ + RenderRegion::apply_to_pass│
    └──────────────┬──────────────┘
                   │
                   v
         ┌────────────────────┐
         │ Lua UI constrained │
         │ to game viewport   │
         └────────────────────┘


CURRENT STATE (What Actually Happens)
┌──────────────────────────────────────┐
│  RenderRegion from ViewportState     │
│  (game_viewport.region())            │
└──────────────┬───────────────────────┘
               │
               ├─> Used for game rendering only
               │   (NOT connected to UI)
               │
               v
      ┌────────────────────┐
      │ Game view renders  │
      │ clipped           │
      └────────────────────┘

┌──────────────────────────────────────┐
│  UiCommand (from Lua scripts)        │
└──────────────┬───────────────────────┘
               │
               v
     ┌────────────────────┐
     │ render_and_collect │
     │ NO viewport        │
     │ constraint         │
     └────────┬───────────┘
              │
              v
       ┌──────────────────┐
       │ Lua UI renders   │
       │ full panel size  │
       │ (overlaps game)  │
       └──────────────────┘

```

---

## Summary

The system has **two parallel viewport/rendering systems**:

1. **Game Viewport (RenderRegion-based)**
   - Constrains GPU rendering to viewport bounds
   - Aware of camera, aspect ratio, DPI scaling
   - NOT connected to Lua UI

2. **Lua UI (egui-based)**
   - Command recording pattern
   - Renders within editor panels
   - NO RenderRegion support
   - NO viewport constraint capability

To enable viewport-constrained Lua UI rendering, these two systems need to be **bridged**.

