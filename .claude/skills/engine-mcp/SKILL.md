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
- Lights: up to 4 directional lights; point and spot lights have no practical limit
  (4096, clustered, so each surface only pays for the lights reaching it; set a `range`
  so they stay local). Shadowed point and spot lights (`cast_shadow`) share one shadow
  atlas, ranked by `shadow_priority` (point_light/spot_light field, default 1) x size on
  screen x brightness: nearer lights get bigger tiles, lights past the budget go
  unshadowed. `entity_get` shows a light's `shadow` state (`shadow_map` with `tile_size`
  and `rank`, `ray_traced`, `out_of_view`, `over_budget`); `engine_stats renderer.lights`
  the totals (shadowed, over_budget, atlas size and use) and `shadow_views_deferred` the
  shadow views waiting for the per-frame update budget. Graphics settings
  `shadow_atlas_size` (1024-8192) and `shadow_updates_per_frame` (default 48) trade
  shadow room and freshness for cost.

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
  `viewport_preview` keeps a shape on across several calls; `playtest` takes `aspect` too.
  A screenshot camera other than the current view is a camera cut: TAA and the path tracer's
  denoiser restart, so one frame looks noisy or soft. Add `settle_frames: 32` (more for path
  tracing) to hold the pose that long before capturing; nothing in the scene moves. Use
  `visual_capture` to compare settings. `screenshot {points: [[x, y]], rect: [x, y, w, h]}`
  also returns the final displayed 8-bit values there (after tonemapping, AA, grain and
  dither), with mean and standard deviation over `rect`, in the captured image's pixels. `camera_set` edits the game camera entity
  while there is one; prefer a screenshot pose to look around. **Look at the screenshot after visual changes** instead of assuming they worked.
- `environment_set` controls background, ambient light and HDR sky. The scene renders
  in HDR; exposure (manual or auto), tonemapping, colour grading, bloom, SSAO,
  screen-space reflections (`ssr`: glossy reflections on smooth, low-roughness surfaces),
  vignette, chromatic aberration, film grain, sharpening, FXAA/TAA and upscaling
  (`upscaling.render_scale` below 1 renders the scene smaller and upscales it with AMD
  FSR 3.1 by default, `method: "taau"` for the cheaper engine upscaler) live on
  `post_process_volume`
  entities. `postprocess_set` edits the scene's infinite volume (creating it if needed);
  for an area with its own look, spawn a `post_process_volume` with a box or sphere
  shape, higher `priority` and a `blend_distance`. `postprocess_get` shows the blended
  result at the camera and each volume's influence.
- Local exposure (a volume's `local_exposure` group: `enabled`, `shadows` 0.5, `highlights`
  0.25, `detail` 1) varies exposure across the frame like an eye adapting to where it
  looks: a bilateral grid of the image's brightness gives each region's brightness (regions
  end at edges, so no halos), which moves toward middle grey by `shadows` or `highlights`
  while pixels keep their detail. Use it when one exposure can't show both a sunlit area
  and the dark corners beside it (an interior lit through windows): auto exposure's
  `min_ev` only helps when the whole view is dark, and lifting shadows in the grading
  washes everything out. `engine_stats` `renderer.gpu` shows `local_exposure` (about
  0.015 ms at 1045x687).
- The player's graphics settings (the engine's settings page: Escape in games, Window >
  Graphics settings in the editor) decide anti-aliasing, upscaling and the ray-traced
  effects over any volume: `graphics_get` / `graphics_set {anti_aliasing, render_scale,
  upscaler, reflections, ...}` read and change them (saved per game and user, so set
  them back after testing), and `postprocess_get`'s `rendered` shows the result. To
  compare those effects without changing the player's choice, use `visual_capture`
  variants, which apply last. `show_fps` draws the yellow FPS readout (on by default).
  `ray_tracing: false` is the master switch: every effect takes its raster version and
  light probes show their bakes, which is how a GPU without ray tracing (or the web) sees
  the scene; the individual choices come back with `ray_tracing: true`.
- Ray-traced global illumination (`graphics_set {global_illumination: "ray_traced"}`, or a
  volume's `global_illumination` group with `half_resolution`, `bounces`, `max_history`) sits
  between raster and path tracing: the main pass keeps its direct light and ray-traced
  shadows, the reflection passes keep the specular light, and the path tracer traces only the
  indirect diffuse light (bounces, emissive surfaces, the sky through openings), denoised and
  added on top. The environment's diffuse light and flat ambient leave the main pass then (no
  more sky light through walls), and SSAO stands down. Path tracing wins when both are on.
  `engine_stats` `renderer.global_illumination` confirms a frame used it; `renderer.gpu` splits
  out `global_illumination` (the trace), `gi_denoise` and `gi_composite`. On the chess study at
  about 1045x687 it costs about 1.1 ms against path tracing's 6.5 ms, and the image comes out
  close to the path-traced one. Like path tracing, its bounce rays reuse last frame's image,
  so let a still view settle for a second before judging; switch it off when done.
- Light probes give raster lighting bounced light and keep the sky out of interiors (works
  on the web): `entity_spawn {kind: "light_probe_volume", position, light_probe_volume: {size:
  [x, y, z], spacing}}` with the box fitted inside the room or level (centred on the entity,
  turning with it; 0.5 m spacing suits rooms; selecting it in the editor outlines where the
  probes sit). Up to 8 volumes: one per room, or a finer one inside a big one (where they
  overlap the higher `priority`, then the smaller volume, wins). Surfaces inside take diffuse and rough specular
  environment light from the probes, which see walls. With ray tracing the probes update live
  (`engine_stats` `renderer.gpu` `light_probes`, about 0.6 ms for 850 probes); for web builds
  and GPUs without it, `light_probes_bake` (all volumes in turn, or `entity`; high quality, a
  few seconds each; then `wait` the frames it reports) saves `content/lighting/<volume>.probes`
  and sets each volume's `baked` path: `project_save` keeps them. Bake again after moving the volume or changing the room's lighting.
  `light_probes_status` shows the volumes, bake progress and queue, the last bake's outcome
  and what each shows; `use_baked: true` previews the bake in the editor; `light_probes_debug {show: true}`
  draws a sphere per probe lit by its data (dark red: inside geometry, yellow rim: relocated);
  `texture_fetch probes.irradiance` / `probes.distance` show the probes' data. Probes inside or
  against walls move out of them (`relocate`, default on, within 0.45 spacing). In the editor
  the volume's inspector section has the same: Bake (Preview or High), progress, Show probes.
- Path tracing (`graphics_set {path_tracing: true, path_tracing_bounces, path_tracing_samples,
  path_tracing_denoiser}`, or a volume's `path_tracing` group) replaces the raster lighting of
  opaque surfaces with denoised path-traced light; it needs `engine_info`
  `surface.path_tracing`. `engine_stats` `renderer.path_traced` confirms a frame used it, and
  `renderer.gpu` splits out `path_tracing` (the trace), `caustics` and `denoise`.
  Caustics (`path_tracing_caustics`, on by default) trace photons from point, spot and
  directional lights through glass (`transmission`) and off smooth metals (roughness
  <= 0.2) onto diffuse surfaces; they show best on light, matte receivers. Glass casts a
  shadow while they are on. Bounce rays reuse last frame's finished image where it shows
  what they hit (a radiance cache), so look at a still view for a second before judging.
- Debugging rendering: `texture_fetch` (no arguments) lists every render target (G-buffer
  normal/motion and position, depth, the HDR scene before tonemapping, shadow-map layers, the
  ray-traced shadow mask, SSAO, reflections, TAA, the path tracer's signals, history and caustic
  sums) with what each channel holds; `texture_fetch {texture, rect, points, channels, mode}`
  returns per-channel min/max/mean and NaN/inf counts, raw values at points and an image
  (`mode: "tonemap"` for HDR colour, `"false_color"` for one channel such as depth or a history
  length). It reads what the last frame left and changes nothing on screen; prefer it to adding
  debug code. Native builds only.
- Shader edits need no rebuild: an engine built from this repository watches `src/shader` and
  rebuilds the main pass, post-processing (including the path tracer and RT passes) and RT
  shadows when a `.wgsl` file is saved (`shader_reload` does it on demand and returns what
  changed). A shader that fails to compile changes nothing: the error is in `logs` (target
  `shader`) and the old pipelines keep running. `not_reloaded` lists changed files used only by
  passes that are not rebuilt (shadow maps, skinning, particles, environment, HDR output);
  those, and any Rust change, need `engine_launch {restart: true}`. Save the project first:
  a restart loses unsaved edits.
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
  `bevel_segments` soften profile corners; `thickness` makes a hollow vessel with an inner
  wall (a cup, glass, bowl, vase, or a bell when the profile ends at radius 0). `mesh_extrude
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
- Asset library (`docs/asset_library.md`): characters, animations and models shared by
  every project (Quaternius CC0 packs, Mixamo, the user's own), outside projects and git.
  `library_search {query, kind: "animation"|"character"|"model", tags}` finds them (with
  duration, root_speed, in_place, skeleton and a thumbnail path you can Read);
  `library_add {ids, entity}` copies animations into `content/animations` onto that
  character's animator, or imports characters/models into the scene. Clips work across
  rigs (Mixamo, Unreal-mannequin and Rigify names bind by humanoid role; both rigs in
  T-pose). `library_fetch {source: "mixamo", names}` downloads named Mixamo animations
  (the user's token; files never committed; `mixamo_catalog {query}` finds names, or
  `{characters: true}` lists the 108 characters), `library_ingest {paths}` adds files.
  Prefer the library over generating or importing animations by hand.
- Exports carry only what the game uses: `project_content` shows the files a build takes
  (from the main scene's references and scripts' string literals) and what it leaves
  out. A script building asset paths at run time must name the folder as a literal.
- Playable characters (`docs/characters.md` is the full guide): FBX/.blend/OBJ files
  import through Blender (`asset_import` returns `converting: true`; watch `logs`), so a
  Mixamo character FBX imports as a model and its "Without Skin" animation FBXs as clips:
  `asset_import {path, entity: "<model> (Instance)"}` adds them to that animator, named
  after the file. Then `character_make {entity: "<model> (Instance)", script: true}`
  makes a `Player` with a sized `character_controller`, a generated `animation_tree`, a
  spring camera on the active camera and `content/scripts/player.lua` (third-person
  controls); read its `notes` (a floor without a collider, a turned model). The
  `animation_tree` (Godot-style: states with a clip or 1D/2D blend space, transitions with
  conditions such as `"!grounded && air_time > 0.15"`) is fed `speed`, `forward_speed`,
  `side_speed`, `grounded`, `landing_speed` and the `jumped`/`landed` triggers by the
  controller; `entity_update {animation_tree: {generate: true}}` rebuilds it from clip
  names, `{states, transitions, parameters}` replace parts, and `entity_get` shows the
  live state in play mode. Lua: `animation_set`, `animation_trigger`, `animation_state`.
- Physics (play mode, Rapier): give scenery `collider: {}` (auto-fits primitives and
  meshes; `include_children: true` on an imported level's root takes its child meshes),
  props `rigid_body: {mass}` too, moving platforms `rigid_body: {kind: "kinematic"}` (a
  script moves the transform), trigger volumes `collider: {sensor: true}` (scripts get
  `on_trigger_enter(self, other)`). A player is a character root with
  `character_controller: {}` (origin at the feet, model facing +Z) and a camera with
  `spring_camera: {target: "<name>"}` plus `active_camera: true`; Lua steers it with
  `character_move` / `character_jump` and reads `character_state`. The engine's
  third-person controller is a built-in module: `require("engine.third_person")` in any
  project (`scheme = "mmo"` gives World of Warcraft mouse controls; lua_api_reference
  "Third-person controller"). `debug_physics
  {enabled: true}` draws every collider; the editor outlines the selected one. The
  simulation is rebuilt when play starts, so check physics in play mode (`playtest`).
- Navigation (`docs/navigation.md`): `entity_spawn {name: "NavMesh", nav_mesh_surface: {}}`
  then `navmesh_bake` (bakes the scene's static geometry, floors over floors stay separate;
  saved under `content/navigation/`, `project_save` keeps the path). Mark what nobody walks
  on with `nav_modifier: {walkable: false}` (water, roofs) or `{ignore: true}`, moving
  props with `nav_obstacle: {}` (steered round, carved once still), jumps and drops with
  `nav_link: {end}`. Characters get `nav_agent: {speed}`; Lua sends them with
  `nav_set_destination` and gets `on_nav_arrived`; an animation tree on an agent is fed its
  speed. `navigation_debug {mesh, agents, paths}` draws it in screenshots,
  `navigation_path {from, to}` tests a path, `navigation_agents` lists states (play mode).
- AI (`docs/ai.md`): `entity_update {ai_agent: {brain, tags, group, perception: {sight_range}}}`
  gives an entity a brain (a behaviour tree or state machine a script defines with
  `ai_tree` / `ai_machine`, or one from `ai_define {name, brain}` as JSON, or a project JSON
  file). Brains think in play mode. `ai_status` lists every agent's state and action,
  `ai_inspect {entity}` shows its blackboard, tree nodes (last results, utility scores),
  machine transitions with history, and senses. Steer with `ai_send {entity, event}`,
  `ai_set {entity, key, value}`, `ai_noise {position, radius, tag}`, `ai_control {entity,
  restart, paused}`; `ai_debug {all: true}` draws view cones, goals, percepts and state
  rings in screenshots. Lua errors in brains show in `ai_status` (error) and the log.
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
- `visual_compare {a, b}` compares two saved captures (PNG files, or `visual_capture`
  directories; `variant` picks one) with the same metrics, without rendering: the way to
  compare builds or sessions. Capture the same view in each with `visual_capture
  {freeze_noise: true}` (holds TAA jitter, grain/dither and live light-probe updates still, so
  an unchanged view renders identically; overlays are left out by default) or with
  `render_freeze_noise` + `screenshot {save_path, overlays: false}` (`overlays: false` drops
  the FPS readout and editor grid; the frame is taken one frame later). Turn
  `render_freeze_noise {enabled: false}` back afterwards: it slightly reduces anti-aliasing.
- `profile_capture {frames}` captures percentiles of CPU frame/stage times and of the GPU
  time per render stage (`gpu_stages_ms`, `gpu_total_ms`, from timestamp queries; with
  `light_culling`, the clustered lights' compute pass), the renderer counters' min/max/last
  (`renderer_counters`: draw calls, shadow views rendered/cached, culled instances), script cost and asset growth. `baseline` compares a
  previous report. Prefer its GPU percentiles over one `engine_stats` reading when another
  process shares the GPU.
- `engine_stats {history: N}` adds one row per frame for the last N frames (draw calls,
  shadow views rendered/cached, culled instances, GPU stage times), oldest first: use it to
  see the frame where a change applied (remote edits show from the next frame on) instead
  of racing a single reading; `compact: true` drops the notes. `wait {until_frame}` waits for
  an absolute frame number.
- `engine_profile_cpu {seconds, top}` samples the engine with Linux `perf` and returns the
  hottest symbols (self time by shared object; `callers: true` attributes to callers too),
  for when `render_gpu` or `ui_build` look high. `pid` samples an engine not launched here.
- `renderdoc_capture` (engine started with `engine_launch {renderdoc: true}`) captures the
  next frame and returns an `.rdc` path; open it with the `renderdoc` MCP server
  (`open_capture`, then `get_frame_overview`, `get_pass_timing`, `get_draw_call_state`,
  `disassemble_shader`, `pixel_history`, `debug_shader_at_pixel` / `debug_compute_thread`).
  Use it when a screenshot or `texture_fetch` shows something wrong but not why, or to see
  which pass or draw costs GPU time. Restart without `renderdoc` before measuring frame
  rates; delete captures (tens to hundreds of MB) when done.
- `engine_launch {binary, window_size}` runs a binary built elsewhere (a baseline commit in a
  worktree, see the test-bed notes) with the same port handling and `ENGINE_SHADER_RELOAD=0`,
  and opens the window at `[width, height]` (`ENGINE_WINDOW_SIZE`), e.g. `[800, 450]` to
  keep test windows light.
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
  It refuses while playing; stop first. Unchanged files are left alone and each scene file
  keeps its compact or indented layout, so a save's diff is the real change.
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
  may share the GPU and swing the frame rate. Over many frames use `profile_capture`'s
  `gpu_stages_ms` percentiles; `engine_stats {history: N}` shows the last N frames.

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
