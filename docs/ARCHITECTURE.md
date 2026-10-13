# Engine architecture

A map of the engine for people (and agents) new to the code: how a frame runs, where
each subsystem lives, and the invariants that are easy to break. Read this before
making non-trivial changes. Keep it current when you change something it describes.

Lua scripting from a script author's point of view is documented in
[`scripts/LUA_SCRIPTING_README.md`](../scripts/LUA_SCRIPTING_README.md).
Contribution rules (formatting, clippy, commits) are in [`Agents.md`](../Agents.md).

## At a glance

- One crate, `wgpu-cube` (lib `wgpu_cube`), about 76k lines of Rust.
- Stack: wgpu 30 / naga 30, egui 0.36 (+ egui_tiles 0.17), winit 0.30, hecs 0.10 (ECS),
  glam 0.28, mlua 0.10 (Lua 5.4, vendored).
- Requires Rust 1.95 (`rust-version` in `Cargo.toml`).
- Binaries:
  - `editor` (`--features egui`): the scene editor.
  - `player [project-dir]`: runs a saved project; it exits with `Io(NotFound)` if the
    directory has no `project.json`.
  - About 23 examples under `examples/`. Some need the glTF assets in `web/assets/`.
- Run binaries from the repo root. Plugin manifests, built-in scripts and example assets
  are resolved relative to the working directory.
- **Per-machine settings** (`settings.json` in the working directory, gitignored;
  `RenderSettings` in `src/settings.rs`): MSAA, shadow map size, resolution, present
  mode and `monitor` (1-based index, output name like `HDMI-A-0`, or `primary`). The
  `ENGINE_MONITOR` environment variable overrides the monitor. The window is centered on
  that monitor and shrunk to fit; positioning only works on X11, not Wayland.
- The web/WASM build is currently broken; see [Known issues](#known-issues-and-roadmap).

## Source map

| Path | What lives there |
|------|------------------|
| `src/render_application.rs` | `RenderApplication` trait (the app-facing API) and `run_application`; `DefaultUI` |
| `src/app/core/` | `App` core: `scheduler.rs` (startup/update/GPU stages), runtime modes, render hooks |
| `src/app/platform_winit/` | winit event loop; `mod.rs` holds the per-frame sequence (`RedrawRequested`) |
| `src/app/editor.rs` | Glue between the app loop and egui (`EditorState`) |
| `src/renderer/` | `Renderer` (`renderer_core.rs`), pipelines, shadows, post-processing, `ShaderBuilder` |
| `src/shader/` | WGSL modules composed by `ShaderBuilder` |
| `src/scene/` | `Scene`, `SceneWorkspace`, components, transforms, glTF loading, serialization, snapshots |
| `src/asset/` | Shared `Assets` (meshes, materials, textures) and handles |
| `src/scripting/lua/` | Lua runtime: `state.rs` (`ScriptingState`), `api/` (functions exposed to Lua), `commands.rs` |
| `src/gpu_particles/` | Compute-driven particle systems and behaviours |
| `src/ui/` | egui integration (`egui_integration.rs`), shared windows (hierarchy, stats, log, post-process) |
| `src/project.rs` | Project manifest (`project.json`), scene documents, active project root |
| `src/bin/editor/` | The editor: `application/` (systems, commands, UI), inspector, asset browser, shader/script editors |
| `src/bin/player/` | Standalone project player |
| `src/remote/` | Remote control: TCP server, request hub, built-in handlers, frame capture, log tap |
| `tools/engine-mcp/` | MCP server (separate crate) that exposes remote control to AI agents |
| `examples/scripts/` | Built-in Lua scripts and the editor UI plugin manifest (`ui_plugins.toml`) |

## Frame lifecycle

Each frame, `WindowEvent::RedrawRequested` in `src/app/platform_winit/mod.rs` runs:

1. `sync_runtime_state`: apply any pending Editor ↔ Playing transition.
2. `begin_frame`, then the **update stage** (`scheduler.rs::run_update_stage`):
   - `Playing`: `Scene::update_with_mode(dt, false)`. This runs game scripts and
     animations and advances scene time.
   - `Editor`: `Scene::run_editor_scripts(dt)`, which ticks only `@editor`/`@tool` scripts.
   - App update systems run (`RenderApplication::update`, plus systems registered on the
     `AppBuilder`), then transforms propagate.
3. **UI** (egui feature only): `EditorState::run_ui_frame` runs the egui frame.
   `RenderApplication::ui` executes here.
4. **GPU stage**: GPU systems run (`RenderApplication::gpu_update`). Workspace requests
   (open/close/swap scenes) are applied, and texture bind groups are rebuilt if needed.
5. **Render**: `Renderer::render` (see below), then egui is drawn on top, then
   `Renderer::present`.
6. If the frame can't be acquired (`renderer::SurfaceError`: occluded, timeout, outdated,
   lost), the frame is skipped. The surface is reconfigured when it is lost or outdated.

`RenderApplication::initial_runtime_mode()` defaults to `Playing`; the editor overrides it
to `Editor`. The editor starts and stops play mode through `RuntimeStateHandle`.

## Rendering

`Renderer::render` (`src/renderer/renderer_core.rs`) runs these passes in order:
shadow maps → depth prepass → main colour pass → custom stage (`BeforePostprocess`) →
post-processing (SSAO, bloom, FXAA, colour grading, MSAA depth resolve) →
custom stage (`AfterPostprocess`) → surface passes (composite, transparent and overlay) →
UI hook.

- **Bind groups** for the main pipeline: 0 = camera, 1 = objects, 2 = lights,
  3 = textures. Group 3 is bindless (`binding_array`, up to `MAX_TEXTURES = 256`) when the
  adapter supports it. Otherwise it is "traditional", with one binding per material slot.
- **`ShaderBuilder`** composes WGSL from the modules in `src/shader/`. It patches
  constants and expands markers such as `// @include_lighting`.
- **Surface**: the swapchain uses an sRGB format but also registers a non-sRGB
  `view_formats` entry. egui renders through `Renderer::ui_view()` (the linear view)
  because it does its own gamma handling; the sRGB view makes the UI look washed out.
- **Render region**: `RenderRegion` restricts the scene to a viewport rectangle (the
  editor's Scene/Game panes).
- **Custom rendering**: apps hook in through `RenderApplication::custom_render` and
  `custom_render_stage`.
- **Picking**: GPU picking renders entity ids and reads back one pixel asynchronously
  (`request_pick` / `poll_pick_result`).
- **Bloom, SSAO and the other effects** are toggled through `PostProcessEffects`.

## Scenes, assets and projects

- **`SceneWorkspace`** (`src/scene/workspace.rs`) holds the open scenes (editor tabs). It
  tracks the active one and owns the shared `Assets` cache.
- **`Scene`** wraps a hecs `World`, plus the scene graph, camera, environment, gizmos,
  imports and the scripting runtime. Hierarchy uses `Parent`/`Children` components.
  `TransformComponent` holds the local transform; `WorldTransform` is derived by
  `propagate_transforms`.
- **Snapshots**: `SceneStateSnapshot` captures a whole scene (assets, environment,
  camera, tree). The editor takes one when entering play mode and restores it on Stop.
  Undo history (`src/bin/editor/history.rs`) also stores full snapshots, up to 64.
- **Serialization**: a scene exports as a `SceneTreeAsset`; script components serialize as
  `SerializedLuaScript`.
- **Project layout on disk**:
  ```
  <project>/project.json                      # ProjectManifest: metadata, scene list, environment, editor camera
  <project>/content/scenes/<id>/scene.json
  <project>/content/scripts/*.lua
  <project>/content/materials/*.mat.json
  <project>/content/environment/*.hdr
  ```
- **Active project root**: `project::set_active_project_root` stores it in a global, and
  `resolve_project_path` resolves relative asset paths against it.
- **glTF**: `SceneLoader` / `src/scene/loader/` import glTF. Animation (`scene/animation.rs`)
  drives node transforms and material properties with step, linear or cubic-spline
  keyframes. There is no skinning.

## Scripting (Lua)

- **One `ScriptingState` per world** (`src/scripting/lua/state.rs`), owned by the
  scene's runtime. Script slots are keyed by `Entity` and validated by the
  `LuaScriptComponent`'s **binding id**. That id is a process-unique counter; a cloned
  component gets a new id.
  - Never identify scripts or worlds by memory address.
  - Never persist a "has started" flag in the component. A restored or reloaded scene
    must run `on_created` again.
- **Hooks**: `on_created(entity)`, `update(entity, dt)`, `on_destroyed(entity)`, `on_ui(...)`.
  - Every hook can read the world through `WorldGuard`.
  - Structural changes (spawn, despawn, `set_parent`, ...) are queued as commands and
    applied after the hook returns. Each command is applied independently, so one stale
    entity doesn't block the rest.
  - `set_parent` rejects cycles.
- **Modes** come from a header annotation:
  - none: play mode only
  - `-- @editor`: editor mode only
  - `-- @tool`: both modes
- **Events**: `emit_event` queues an event, and handlers run after the emitting hook in the
  same frame. At most `MAX_EVENTS_PER_FRAME` (4096) are dispatched per frame, which stops
  feedback loops.
- **Commands and events from `on_ui`** are deferred to the next update.
- **Coroutines**: `wait(seconds)` and `wait_frames(n)` yield tagged values from Lua. Rust
  resumes them without holding a borrow across the resume.
- **Errors**: a compile failure is cached. Each distinct error is logged once
  (`last_error()`) and cleared when the hook succeeds again. Editor plugin windows show
  the error inline.
- **Script UI**: `on_ui` records `UiCommand`s and egui renders them later. Widget ids come
  from `claim_id(kind, key)`. Value widgets report changes only, so scripts can reset
  values. `ui:anchored_panel` places HUD panels relative to the game viewport, and
  `ui:get_viewport_size()` and `ui:get_pixels_per_point()` expose the viewport.
- **Editor UI plugins** (`examples/scripts/ui_plugins.toml`, for example the welcome
  screen) run in the editor's own `UiPluginManager` with a separate `ScriptingState` and
  world. Their state survives play/stop and scene switches. They can't see the scene
  world.
- **Hot reload** (`src/bin/editor/application/script_watcher.rs`):
  - Watches `examples/scripts`, `scripts` and the open project's content root,
    recursively, with a 500 ms debounce.
  - Reloads only the changed script (`reload_path`) and keeps its state.
- **File sandbox** (`api/file_io.rs`):
  - Lua file access is limited to `ALLOWED_DIRS` (`scripts`, `examples/scripts`) plus
    paths granted with `grant_path_access`. Grants come from file dialogs, the text
    editor bridge and the open project directory.
  - Paths are resolved through `..` and symlinks before checking.
  - Don't add another way to reach the filesystem from Lua without going through this check.

## egui integration and the editor

- **The integration** (`src/ui/egui_integration.rs`) runs each frame through
  `egui::Context::run_ui`. The UI callback, and therefore `RenderApplication::ui`,
  receives egui's root `&mut egui::Ui`:
  - Panels (`egui::Panel::top/left`, `CentralPanel`) must be shown inside that `Ui`.
  - Windows use `ui.ctx()`.
  - Using `run_ui` (rather than `begin_pass`/`end_pass`) keeps egui's built-in plugins
    working: label text selection, and the drag and drop that egui_tiles relies on.
- **`max_passes` is 1 on purpose.** The editor's UI code has side effects (it queues
  actions and renders script UI), and a second, discarded pass would repeat them.
- **Texture deltas** (font atlas, images) must always reach the GPU. In debug builds,
  egui panics if a `TexturesDelta` is dropped without being applied. Unrendered output is
  merged into the next frame instead of being dropped.
- **Editor structure** (`src/bin/editor/application/`):
  - `EditorApplication` holds an `EditorSharedState` and a list of `EditorSystem`s
    (camera, selection, history, project, script editor, shader editor, asset browser,
    particles). Each system implements `update`, `gpu_update` and `ui`.
  - UI code doesn't mutate the scene directly. It queues `EditorCommand`s (inspector
    edits arrive as `InspectorAction`s). The next frame's update stage drains the queue
    (`drain_update_commands`) and applies them through `action_handlers/`, recording undo
    history. Script, shader, scene-creation and undo/redo commands stay on the queue for
    the `EditorSystem` that owns them.
  - The dock layout is an `egui_tiles::Tree<EditorPane>`.
  - Play mode snapshots the scene (see above); the Game pane being active implies
    Playing.

## GPU particles

`src/gpu_particles/` runs simulation in compute shaders. Behaviours: physics, starfield,
boids, and "optimized boids", which uses a spatial grid and a GPU radix sort.

- **Alpha sorting** reads particle data back asynchronously (`system/sorting.rs`) and
  sorts on the CPU.
- **Dead-list compaction** also uses a readback.
- **Wiring**: `ParticleSystemComponent` and `ParticleEmitterComponent` configure systems
  from the ECS; the editor drives them through its particle system.

## Remote control and MCP

A running app can be driven over a local socket, which is how AI agents (through the
MCP server in `tools/engine-mcp/`) inspect and edit scenes, run Lua, inject input and
take screenshots. The `engine-mcp` skill (`.claude/skills/engine-mcp/`) describes the
agent workflow.

- **Enabling**: the editor listens on `127.0.0.1:47811`
  (`remote::DEFAULT_EDITOR_PORT`). Other apps opt in through
  `RenderApplication::remote_control_port` or the `ENGINE_REMOTE_PORT` environment
  variable, which also overrides the port; `ENGINE_REMOTE_PORT=off` disables it.
- **Protocol**: newline-delimited JSON, `{"id", "method", "params"}` in, one
  `{"id", "result"}` or `{"id", "error": {"message"}}` out per request. `engine.methods`
  lists what the running app supports.
- **Threading**: `remote/server.rs` accepts connections on background threads and
  forwards requests over a channel. Nothing touches the scene off the main thread.
- **Execution**: `remote::process_requests` runs as a GPU system registered after the
  app's own (see `build_winit_app_internal`). Each request goes first to
  `RenderApplication::remote_command` (the editor handles projects, scene tabs,
  undo/redo, selection and imports in `src/bin/editor/application/remote.rs`), then to
  the built-in handlers in `remote/handlers/`. After requests change the scene,
  `remote_scene_modified` runs once; the editor records an undo step there.
- **Frame-loop hooks** (`app/platform_winit/mod.rs`): injected input is applied at the
  start of a frame, before the update stage, so scripts see `just_pressed` exactly as
  for real keys. Screenshots copy the swapchain image after egui is drawn and before
  `present` (the surface is configured with `COPY_SRC` when supported). `end_frame`
  advances the hub's frame counter and answers `wait` requests.
- **Post-processing**: in egui builds the UI's settings are pushed to the renderer
  every frame, so remote changes are also written into that handle
  (`take_postprocess_override`).
- **Logs**: every log record also goes to `remote::log_tap`, a ring buffer with
  sequence numbers, so clients can poll for new lines. Panics are recorded there too.
- **Lua console**: `ScriptingState::eval` runs code with the full scripting API, either
  in a persistent console environment or inside one entity's script environment.

## Invariants and gotchas

- **GPU structs** uploaded from Rust need `#[repr(C)]`, 16-byte alignment and explicit
  padding that matches WGSL. WGSL struct members are separated by `,`.
- **Bindless shaders** must start with `enable wgpu_binding_array;` (naga 30).
  `ShaderBuilder::finalize` adds it automatically whenever the source uses
  `binding_array<`. Hand-written WGSL that bypasses the builder must add it itself.
- **wgpu 30 API changes**:
  - Pipeline layouts take `Option<&BindGroupLayout>` and `immediate_size` (push constants
    are now "immediates").
  - Vertex buffer layouts are `Option`s.
  - `get_mapped_range` returns a `Result`.
  - Frames are presented with `queue.present` (`Renderer::present`).
- **Camera**: `SceneCamera::apply_from_entity` keeps the current camera when the entity
  hasn't moved. Rebuilding an unchanged view resets the orbit focus distance.
- **Script lists**: the inspector's script picker and the file sandbox are relative to the
  working directory. Run from the repo root.

## Verifying changes

- **Required before committing**: `cargo fmt --all`,
  `cargo clippy --all-targets --all-features -- -D warnings` and
  `cargo test --all-features`. CI (`.github/workflows/ci.yml`) runs the same.
- **Ignored tests**: `cargo test --all-features -- --ignored` runs GPU-heavy and slow
  tests. `packaged_gltf_roundtrip_without_source` is a known failure (see below).
- **Run it on the GPU**: tests don't create real pipelines, so naga or wgpu validation
  failures and egui debug assertions only appear at runtime. Use the `smoke-test` skill
  (`.claude/skills/smoke-test/smoke.sh`). It builds the editor and examples, runs each
  one briefly, scans the logs and can screenshot the engine window.
- **Lua regression tests** live in `src/scripting/lua/regression_tests.rs` and
  `integration_tests.rs`.

## Known issues and roadmap

- **Web/WASM build is broken**: `mlua`'s vendored Lua can't target
  `wasm32-unknown-unknown`. Options:
  - compile scripting out on wasm with a stub `ScriptingState`
  - use the emscripten target
  - switch Lua implementation
- **glTF data-URI images**: images embedded as `data:` URIs are treated as file paths, so
  `packaged_gltf_roundtrip_without_source` fails.
- **Inspector script picker** lists only `scripts/` and `examples/scripts/`, not the open
  project's `content/scripts/`.
- **`player` error**: without a project directory it prints a bare
  `Io(NotFound)` instead of saying no `project.json` was found.
- **Editor UI plugins can't access the scene world**, so tools such as scene statistics
  or quick actions can't be written as plugins yet.
- **Undo history** stores a full scene snapshot per change, which may get expensive for
  large scenes.
- **Project environment is not restored on load**: `ProjectManifest` saves the scene
  environment in `project.json`, but neither the editor nor the player applies it when
  loading. Only an Environment entity in the scene round-trips; the remote
  `environment.set` creates one for that reason.
- **Recent projects** are stored in the repo (`examples/scripts/.recent_projects.txt`)
  rather than in a per-user config directory.
- **Dependencies not yet upgraded**: mlua 0.12, glam 0.34, hecs 0.11, thiserror 2,
  toml 1.x, rand 0.10, base64 0.23, pollster 1.0, rfd 0.17.
- **In-game UI roadmap**: anchored panels and viewport queries are done. A WYSIWYG design
  mode (dragging anchored panels in the editor and saving offsets to the entity) and
  flex/grid layout containers are not implemented. The original plan is in git history
  (`INGAME_UI_PLAN.md`).
- **Not yet reviewed in depth**: the asset browser, the project system and the renderer
  internals (shadows, post-processing, batching).
