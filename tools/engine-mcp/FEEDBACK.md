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

Tide Observatory round (2026-10-14, commit `c44fc8d`): built a kinetic brass sculpture
through MCP, now in [`games/tide-observatory`](../../games/tide-observatory/README.md).
Sweeps, lathes, composed meshes and generated PBR maps were enough to author the scene
without imported models. Atomic rollback caught an invalid lathe profile cleanly.
Nine fixed-step interaction assertions passed over 600 steps. Live inspection exposed
the camera and TAA bugs B17–B19, fixed in that commit; browser smoke testing confirmed
the open runtime issues B20–B21. The full native tests, strict Clippy, formatting and
six GPU smoke targets passed. The editor undo test caught a camera regression that
the library and scene-level tests alone did not cover.

Shadow follow-up: B22 was found after that commit and subsequently fixed.
A regression test covers 144 combinations of orbit angle, height and distance;
native captures at low, middle and high views with maximum zoom-out show the full
shadow, and the user confirmed the disappearing-shadow issue was resolved.

Cleanup round (2026-10-15): worked through what the Tide Observatory round left open. B20 and B21
are fixed, and fixing B21 exposed B23 (particle readbacks that cannot work in a browser), also fixed.
Tide Observatory now loads and animates in a WebGPU browser with no errors. The post-processing
pain point turned out to be done already (post-process volumes), the two performance wish items were
measured instead of changed (they are cheap in a release build), and the MCP gained
`batch {aggregate}`, a browser preflight in `project_validate` / `web_build`, and a note on approval
failures. Not done: the visual comparison tool (see the wish list). Checked with Rust tests, strict
Clippy, native smoke runs (with and without bindless textures), `verify_additions.py --web`, and a
WebGPU browser through `web_playtest`.

Follow-up round (2026-10-15): the two items the cleanup round left open. Lua gained
`set_postprocess` (done), and the visual comparison tool is half done: a render-only
post-process override and a `visual_capture` tool that renders one frozen view under several
variants and records what each image was rendered with. Still open: comparing the images
(difference images, per-region variation). Checked with Rust tests (10 new), strict Clippy, a
wasm32 library check, and a native editor driven through the MCP in edit and play mode (see the
wish list for what was measured).

Comparison round (2026-10-15): the comparison half of the visual tool, so it is done. `visual_capture`
now compares the frames it takes (engine-side, from the frames in memory): per-region differences to
the baseline, the noise between a variant's own frames, amplified difference images and a settle
check, in the reply and `manifest.json`. A first version built on the plain mean difference was
useless on real captures (output dither makes every pair of frames differ by about 0.8 levels per
pixel, with TAA on or off), which led to the block-averaged "smooth" difference; see the wish list
for the numbers. Checked with Rust tests (34 new), strict Clippy, a wasm32 library check and a native
editor driven through the MCP, TAA on against off with an orbiting block frozen in play mode.

Path tracing round (2026-10-15, commits `c1d9ce4` and `b331fe0`): engine work rather than a game.
Path tracing with a denoiser and caustic photons went into the renderer, and RT Lab gained a caustics
table built through the MCP (`mesh_lathe`, `entity_spawn`, spot lights, a camera-tour shot). Most of
the MCP friction came from debugging the renderer: seven rebuild-and-relaunch cycles (30 to 40 s of
build each) only to look at intermediate buffers, and every shader edit needing a rebuild. Findings:
B24, the pain points from "Judging temporal effects needs a still view" onward, and the wish-list
items after the visual comparison tool. Not verified: whether `visual_capture` alone would have
covered the still-view comparisons (it was not tried; see that pain point).

Path tracing follow-up (2026-10-15): the rest of that round's wishes. Shader hot reload, held-view
screenshots with displayed-pixel numbers, format-preserving saves (which also fixed B24) and lathe
shells are done; every item from that round is now done. Checked with Rust tests (3 new, 2 extended), strict
Clippy, a wasm32 library check, the MCP server's tests, and the release editor on a scratch copy of
RT Lab driven through the MCP (`engine_call` for the new parameters, since the running MCP server
predated them); see each wish for what was measured.

Marionette round (2026-10-17): a skeletal animation show and crowd benchmark,
[`games/marionette`](../../games/marionette/README.md), built from the asset library's 47
characters (Mixamo and Quaternius) and 94 clips: up to 1,051 characters at once (2,933 skinned
meshes, 14.5M skinned vertices, 80k entities). The arena was authored over MCP (`mesh_lathe`,
`mesh_compose`, spots, a baked light probe volume); the show is Lua. Crowds exposed eight
engine bugs, all fixed here (B25 to B32), and the benchmark drove a crowd performance pass:
the stadium part went from 15 to 29 fps on raster, measured both times with the user's game
running alongside (animators pose on the thread pool, transform propagation walks one hecs
view, a character's meshes share one palette, passes borrow meshes, despawns share one index).
With nothing else running it reaches 35 fps, and the whole show averages 141 fps. New Lua: `clone_entity`, `frame_stats`, `graphics_settings`,
`set_graphics_settings`; MCP `graphics_set {save: false}`. Checked with the full Rust test
suite (6 new tests), strict Clippy, and the release player and editor driven over MCP, with
the benchmark's own before/after numbers.

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
  gone). The Lua guide's "Entity Handles" section lists which commands accept a handle spawned
  in the same hook and which need an entity that already exists (checked against `commands.rs`
  on 2026-10-15; this note used to say it was undocumented).

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

### B17. Lua `look_at` rolls and flips an orbiting camera (fixed, engine)

- Repro: orbit an elevated camera around a target and call Lua `look_at` each update.
  The horizon tilts and can flip when the camera goes behind the target.
- Cause: Lua used `Quat::from_rotation_arc(-Z, direction)`, which constrains forward
  without constraining up. This resembles gimbal lock but is an orientation ambiguity.
  MCP camera placement already used a level Y-up look rotation.
- **Resolution:** Lua and MCP share that look rotation for pending and existing
  entities. Coincident targets leave rotation unchanged. A full-orbit regression test
  and 73 live MCP probe angles verified aim and a level horizon.

### B18. Camera synchronization ignores changes to roll (fixed, engine)

- Repro: change a camera entity's roll while keeping its position and forward direction
  fixed. `SceneCamera::apply_from_entity` treats it as unchanged and keeps the old up.
- **Resolution:** compare the actual perpendicular view-up direction and sync roll
  without resetting the orbit focus distance. Preserve equivalent saved up hints,
  including world Y, so editor undo retains the saved camera. Both the new roll test
  and the existing editor undo test pass.

### B19. TAA jitter disagrees with screen-space effect projections (fixed, engine)

- Repro: enable TAA, ambient occlusion and reflections over a large floor at a grazing
  camera angle. The distant floor shows broad brightness flicker. With FXAA and the
  other effects still enabled, the frozen view was stable.
- Cause: the G-buffer rasterized through a jittered camera, but ambient occlusion and
  reflection lookups projected through unjittered matrices.
- **Resolution:** apply the rasterization jitter to `PostProcessCamera`'s projection,
  view-projection and inverse. Motion vectors and overlays retain their unjittered
  matrices. In the checked image region, pixels changing by more than five 8-bit color
  levels dropped from roughly 2,500 to around 10 per comparison. This is a sampled
  visual check, not a guarantee of identical frames. The projection regression test
  passes, and the user confirmed the scene looked stable with TAA re-enabled.

### B20. Browser material requests decode literal percent escapes (fixed, engine)

- Repro: export a scene whose material filenames contain `%2e` in the scene namespace,
  then package and run it with `web_build` and `web_playtest`. The files exist in the
  staged output, but material requests return HTTP 404.
- Cause: the filesystem path is passed directly to the HTTP request. URL decoding
  changes the literal filename.
- **Resolution:** web requests percent-encode the path (`io::percent_encode_path`: everything
  except `A-Za-z0-9-._~/`), so `%2e` in a file name is requested as `%252e` and the server decodes
  it back to the exact name. glTF URIs are already encoded by spec, so the web glTF loader decodes
  them before joining them to a path; absolute `http(s)://` URLs pass through. Unit tests cover
  literal `%`, spaces, `#`, `?`, non-ASCII names and round trips through `percent_decode_uri`.
  Verified in a WebGPU browser with the Tide Observatory build: the old build gave 71 errors (HTTP
  404 for each `%2e` material); the rebuilt one has 0 errors and 0 network failures. A scratch
  project with a script named `pct%2ehello.lua` also runs in the browser (`verify_additions.py
  --web` checks its console output).

### B21. Browser GPU particles require a bindless group at draw time (fixed, engine)

- Repro: run Tide Observatory in a WebGPU browser on an adapter without bindless
  textures. The device initializes and the scene starts loading, then its emitted
  particles trigger `GPU-driven particles require bindless textures`.
- Cause: particle pipelines support traditional layouts, but the render path still
  requests the renderer's global bindless texture group. A successful WASM package build is
  insufficient evidence of a working demo.
- **Resolution:** without bindless textures the particle system binds its own material's textures
  (group 3 is the per-material layout there; `Renderer::material_textures_bind_group`) instead of
  `expect`ing the global group. The adapter check in the renderer was a hard-coded `false`; it is
  now the `WGPU_CUBE_FORCE_TRADITIONAL_TEXTURES=1` environment variable, so this path can be tested
  natively: `particle_effects` renders its textured and untextured systems with it and without
  it. A WebGPU browser never offers bindless textures, so every web build takes this path. The next
  failure there was B23.

### B23. Particle readbacks block on the GPU, which a browser cannot do (fixed, engine)

- Repro: run Tide Observatory in a WebGPU browser after the B21 fix. 200+ errors:
  `Particle dead list mapping did not complete` and `Failed to map particle dead list: Error
  occurred when trying to async map a buffer`. The dead list is how dead particle slots are
  reclaimed, so slots never came back and a system would stop emitting after `max_particles`
  spawns. Alpha-sorted systems had the same problem with their sort readback, silently (an
  unsorted draw is the fallback). B21's panic had hidden all of it.
- Cause: both readbacks called `map_async` and then `device.poll(Wait)` in the same frame. A
  browser cannot wait, so the callback had not run, the buffer stayed in a pending map and every
  later `map_async` failed. Natively the blocking poll also stalled the CPU on the GPU each frame.
- **Resolution:** a non-blocking `AsyncReadback` (`gpu_particles/system/readback.rs`): record the
  copy this frame, map the buffer next frame, and take the data on the first frame after the
  callback has run (`poll(Poll)` natively; the browser calls back between frames). The GPU dead
  list now keeps accumulating deaths until a copy is taken and restarts only then (a particle dies
  once: its lifetime becomes -1), so none are lost while a readback is in flight. Reclaimed slots
  arrive two or three frames later than before, and a sort order is a couple of frames older.
  Verified: `particle_effects` (a 25,000-slot fountain, fireworks and an alpha-sorted smoke trail)
  still emits after 30 s natively with a clean log, and Tide Observatory runs in a WebGPU browser
  with 0 errors and 0 network failures.

### B22. Lua camera focus is lost, moving the directional shadow volume (fixed, engine)

- Repro: orbit Tide Observatory's elevated focus point, raise/lower the camera, or
  zoom out. A straight boundary cuts through the ring shadow; at some views it vanishes.
- Cause: Lua `look_at` kept only the entity rotation. Camera synchronization replaced
  the real focus point with `eye + forward`. The shadow-placement fallback guessed a
  ground-plane intersection from that ray, which moves far behind an elevated subject
  at shallow angles and can shift the finite shadow volume off the sculpture.
- **Resolution:** retain Lua's aim point as a transient `LookAtTarget` hint for pending
  and existing entities. Use it as the camera target while it agrees with the current
  world forward direction. Ignore stale hints after unrelated translation or rotation.
  The existing free-camera fallback remains available when no valid hint exists.
- Verification: the regression failed before the fix and now passes for all 144 poses,
  checking a fixed shadow matrix and coverage of the sculpture and surrounding floor.
  Two additional tests reject stale hints. Native before/after captures show the clipped
  ring shadow becoming complete without enlarging the light's shadow extent.

### B24. `project_save` repoints an imported model's material at a duplicate file (observed, engine)

- Repro: open `games/rt-lab` (committed at `f420623`), add entities that do not touch the gem,
  `project_save {compact: true}`. `content/gltf/gem/meta.json` now maps material 0 (and both
  `node:0/primitive:0` keys) to `content/materials/gem_glb_000_1.mat.json` instead of
  `gem_glb_000.mat.json`. Both files are tracked and exist; nothing about the gem was edited.
- Cause not investigated. It looks like the save picks whichever of two equivalent material
  files it meets first. Harmless while the two files match, but it is churn in every commit,
  and the scene silently stops using the file a person may be editing.
- Workaround: `git checkout content/gltf/gem/meta.json` after saving.
- **Resolution (fixed, engine):** the cause was two scenes giving the same glTF material different
  files: RT Lab's main scene uses `gem_glb_000_1.mat.json` (the gem with dispersion), the imported
  `gem.scene.json` uses `gem_glb_000.mat.json`. `meta.json` holds one default for the model, each
  scene's save wrote its own choice into it, and the scenes were saved in `HashMap` order, so
  whichever came last won. Saves now keep a `meta.json` entry that still names an existing file
  (replacing only missing or dangling ones), go through scenes in document order, and skip
  writing files whose content would not change. Test: `persist_material_assets_writes_meta_mapping`.
  On a copy of RT Lab, adding a cube and saving changed only the main scene (a 1-line diff in its
  compact file) plus the cube's new material; `meta.json` was not touched.

### B25. The player never imported models a script asked for (fixed, engine)

- `import_gltf(entity, path, scale)` queued the import, but only an editor system drained the
  queue, so in the player (and web builds) nothing ever appeared. Marionette's loading screen
  waited forever at 0 of 47 with no error.
- **Fixed:** the scheduler's GPU stage processes pending imports for every app, next to the
  scripts' mesh and material commands; the editor's own call is gone.

### B26. Ray tracing crashed past 128 MiB of geometry (fixed, engine)

- With about 900 skinned characters the ray-traced scene's vertex pool doubled to 256 MiB, more
  than the device's `max_storage_buffer_binding_size`, and wgpu panicked creating
  `RayTracingSceneBindGroup` ("Buffer binding 1 range 268435456 exceeds ... limit 134217728").
- **Fixed:** the pool never grows past what the device can bind (geometry that does not fit is
  left out of ray-traced effects, warned once), and the ray tracing device asks for the
  adapter's limit up to 1 GiB. 900 castle guards then ran without errors.

### B27. A copied character kept posing the original (confirmed, fixed, engine)

- `entity.duplicate` copied a playing `Animator` with its cached bone binding;
  `Binding::is_valid` only checks that the bones exist, so in play mode the copy kept driving
  the original's bones. Read in source while adding Lua `clone_entity`.
- **Fixed:** duplication is one function (`scene::duplicate::duplicate_subtree`) used by the
  remote duplicate and `clone_entity`; copies start animator and animation tree playback afresh.

### B28. Spawned handles never worked with reading functions (fixed, engine)

- `local h = spawn_entity("X")` then, in any later hook, `get_children(h)`, `entity_exists(h)`,
  `get_component(h, ...)` or `animator_state(h)` saw nothing: they decoded the negative handle as
  an entity id. Only `find_entity_by_name` recovered the real id.
- **Fixed:** reading functions resolve handles through the script's registry
  (`guards::resolve_entity`). Test: `reading_functions_accept_spawned_handles_in_later_hooks`.

### B29. Double-sided materials were drawn single-sided (fixed, engine)

- `MaterialFlags::DOUBLE_SIDED` was stored, saved and reported by `entity_get`, but every
  render object used back-face culling. The arena's stands (a lathe seen from inside) vanished,
  and all 47 characters in the asset library (Blender exports glTF materials double-sided by
  default) lost the back faces of hair cards, capes and lashes.
- **Fixed:** the cull mode comes from the material; the depth prepass and shadow maps have
  no-culling pipelines for double-sided batches, and the PBR shader flips the normal on back
  faces (`front_facing`).

### B30. A script's graphics settings were overwritten on the first render (fixed, engine)

- The project's saved graphics settings were loaded lazily in the first render, after the first
  script update, so `set_graphics_settings` in `on_created` or the first `update` was replaced:
  the show asked for screen-space reflections and still rendered ray-traced ones.
- **Fixed:** `graphics_settings()` loads the open project's settings whenever it is used.

### B31. Importing a model again uploaded it again, for good (fixed, engine)

- Each `import_gltf` of the same file registered new meshes and textures, and nothing frees
  textures. After one hot reload of a script that imports in `on_created`, the stadium filled
  the 24 GB of VRAM (24.4 GB used, 3.4 GB spilled to system memory) and its GPU time went from
  about 10 to 98 ms a frame (paging), on a GPU shared with the user's game; the texture count
  had doubled (190 to 376).
- **Fixed:** each scene keeps the models it imported (by path and scale, reloaded when the file
  changes): importing again only instantiates. Across a reload of all 47 characters: textures 190
  and meshes 139 before and after, VRAM unchanged (14.6 GB at the title screen). Still open: textures and meshes of models no longer used are
  never released (see the wish list).

### B32. Every `despawn_entity` scanned the whole world (fixed, engine)

- To catch children missing from a child list, each despawn indexed every entity's `Parent`
  before walking its subtree, so a hook despawning many things cost (despawns x world size):
  clearing the battle's 300 characters with about 25k other entities alive took one 4.2 s frame,
  and the visited check was a linear `Vec::contains`. The show's benchmark caught it as a
  mysterious 5 ms "scripts" average in the stadium (the stall frame carried the part's clock past
  its warm-up and was measured).
- **Fixed:** the index is built once per batch of commands and shared by its despawns, visited
  sets are hashed (also in `duplicate_subtree`), and the benchmark leaves out frames that began
  in a part's warm-up. Test: `despawning_many_subtrees_in_one_hook_removes_every_descendant`.

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
  own cost was not reduced. Measured again on 2026-10-15 in the release editor: `ui_build` is
  0.43 ms with a few entities (324 fps; the 6.5 ms above was a debug build), so there is nothing
  to fix there; debug-build editor timings say little about the game.
- **Hidden entities seem to cost frame time.** Observed in the player (noisy, not isolated): 460
  entities 8.4 ms (119 fps), about 900 entities 16 to 21 ms, about 1400 entities 22 ms; 500 extra
  `Visible = false` cubes at y = -80 read 22.6 ms, and 21 ms again after they were despawned,
  because the game's own load moves the frame time too. If it holds, a pool of "hidden" sparks
  and actors is not free, and trimming it leaks materials (B11). Robotron cut its part counts
  instead (grunts three entities, sparks 28 per colour). A per-system breakdown would settle it.
  **Partly done:** hidden entities are now dropped before any render work, despawning no longer
  leaks (B11), and `frame_breakdown` shows render_collect and transforms separately. Hidden
  entities still go through transform propagation; not re-measured with a large pool.
  **Measured** on 2026-10-15 (release editor; a hidden root, 300 hidden groups and 2,700 hidden
  cubes, 3,006 entities): `transforms` 0.34 ms per frame (two propagation passes, about 0.06 us per
  entity per pass), `render_collect` 0.12 ms, `ui_build` 1.4 to 1.8 ms; the frame was 6.5 ms (150
  fps) against 3.1 ms with 5 entities. In a debug editor the same pool costs 6 ms of `transforms`
  and 11 ms of `ui_build`, which is why it looked expensive. Hidden pools are cheap.
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
  `set_postprocess` Lua call. **Done** (by the post-process volumes work, which post-dates this
  note): post-processing lives on `Post Process Volume` entities that `postprocess_set` edits (it
  creates an infinite one when the scene has none) and that save with the scene; the old
  `project.json` `postprocess` block and the environment's colour grading are migrated into a
  volume on load. The test `post_process_volume_survives_save_and_load` covers the scene round trip
  (settings, overrides, priority). Scripts can now change it too: `set_postprocess` (see the wish list).

- **Visual comparisons require manual state management.** Diagnosing B19 meant pausing
  animation, recording the camera and effect settings, toggling one effect at a time,
  capturing PNG sequences, and comparing selected image regions externally. Pausing
  the sculpture did not disable camera input; live dragging invalidated some early
  comparisons. **Open:** capture an isolated scene snapshot with fixed camera and
  simulation, and record the effective settings and frame/jitter indices with each image.
  **Done** (2026-10-15): `visual_capture` locks the camera, freezes play, applies each
  variant as a render-only override, records settings, jitter index, frame and scene time with
  every image, and compares the images (difference images, per-region statistics, jitter noise,
  settle check); see the visual comparison tool in the wish list.
- **Approval failures can look like engine failures.** This session initially accepted
  launch and project creation, then blocked scene edits with `MCP tool call requires
  approval, but approval policy is never`. Changing the client approval configuration
  restored editing. This was a host/session policy issue, not an engine bug. **Done:** the
  `engine://workflow` resource and the README ("Troubleshooting") say that the error comes from
  the client, that reads can work while edits are refused, and how `engine_status` /
  `engine_output` tell a real connection problem apart. The client's approval controls are
  untouched.
- **Quiet batches still return a report for every successful call.** `quiet: true`
  successfully omits full geometry recipes, but a large scene batch still returns many
  component counts and bounds. Consuming `structuredContent` directly helped avoid
  repeating the text copy. **Done:** `batch {aggregate: true}` returns one summary instead of a
  row per call: calls per tool, the entities returned (listed up to 24, else count, first and
  last), alias ids, every failed call in full, and the unchanged `ok` / `failure` /
  `rolled_back` verdict. A 66-entity atomic batch came back as 476 bytes instead of 13,287.
  The detailed report is still the default.
- **Browser readiness is easy to overestimate.** Native `project_validate` and a WASM build both
  passed while B20 and B21 broke the browser run, and `web_build` stages only `project.json` and
  `content/`, so an asset anywhere else 404s however well the project validates natively.
  **Done:** `project_validate` has a `browser` block (errors: references `web_build` would not
  serve; warnings: files differing only in letter case, Lua calls that need a file system such as
  `read_file`/`write_file`/`io.open`/`os.getenv`, script string literals naming a missing file) and
  `web_build` stops on its errors before the slow build (`ignore_browser_issues: true` packages
  anyway). It reads source text only: paths built at run time and adapter differences still need
  `web_playtest`, which is how B23 was found. Run over the six game projects it raised no false
  positives.

- **Judging temporal effects needs a still view, and the view moves the game camera.** A path
  tracer's denoiser (like TAA, but over 32 frames) only shows its real output after the view has
  stayed put. `screenshot {camera: pose}` renders one frame from the new pose, which the
  renderer treats as a camera cut, so every such capture showed one-frame, unaccumulated noise
  and looked like a denoiser bug. The alternative, `camera_set`, moves the scene's active camera
  entity in the editor ("the view camera drives the active camera entity"), so before each of
  four `project_save` calls I had to write the GameCamera's transform and field of view back
  by hand. **Open.** `visual_capture {camera: pose, settle_frames, variants}` already locks a pose,
  settles and applies variants last (so `{postprocess: {path_tracing: {enabled: true,
  denoiser: false}}}` would override the player's graphics settings). It was not tried here
  because it reads as a post-processing comparison tool. Suggestion: say in the skill and the
  `screenshot` description that `screenshot {camera}` is a camera cut, and point at
  `visual_capture` for anything temporal. **Done:** the description and the skill say so, and
  `screenshot {camera, settle_frames}` holds the pose first (see the held view wish).
- **Looking at intermediate render buffers means adding debug code.** Finding why denoised
  reflections faded (the virtual-motion reprojection was off by the TAA jitter) and why a steel ring
  stayed grainy (residual variance, not a failed history test) needed visualizations of history
  length, reprojection distance and which validity test failed. Each one was a temporary
  `PT_DEBUG` environment-variable switch in WGSL plus a rebuild and relaunch, seven times in all.
  **Done:** see the offscreen texture fetch wish.
- **Every shader edit is a rebuild, and a restart loses unsaved scene edits.** The WGSL is built
  into the binary with `include_str!`, so even a one-constant change is `engine_launch
  {restart: true}`: 30 to 40 s of build plus reopening the project. Edits made through the MCP
  since the last save are lost, so building the caustics table needed saves partway through, each
  with the camera put back first. **Done:** see the shader reload wish.
- **`project_save` pretty-prints files that were compact.** On RT Lab, whose scene files were
  committed compact, a plain `project_save` produced a diff of 1,288,356 lines (the imported
  model scenes rewritten indented). `project_save {compact: true}` brought it back to the real
  change. **Done:** each file keeps the layout it has on disk, and unchanged files are not
  rewritten (see the format-preserving saves wish).
- **`mesh_lathe` cannot make a hollow vessel.** A tumbler (outer wall up, inner wall down) is
  rejected with `profile must rise bottom to top, with distinct points and no axis-only edge`, so
  the caustics table got a solid glass rod instead of a glass. A two-point profile with
  `caps: false` and a double-sided material did make a thin ring. **Done:** see the lathe wish.
- **Comparing brightness meant leaving the MCP.** Whether two captures differed (fixed grain or
  temporal noise) was checked by capturing the engine window with `xdotool` + `import` and comparing
  crops with ImageMagick (`compare -metric AE`). `visual_capture`'s comparison covers this
  between variants, but there is no way to read a pixel's or a region's HDR value from one frame.
  **Done:** see the offscreen texture fetch and pixel probe wishes.

- **Long runs outlast `wait`.** A full benchmark takes five minutes; `wait` moves to the
  background after 120 s and caps at 240 s, so finishing meant guessing durations and polling
  `lua_eval` for `SHOW.mode == "results"`. See the wait-until wish.
- **GPU memory was invisible.** The 2 fps stadium (B26's ray tracing, then B31) was paging:
  visible only in `/sys/class/drm/card1/device/mem_info_vram_used`, not in `engine_stats`, whose
  GPU timings just showed a huge `acceleration_structures` stage.
- **CPU profiles had no callers.** `engine_profile_cpu {callers: true}` and `perf report -G` on
  its data showed no call stacks for hot symbols (`Mesh::clone`, `Path::components`, SipHash);
  finding their callers meant reading code.
- **`graphics_set` always saved.** Trying configurations changed the user's saved settings.
  **Done:** `graphics_set {save: false}` applies a change to this run only (and Lua's
  `set_graphics_settings(fields, { save = false })`).
- **Flat lathes are rejected.** A floor ring (`[[17, 0.006], [24.8, 0.006]]`) fails with
  `lathe profile must have positive height`; a 2 mm slope works around it.
- **`library_add` instances characters into the scene.** For a game that spawns its cast at run
  time the files only needed copying into `content/`; the game got its own
  `tools/fetch_cast.py` (copying 47 characters and 94 clips by id from `library.json`).
- **The editor slows down with crowds.** With 41k entities `ui_build` took 35 ms and with 80k
  86 ms: the hierarchy panel copied its whole snapshot every frame. **Partly done:** the
  snapshot is shared (`Arc`); rows are still built for every entity.
- **A failed `lua_eval` still applies what ran.** `T = spawn_entity("Template")` followed by an
  erroring `import_gltf(T, ...)` returned the error but left the entity spawned (observed).

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
- ✅ Make the editor's own frame cheaper (`ui_build` about 6.5 ms in a debug build), and skip
  hidden subtrees in transform propagation. (Closed by measurement, no change: see the two pain
  points above. In a release editor `ui_build` is 0.43 ms and 3,000 hidden entities cost 0.34 ms of
  transforms. Skipping hidden subtrees would also freeze lights, cameras and script reads under a
  hidden parent, which do not follow `Visible`, for a gain of about 5% of a frame. A single-lookup
  rewrite of the propagation loop was tried and gained only about 10% in a debug build, so it was
  not kept.)

- ✅ **Visual comparison tool:** lock a camera and scene snapshot, capture consecutive
  rendered frames for several effect configurations, and report difference images,
  per-region variation and settings. Preserve and restore the user's running scene.
  Cover the actual render/TAA frames, not only deterministic Lua simulation steps.
  **Capture done** (2026-10-15): `visual_capture {variants: [{name, postprocess, base}],
  frames, settle_frames, camera, dir}` (engine method `visual.capture`). A variant is a
  render-only `PostProcessOverride` applied after the scene's volumes are blended, so no scene
  edit has to be undone and `project_save` is unaffected; `base: "defaults"` starts from the
  engine defaults to isolate one effect. The view is locked for every frame (rendered through
  the camera override, so the editor grid and gizmos are out of the images and TAA history never
  sees another camera), play mode is frozen with time scale 0 and the time scale and override are
  restored afterwards. Per variant: apply, `settle_frames` (default 16), then `frames` consecutive
  PNGs of the scene viewport. The reply and `manifest.json` list per image the engine frame, TAA
  jitter index and scene time, and per variant the settings that differ from the baseline (the
  manifest holds the full effective settings); `postprocess_get` shows an active override as
  `render_override`. Unknown setting names and wrong types are rejected before anything runs.
  Verified in a native editor: in edit mode four variants (baseline, TAA off, SSAO off, bare)
  gave consecutive jitter indices 253/254 with TAA on and a stuck index with it off; in play mode
  at time scale 2 with a block orbiting, all six images had one scene time (9.2497), consecutive
  frames differed by at most 11 levels (dither noise) against 87 between TAA on and off, 2 s of
  waiting after the capture advanced game time by 4.1 s (scale restored), and `render_override`
  was gone. Edit mode cannot pause tool scripts, so the result carries a warning and `scene_time`.
  **Comparison done** (2026-10-15), engine-side as suggested (`src/remote/visual_compare.rs`; the
  MCP crate needs no PNG decoder). Each frame is read back into memory as it is captured and
  compared at once, so at most a few frames are held. Per variant (`compare: true` by default):
  - the last image against the baseline's (the first variant's): per pixel the largest R/G/B
    difference in levels, then mean, max, share over `threshold` (default 4) and `smooth`, for the
    whole image and a `grid` (default 4x4; `N` or `[cols, rows]`) of regions, plus a `<name>_diff.png`
    (per channel, amplified by `diff_gain`, default 16);
  - the same statistics between the variant's own consecutive saved frames (its noise), plus a
    `<name>_noise.png`; `differs` and each region's `above_noise` say whether a difference clearly
    exceeds the larger noise of the variant and the baseline (`score` is how many times over the
    limit; it needs `frames` >= 2, and a warning says so for 1);
  - a settle check: the last 16 settle frames are read back too (`settle_check`, default on; one
    GPU readback per frame, no PNG), and `settle` gives the means of the last 8 and the 8 earlier
    frame-to-frame differences (smooth and share over the threshold) with `converged`, plus
    `first_to_last` and `drifting` when `frames` >= 3. It needs 16 differences (`settle_frames` 16
    with `frames` 1 or more does); a warning names the variant when history had not settled;
  - TAA variants are lined up on the reference's jitter phase (see below), reported as
    `sequence.jitter_phase`.
  The reply keeps the whole-image numbers, the three most-above-noise regions, the settle verdict
  and the image paths; `manifest.json` adds every region (with its noise), both settle series and a
  `meaning` string for each. `compare: false` gives the old reply with no extra readbacks.
  Measured in a native editor (floor, a static block and a block orbiting at time scale 2, TAA
  baseline, frozen in play mode: one scene time for all images, time scale back at 2 afterwards,
  no `render_override` left): every pair of frames differed by 0.8 levels on average per pixel,
  TAA on or off (TAA off at most 3 levels, so it is the output dither; TAA on up to 11 to 36 at
  edges), which hid everything in the mean. `smooth` (the mean difference of 8x8 block averages)
  averages that out: noise 0.11 against 0.14 to 0.24 in regions where TAA moves edges. TAA off:
  6 to 8 of 16 regions above noise, the strongest on the blocks' edges (max 94 to 129 levels,
  0.3 to 1.4% of pixels over 4, region scores 4 to 6; the whole-image score was only 1.4 to 1.8,
  which is why regions matter). SSAO off: a broad faint darkening of the floor, smooth 0.72 to 0.90
  against 0.11, 10 to 13 of 16 regions (the plain mean, 1.1 to 1.4 against 0.8, would not have
  convinced anyone). Three things only real captures showed, all fixed: (1) a settle check on the
  plain mean called TAA converged after 4 frames because the series was lost in dither, so it now
  needs both the smooth difference and the share over the threshold to have levelled off; (2) the
  frame-to-frame series repeats every 8 frames (the TAA jitter sequence, now
  `TAA_JITTER_FRAMES`), and a rule comparing the last third of the series with the third before it
  called an identical variant unsettled (share over the threshold 0.0402% against 0.0552%), so it
  now compares the last 8 frames with the 8 before and needs 16 differences; (3) frames at
  different jitter phases differ by more than consecutive frames do, so with the first saved image
  at an arbitrary phase two of three identical variants came out `differs: true` (scores 1.1 to
  1.5 in a few regions). Each variant's first saved image now waits, as extra settle frames that
  are read back like the others, up to 7 frames until its jitter phase equals the reference's;
  after that four identical variants gave `differs: false`, score 0.32 each, 0 of 16 regions, max
  difference 9 to 10 (14 before) and 0.014 to 0.017% of pixels over 4 (0.04 to 0.11% before). On
  this static scene TAA converges in one or two frames (the series is flat from the first frame;
  the engine resets the history when TAA is turned on), so every verdict was `converged: true`. No
  slowly converging effect could be made to test the negative case (auto exposure jumps at once
  as well), so that path is covered by unit tests with synthetic decays only, plus the three real
  series as fixtures for the false alarm. `threshold: 0` makes every pixel count (75% over, from
  dither) and blinds the share signal; keep it above 3. Edit mode with an open project works
  (warning about scripts); with the startup dialog still up the viewport crop shows the dialog, so
  all frames are identical and every number is 0. With the MCP server not restarted, the client
  sent `compare: false` as the string "false" and `camera: "game"` with quotes (the old schema does
  not know them); `engine_call visual.capture` takes the real JSON.
- ✅ **Browser capability preflight:** report unsupported effects and asset URL issues
  before treating a saved project's native validation or WASM build as browser readiness.
  (`project_validate`'s `browser` block and `web_build`'s early stop; see the pain point above.
  The "unsupported effects" half is mostly moot now: the bindless-texture case is handled (B21)
  rather than reported, and no other effect is gated on a browser-only capability that is known
  statically.)
- ✅ Lua `set_postprocess(settings)` (or `set_component` support for `PostProcessVolume`), so a game
  can change bloom, vignette or colour grading at run time. (`set_postprocess(settings)` edits the
  scene's infinite volume, created if missing; `set_postprocess(volume_entity, settings)` edits a
  given volume; settings are those of `postprocess_set`, plus `volume = {priority, weight, ...}`.
  It shares one function with `postprocess_set`, so selective volumes tick their overrides the
  same way, and it is stricter: an unknown setting name (`bloom.intensty`) or a wrong type is a Lua
  error naming the setting instead of being dropped silently. That check now also applies to
  `postprocess_set`. Changes queue like other script commands and apply before the frame is drawn.
  Verified in a native editor: an edit-mode call changed bloom and vignette and a typo changed
  nothing; a play-mode call to `tonemapping = "agx"` took effect and was undone on stop, with
  `discarded_on_stop` reported. Scripts cannot read the blended result back; they keep their
  own values.)

- ✅ **Fetch offscreen textures:** `texture_fetch {texture, rect, channels, points}` reading any
  named render target back from the last frame: the HDR scene before tonemapping, G-buffer
  normal/motion and position, depth, shadow maps, the ray-traced shadow mask, reflection
  textures, the path tracer's raw and denoised signals, history, moments and caustic sums. It
  returns an image (float formats mapped into view by min/max or a false-colour ramp, alpha as
  its own channel), per-channel min, max, mean and NaN/infinity counts, and raw values at
  `points`. Today the only readbacks are the presented 8-bit frame (`capture_surface` in
  `src/remote/capture.rs`, behind `screenshot` and `visual_capture`) and the editor's one-pixel
  pick id, and no remote method reads a GPU texture. Needs the renderer to register its targets
  by name (each pass adds its own) and `COPY_SRC` on them (the path tracer's targets lack it);
  `capture_surface`'s copy generalizes to rgba16f/32f, r32uint and depth. Unlike a debug view it
  leaves the displayed frame alone and gives numbers. This covers the pixel probe below, and
  would have replaced the seven `PT_DEBUG` rebuilds and the ImageMagick comparisons of this round.
  **Done** (2026-10-15): `texture_fetch` (engine method `texture.fetch`). The renderer lists 25
  targets by name with what each channel holds (`Renderer::fetchable_textures`, each pass adding
  its own; `src/renderer/readback.rs`), `COPY_SRC` was added where it was missing, and the
  readback decodes f16/f32, u32, unorm8 (RGBA/BGRA), `Depth32Float` and storage buffers laid out
  as pixels (the caustic sums). Modes: `range` (default 0.5th to 99.5th percentile), `tonemap`,
  `false_color` (Turbo); NaN/inf pixels magenta; nearest-neighbour shrinking keeps values exact.
  Verified in the release editor on RT Lab: the path tracer's specular history length as false
  colour (full 32 frames except jitter-disoccluded edges, 0 on glass: the visualization that took
  seven rebuilds before), the HDR scene tonemapped with exact values at points (and negative
  colour values in it, min -0.18, which no screenshot shows), depth sub-rectangles, a point
  shadow map's layer 23, the R32Uint shadow mask and the caustic buffer. Two things the checking
  turned up, both fixed: depth formats can only be copied whole (the first sub-rectangle read
  panicked the editor with a wgpu validation error; depth is now copied whole and cropped), and a
  rejected copy must not end the process (the readback now runs in a validation error scope and
  returns the error). Not covered: multisampled targets (listed by their resolved copies) and the
  web (the readback waits on the GPU).
- ✅ **Shader hot reload in development builds:** load WGSL from `src/shader` on disk when it
  exists and rebuild the affected pipelines on change (falling back to the built-in source, and
  keeping the old pipeline when the new one fails validation, with the error in `logs`). Most
  renderer iteration is shader-only and would no longer need a restart.
  **Done** (2026-10-15): shaders are read through `wgsl!` (`src/renderer/shader_source.rs`):
  built-in text until a reload, then the disk text. A native engine whose source tree is on disk
  polls `src/shader` (every 0.4 s, by modification time; `ENGINE_SHADER_DIR` points elsewhere,
  `ENGINE_SHADER_RELOAD=0` turns it off) and calls `Renderer::reload_shaders`, which rebuilds,
  inside a validation error scope, the main pass (PBR and error modules, depth prepass,
  background; pipelines and shader materials rebuild lazily), every post-processing stage
  including RT reflections, RT AO, the path tracer and FSR, and RT shadows. Composed sources are
  checked with naga first. Any failure puts the previous text back and keeps every old pipeline;
  the error goes to `logs` (target `shader`). The `shader_reload` tool (engine `shader.reload`)
  does it on demand and returns `changed` and `not_reloaded` (changed files used only by passes
  that are not rebuilt: shadow maps, skinning, particles, environment, HDR output). Verified in the
  release editor: zeroing red in `final.wgsl` took effect on its own within a second
  (displayed red 0, green and blue unchanged); a call to an undefined function was refused with
  naga's message and the red-free pipeline kept running; restoring the file brought red back; with
  path tracing on, zeroing blue in `pt_composite.wgsl` through `shader.reload` took effect (mean
  blue 0.56 over a region, red and green unchanged); a change to `shadow.wgsl` came back in
  `not_reloaded`. No wgpu errors. Limits: error line numbers count lines in the composed source
  (naga shows the offending line, so it is findable); changes to Rust (layouts, uniforms) still
  need a rebuild.
- ✅ **Format-preserving saves:** `project_save` keeps each file compact or indented as it found
  it, and leaves files whose content did not change (including import `meta.json` mappings, see
  B24) untouched. **Done** (2026-10-15): `io::write_json` with `JsonLayout::Preserve` for scene
  files, materials, `meta.json` and `project.json`; `project_save {compact}` still forces one
  layout (true compact, false indented) and omitted keeps each file's. The editor's own saves
  preserve too. Test: `repeated_captures_reuse_inline_material_files_and_compact_saves_roundtrip`
  (a compact file stays compact and is not rewritten). On RT Lab: 2 files written instead of the
  whole project, scene diff 1 line.
- ✅ **Lathe shells:** `mesh_lathe {thickness}` (or allowing a profile that comes back down) for
  cups, glasses, bowls and vases with an inner wall. **Done** (2026-10-15): `thickness` offsets
  the (beveled) outer profile inward with mitred corners and closes the outline: flat rims at open
  ends, a floor when the profile starts on the axis, a dome open below when it ends there. Profiles
  whose inner wall would fold over are rejected. `mesh_compose` lathe parts take it too. Test:
  `lathe_thickness_makes_closed_outward_facing_shells` (tube and tumbler volumes within 1 %,
  outward winding everywhere, the floor at the thickness). A glass tumbler rendered with path
  tracing shows its rim and inner floor.
- ✅ **Pixel probe:** linear HDR values at viewport pixels from the last frame, for checking
  brightness and noise with numbers instead of by eye. **Done:** `texture_fetch {texture:
  "scene", points}` (and `display` for the tonemapped image) gives them, and (2026-10-15)
  `screenshot {points, rect}` returns the final 8-bit values as displayed (after anti-aliasing,
  grain and dither) at points, with per-channel min, max, mean and standard deviation over `rect`,
  in the captured image's pixels. The shader reload checks above were measured with it.
- ✅ **A held view for `screenshot`:** `screenshot {camera, settle_frames}` that keeps the pose
  for that many frames before capturing, without touching the scene's camera entity, for quick
  looks at temporal effects (`visual_capture` does this for comparisons; a single look should not
  need a variant list). **Done** (2026-10-15): the capture renders `settle_frames` frames
  (up to 600) through its camera or aspect before capturing; other screenshots wait meanwhile so
  none sees that camera. With path tracing on, the same pose without settling shows unconverged
  grain and with 64 settle frames the denoised image. Test:
  `held_screenshots_settle_before_capturing_and_report_displayed_pixels`.

- ⬜ **Wait for a condition:** `wait {until: "<Lua expression>", entity, timeout}` that returns
  when a script's state says so (`SHOW.mode == "results"`), for benchmarks, loading screens
  and scripted sequences longer than a tool call.
- ⬜ **GPU memory in `engine_stats`:** bytes the engine holds by kind (textures, meshes, skinned
  outputs, ray tracing pool and acceleration structures, render targets), so paging shows up
  where the timings do.
- ⬜ **Call stacks in `engine_profile_cpu`:** record with call graphs (frame pointers or DWARF)
  so `callers` attributes hot leaves.
- ⬜ **`library_add {ids, copy_only: true}`** (or a `library_export {ids, dest}`): copy characters
  and clips into `content/` without instancing them, for games that spawn at run time.
- ⬜ **Flat lathes:** allow profiles whose points share one height (discs and annuli).
- ⬜ **Release unused imported resources:** textures and meshes of models nothing uses any more
  stay on the GPU for the scene's life (B31's cache only stops them multiplying).

## Tide Observatory: what worked well

- Procedural sweeps made thin orbital rings and curved supports; lathes and composed
  meshes supplied the basin, lanterns and engraved divisions. Seeded texture generation
  produced reusable brass, marble and water PBR maps entirely through MCP.
- Atomic batches were a useful authoring boundary: a bad profile rolled back the whole
  edit without leaving half a scene. Existing quiet batches and compact saves helped
  keep routine geometry authoring manageable.
- Screenshots, Lua evaluation and input injection made the controls and renderer
  inspectable. Fixed-step assertions covered orbit, zoom, pause/resume, captions, reset
  and stable mesh counts; screenshots checked the composition at 16:9 and 4:3.
- `project_validate`, native player restart, profiling and browser smoke tests answered
  different questions. Native validation and WASM compilation passed while the browser
  correctly exposed runtime failures; full repository tests caught the undo regression.

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
