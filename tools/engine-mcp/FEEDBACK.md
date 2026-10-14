# Engine MCP: bugs, pain points and wish list

Collected while building a Breakout game end to end through the MCP
(`games/breakout`, 2026-10-13). Add new findings at the bottom of the matching section.

## Status

First round of fixes: commit `7f0fd5f` (branch `fix/engine-mcp-feedback`, 2026-10-13).
Every bug is fixed and was checked against a running editor and player.

Second round (branch `feat/engine-mcp-feedback-2`, 2026-10-13): every remaining pain
point and wish-list item is done (audio, Lua meshes/materials/lights/cameras,
world-space text, non-glTF import, texture reload, game-camera screenshots, bulk delete,
scene diff, playtest, time scale, batch), and web builds run Lua and play audio.
Breakout now plays its sounds and shows score popups.
Checked in the editor and the player, partly through a fresh MCP server binary. Each entry below
ends with a **Resolution** (bugs), **Done** / **Partly done** / **Open** (pain points) or a
✅ / ⬜ mark (wish list).

Third round (2026-10-13): a second game, `games/neon-rift` (a top-down arena shooter with an
attract mode, driven by one Lua controller that spawns its own entity pools). Its findings
were one bug (B10), the pain points from "Lua cannot delete entities" onward, and the
wish-list items that were ⬜. What worked well: `lua_eval` with a
global `GAME` table, `runtime_set {time_scale: 16}` soak tests (300 s of game time in about
20), `batch`, `entity_duplicate` and bulk `entity_update` for the arena, accurate Lua error
line numbers, and `input_send` down/up events for human-control tests.

Fourth round (2026-10-13): nearly everything the third round found is done (B10, despawn,
screen/world conversion, HDR emissive, text size and color, sound summaries, the cone
docs, player script hot reload, screenshot aspect and pose, guide sections, server
staleness, log filtering); what is left is marked ⬜ or **Partly done**. Checked live in the
editor and the player with Neon Rift, and by driving a freshly built MCP server over stdio.
Two things the checking turned up: `emissive` did nothing at all without an emissive
texture (the shader only added it for textured materials), and entities spawned and
parented in one Lua hook got a `Parent` link but no entry in the parent's child list.

Fifth round (2026-10-13): a third game, `games/robotron` (Robotron 2084: a twin-stick arena
shooter with grunts, hulks, brains, spheroids, quarks, tanks and a family to rescue). It boots
into an attract mode where an autopilot plays the game, so it can be watched and soak-tested
without a human. One Lua controller (about 1700 lines) spawns and pools every actor. What worked
well: `time_scale 16` soak tests with a cumulative `GAME.stats` table (500 to 700 s of game time
per minute of waiting), `lua_eval` hooks through a global `RT` table (`RT.jump(wave)`,
`RT.spawn(kind, x, z)`, `RT.god(true)`, `RT.units()`) to look at one enemy type without
playing up to it, `time_scale 0` plus `screenshot {camera: {position, target}}` to inspect a
frozen frame from any angle, script hot reload inside the running player, `input_send` mouse
and key events for the human-control path, `entity_delete {name_contains}` and `batch`
to rebuild the arena. The findings below are B11, B12 and the pain points from "`project_create`
does not create the scripts folder" onward; the new wish-list items are at the end of that section.

Sixth round (2026-10-13): everything the fifth round found is done or partly done (marked below),
including B11 and B12. Checked live in the editor through a freshly built MCP server over stdio
(a scratch project with a modular Lua game), and Robotron in the player for regressions (no
warnings, 7.8 ms frames). Two things the checking turned up: GPU particles only ever ran in the
editor (the runtime lived in the editor binary), and a stalled frame (a screenshot readback in a
debug build) aged every particle at once, so whole bursts vanished between screenshots. Not
checked live: script particle bursts in the player (same runtime as the editor; the player only
gained the hooks), and the web build beyond `cargo check` (it compiles with no new warnings).

Seventh round (2026-10-13): Robotron's visual overhaul (voxel models of the 1982 sprites, about
4,000 entities on the 60-grunt wave) turned up four engine bugs, all fixed here: B13 to B16. In the
release player that wave runs at 5.7 ms frames (Lua 0.46 ms); in the debug editor it went from
7 fps to about 20 fps once B15 and B16 were fixed.

MCP-server changes (new tool parameters, blocking project tools) only take effect after the
MCP server is restarted; engine changes take effect after `engine_launch` rebuilds.

Authoring extensions (2026-10-14): added explicit seeded simulation steps and playtest
assertions/scenarios, world-AABB queries and constrained placement/scatter, scene transactions
with dry runs/aliases/single undo, prefab source authoring/inspection, CPU profile captures,
saved-project validation, web packaging/browser smoke tests, sweeps and heightfields.
MCP responses now include structured content, output schemas and behavior annotations.
Verification exposed an existing prefab persistence gap: linked child nodes were omitted
from project export and origin metadata was lost in play/undo snapshots. Both now round-trip,
including placement and root visibility; tree placement also preserves prefab transforms.
Updating a prefab from another selection keeps the source root name stable. Checked through
the real MCP server in an isolated editor, saved/reopened projects and a WebGPU browser;
Rust tests, Clippy and editor/Robotron GPU smoke checks pass. Scope limits are in the README:
spatial queries use boxes, explicit steps cover Lua/scene animation, profiling measures CPU,
and existing instances reload new sources when recreated or reopened.

How to read the sections:

- **Bugs**: something behaves wrongly or contradicts the docs. Each entry has a status:
  *confirmed* (reproduced, and the cause read in source or checked directly), *observed*
  (seen, cause not investigated) or *fixed*.
- **Pain points**: works as designed, but cost time or tokens.
- **Wish list**: features that would have removed a pain point.

## Bugs

### B1. Play -> stop restores the scene *after* the first play frame's scripts ran (fixed, engine)

`project_save` then writes the damage to disk.

- Repro: attach a script whose `on_created` or first `update` moves, scales or hides an
  entity, `runtime_set play`, `runtime_set stop`, then `entity_get` the entity. It still has
  the first-frame values. `project_save` persists them. My bricks ended up at scale 0.001,
  z=5 in the saved scene, and the editor showed an empty playfield; the ball trail spheres
  were saved hidden for the same reason.
- Cause: `Scheduler::run_update_stage` (`src/app/core/scheduler.rs:353`) runs
  `scene.update_with_mode(dt, false)` (Lua `on_created` + `update`) *before* the update
  systems. The editor takes its snapshot in one of those systems
  (`detect_mode_transition`, `src/bin/editor/application/runtime_mode.rs:32`), so the snapshot is
  taken after the first play frame's scripts have already run, despite the comment
  "saves state BEFORE any animation changes".
- Fix idea: capture the snapshot before `update_with_mode` on the frame the mode flips
  (detect the transition in the scheduler, or run the detection first).
- Workaround in `games/breakout/content/scripts/breakout.lua`: do nothing on the first play
  frame and start the game on the second.
- **Resolution:** the runtime controller already snapshots the scene before the first play
  frame; the editor then restored its own late snapshot over it. The editor's snapshot is
  gone, so stop restores the pre-play scene. Verified: a script that moved a cube to z=5 and
  scaled it to 0.001 on its first frame left it at its original transform after stop. The
  breakout first-frame workaround is no longer needed.

### B2. `cast_shadow` on a mesh is accepted but does nothing (fixed, engine + MCP)

- `CanCastShadow` is only read for lights (`src/scene/internal/lights.rs`, `shadow_enabled`);
  every mesh casts shadows. `entity_spawn`/`entity_update` still accept `cast_shadow: false`
  for a mesh, report success, and `entity_get` shows `"cast_shadow": false`.
- Repro: spawn a cube next to a shadow-casting directional light, set `cast_shadow: false` on
  it, screenshot. The shadow is unchanged.
- Fix idea: reject or warn on `cast_shadow` for non-light entities in the MCP handler, or
  make the shadow pass honour the flag on meshes.
- **Resolution:** the shadow pass now honours the flag on meshes (a per-object `NO_SHADOW`
  material flag set at render time). The tool schema documents what it means for lights and
  meshes. Verified by screenshot: the flagged cube casts no shadow, its neighbours do.

### B3. Entity ids are renumbered after play -> stop (fixed, engine)

- After the first play/stop, `Ball` and `Paddle` swapped ids and every later id shifted
  (`Bricks` 4294967305 -> 4294967314). The docs say ids are the same handle Lua uses and
  tools hand them back for reuse, so ids held from before a stop silently point at other
  entities. Names stayed correct.
- Fix idea: preserve ids on restore, or document that ids are invalid after a stop and have
  the tool results say so.
- **Resolution:** scene snapshots remember each entity and respawn it with the same id, so
  ids survive play/stop and undo. Verified live and covered by a unit test
  (`snapshot_restore_keeps_entity_ids_and_adds_no_materials`).

### B4. Entities gain a default `material` component after play -> stop (fixed, engine)

- After stop, empties, lights, the camera and the Environment entity list a `material`
  component (handle 0, `shared_by` 6) that they did not have before play.
- Impact unknown (no mesh, so nothing renders), but it changes what `scene_list_entities`
  reports and probably what gets saved.
- **Resolution:** instantiating a scene asset only resolves a material for entities that have
  a mesh or material data. Fixes restores and loading saved scenes. Verified live and in the
  saved scene file.

### B5. `camera_set` returns a non-orthogonal `up` vector (fixed, MCP/engine)

- `position [0,-0.2,21]`, `target [0,-0.2,0]` returns `forward [0,0,-1]` but
  `up [0, 0.8944, -0.4472]`, which is not perpendicular to forward. Rendering looked right,
  so it is probably a stale pitch term in the reported value.
- **Resolution:** `camera_get`/`camera_set` report the up vector the view actually uses,
  re-orthogonalised against forward. Verified.

### B6. Editor viewport renders the scene dimmer than the player (fixed, engine)

- Same project, same environment and grading: the editor's Scene/Game view was visibly
  darker and less saturated than the standalone player (an unlit white ball looked light
  grey in the editor and white in the player). I tuned the look against the player.
  A screenshot from the editor is not a reliable preview of the game.
- Suspect a double sRGB conversion or a missing tone-mapping/grading step in the editor's
  viewport path. Not investigated.
- **Resolution:** the cause was bloom, not grading or sRGB. The editor draws the scene into a
  sub-region of full-window post-processing targets; SSAO accounted for that, bloom did not,
  so its blur covered a larger share of the smaller view and sampled outside it. Bloom now
  scales its steps by the region and clamps samples to it. Verified with exposure 1.6,
  saturation 1.4, contrast 1.1: editor game view and player give identical pixels.

### B7. `lua_api_reference` is wrong or incomplete (fixed, docs)

- `get_component(e, "Transform")` is described as returning `{x, y, z}`-style tables. It
  returns arrays (`translation[1..3]`, `scale[1..3]`, `rotation[1..4]`). `get_world_translation`
  does return `{x, y, z}`. Cost me one failed `lua_eval`.
- `ui:anchored_panel(id, anchor, offset_x, offset_y, width, height, callback)` (HUD overlays)
  is not documented; it only turned up in `examples/scripts/ingame_hud.lua` and the source.
- It says "Script Access API ... is not yet implemented" while listing its functions
  (`get_entity_scripts`, `read_script_source`, `reload_script`).
- **Resolution:** `scripts/LUA_SCRIPTING_README.md` now says component vectors are arrays,
  documents `anchored_panel` (anchors, offsets, nil sizes) and the layout helpers, drops the
  stale "not implemented" line, lists the real limitations (no mesh/material/light creation,
  no audio) and documents the new `is_playing()`.

### B8. Player build shows editor-style UI over the game (fixed, questionable)

- `engine_launch {target: "player", project}` shows a File/Window menu and a Scene Hierarchy
  panel beside the game. Not sure that is intended for the player. Script `on_ui` HUDs did
  not appear there, so in-game text is unavailable in the player either way.
- **Resolution:** the player hides the default debug UI and draws scripts' `on_ui` output:
  anchored panels as HUD overlays, other widgets in windows. Anchored panels now have a dark
  backdrop and grow to fit their text (the old ones were plain grey text, nearly invisible
  over a scene). Verified in the player.

### B9. `set_component` (and `remove_component`) reject entities spawned in the same hook (fixed, engine)

- Repro: `local e = spawn_primitive("cube", "X"); set_component(e, "Visible", false)` in
  `on_created`. It raises `entity handle is not yet available` and aborts the hook, while
  `set_material`, `set_scale`, `set_translation`, `look_at`, `add_component` and `set_parent`
  work on the same handle in the same hook.
- Cause: `src/scripting/lua/commands.rs`. Those commands check `self.pending` first;
  `set_component` (line 178) and `remove_component` (line 216) go straight to
  `resolve_entity_bits`, which errors for a registered-but-unresolved handle. Same class of
  bug as the `look_at`/`translate`/`rotate` fix in the lights and cameras commit.
- Workaround: `add_component(e, "Visible", false)` works on the pending entity (Visible and
  CanCastShadow are add-supported), or finish the entity on the first `update`.
- Fix idea: make `set_component` write into `entry.components` for a pending entity, as
  `add_component` does, and make `remove_component` drop the key. Document which commands
  accept a just-spawned handle.
- **Resolution:** done as suggested. `set_component` (setting and adding are the same
  operation) stores the value with the pending entity, and `remove_component` drops a
  component queued for it earlier in the chunk, so a set followed by a remove never adds it.
  Unit test `set_and_remove_component_accept_entities_spawned_in_the_same_frame`; all 72
  scripting tests pass. Checked in the editor: `spawn_primitive` followed by
  `set_component(e, "Visible", false)` and `set_component(e, "CanCastShadow", false)` in one
  `lua_eval` chunk leaves the entity with `visible: false` and `cast_shadow: false`, and Neon
  Rift's pool creation now uses `set_component` directly (the deferred-finish workaround is
  gone). Still undocumented: the Lua guide does not list which commands accept a handle
  spawned in the same chunk.

### B10. `screenshot {camera: "game"}` returns the whole editor window by default (fixed, engine)

- With `camera: "game"` and no `target`, the result was the full editor window with the
  free-camera viewport inside it. Adding `target: "viewport"` gives just the game view. The
  tool description says the viewport is the default. Not investigated. The editor's ground
  grid also showed outside the arena in game-camera renders.
- **Resolution:** the engine's default target was `window` while the tool schema promised the
  viewport. A capture with `camera` (or `aspect`) now defaults to `viewport`, and the schema says
  what each default is. The editor grid is an editor-only overlay that was drawn into every frame
  regardless of the camera; the renderer now has an `editor_overlays` flag that is off for
  frames rendered through another camera. Verified: a game-camera capture is the 1045x688
  viewport with no grid outside the arena.

### B11. Despawned entities leak their material assets (fixed, engine)

- Repro, in a running player with `time_scale` 0 for stable counts: `engine_stats` shows
  `assets.materials` 2907. A Lua chunk spawns 100 primitives (`spawn_primitive("cube", ...)`):
  3007. Despawn all 100 with `despawn_entity`: still 3007, and `entities` drops by 100. Each
  `spawn_primitive` allocates one material and nothing ever releases it.
- It also holds across play -> stop: an idle editor with 40 entities reported 336 materials after
  a few play sessions (observed, not isolated). One 45 s soak at 16x of Robotron ended with 7339
  materials for 756 entities, because the script despawned surplus pooled units between games.
- Effect: a script that creates and despawns entities over time (effects, wave enemies, bullets
  that are not pooled) grows asset memory without bound, so the only safe pattern is "spawn once,
  hide forever", which in turn keeps every entity's frame cost (see "Hidden entities still cost
  frame time"). Robotron trims its pools only when a new game starts and says why in a comment.
- Fix idea: reference-count materials by the entities that use them and free the slot when the
  count reaches zero (also when play mode stops and the scene is restored).
- **Resolution:** asset caches can release slots (reused by later inserts), and the scene
  releases inline PBR materials no entity in any of its worlds uses (file-backed and shader
  materials are kept). It sweeps after Lua despawns (once the frame's mesh/material commands
  have run), after `entity_delete` and after play -> stop. `engine_stats` and `assets_list`
  count live materials. Verified: 100 `spawn_primitive` cubes took materials from 2 to 102;
  despawning them brought it back to 2. Unit test `released_slots_are_reused`.

### B12. `Visible = false` on a parent does not hide its children (fixed, engine)

- Repro: `local root = spawn_entity("R"); local c = spawn_primitive("cube", "C"); set_parent(c, root);
  set_component(root, "Visible", false)`. The cube still renders. An actor made of a root and
  four parts (Robotron's hulk) needs five `set_component` calls to hide,.
- Fix idea: make visibility inherited in the renderer (an entity is drawn only if it and all its
  ancestors are visible), as every other engine does, or document that it is not and add
  `set_visible_recursive(entity, bool)` to the Lua API.
- **Resolution:** visibility is inherited when the renderer collects meshes (an entity is
  drawn only if it and every ancestor are visible), and hidden entities are now skipped before
  any per-entity work (no name clone or material lookup). Verified by screenshot: hiding a
  parent hid its child cube. Unit test `hidden_parent_hides_its_descendants`; documented in
  the Lua guide's Hierarchy section.

### B13. A bare `spawn_entity` strands everything parented below it (fixed, engine)

- Repro: `local g = spawn_entity("Group"); set_parent(g, root); local c = spawn_primitive("cube");
  set_parent(c, g)`, never moving `g`. `c` renders at its *local* position near the origin,
  whatever `root` does. Robotron's walk-frame nodes (one per animation frame of each actor)
  left a pile of legs at the arena's centre.
- Cause: script-spawned entities only got a `TransformComponent` when the script set a
  translation, rotation or scale in the same hook, and transform propagation skips an entity
  without one, along with its whole subtree.
- **Resolution:** every script-spawned entity gets a transform (identity unless set). Unit test
  `a_bare_spawned_group_node_carries_its_children_along`.

### B14. Directional shadows vanish when looking through a game camera (fixed, engine)

- Repro: a game camera 45 m above the floor looking down at it; the sun casts shadows in
  editor close-ups but none through the game camera.
- Cause: the shadow area is centred on the camera's target, and a game camera's target is the
  eye plus one metre, so the shadow box hung in the air in front of the camera.
- **Resolution:** for such a camera the shadow area centres where its view meets the ground
  plane (y = 0), or half the shadow area ahead when it looks level; an editor orbit target is
  used as before. Unit test
  `directional_shadow_of_a_game_camera_centers_where_it_looks_at_the_ground`; checked by
  screenshot through Robotron's camera.

### B15. The hierarchy panel lays out every entity every frame (fixed, editor)

- Repro: play a game that spawns about 1,000 entities; `engine_stats` shows `ui_build` at
  128 ms per frame (debug editor), with every row of the expanded tree laid out.
- **Resolution:** the tree is flattened into the rows whose ancestors are expanded and drawn
  with `ScrollArea::show_rows`, so only the rows in view cost anything. Selecting an entity
  still scrolls its row into view.

### B16. One debug warning per entity spawned, flooding the console (fixed, engine)

- Repro: spawn 1,000 entities from a script in a debug build: 1,000 lines of
  `Entity 'RT_v' using LOCAL transform (no WorldTransform)`, and the console laying out its
  whole 1,000-line buffer each frame kept `ui_build` at about 38 ms after B15.
- **Resolution:** reported once per run. (Most of these were B13; the rest are entities drawn
  on the frame they were spawned, before propagation, which is harmless.)

## Pain points

- **No bulk creation.** A grid of 72 bricks costs 72 `entity_spawn`/`entity_duplicate`
  calls, because Lua cannot add meshes or materials and there is no batch tool.
  **Done:** `entity_duplicate {count, step, row_step, columns}` lays out a row or grid in one
  call and returns only ids and names.
- **The engine has no audio at all.** No `play_sound` in Lua and no audio crate in
  `Cargo.toml`, so "add sounds" cannot be done from the MCP side. **Done:** `rodio`
  playback (wav/ogg/mp3/flac) with Lua `play_sound`/`stop_sound`/`stop_all_sounds`/
  `set_master_volume`, MCP `sound_play`/`sound_stop`/`audio_status`. Sounds stop with
  play mode. A play history lets agents check which sounds played. Web builds play
  through the Web Audio API (see the end of this file).
- **Lua cannot add or change meshes, materials or lights.** Anything that varies at
  runtime (brick colours, damage states) needs one pre-made entity per variant. **Done**
  for meshes and materials: `spawn_primitive`, `set_mesh`, `set_material_color`,
  `set_material` (copy-on-write for shared materials). Lights and cameras since
  2026-10-13: `spawn_light`, `set_light`, `spawn_camera`, `set_camera`,
  `set_active_camera`, sharing the field parsing of `entity_update`. `look_at`,
  `translate` and `rotate` now also work on entities spawned in the same chunk, which
  is how a new light or camera gets aimed (they used to fail with "entity handle is not
  yet available"). Checked in the editor: lights, shadows, camera switching, and stop
  removing play-mode lights and cameras and restoring the camera.
- **HUD text exists only in the editor.** `on_ui` needs egui, so the standalone `player`
  shows no score, and there is no world-space text. **Done:** HUDs work in the player,
  and `ui:world_label(x, y, z, text, opts)` draws text at a world position (projected
  through the view camera; drawn over the scene, not occluded).
- **`project_create` returns `queued`.** I had to `wait` and poll `editor_state` to learn the
  project had opened. **Done:** `project_create` and `project_open` return once the project
  is open (`project_create` also returns the editor state).
- **No way to bring in non-glTF assets through the MCP.** `asset_import` is glTF only. PNGs
  (and WAVs) are written with plain file tools and referenced by absolute path. **Done:**
  `asset_import` copies images, audio, fonts, `.lua` and `.wgsl` into
  `content/<kind>/` (or `dest`) and returns the project-relative path.
- **Lua has no "am I playing?" query.** That is why `@tool` versus unannotated scripts needs
  care when a script has both game logic and `on_ui`. **Done:** `is_playing()`.
- **Texture files are cached by path.** Rewriting a PNG on disk and re-assigning the same path
  shows the old image; I had to write the new image under a different filename. There is no
  texture reload. **Done:** textures reload into their existing slots, through
  `asset_reload {path}`, automatically when the editor sees an image in the project
  change, and when `asset_import {overwrite: true}` replaces one.
- **Edits made during play are silently lost on stop.** I changed `environment_set` while
  playing (to tune the look against a running game) and had to redo it. The result gives no
  hint that the change is temporary. A `"discarded_on_stop": true` field would help.
  **Done:** every call that changes the scene during play returns `discarded_on_stop: true`.
- **Relative script paths are rewritten to absolute ones.** `script: {path: "content/scripts/x.lua"}`
  compiled fine but is stored as `/home/.../x.lua`, so a project can't be made portable
  from the MCP. Textures are saved project-relative, so scripts are the odd one out.
  **Done:** script paths inside the project are saved relative to it and resolved against it
  on load (verified in the editor and the player).
- **Tool results are very verbose for bulk work.** Each `entity_duplicate` returns ~20 lines,
  so 66 duplicates cost ~15k tokens of output I never needed. A `quiet`/`fields` option, or
  just returning `{id, name}`, would cut this a lot. **Done:** `quiet: true` on
  `entity_spawn`/`entity_update`/`entity_duplicate`; multi-copy duplicates and bulk updates
  are terse by default.
- **Nothing to apply the same change to many entities.** Re-tuning the brick material would
  have needed 72 `entity_update` calls; Lua cannot touch materials. A `scene_list_entities`
  filter plus a bulk `entity_update {entities: [...], ...}` (or `name_contains` on update)
  would fix it. **Done:** `entity_update` takes `entities` (ids/names) or `name_contains`.
- **Viewport screenshots show the editor's free camera until `camera_set` is called**, not
  the scene's active game camera. Easy to misread as a broken scene. **Done:**
  `screenshot {camera: "game" | <camera entity>}` renders that frame through the camera
  without gizmos and without moving the editor view.
- **`lua_eval` can't see script locals.** I had to export the game state as a global
  (`GAME = g`) to poke it during tests; a documented debug convention would be nicer.
  **Done:** `lua_api_reference` documents keeping test-visible state in a global table or
  the state API.

- **Lua cannot delete entities, and `on_created` re-runs on every hot reload.** The API
  has `spawn_entity` and `spawn_primitive` but no despawn, so short-lived objects (bullets,
  sparks, enemies) need fixed pools of hidden entities, and a script that spawns its own
  pool leaks a full set per saved edit. Neon Rift names every pooled entity and reuses it
  with `find_entity_by_name`. **Done:** `despawn_entity(e)` removes an entity and its
  descendants (applied when the hook returns; idempotent; an entity spawned in the same hook is
  just never created), `entity_exists(e)` tells whether a handle is still alive, and
  `on_destroyed(self_entity, reason)` now also runs before a hot reload (`reason` is `"reload"`;
  `"removed"` otherwise), so scripts can clean up their pools. Reload hooks run at the start of
  the next frame, because they need the world. Despawn follows `Parent` links as well as child
  lists, so hierarchies built by scripts go away completely.
- **No screen/world conversion in Lua, and the viewport size exists only in `on_ui`.**
  Mouse aiming on a ground plane needs the camera ray, which means re-deriving the
  projection in Lua. It is also unclear what `get_mouse_position()` is relative to inside
  the editor viewport. Neon Rift is keyboard-only because of this. **Done:** `get_viewport_size()`,
  `world_to_screen(x, y, z)`, `screen_to_world_ray(x, y)` and `screen_to_ground(x, y[, height])`
  work in window pixels, the same space as `get_mouse_position()` (the editor's game view is a
  part of the window, and `get_viewport_size()` gives its `{x, y, width, height}`). They use the
  active camera and the area the renderer last drew into. Checked in the editor: a ground point
  projected to the screen and back lands within 0.0002 m, and the view centre maps to the point
  under the camera.
- **`camera_set` also moves the game camera entity.** In the editor the view camera drives
  the active camera entity, so looking closer at something silently overwrote the
  GameCamera's transform. **Partly done:** while the scene has an active
  camera entity the view is driven by that entity (every frame), so a view-only move cannot be
  made without deactivating it. Instead `screenshot {camera: {position, target, fov_deg}}`
  renders a one-off pose without moving anything, and the `camera_set` description says it
  edits the game camera.
- **`playtest` and `audio_status` return every sound that played.** A 5 second demo run
  gave about 65 entries (id, path, pitch, volume, time), thousands of tokens that say
  "shoot.wav many times". **Done:** `audio_status {summary: true}` groups plays by file (count,
  first and last time, volume and pitch ranges, silent plays), and `playtest` reports
  `sounds_summary` by default (`sounds: "list"` for every play, `false` for none).
- **The cone's orientation is undocumented.** "cylinder and cone lie along the Z axis" does
  not say which way. The cone's tip is at local +Z (bounds z in -1..0, base at -1), the
  opposite of the engine's -Z forward, so ships and arrows need a 180 degree yaw. **Done**
  (the Lua guide and the `entity_spawn` schema now say so).
- **Emissive is capped at 1.0 and not HDR.** A lit material with `emissive: 1` and a
  saturated base color rendered dimmer than the same color `unlit`, and only the unlit ones
  bloomed strongly at threshold 0.7. Neon looks want emissive above 1. **Done.** Cause: the
  shader only added emission for materials with an emissive texture, so `emissive` on a plain
  material did nothing (the lit material was just lit). It now glows in the base color, and the
  strength is stored as a `u16` (255 is 1.0, up to 64), so values above 1 are HDR. Saved
  materials load unchanged. Verified: emissive 0, 1 and 4 on red spheres look dark, red and
  blooming.
- **`ui:label`/`ui:heading` cannot be sized or colored.** Neon Rift draws its big title, wave
  and game-over banners with `ui:world_label` (which takes `size`/`color` and centers on the
  point), which works but is a trick. **Done:** `ui:label(text, {size, color})`,
  `ui:heading(text, {size, color})` and `ui:screen_label(anchor, x, y, text, {size, color,
  backdrop})` (text at a fixed viewport position, no panel).
- **The player target does not hot-reload script edits** (observed). The editor does; the
  running player kept the old code after an edit until `engine_launch {restart: true}`.
  **Done:** the native player polls the project's `.lua` files twice a second and
  reloads edited ones (web builds have no filesystem to watch). Checked with Neon Rift: an edit
  and its revert each reloaded the script, and the entity count stayed the same.
- **The MCP server binary can be older than the repo without any sign.** The `lua_eval`
  description I was shown still said "Lua cannot create lights or cameras", but the source
  no longer does: the server binary was built before the lights and cameras commit.
  `engine_launch` rebuilds the engine, not the MCP server. **Done:** `engine_info` has an
  `mcp_server` block with `restart_needed` (the server's sources or binary are newer than the
  process), its version and the repo's commit and dirty flag.
- **Framing is eyeballed across aspect ratios.** The editor viewport is about 1.5:1 and the
  player window 16:9, so a camera that fits one can crop the other. **Done:** `screenshot
  {aspect: "16:9"}` (or `"4:3"`, `1.5`) renders in the largest region of that shape that fits
  the viewport and crops to it.

- **`project_create` does not create the scripts folder.** `editor_state.project.scripts_dir`
  names `content/scripts`, but only `content/materials` and `content/scenes` exist, so the first
  script write fails until `mkdir -p` (same for `content/audio`). **Done:** new projects get
  `content/scripts`, `audio` and `textures`.
- **A new project is an empty scene with a game-unfriendly environment** (ambient 0.03, pale
  blue clear color, no light, no camera, bloom off). Robotron needed the same ten calls as Neon
  Rift (environment, bloom, sun, camera, floor, border). This is the open "starter scene"
  wish below; a `project_create {template: "arena"}` would remove it. **Done:**
  `project_create` builds a starter scene by default (dark backdrop, ambient 0.35, grading,
  a shadowed `Sun`, an active `GameCamera` looking at the origin, a 40 m `Floor`) and saves it;
  `template: "empty"` skips it. Bloom turned out to be on by default; post-processing is an
  engine setting, not stored in projects.
- **`playtest {eval}` and `lua_eval` without `entity` cannot see script globals.** `eval:
  "return GAME.wave"` fails with `attempt to index a nil value (global 'GAME')`, which reads as a
  game bug. The fix is `entity: "GameController"` (it is documented, but the error does not say
  so). **Done:** the error now ends with a hint naming the script(s) that define the global
  (`pass entity: "GameController"`), and `playtest`'s eval reruns inside the script when exactly
  one defines it (the result says `ran_in_script_of`). Verified live.
- **`entity_duplicate {count: 20, quiet: true}` still returns every copy's id and name** (about
  700 tokens for 20, 1400 for 40). `quiet` should return only the first and last id and the
  count. **Done** (count, first, last; ids need not be consecutive, so the note points to names).
- **`project_save` returns `{"queued": "project.save"}`** and nothing says it finished or what
  was written; I checked with `git status`. `project_create` and `project_open` were fixed to
  block; `project_save` should return the written paths. **Done:** it returns once the editor
  has saved (an error if the save failed) with the files written.
- **`screenshot {aspect: "16:9"}` crops the frame that was rendered; it does not re-lay the
  scene out.** A script that fits its camera to `get_viewport_size()` (Robotron does, so one
  arena fits 16:9 and 4:3) still sees the real viewport, the HUD is cropped away, and a "4:3"
  preview looks like a framing bug. Previewing another shape needs a real resize
  (`resize_window` or `screenshot {viewport_size: [w, h]}` that also updates
  `get_viewport_size()`). **Done:** a viewport preview (`view.preview`, MCP `viewport_preview`)
  lays the game out at an aspect or exact pixel size inside the view: the renderer draws into
  it, scripts get it from `get_viewport_size()` and HUDs (editor and player) anchor to it.
  `screenshot {aspect | viewport_size}` and `playtest {aspect | viewport_size}` turn it on for
  their duration. Verified: a [390, 640] preview made `get_viewport_size()` report 390x640.
- **The editor runs about five times slower than the player, and `engine_stats` cannot say
  why.** Idle editor, 40 entities: 26 fps (38 ms). Player, Robotron with 460 entities: 119 fps
  (8.4 ms). Lua time was 0.5 ms per frame (measured with `os.clock`), so the rest is the
  engine. Games that clamp `dt` (Robotron clamps at 34 ms) run slower than real time at 26 fps, and frame
  times measured there say nothing about the game. A breakdown (scripts, transform update, culling,
  render, UI, egui) in `engine_stats` would show where it goes. The skill should say: measure
  performance in the player. **Partly done:** `engine_stats.frame_breakdown` times each stage
  (animations, transforms, scripts, update_systems, gpu_systems, script_assets,
  render_collect, render_gpu, ui_build, ui_render, present), last frame and running average,
  and the skill says to measure in the player. Measured (debug build, scratch scene): the
  editor spends about 6.5 ms in `ui_build` and 5 ms in `render_gpu`; Robotron in the player
  sums to 7.4 ms (render_gpu 2.7, ui_build 1.6, render_collect 0.8, scripts 0.6). The editor's
  own cost was not reduced.
- **Hidden entities seem to cost frame time.** Observed in the player (noisy, not isolated): 460
  entities 8.4 ms (119 fps), about 900 entities 16 to 21 ms, about 1400 entities 22 ms; 500 extra
  `Visible = false` cubes at y = -80 read 22.6 ms, and 21 ms again after they were despawned,
  because the game's own load moves the frame time too. If it holds, a pool of "hidden" sparks
  and actors is not free, and trimming it leaks materials (B11). Robotron cut its part counts
  instead (grunts three entities, sparks 28 per colour). A per-system breakdown would settle it.
  **Partly done:** hidden entities are now dropped before any render work, despawning no longer
  leaks (B11), and `frame_breakdown` shows render_collect and transforms separately. Hidden
  entities still go through transform propagation; not re-measured with a large pool.
- **Every primitive has its own material and there is no way to share one.** 100 sparks of
  one color are 100 materials. Robotron keeps one pool per palette color so a spark never needs
  `set_material`. Wish: `spawn_primitive(kind, name, {material_of = other_entity})` or a
  `share_material(dst, src)`. **Done:** both. Verified: 50 spheres spawned with `material_of`
  added one material.
- **No Lua access to the GPU particle system.** Explosions, warp-in rings and trails are all
  spark entities moved from Lua (about 300 entities and 600 commands per busy frame). A
  `emit_particles(x, y, z, {count, color, speed, life, gravity})` would replace all of it.
  **Done:** `emit_particles(x, y, z, {count, color, color_end, intensity, speed, life, size,
  size_end, velocity, gravity, drag, radius, shape = "sphere" | "ring", blend})`. Bursts share
  GPU particle systems per physics setting, follow `time_scale` (frozen at 0), and are cleared
  when play starts or stops; idle systems cost nothing. The particle runtime moved from the
  editor binary into the engine (`gpu_particles::ParticleRuntime`), so the player now runs
  particle systems too. Particle steps are clamped to 50 ms of real time, because a stalled
  frame used to end every burst at once. Verified in the editor (bursts, ring shockwaves,
  bloom, pause).
- **No `require` or `include`.** `require` is `nil` (`package`, `io` and `os` tables exist), so
  a game is one file: Robotron is about 1700 lines in `robotron.lua`, and the autopilot, the
  wave table and the enemy definitions cannot be separate files. Allow `require "name"`
  resolving against the project's scripts folder (with hot reload of dependents). **Done:**
  `require(name)` (module, cached per script, own globals) and `include(name)` (runs in the
  script's environment), looked up next to the script, then under the scripts root; editing
  either reloads the scripts that use it. Verified: editing `lib/waves.lua` reloaded
  `game.lua`.
- **`write_file` writes relative to the engine's working directory, not the project.**
  `write_file("scripts/x.txt", "1")` created `<repo>/scripts/x.txt` and `read_file` found it, so
  an arcade high score cannot be stored with a project (Robotron's high score resets on
  restart). Wish: `save_data(key, text)` / `load_data(key)` in a per-project or per-user data
  directory (and localStorage on the web). **Done:** `save_data(key, text)` / `load_data(key)`,
  per user and project folder (`~/.local/share/wgpu-cube/saves/<project>/` on Linux,
  `localStorage` on the web). Verified live.
- **The Lua guide does not give primitive sizes beyond the cone and plane.** The torus has an
  outer radius of 1.35 and a tube of 0.35 with its axis along Z (`entity_get` shows the
  bounds), so a thin ring cannot be made by scaling it (the tube scales with the ring). Robotron's
  shockwaves are rings of sparks instead. Add a size table (cube 1, sphere radius 1, cylinder
  radius 1 and length 1 along Z, torus 1.35/0.35 along Z) to the guide and the
  `entity_spawn` schema. **Done** (a size table in the guide; the schema and skill list them).
- **`ui:screen_label` offsets are insets, which is easy to get backwards.** For `top_right` and
  `bottom_*` anchors a positive offset moves the text inward; I passed negative numbers and the
  text sat off screen (the screenshot showed "LIVES I I" cut at the edge). The guide says "inset
  offsets"; one example per corner would save the round trip. **Done** (an example per corner
  and a note never to pass negative offsets).
- **I cannot judge sounds.** Robotron's 20 effects and a 7.5 s music loop are synthesized with
  numpy and checked only through `audio_status` (that they play, how often, at what volume).
  Clipping, silence, a click at the loop point or a 3 s "short" effect would go unnoticed. Wish:
  `sound_inspect {path}` returning duration, peak, RMS, clipped samples, loop-seam
  discontinuity and a spectrogram image. **Done:** `sound_inspect` (engine `audio.inspect`,
  native builds) reports those plus DC offset, leading/trailing silence, loudness by tenth and
  plain-language notes, with a waveform-over-spectrogram image. Checked on Robotron's music
  loop (seamless, -4.4 dBFS peak) and shoot effect.
- **Hot reload restarts the game.** Script locals (all of Robotron's state) reset on every
  saved edit, which is documented, and `set_*` state survives. For a game with a large state
  table that means every tweak restarts the demo at wave 1; `on_destroyed(self, "reload")`
  could return a table that `on_created(self, previous)` receives, so a game can keep its
  state across edits. **Done** as suggested; entering or leaving play starts fresh. Verified:
  `GAME.wave = 9` survived a reload triggered by a module edit.

- **Post-processing settings are not saved with the project.** `postprocess_set` (bloom
  threshold, SSAO radius) changes the running engine only; after `project_save` and a restart
  `postprocess_get` showed the defaults again, and Lua cannot set them. Robotron's look had to be
  tuned against the defaults. Wish: keep them in `project.json` like the environment, or a
  `set_postprocess` Lua call.

## Wish list

- ✅ `entity_duplicate {count, step}` for a linear/grid array, or a generic `batch` tool that
  takes a list of tool calls and runs them in one round trip. (Both done.)
- ✅ Lua `set_material_color(entity, r, g, b)` (plus emissive and texture) and a Lua way to
  spawn a primitive mesh, so pools and per-object colour are not needed. (`set_material`
  covers emissive and textures; `spawn_primitive`, `set_mesh`.)
- ✅ Audio: `play_sound(path, {volume, pitch, loop})` in Lua and a matching `sound_play` MCP
  tool for testing, plus `.wav`/`.ogg` loading from the project's `content/`. (Also mp3 and
  flac, and `audio_status` to see what played.)
- ✅ `project_create`/`project_open` that block until the project is open.
- ✅ `assets_import {path}` for images, audio and fonts that copies into `content/`. (As
  `asset_import`.)
- ✅ World-space text (a billboard text component) and HUD support in the player. (As
  `ui:world_label`, an overlay rather than a component.)
- ✅ A `describe_scene`/`scene_diff` tool to see exactly what a play session changed.
  (`scene_mark` + `scene_diff`; `playtest` includes the diff.)
- ✅ Bulk `entity_update` / `entity_delete` by id list or name filter, with a terse result
  mode.
- ✅ Texture/script/asset hot reload by path (and an `asset_reload` tool).
- ✅ A `playtest` helper: run N seconds of play with scripted input and return a state snapshot
  (`lua_eval` result) plus a screenshot at the end, so one call replaces
  `runtime_set` + `input_send` + `wait` + `lua_eval` + `screenshot`. (Also reports failing
  scripts, warnings, sounds played and the scene diff.)
- ✅ A `time_scale` setting (`runtime_set {time_scale: 4}`) so automated playtests of long
  flows (clearing a level took about a minute at 1x) can run faster than real time. (Up to
  16x, run as several normal-sized steps per frame so game logic behaves as at 1x; resets to
  1 on stop.)
- ✅ `screenshot {entity}` or `camera: "game"` to render through the scene's active camera
  while editing.
- ✅ Return `{"discarded_on_stop": true}` on mutating calls made during play.

- ✅ Lua `despawn_entity(e)`, plus a cleanup hook that also runs before a hot reload, so
  scripts can own their pools without leaking. (`on_destroyed(self, "reload")`.)
- ✅ Lua `screen_to_world_ray(x, y)` and `world_to_screen(x, y, z)`, and the viewport size
  outside `on_ui`. (Plus `screen_to_ground`; coordinates are window pixels like the mouse
  position, with `get_viewport_size()` giving the game view's rectangle.)
- ✅ Lua `get_time()` / `elapsed()`, and `get_forward(entity)` (yaw 0 faces -Z, so
  `forward = (-sin(yaw), 0, -cos(yaw))` has to be derived by hand). (`get_time()`,
  `get_forward`, `get_right`, `get_up`.)
- ✅ A `sounds: false | "summary"` option on `playtest` (counts per path with first and last
  time), and the same grouping in `audio_status`.
- ✅ `camera_set {view_only: true}` as the default (or a one-off `position`/`target` on
  `screenshot`), so looking around never edits scene data. (The one-off pose on `screenshot`;
  see the `camera_set` pain point for why there is no view-only mode.)
- ✅ `screenshot {aspect}` or `{size}` to preview the game camera at 16:9 and 4:3. (`aspect`.)
- ✅ HDR emissive (`emissive` above 1) for bloom without losing shading.
- ✅ `ui:label_at(anchor, x, y, text, {size, color})`, or size and color on `ui:label`. (Both:
  `ui:screen_label` and options on `ui:label`/`ui:heading`.)
- ✅ `lua_api_reference {section}` or `{search}` (it is one ~6k-token blob). (`section`,
  `search` and `outline`.)
- ✅ A note in the skill on which MCP tools to load up front (they are all deferred).
- ✅ A version or commit field in `engine_info` (MCP server build vs. repo HEAD).
  (`mcp_server`.)
- ✅ A `since_play_start` filter on `logs`: an error fixed minutes ago still tops
  `logs {min_level: "warn"}`.
- ✅ A lean `playtest` mode (no screenshot, diff or sound list) for long AI soak tests.
  (`lean: true`.)
- ✅ A starter game scene (environment, bloom, camera, light, floor) for `project_create`.

- ✅ A `project_create` template with scripts and audio folders and a starter scene
  (environment, bloom, camera, light, floor); see the pain point above.
- ✅ Release materials of despawned entities (B11) and inherit `Visible` from parents (B12).
- ✅ `emit_particles` from Lua (the GPU particle system), `share_material`, and a frame-time
  breakdown in `engine_stats`.
- ✅ `require` for project scripts, `save_data`/`load_data`, and state that survives hot reload.
- ✅ `sound_inspect {path}` (duration, peak, RMS, clipping, loop seam, spectrogram).
- ✅ `screenshot {viewport_size: [w, h]}` that really resizes, so fitted cameras and HUDs can be
  checked at 16:9 and 4:3. (A viewport preview inside the view rather than a window resize.)
- ✅ A skill note: expose a global `RT` table of test hooks (`jump`, `spawn`, `god`, `units`)
  next to the `GAME` state table; it made every enemy type testable in seconds. (Also in the
  Lua reference's remote-control notes.)
- ⬜ Make the editor's own frame cheaper (`ui_build` about 6.5 ms in a debug build), and skip
  hidden subtrees in transform propagation.

## Open: Lua scripting on the web

Web builds do not compile. `cargo check --lib --target wasm32-unknown-unknown` fails in
the `mlua-sys` build script: `lua-src` cannot build the C Lua VM for that target. The C VM
needs `setjmp`/`longjmp` and a libc, and `wasm32-unknown-unknown` has neither. mlua only
supports `wasm32-unknown-emscripten`, which wgpu, winit and wasm-bindgen do not use. The
problem is the VM build, not the Lua language.

**Recommendation: keep Lua and fix the VM build, rather than switching languages.**

- **Cost of switching:** the bindings are about 9k lines (`src/scripting/lua`), plus the
  Lua guide, the MCP tooling (`lua_eval`, `playtest`, `lua_api_reference`), the example
  scripts and Breakout. A second language means writing and maintaining all of that twice.
- **Agents:** LLMs write correct Lua game code far more reliably than Rhai, Koto or Rune,
  which matters for games built through the MCP.
- **History:** the project already moved from Rune to Lua once.
- **Alternatives looked at:**
  - Rhai: mature, pure Rust and works on wasm, but a slow tree-walking interpreter with
    less familiar syntax, and a full API rewrite.
  - Koto: Lua-like and pure Rust, but a small ecosystem.
  - piccolo: a pure-Rust Lua VM, but experimental, with an incomplete stdlib and an API
    unlike mlua's, so also a rewrite.
  - rquickjs (JS) and Luau: C/C++ VMs with the same wasm build problem.

Plan:

1. ✅ Spike: compile Lua 5.4's C sources for `wasm32-unknown-unknown` with clang (installed:
   clang 23, wasm32 backend). Map `setjmp`/`longjmp` to wasm exception handling
   (`-mllvm -wasm-enable-sjlj`, supported by all current browsers) and provide a small libc
   shim backed by Rust's allocator. Link it into mlua's external-library mode so the
   binding code stays unchanged.
   **Done** (2026-10-13), slightly differently: mlua refuses non-vendored wasm builds, so
   `lua-src` is vendored in `third_party/lua-src` (via `[patch.crates-io]`) with a
   `wasm32-unknown-unknown` build. It adds a C libc shim (`wasm/libc.c`), the sjlj runtime,
   and Rust helpers for malloc, float formatting/parsing, console and clock
   (`src/scripting/lua/wasm_libc.rs`). No binding code changed. `cargo build --target
   wasm32-unknown-unknown` now builds and links. Checked with a 99-line Lua conformance
   script (number formatting incl. `%a`/`%q`, hex `tonumber`, errors through Rust
   callbacks, stack overflow, coroutines, `<close>`, `os.date`). Native, node and Chrome
   give identical output apart from pointer addresses and `os.getenv`. The engine itself runs
   Lua in the browser: `examples/lua_cube.rs` (`./build_web.sh lua_cube`) logs its
   `on_created` line, and its `update` spins the cube and pulses its colour (headless
   Brave with WebGPU on Vulkan, RX 7900 XTX). Web limits: no filesystem for `io`, `os.getenv` is nil,
   local time is UTC. Building needs clang.
2. ⬜ ~~If the spike fails: put a thin backend trait in front of scripting and use Rhai only
   on web.~~ Not needed.

Found while testing (not Lua, fixed): web builds sometimes started with the view filled
by one colour, in about 5 of 12 page loads for `minimal` too. The canvas's backing store
was 1x1 and the browser stretched that pixel. The async renderer took the canvas size
before layout (0x0, clamped to 1x1), and the Resized event that followed was dropped
when it arrived before the renderer existed. The renderer now resizes to the window's
current size when it becomes ready (`ensure_renderer_ready`); 10 of 10 loads render
correctly.

Note: `build_web.sh` needs the wasm-bindgen CLI to match `Cargo.lock` (0.2.129); it now
checks and prints the install command. The CLI that was installed (0.2.114) also cannot
process modules with exception handling.

Breakout runs on the web (2026-10-13): `./build_web.sh --project games/breakout` builds the
player with egui and stages the project in `target/web/breakout/`. Getting there needed
a vendored egui-winit (0.36.2 does not compile for wasm32), `Arc<Window>` on the web,
HTTP-backed reads for scenes, materials and scripts (`io::read_to_string`), no
`exists()` checks for materials and sounds on the web, and a web-safe wall clock
(`time::unix_millis`). The canvas now fills the window below the toolbar.

Related web gap: file hot-reload is native only. Not planned: a web build has no editor
and no filesystem to watch, so it would mean polling the server.

Found while testing lights (documented, not changed): the `plane` primitive is a quad
in the XY plane facing +Z, so `set_scale(floor, 10, 1, 10)` gives a 10x1 wall, not a
floor. Changing it would move the planes in existing scenes; the Lua guide now says to
rotate it -90° about X.

Web audio (2026-10-13, done): web builds play sounds through the Web Audio API
(`src/audio.rs`, `mod web`) rather than rodio, which would have needed cpal's WebAudio
backend and Rust-side decoding. Files are fetched relative to the page and decoded by
the browser (`decodeAudioData`), cached per path, and played as
`AudioBufferSourceNode`s with per-sound and master gain nodes; volume, pitch, loop, stop,
`stop_all_sounds` and master volume behave as on native. The first play of a file starts
once it has loaded. A missing file is logged and only makes later plays of that path
fail. The context opens suspended (autoplay policy) and resumes on the first click, tap
or key press; sounds queued before that play then. Checked in headless Brave with
Breakout: the intro sound waited for the first input, then autopilot started 17 sounds
with the right clip lengths and no audio warnings.
