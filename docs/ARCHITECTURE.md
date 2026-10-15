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
- Native and web players run Lua gameplay. `build_web.sh --project <dir> --no-serve`
  packages saved projects; the vendored Lua shim requires clang and matching wasm-bindgen.

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
| `src/procgen/` | Profile lathes, beveled polygon extrusions, mesh composition, voxel meshes, seeded height fields/PBR maps and PCM sound synthesis |
| `src/scripting/lua/` | Lua runtime: `state.rs` (`ScriptingState`), `api/` (functions exposed to Lua), `commands.rs` |
| `src/gpu_particles/` | Compute-driven particle systems and behaviours |
| `src/ui/` | egui integration (`egui_integration.rs`), shared windows (hierarchy, stats, log, environment) and widgets (`FineSlider`) |
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
transparent pass → post-processing → custom stage (`AfterPostprocess`) →
surface passes (overlay, gizmos) → UI hook.

The scene renders in HDR (`Rgba16Float`, `postprocess::HDR_FORMAT`), transparent
surfaces included, so light above 1.0 survives to bloom and tonemapping; scene shaders
write linear radiance and leave exposure and tonemapping to post-processing. The G-buffer
holds the octahedral normal and motion (`GBUFFER_NORMAL_FORMAT`) and the world position,
whose `w` is 0 for background and on a surface at least 1, with the smoothness
(1 − roughness, 0 when unlit) and specular colour F0 packed into the half float's bits
(`encode_surface_w` in `common.wgsl`); the colour target's alpha is -transmission on
transmissive surfaces drawn in the opaque pass for ray-traced refraction. Post-processing
(`src/renderer/postprocess/`) runs: MSAA depth resolve → ray-traced refraction (when glass
is on screen) → reflections (half-res, or full with `ssr.full_resolution`,
trace through the G-buffer positions, or ray traced with `ssr.ray_traced`, accumulated
over frames, separable blur, added to the HDR scene weighted by smoothness² and Fresnel
from F0, minus the flat ambient a found reflection replaces on metals) → TAA (Halton-jittered camera,
closest-depth motion vector, Catmull-Rom history, YCoCg variance clipping) → motion blur
→ SSAO → bloom (13-tap Karis downsample chain, tent upsample blended by scatter) → depth
of field (half-res CoC prefilter, golden-angle bokeh gather) → auto exposure (compute:
luminance histogram, adapted EV in a storage buffer) → composite (AO, DoF, bloom,
exposure, vignette, white balance and grading, tonemapping, lift/gamma/gain, `.cube`
LUT; into a half-float display target) → final (FXAA, CAS sharpening, film grain,
dither; onto the surface). Later passes read the TAA/motion-blur output when those run.
Overlays and gizmos draw after that, untonemapped, through an unjittered camera
(`CameraBuffer::stable_bind_group`).
Screen-space reflection and ambient-occlusion projections use the same TAA jitter
as the G-buffer rasterization; motion vectors continue to compare unjittered
matrices. Using a stable projection to look up jittered surfaces causes shading
to flicker, especially at a distant horizon.
- **HDR display output** (`src/renderer/hdr_output.rs`), opt-in through `settings.json`
  (`"hdr_output": "off" | "auto" | "simulate"`, `"hdr_paper_white_nits"`,
  `"hdr_peak_nits"`) or the `ENGINE_HDR` environment variable. `auto` picks scRGB
  (`Rgba16Float` + extended linear sRGB), else HDR10 (`Rgb10a2Unorm` + BT.2100 PQ), else
  logs a warning and stays SDR. In HDR mode the final post pass, overlays and the
  `AfterPostprocess` stage draw into a half-float composition texture (linear, 1.0 =
  paper white; `Renderer::output_format`), egui draws into its own 8-bit layer
  (`ui_format`/`ui_view`), and `PresentStage` composites both and encodes for the display
  when the frame is presented or captured (`Renderer::resolve_output`). Tonemapping then
  rolls highlights off toward the display's peak instead of 1.0; lift/gamma/gain and the
  LUT grade the paper-white range. `simulate` runs that path on an SDR surface, which is
  how it is tested on machines without an HDR display; the scRGB/PQ encoders have unit
  tests but have not been seen on an HDR screen.
- **G-buffer**: colour (HDR), normal, world position and (while picking) the pick id —
  exactly WebGPU's 32-byte colour-attachment limit, so motion vectors share the normal
  texel: octahedral normal in xy, screen motion (UV per frame) in zw. Motion vectors
  compare `CameraUniform::unjittered_view_proj` with `prev_view_proj`, and each
  instance's model matrix with last frame's (`prev_models`, group 1 binding 2, matched
  by pick id + mesh).

- **Bind groups** for the main pipeline: 0 = camera, 1 = objects, 2 = lights,
  3 = textures. Group 3 is bindless (`binding_array`, up to `MAX_TEXTURES = 256`) when the
  adapter supports it. Otherwise it is "traditional", with one binding per material slot.
  Browsers never offer bindless textures, so every web build takes the traditional path;
  GPU particle systems bind their own material's textures there
  (`Renderer::material_textures_bind_group`). `WGPU_CUBE_FORCE_TRADITIONAL_TEXTURES=1`
  forces that path natively for testing.
- **Web asset requests** (`src/io.rs`) percent-encode each file path (`percent_encode_path`),
  so a file name holding a literal `%2e` is requested as `%252e`. glTF URIs are already
  encoded and are decoded before they are joined to a path.
- **GPU readbacks never block** (`gpu_particles/system/readback.rs`): a browser cannot wait on
  the GPU, so the particle dead list and the alpha-sort copy are recorded in one frame, mapped
  in the next and consumed once the map callback has run.
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
- **Post-processing settings** (`PostProcessSettings`) come from `PostProcessVolume`
  components: each frame `AppCore::render_scene` blends the scene's enabled volumes at
  the camera (`scene::blend_post_process_volumes`, lowest priority first; infinite
  volumes apply everywhere, box/sphere ones fade over `blend_distance`) and hands the
  result to `Renderer::set_postprocess_settings`. A scene without volumes gets the
  defaults. Projects saved before volumes stored effects in `project.json`
  (`postprocess`) and grading in the environment; `SceneAsset::migrate_legacy_post_processing`
  turns those into an infinite volume entity on load, and they are no longer written.

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
  keyframes, bound to the entities a file was imported with.
- **Skeletal animation** (`src/scene/skeletal/`):
  - Skinned glTF primitives keep `JOINTS_0`/`WEIGHTS_0` as a second stream on `MeshData`
    (`skin: Vec<SkinVertex>`, 16 bytes per vertex, four influences, weights quantized to
    sum exactly to 65535). Joints are ordinary entities; `SkinnedMesh` on the mesh entity
    lists them with their inverse bind matrices (scaled with the import scale).
  - Each frame `build_render_objects` computes a joint palette per skinned entity
    (`inverse(mesh_world) * joint_world * inverse_bind`) and hands it to the batcher as a
    `SkinBinding`. Before the shadow pass, `renderer/skinning.rs` deforms every skinned
    instance into a vertex buffer of its own (compute shader `shader/skinning.wgsl`, or
    the identical CPU `skin_vertices` on devices without compute/storage buffers;
    `WGPU_CUBE_FORCE_CPU_SKINNING=1` forces it). Each instance is its own batch, and all
    passes draw it like a rigid mesh, so no vertex shader knows about skinning. The same
    buffers can feed ray-tracing BLAS refits later. Motion vectors use the model matrix
    only (limb motion is not in them yet).
  - Skinned imports get an `Animator` on their root. It plays `SkeletalClip`s read from
    glTF/GLB files (`ClipLibrary`, cached in `Assets::clip_library`), bound to bones by
    normalized name, so animation-only files drive any rig with matching names.
    Layers with bone masks, cross-fades, per-bone translation retargeting by rest-length
    ratio and root motion (`keep`/`in_place`/`apply`). Animators run only while
    `Scene::set_animation_playback(true)` (play mode), so the editor saves rest poses.
  - Copy paths remap joints: `SceneAsset::instantiate` (entity indices),
    `merge_world_as_child` and the remote duplicate. Lua: `animator_*`, `find_bone`.
    MCP: `entity_get`/`entity_update {animator}`, `animation_inspect`, `debug_skeleton`.

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
- **Post-processing**: `postprocess.set` edits a volume entity (the lowest-priority
  infinite one by default, created if missing), so changes save with the scene.
- **Procedural assets** (`remote/handlers/procedural.rs`): `texture.generate` writes
  tileable PNG map sets, `texture.maps` derives maps from an existing height image,
  and `audio.generate` writes seeded PCM WAV effects. Files and parameter recipes
  save immediately under the project's content folder; output paths and compatible
  material bindings are returned. Existing textures reload, and replaced sound files
  are evicted from the playback cache. CPU algorithms live in `src/procgen/` and can
  be tested without a GPU or audio device. Generation tools are native authoring
  features; their output assets also load on the web.
- **Procedural geometry** (`remote/handlers/geometry.rs`): `mesh.lathe` turns and bevels
  radius/height profiles, `mesh.extrude` triangulates and bevels concave polygons with
  taper/twist, and `mesh.compose` joins transformed parts. Outputs have UVs, normals,
  tangents, content-based GPU sharing and bounds. Scene saves embed their `MeshData`.
- **Logs**: every log record also goes to `remote::log_tap`, a ring buffer with
  sequence numbers, so clients can poll for new lines. Panics are recorded there too.
- **Lua console**: `ScriptingState::eval` runs code with the full scripting API, either
  in a persistent console environment or inside one entity's script environment.
- **Explicit simulation**: `RuntimeStateHandle` stores manual mode, seed and step count.
  Entering manual play initializes scripts once; ordinary update frames skip Lua/animation.
  `runtime.step` applies step-indexed input, updates scenes and realizes script assets for
  each fixed step. GPU particles, UI and custom app systems are outside this scope.
- **Scene transactions**: `scene.transaction` validates whitelisted scene calls in a
  speculative snapshot, resolves aliases to previous results and commits once. Errors and
  dry runs restore the original scene, including its scripting runtime; file/audio/Lua
  operations are excluded. The editor records one undo entry on commit.
- **Spatial tools** use transformed world AABBs, visibility inheritance and hierarchy
  exclusions. Ray normals describe box faces; placement/scatter are approximate authoring.
- **Profiling**: `profile.capture` samples consecutive frames through the remote GPU system.
  Percentiles use nearest rank; CPU stage/submit cost and cumulative Lua hook counters are
  available, along with entity/asset growth. GPU execution timestamps are not measured.
- **Prefab authoring** exports a selected subtree into a registered scene document. Source
  updates affect future/reopened instances; current instance values are preserved.
  Project exports include linked child scene nodes as prefab references with placement and
  visibility overrides; snapshots preserve origin metadata in serialized entity order.
- **Verification**: read-only `project.validate` checks saved structure/assets/Lua syntax.
  MCP `web_build` wraps the non-serving build script; `web_playtest` uses Node's WebSocket
  and Chromium CDP with a disposable loopback server/browser profile. Results contain
  structured JSON, output schemas and tool annotations.

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
  hasn't moved, while syncing its up vector when roll changes. Equivalent saved up
  hints (such as world Y) are preserved across undo.
  Rebuilding an unchanged view resets the orbit focus distance. Lua `look_at` and
  MCP camera placement share a Y-up, zero-roll look rotation so a full camera orbit
  keeps the horizon level.
  Lua `look_at` also supplies a transient `LookAtTarget` hint. While it still agrees
  with the entity's world forward direction, the camera keeps that actual focus point
  instead of replacing it with `eye + forward`. Directional shadows then stay centered
  on an orbit's subject, including elevated subjects and shallow viewing angles. A
  stale hint is ignored after the camera moves or turns away; scripts regenerate hints
  at runtime rather than serializing them into scene assets.
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

- **Web filesystem limits**: Lua uses the vendored wasm libc/SjLj shim. File watching and
  authoring remain native-only; player assets load through HTTP, and audio needs a gesture.
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
