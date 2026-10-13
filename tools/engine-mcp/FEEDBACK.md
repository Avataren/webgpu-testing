# Engine MCP: bugs, pain points and wish list

Collected while building a Breakout game end to end through the MCP
(`games/breakout`, 2026-10-13). Add new findings at the bottom of the matching section.

## Status

First round of fixes: commit `7f0fd5f` (branch `fix/engine-mcp-feedback`, 2026-10-13).
Every bug is fixed and was checked against a running editor and player.

Second round (branch `feat/engine-mcp-feedback-2`, 2026-10-13): every remaining pain
point and wish-list item is done (audio, Lua meshes/materials, world-space text,
non-glTF import, texture reload, game-camera screenshots, bulk delete, scene diff,
playtest, time scale, batch). Breakout now plays its sounds and shows score popups.
Checked in the editor and the player, partly through a fresh MCP server binary. Each entry below
ends with a **Resolution** (bugs), **Done** / **Partly done** / **Open** (pain points) or a
✅ / ⬜ mark (wish list).

MCP-server changes (new tool parameters, blocking project tools) only take effect after the
MCP server is restarted; engine changes take effect after `engine_launch` rebuilds.

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

## Pain points

- **No bulk creation.** A grid of 72 bricks costs 72 `entity_spawn`/`entity_duplicate`
  calls, because Lua cannot add meshes or materials and there is no batch tool.
  **Done:** `entity_duplicate {count, step, row_step, columns}` lays out a row or grid in one
  call and returns only ids and names.
- **The engine has no audio at all.** No `play_sound` in Lua and no audio crate in
  `Cargo.toml`, so "add sounds" cannot be done from the MCP side. **Done:** `rodio`
  playback (wav/ogg/mp3/flac) with Lua `play_sound`/`stop_sound`/`stop_all_sounds`/
  `set_master_volume`, MCP `sound_play`/`sound_stop`/`audio_status`. Sounds stop with
  play mode. A play history lets agents check which sounds played. Native only; web
  builds record plays but stay silent.
- **Lua cannot add or change meshes, materials or lights.** Anything that varies at
  runtime (brick colours, damage states) needs one pre-made entity per variant. **Done**
  for meshes and materials: `spawn_primitive`, `set_mesh`, `set_material_color`,
  `set_material` (copy-on-write for shared materials). Lights and cameras still need the
  editor or the tools.
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

Related web gaps: audio records plays but is silent on the web (rodio would need cpal's
WebAudio backend), and file hot-reload is native only.
