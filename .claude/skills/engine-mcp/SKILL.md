---
name: engine-mcp
description: Drive the running engine/editor through the `engine` MCP server (mcp__engine__* tools) to build scenes and whole games, write and playtest Lua gameplay, inspect or edit live objects, inject input and take screenshots. Use whenever the user wants something built, changed, inspected or tested inside the engine rather than in its Rust source, or asks to see what the engine shows.
---

# Building with the engine over MCP

The `engine` MCP server (`tools/engine-mcp/`, registered in `.mcp.json`) talks to the
engine's remote-control socket (`src/remote/`). Its tools appear as `mcp__engine__<name>`.
If they are missing, the server has not been approved or loaded; ask the user to
restart Claude Code in the repo root and approve it.

In clients that load tools on demand (Claude Code does), fetch the ones you will need in
one `ToolSearch` call rather than one by one. A good starting set:
`engine_launch`, `engine_info`, `engine_status`, `editor_state`, `project_open`,
`entity_spawn`, `entity_update`, `entity_get`, `scene_list_entities`, `lua_eval`,
`lua_api_reference`, `script_status`, `runtime_set`, `wait`, `playtest`, `screenshot`,
`logs`, `input_send`, `project_save`, `engine_stop`.

## Start every session the same way

1. `engine_launch` (target `editor`, the default). It builds incrementally, starts the
   editor and waits until it accepts commands. If the user already has an editor open
   on port 47811, it attaches to that one instead; never restart an editor you did not
   launch, because the user may have unsaved work in it.
   Don't pass `monitor` unless the user asks: their `settings.json` chooses which monitor
   engine windows open on (they may be using the other one).
2. `engine_info`: runtime mode, active scene, value conventions and, in the editor,
   `editor.project` and `editor.startup_dialog_visible`.
3. No project open? Use `project_create {name, location}` or `project_open {path}`; both
   return once the project is open. The startup dialog blocks the editor UI until then.
   `project_create` makes `content/scripts`, `audio` and `textures`, and starts the scene
   with a dark backdrop, ambient light, a shadowed sun, an active `GameCamera` looking at
   the origin and a 40 m `Floor` (`template: "empty"` for a bare scene).
   `editor_state.project.scripts_dir` is where scripts go.

After changing engine *Rust* code, the running process still has the old code:
`engine_build` to see compiler errors, then `engine_launch {restart: true}` (only for an
engine you launched; save first).

## Conventions

- Meters, Y up, -Z forward, right-handed. Rotations are Euler degrees `[x, y, z]`,
  applied yaw (Y), then pitch (X), then roll (Z), the same as Lua's `set_rotation`
  (which takes radians). Lights and cameras shine/look along their -Z.
- Colors: `[r, g, b(, a)]` in 0..1 or `"#rrggbb"`.
- Entities: the numeric id from any result (the same i64 handle Lua uses) or a
  *unique* name. Duplicate names are an error that lists the ids.
- Primitives differ in size: the cube spans -0.5..0.5, the sphere has radius 1, the
  cylinder (radius 1, length 1), cone and torus (ring 1, tube 0.35) lie along Z.
  `entity_get` shows `mesh.local_bounds`. The plane is 1x1 in XY facing +Z, so a floor is
  `entity_spawn {kind: "plane", rotation: [-90, 0, 0], scale: 30}`.
- A scene needs a light to be lit, for example
  `entity_spawn {kind: "directional_light"}`.

## Building scenes

- `entity_spawn` creates and configures in one call: it accepts every `entity_update`
  field (`material`, `script`, `parent`, lights, ...). Prefer one call per object over
  spawning first and updating afterwards.
- `entity_update` changes only the fields you pass. Setting a component field to `null`
  removes that component.
- Materials are copied on write when other entities share them, so recoloring one object
  never recolors others; pass `edit_shared: true` to change all users.
- For many objects: `entity_duplicate {entity, count, step, row_step, columns}` lays out a
  row or grid in one call; `entity_update`/`entity_delete` take `entities` or
  `name_contains` to change or remove many at once; `batch` runs a list of tool calls in
  one round trip. Pass `quiet: true` for terse results, or `aggregate: true` for one summary
  (calls per tool, entities returned, aliases, failed calls in full) instead of a row per call.
- `camera_set {focus: <entity>}` or `{position, target}` to frame the shot, then
  `screenshot {target: "viewport"}` (just the 3D view) or `"window"` (with the editor
  UI). `screenshot {camera: "game"}` (or a camera entity) renders that one frame through
  the game camera (viewport only, no gizmos or editor grid) without moving the editor
  view. `camera` can also be a one-off pose `{position, target, fov_deg}` for looking
  around, and `aspect: "16:9"` (or `"4:3"`, `1.5`, or `viewport_size: [w, h]`) lays the
  game out at another window shape for the capture: scripts see it through
  `get_viewport_size()` and HUDs anchor to it, so check the game camera and HUD at both.
  `viewport_preview` keeps a shape on across several calls; `playtest` takes `aspect` too. `camera_set` edits the game camera entity
  while there is one; prefer a screenshot pose to look around. **Look at the screenshot after visual changes** instead of assuming they worked.
- `environment_set` controls background, ambient light and HDR sky. The scene renders
  in HDR; exposure (manual or auto), tonemapping, colour grading, bloom, SSAO,
  screen-space reflections (`ssr`: glossy reflections on smooth, low-roughness surfaces),
  vignette, chromatic aberration, film grain, sharpening and FXAA live on `post_process_volume`
  entities. `postprocess_set` edits the scene's infinite volume (creating it if needed);
  for an area with its own look, spawn a `post_process_volume` with a box or sphere
  shape, higher `priority` and a `blend_distance`. `postprocess_get` shows the blended
  result at the camera and each volume's influence.
- `asset_import {path}` brings in a glTF model (samples are under `web/assets/`), or copies
  an image, sound, font, script or shader into the project's `content/` and returns the
  project-relative path to reference. After rewriting an image on disk, `asset_reload`
  (or the editor's file watcher) shows the new version under the same path.
- `sound_play` tries a sound; `audio_status` lists what played (you cannot hear it). A
  busy game plays the same files hundreds of times, so pass `summary: true` for one entry
  per file. Run `sound_inspect {path}` on every sound you generate or import: it reports
  length, peak/RMS, clipping, silence at either end and whether the loop seam clicks,
  with notes on problems and a waveform/spectrogram image.
- Procedural geometry: `mesh_lathe {profile = [[radius, height], ...]}` turns a profile
  around Y (bottom to top, equal heights for ledges, zero radius for poles); `bevel` and
  `bevel_segments` soften profile corners. `mesh_extrude
  {polygon, depth, bevel}` extrudes a concave XY outline along centered Z, with rounded
  rim bevels (`bevel_segments`), taper and twist (`steps` controls subdivision).
  `mesh_compose {parts}` joins locally transformed `shape: "lathe"`, `"extrude"` or
  `"primitive"` parts into one mesh/material, without a boolean union. All three have
  UVs/normals/tangents, accept material/transforms/parent, and can replace `entity`'s
  mesh while keeping its material. Identical geometry shares a GPU mesh; `project_save`
  embeds it for reopening and export. Chess example: `examples/projects/procedural-chess`;
  regenerate with `tools/engine-mcp/examples/procedural_chess.py`.
- Characters: skinned glTF files (Khronos samples in `web/assets/skinned/`: CesiumMan,
  Fox, RiggedFigure) import with an `animator` on their root that plays the file's first
  clip in play mode (scale the `(Instance)` root of centimetre models such as the Fox,
  e.g. 0.012). `entity_get` on the root lists clips with bone coverage and what plays.
  `entity_update {entity, animator: {play: {clip, fade}}}` cross-fades during play (while
  editing it sets `default_clip`); `root_motion: "apply"` moves the entity by the walk
  instead of sliding the hips. Animation-only GLBs import with `asset_import {path, kind:
  "animation"}` into `content/animations` and attach with `animator: {add_clips: path}`;
  clips bind by bone name (`mixamorig:Hips` matches `Hips`). Layers: `animator: {layer:
  {index: 1, weight, mask: "Spine"}}` then play with `layer: 1`. `debug_skeleton {enabled:
  true}` draws bones over the mesh in screenshots. Lua: `animator_play`, `animator_state`,
  `find_bone` (attach props to hands). Verify motion with `runtime_set {mode: "play",
  manual: true}` + `runtime_step` + `screenshot`.
- Procedural assets: `texture_generate {name, pattern, seed, colors, normal_strength}`
  writes tileable PNGs (noise, bricks, checker, wood, marble, voronoi) and matching
  normal, height, metallic/roughness and occlusion maps into `content/textures`.
  `texture_maps {name, source}` derives maps from a height image; `flip_y`, `channel`,
  `invert` and `tileable` control conversion. Both return previews and ready-to-use
  `material` fields, or apply them directly with `entity`. ORM packs occlusion into R,
  roughness into G and metallic into B. Height is an export map, not displacement.
  `sound_generate {name, preset, seed}` writes WAVs into `content/audio` (tone, blip,
  laser, explosion, pickup, jump, hit, powerup), with pitch/envelope/noise/filter controls
  and automatic inspection. Files save immediately with JSON parameter recipes;
  `overwrite: true` replaces them, reloading textures and clearing sound caches.
  Use `image: false` for terse batch results. Scene material assignments still need
  `project_save`.
- Material `emissive` makes a surface glow in its own color; above 1 it is HDR, which is
  what lets bloom pick up a lit surface (try 2 to 5 with bloom on).
- Glass, gems and water: material `transmission` (1 = clear) with `ior` (1.5 glass, 2.42
  diamond), roughness near 0 and metallic 0; the color tints what shows through. With
  hardware ray tracing (`engine_info` `surface.ray_tracing`) light refracts through the
  object (internal reflections, absorption); otherwise it is drawn see-through without
  bending. `dispersion` (0.36 diamond; 1 or more for showy rainbow fire) splits the light
  into colours as it refracts. glTF imports read KHR_materials_transmission / _ior /
  _dispersion.
- The cone's tip points toward +Z, opposite to forward: ships and arrows made from cones
  need a 180 degree yaw.

## Gameplay with Lua

Call `lua_api_reference` before writing scripts. The whole guide is about 6k tokens;
`lua_api_reference {outline: true}` lists its sections, `{section: "UI API"}` returns one
and `{search: "despawn"}` finds lines. Then:

1. Write the `.lua` file into `editor_state.project.scripts_dir` with your own file
   tools.
2. Attach it: `entity_update {entity, script: {path: "<absolute path>"}}`. It compiles on
   the next frame. While a project is open, the editor hot-reloads saved edits to files
   under its `content/` folder, so later changes need no reattach.
3. `script_status`: `compiled` and `last_error` for every script. Errors read
   `file.lua:LINE: message`.
4. Playtest in one call: `playtest {seconds, time_scale, input, entity, eval}` plays,
   injects input, runs (faster than real time with `time_scale` up to 16), then reports
   failing scripts, warnings, the sounds played (grouped by file; `sounds: "list"` for
   every play), a `scene_diff` of what changed, the `eval` result and a screenshot, and
   stops. For long soak tests pass `lean: true` (no screenshot, diff or sounds). Step by step: `runtime_set {mode: "play"}`,
   `wait`, inject input, `wait`, then `entity_get`, `lua_eval`, `logs`, `screenshot`.
5. `runtime_set {mode: "stop"}` restores the scene to its pre-play state with the same
   entity ids. Every change made during play (including your remote edits) is discarded;
   such results carry `discarded_on_stop: true`.

Facts that change how you write scripts:

- Scripts without an annotation run `on_created`/`update` **only in play mode**. Use
  `-- @tool` for scripts that should also run while editing, and `-- @editor` for
  editor-only ones.
- Lua can spawn entities with primitive meshes (`spawn_primitive`), change meshes and
  materials (`set_mesh`, `set_material_color`, `set_material`), play sounds
  (`play_sound`), create and change lights and cameras (`spawn_light`, `set_light`,
  `spawn_camera`, `set_camera`, `set_active_camera`), change post-processing
  (`set_postprocess({ vignette = { enabled = true } })`, undone when play stops) and draw text at world positions
  (`ui:world_label`, `ui:screen_label` in `on_ui`). `despawn_entity` removes entities, so
  bullets and sparks can be spawned and removed; a script that spawns its own pool should
  remove it in `on_destroyed(self, reason)`, which also runs before a hot reload (otherwise
  every saved edit leaks a second pool). Materials of despawned entities are freed.
  Hiding a parent hides its children.
- Effects: `emit_particles(x, y, z, {count, color, intensity, speed, life, size, shape,
  gravity})` bursts GPU particles (sparks, explosions, `shape = "ring"` shockwaves) for
  almost no CPU cost; prefer it to spark entities. For pooled entities of one color,
  `spawn_primitive(kind, name, {material_of = first})` or `share_material` makes them
  share one material.
- Voxel art and procedural shapes: `voxel_model{rows = ..., legend = ..., depth = ...}` or
  `voxel_model{size = {W, H, D}, fill = function(x, y, z) ... end}` (with `noise`/`fbm`)
  greedy-meshes the model into one mesh per colour, and `spawn_voxels(model, name,
  {materials = ...})` spawns it with one child per colour. Never build voxel models from one
  cube entity per voxel or per rectangle. Build each model once and spawn it many times:
  identical models share meshes. Ambient occlusion is baked in (`ao`); `colors` (vertex
  colours per key) with `parts`/`part` put many keys in one mesh, e.g. a gradient on one
  emissive material. From the MCP side, `voxel_spawn` does the same with text art or a
  `fill` Lua expression.
- Split a big game into files: `require("waves")` returns a module, `include("autopilot")`
  runs a file inside the script. Editing either hot-reloads the scripts that use it.
- `save_data(key, text)` / `load_data(key)` keep a high score or settings between runs
  (per user and project, outside the repo; `localStorage` on the web).
- Hot reload resets script locals. Return the game state from `on_destroyed(self, reason)`
  when `reason == "reload"` and take it in `on_created(self, previous)` to keep playing
  through edits.
- `get_time()` is game time since play started (it follows `time_scale`); `get_forward`,
  `get_right`, `get_up` give an entity's world directions; `screen_to_ground`,
  `screen_to_world_ray` and `world_to_screen` convert between mouse pixels and the world,
  so mouse aiming does not need hand-written projection math.
- Mutations from a hook are queued and applied after it returns, so reading back a
  value you just set in the same hook gives the old value.
- Keep game state in a global table (`GAME = {...}`) or the state API (`set_f64`,
  `set_state`). Both are readable from outside with `lua_eval {entity, code: "return GAME.wave"}`.
- Add test hooks: a global `RT` table next to `GAME` with functions such as
  `RT.jump(wave)`, `RT.spawn(kind, x, z)`, `RT.god(true)` and `RT.units()`, called through
  `lua_eval {entity}`. Then any enemy type or wave can be checked in seconds, with
  `runtime_set {time_scale: 0}` plus `screenshot {camera: {position, target}}` to look at a
  frozen frame from any angle, instead of playing up to it.
- Input key names: letters are upper case (`"W"`), and the named keys are `Space`,
  `Return`, `Escape`, `ArrowUp`/`Down`/`Left`/`Right`, `Shift`, `Control`, `Alt`, `Tab`.
  Mouse buttons are 0 left, 1 right, 2 middle.

## New authoring and verification tools

- Repeatable tests: `playtest {steps, dt, seed, setup, assertions}` initializes a fresh
  manual session. Assertions are `{code: "boolean Lua expression", at_step?, name?}`;
  omitted `at_step` checks at the end. Failed checkpoints return their step and observations
  before the usual screenshot/stop. `scenario` loads saved JSON; `save_scenario` writes a
  new file. For interactive stepping, configure `runtime_set {mode: "play", manual: true,
  seed}` while stopped, wait two frames, then `runtime_step`. Only Lua gameplay and scene
  animation are stepped; UI, custom app systems, GPU particles and rendering are independent.
- Spatial tools: `scene_query {bounds: {min, max}}` or `{center, radius}` and
  `scene_raycast {origin, direction}` inspect world AABBs. `entity_place` adds `entity`,
  `offset`, `align_normal` and `avoid_overlap`. Box normals approximate the geometry.
  `scene_scatter {entity, bounds, count, spacing, seed}` creates constrained copies; use
  `on_surface: false` for a flat placement plane. Hidden ancestors are respected.
- `batch {atomic: true, calls}` rolls back all scene edits on failure and creates one undo
  entry. `dry_run: true` validates without committing. Name results with `as: "root"` and
  reference them using `{ "$ref": "root/id" }`. Only scene operations are supported;
  files, scripts, project operations, audio and runtime actions cannot be transactional.
- `prefab_create {entity, name}`, `prefab_inspect {document_id}` and
  `prefab_update {entity, document_id}` author reusable sources; `scene_instance` links them.
  Save the project to persist registration. Existing instances keep current values until
  recreated or reopened after a source update.
- `mesh_sweep {path, radius}` builds tubes/rails; `profile` supplies a polygonal section.
  `mesh_heightfield {resolution, size, seed}` creates terrain; `heights` or `source` provides
  explicit/image heights. Both persist embedded geometry with `project_save`.
- `visual_capture {variants: [{name, postprocess}], frames, settle_frames}` renders one
  frozen view under several post-processing variants (render-only, the scene is never
  edited), saves the frames as PNGs plus `manifest.json` (engine frame, TAA jitter index,
  scene time, settings changed from the baseline) and compares them. Use it to answer "does X
  cause the ghosting/halo" with numbers and images instead of toggling settings by hand. Read
  each variant's `compare`: `differs` (clearly more than the noise between its own frames,
  needs `frames` >= 2), `smooth` (broad or faint change; the per-pixel `mean` is mostly output
  dither, about 0.8 levels, so do not read it), `over_pct`/`max` (small strong changes),
  `top_regions` (where, with pixel rects) and `diff_image` (look at it: black is identical,
  amplified 16x). `settle.converged` false or a warning means TAA history had not settled:
  raise `settle_frames` (a verdict needs 16 frames of it; null means too few). TAA variants are
  lined up on the baseline's jitter phase, so identical settings come out `differs: false`. `compare: false` only saves the images. After a new build, restart the
  MCP server (/mcp) or the client sends typed parameters such as `compare` as strings.
- `profile_capture {frames}` captures CPU latency percentiles, stage/script cost and asset
  growth. `baseline` compares a previous report. It does not measure GPU execution.
- After `project_save`, call `project_validate` for paths, structure and Lua syntax. Its
  `browser` block is a separate web preflight (assets outside `content/` are not staged,
  case clashes, Lua file-system calls); native `ok: true` does not mean browser-ready.
  `web_build {project}` packages the existing web pipeline and stops on browser errors;
  replacing artifacts needs `overwrite: true`. `web_playtest {directory}` returns browser failures and a screenshot.
  It needs Node 22+ and Chromium/Brave (`ENGINE_BROWSER`); no npm dependencies or deployment.

## Injecting input (frame-based sessions)

`input_send` is applied at the start of upcoming frames, exactly like real key events,
so `is_key_just_pressed` fires once per tap.

- Tap: `{key: "Space"}` (held 3 frames by default; set `frames` to change it).
- Hold, then release: `{key: "D", action: "down"}`, `wait {seconds: 0.5}`,
  `{key: "D", action: "up"}`.
- Sequences: `events: [...]`, each event with a `delay` in frames.
- `input_clear` releases everything if a test leaves keys stuck.

## lua_eval

Runs Lua against the live world with the full API and returns the result plus anything
printed.

- A bare expression returns its value; otherwise `return` one (several values come back
  as an array).
- Without `entity`, globals persist between calls like a REPL, but script globals
  (`GAME`) are not visible: the error names the script that defines the global. With
  `entity`, the code runs inside that entity's script, so it can read or change the
  script's globals and state. `playtest`'s `eval` finds the script by itself when only one
  defines the global.
- Pass `read_only: true` for pure queries, so they don't add undo steps in the editor.

## Persisting work

- `project_save` writes the project and returns once the files are on disk, listing them.
  It refuses while playing; stop first.
- `environment_set` creates an Environment entity when the scene has none, because only
  that entity round-trips (the project-level environment is not restored on load).
- Script and texture paths inside the project are saved relative to it, so projects
  are portable.
- `editor_undo`/`editor_redo` step through scene changes, including remote edits made
  in editor mode.
- To check the game outside the editor:
  `engine_launch {target: "player", project: "<dir>"}` on a saved project. It starts in
  play mode and hot-reloads edited scripts.
- **Measure performance in the player**, not the editor: the editor's UI and systems cost
  several times more per frame. `engine_stats.frame_breakdown` shows where each frame goes
  on the CPU (scripts, transforms, render_collect, render_gpu, ui, present, ...), and
  `engine_stats.renderer.gpu` the GPU milliseconds per render stage (shadows, main,
  acceleration_structures, reflections, post_process, ...) from timestamp queries, a frame
  or two old (native only). Compare GPU costs with those, not fps: other programs (games)
  may share the GPU and swing the frame rate.

## When something goes wrong

| Symptom | Check |
|---|---|
| Tool says no engine is listening | `engine_status`; `engine_output` shows why the process died |
| Engine crashed or validation errors | `engine_output {contains: "panicked"}` or `{contains: "error"}` |
| Script does nothing | `runtime_mode` must be `playing` for unannotated scripts; then `script_status` |
| Script error | `script_status` and `logs {min_level: "warn"}`; fix the file and it hot-reloads |
| Object invisible | It needs a light, `visible: true`, a sensible scale, and must be in front of the camera; `camera_set {focus}` then screenshot |
| Change vanished | It was made during play mode (stop restores), or the project was never saved |
| New logs only | Pass `after: <latest_seq>` from the previous `logs` call, or `since_play_start: true` to skip what happened before the current play session |
| A tool lacks a parameter this file mentions | The MCP server is older than its sources; `engine_info.mcp_server.restart_needed` says so (restart it with /mcp) |

Close an engine you launched with `engine_stop` when done, after saving. Leave engines
the user started alone.
