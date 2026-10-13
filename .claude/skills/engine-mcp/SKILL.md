---
name: engine-mcp
description: Drive the running engine/editor through the `engine` MCP server (mcp__engine__* tools) to build scenes and whole games, write and playtest Lua gameplay, inspect or edit live objects, inject input and take screenshots. Use whenever the user wants something built, changed, inspected or tested inside the engine rather than in its Rust source, or asks to see what the engine shows.
---

# Building with the engine over MCP

The `engine` MCP server (`tools/engine-mcp/`, registered in `.mcp.json`) talks to the
engine's remote-control socket (`src/remote/`). Its tools appear as `mcp__engine__<name>`.
If they are missing, the server has not been approved or loaded; ask the user to
restart Claude Code in the repo root and approve it.

## Start every session the same way

1. `engine_launch` (target `editor`, the default). It builds incrementally, starts the
   editor and waits until it accepts commands. If the user already has an editor open
   on port 47811, it attaches to that one instead; never restart an editor you did not
   launch, because the user may have unsaved work in it.
   Don't pass `monitor` unless the user asks: their `settings.json` chooses which monitor
   engine windows open on (they may be using the other one).
2. `engine_info`: runtime mode, active scene, value conventions and, in the editor,
   `editor.project` and `editor.startup_dialog_visible`.
3. No project open? Use `project_create {name, location}` or `project_open {path}`, then
   `wait {frames: 30}` and check `editor_state`. The startup dialog blocks the editor UI
   until a project is open. `editor_state.project.scripts_dir` is where scripts go.

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
- Primitives differ in size: the cube spans -0.5..0.5, the sphere has radius 1, and
  cylinder and cone lie along Z. `entity_get` shows `mesh.local_bounds`. The plane is 1x1
  in XY facing +Z, so a floor is
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
- For many objects (grids, rings, rows), loop in `lua_eval`, or call `entity_spawn` once
  and then `entity_duplicate {entity, translation}` the result.
- `camera_set {focus: <entity>}` or `{position, target}` to frame the shot, then
  `screenshot {target: "viewport"}` (just the 3D view) or `"window"` (with the editor
  UI). **Look at the screenshot after visual changes** instead of assuming they worked.
- `environment_set` controls background, ambient light, HDR sky and grading;
  `postprocess_set` toggles SSAO, bloom and FXAA.
- `asset_import {path}` brings in a glTF model (samples are under `web/assets/`).

## Gameplay with Lua

Call `lua_api_reference` before writing scripts. Then:

1. Write the `.lua` file into `editor_state.project.scripts_dir` with your own file
   tools.
2. Attach it: `entity_update {entity, script: {path: "<absolute path>"}}`. It compiles on
   the next frame. While a project is open, the editor hot-reloads saved edits to files
   under its `content/` folder, so later changes need no reattach.
3. `script_status`: `compiled` and `last_error` for every script. Errors read
   `file.lua:LINE: message`.
4. Playtest: `runtime_set {mode: "play"}`, `wait`, inject input, `wait`, then check state
   (`entity_get`, `lua_eval`, `logs {target: "script"}`, `screenshot`).
5. `runtime_set {mode: "stop"}` restores the scene to its pre-play state. Every change
   made during play (including your remote edits) is discarded.

Facts that change how you write scripts:

- Scripts without an annotation run `on_created`/`update` **only in play mode**. Use
  `-- @tool` for scripts that should also run while editing, and `-- @editor` for
  editor-only ones.
- Lua can spawn entities and set transforms, names, visibility, shadows and the built-in
  rotate/orbit animations, but it **cannot add meshes, materials or lights**. For things
  that appear at runtime (bullets, enemies, pickups), pre-place a pool in the editor with
  `entity_spawn`/`entity_duplicate`, start them hidden (`visible: false`), and have
  scripts move and show or hide them (`set_component(e, "Visible", true)`).
- Mutations from a hook are queued and applied after it returns, so reading back a
  value you just set in the same hook gives the old value.
- Keep game state in the state API (`set_f64`/`get_f64`, `set_state`/`get_state`). It
  survives hot reloads and is readable from outside with
  `lua_eval {entity, code: "get_f64('score', 0)"}`.
- Input key names: letters are upper case (`"W"`), and the named keys are `Space`,
  `Return`, `Escape`, `ArrowUp`/`Down`/`Left`/`Right`, `Shift`, `Control`, `Alt`, `Tab`.
  Mouse buttons are 0 left, 1 right, 2 middle.

## Injecting input

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
- Without `entity`, globals persist between calls like a REPL. With `entity`, the code
  runs inside that entity's script, so it can read or change the script's globals and
  state.
- Pass `read_only: true` for pure queries, so they don't add undo steps in the editor.

## Persisting work

- `project_save` writes the project. It refuses while playing; stop first.
- `environment_set` creates an Environment entity when the scene has none, because only
  that entity round-trips (the project-level environment is not restored on load).
- Script paths are stored as given (absolute), so a project that references scripts by
  absolute path is not portable to another machine.
- `editor_undo`/`editor_redo` step through scene changes, including remote edits made
  in editor mode.
- To check the game outside the editor:
  `engine_launch {target: "player", project: "<dir>"}` on a saved project. It starts in
  play mode.

## When something goes wrong

| Symptom | Check |
|---|---|
| Tool says no engine is listening | `engine_status`; `engine_output` shows why the process died |
| Engine crashed or validation errors | `engine_output {contains: "panicked"}` or `{contains: "error"}` |
| Script does nothing | `runtime_mode` must be `playing` for unannotated scripts; then `script_status` |
| Script error | `script_status` and `logs {min_level: "warn"}`; fix the file and it hot-reloads |
| Object invisible | It needs a light, `visible: true`, a sensible scale, and must be in front of the camera; `camera_set {focus}` then screenshot |
| Change vanished | It was made during play mode (stop restores), or the project was never saved |
| New logs only | Pass `after: <latest_seq>` from the previous `logs` call |

Close an engine you launched with `engine_stop` when done, after saving. Leave engines
the user started alone.
