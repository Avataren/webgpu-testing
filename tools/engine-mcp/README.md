# engine-mcp

An [MCP](https://modelcontextprotocol.io) server that gives AI agents (Claude Code and
other MCP clients) full control of the engine: building and launching it, creating
projects, building scenes, writing and running Lua gameplay, injecting input, reading
logs and taking screenshots.

It is a small standalone crate (only `serde_json`), so it builds in seconds without
compiling the engine. It talks to a running engine over the engine's remote-control
socket (`src/remote/`, documented in `docs/ARCHITECTURE.md`).

## Setup

The repository's `.mcp.json` registers it for Claude Code as the `engine` server, so
starting Claude Code in the repo root is enough (approve the server when asked).
`run.sh` builds the binary on first use and whenever its sources change.

The `engine-mcp` skill (`.claude/skills/engine-mcp/SKILL.md`) teaches agents the
workflow and the engine's gotchas; keep it in step with the tools.

For other clients, run `tools/engine-mcp/run.sh` (or the built
`tools/engine-mcp/target/release/engine-mcp --repo <repo>`) as a stdio MCP server.

Options and environment:

| | |
|---|---|
| `--repo DIR` / `ENGINE_REPO` | Engine repository (default: the one the binary was built in) |
| `--port PORT` / `ENGINE_REMOTE_PORT` | Port of the engine's remote control (default 47811) |
| `ENGINE_CARGO` | cargo to build the engine with (default: first toolchain satisfying `rust-version`, stable first) |
| `ENGINE_RUST_LOG` | `RUST_LOG` for launched engines (default `info`) |
| `ENGINE_MONITOR` | Monitor for launched engine windows (inherited; `engine_launch {monitor}` overrides). The engine also reads `monitor` from `settings.json`. |

## How it works

- `engine_launch` builds the target with `cargo build --all-features`, starts it with
  `ENGINE_REMOTE_PORT` set and waits for the socket. If an engine already listens on
  the port (for example an editor you started yourself), it attaches instead. The
  engine's stdout/stderr go to `target/engine-mcp/engine.log` and `engine_output`.
  On Linux, it checks for `DISPLAY` or `WAYLAND_DISPLAY` before building. Use
  `engine_launch {env: {"DISPLAY": ":0"}}` to supply a desktop environment; Wayland
  may also need `XDG_RUNTIME_DIR`. Other child variables inherit from the MCP server
  unless overridden by `env`. The remote port remains controlled by the server.
  Attaching discards any socket left from a previous process on the same port.
- Every other tool is one request to the engine (see `src/tools.rs` for the mapping),
  except `engine_info` (which combines several), `screenshot` (returns an image),
  `lua_api_reference` (reads `scripts/LUA_SCRIPTING_README.md`; `outline`, `section` and
  `search` return just a part of it), `batch` (runs several tools in one call) and
  `playtest` (play, input, wait, report, screenshot, stop; `lean` for soak tests).
  `engine_info` also reports whether this server is older than its sources
  (`mcp_server.restart_needed`).
- The shared authoring workflow is available as the `engine://workflow` MCP resource.
- Closing the MCP server leaves a launched engine running, so unsaved work in the
  editor is never lost; use `engine_stop` to end it.

## Tools

| Area | Tools |
|---|---|
| Process | `engine_launch`, `engine_stop`, `engine_status`, `engine_output`, `engine_build` |
| Overview | `engine_info`, `engine_stats`, `engine_call` (any method), `batch` |
| Scene | `scene_list_entities`, `scene_tree`, `entity_get`, `assets_list`, `scene_mark`, `scene_diff` |
| Editing | `entity_spawn`, `entity_update`, `entity_remove_component`, `entity_delete`, `entity_duplicate` |
| Scripting | `lua_eval`, `script_status`, `script_reload`, `lua_api_reference` |
| Runtime | `runtime_set` (play/stop, time_scale), `wait`, `playtest` |
| Audio | `sound_play`, `sound_stop`, `audio_status` |
| Procedural assets | `mesh_lathe`, `mesh_extrude`, `mesh_compose`, `mesh_sweep`, `mesh_heightfield`, `texture_generate`, `texture_maps`, `sound_generate` |
| Animation | `entity_update {animator}`, `animation_inspect`, `debug_skeleton` (skinned characters; `entity_get` shows `skinned_mesh` and `animator`) |
| Input | `input_send`, `input_state`, `input_clear` |
| View | `screenshot`, `visual_capture`, `camera_get`, `camera_set`, `environment_get`, `environment_set`, `postprocess_get`, `postprocess_set`, `logs` |
| Render debugging | `texture_fetch`, `shader_reload` |
| Global illumination | `entity_spawn {kind: "light_probe_volume"}`, `light_probes_bake`, `light_probes_status` |
| Editor | `editor_state`, `project_create`, `project_open`, `project_save`, `scene_new`, `scene_activate`, `scene_close`, `scene_rename`, `scene_instance`, `asset_import`, `asset_reload`, `editor_select`, `editor_undo`, `editor_redo` |
| Spatial | `scene_query`, `scene_raycast`, `entity_place`, `scene_scatter` |
| Prefabs | `prefab_create`, `prefab_inspect`, `prefab_update` (instances use `scene_instance`) |
| Verification | `runtime_step`, `profile_capture`, `project_validate`, `web_build`, `web_playtest` |

Value conventions: meters, Y up, -Z forward; rotations as Euler degrees `[x, y, z]`
applied yaw, pitch, roll (like Lua's `set_rotation`); colors as `[r, g, b(, a)]` in 0..1
or `"#rrggbb"`; entities by id (the same 64-bit handle Lua uses) or unique name.

Skinned glTF characters (samples in `web/assets/skinned/`) import with an animator on
their root. Clips bind to bones by name, so `asset_import {path, kind: "animation"}` of an
animation-only GLB followed by `entity_update {entity, animator: {add_clips: path}}` gives
a character new moves. Animators play in play mode and under `runtime_step`; while
editing, `animator.play` sets `default_clip` instead. `debug_skeleton {enabled: true}`
draws the bones in screenshots.

`engine_info.limits` reports renderer and procedural geometry capacities. The renderer
uses at most four lights of each type, in scene/world iteration order, shared across
scene instances. It does not rank lights by distance or influence.

`project_save` returns project/scene paths and `written_count`; `verbose: true` adds
the full `written` list. Files whose content would not change are not rewritten, and
each scene file keeps the layout it has on disk (compact or indented; new files are
indented), so saving a project with compact scenes does not reformat them. `compact:
true` writes every scene file without whitespace, `compact: false` indents every one.
An imported model's `meta.json` (which material file new instances of it get) keeps
entries that still name an existing file, so scenes that give the model different
materials do not rewrite it on each save. Inline editor materials use stable entity identities across saves, so
repeated captures update the same material files. Existing orphan files are retained.

## Procedural geometry

`mesh_lathe` revolves `[radius, height]` profile points around Y. Points run bottom to
top; equal heights form ledges and radius zero makes a pole. `segments` controls radial
resolution (64 default), `caps` closes nonzero end radii, and `crease_angle` (45 degrees
default) preserves sharp profile corners while smoothing the rest. For example:

`thickness` makes a hollow shell: the profile is the outer wall, the inner wall is it
offset that far inward, and open ends get flat rims. A profile starting at radius 0 gets
a floor (cups, glasses, bowls, vases); one ending at radius 0 is a dome open below
(bells, lampshades). `caps` is ignored for shells, and the thickness must stay below
the smallest radius and feature size. A tumbler:
`{profile: [[0, 0], [0.35, 0], [0.4, 1]], thickness: 0.02}`.

Use `bevel` to trim and soften profile corners. `bevel_segments: 1` gives a chamfer;
2–8 subdivisions round each corner. Trim distances clamp to fit short profile edges,
so collars and fine details stay intact. Endpoints and zero-radius poles are preserved.

```json
{
  "name": "Pawn body",
  "profile": [[0.4, 0], [0.45, 0.1], [0.3, 0.2], [0.15, 0.8], [0.22, 1]],
  "segments": 64,
  "bevel": 0.02,
  "bevel_segments": 3,
  "material": {"color": "#eadbc0", "roughness": 0.3}
}
```

`mesh_extrude` takes a simple XY `polygon` in either winding and extrudes it along Z,
centered at the origin. Concave silhouettes are triangulated, so a knight's outline or
a king's cross can be modeled as one polygon. `depth` sets the length; `bevel` insets
both end rims, and `bevel_segments` (1–8) subdivides a rounded bevel. `taper` scales the
positive Z end relative to the negative end, `twist` rotates it in degrees, and `steps`
subdivides the length for twisting. Taper and twist are about the XY origin. Holes,
self-intersections and bevels that collapse edges are rejected.

```json
{
  "name": "Beveled cross",
  "polygon": [[-0.08,-0.3],[0.08,-0.3],[0.08,-0.08],[0.25,-0.08],
              [0.25,0.08],[0.08,0.08],[0.08,0.3],[-0.08,0.3],
              [-0.08,0.08],[-0.25,0.08],[-0.25,-0.08],[-0.08,-0.08]],
  "depth": 0.18,
  "bevel": 0.02,
  "bevel_segments": 3
}
```

`mesh_compose` combines transformed parts into one mesh with one material. Parts have
`shape: "lathe"`, `"extrude"` or `"primitive"`, the corresponding shape parameters,
and optional local `translation`, Euler-degree `rotation`, and `[x,y,z]` `scale`.
Nonuniform scales transform normals correctly; negative scales also reverse winding.
Primitive parts use the same sizes and axes as `entity_spawn`. Composition joins
surfaces; it does not perform a boolean union or remove internal intersections.

```json
{
  "name": "Turned ornament",
  "parts": [
    {"shape": "lathe", "profile": [[0.4,0],[0.45,0.1],[0.15,0.8],[0.2,1]]},
    {"shape": "primitive", "primitive": "sphere",
     "translation": [0,1.2,0], "scale": [0.25,0.25,0.25]}
  ]
}
```

All three tools create one entity, or replace the geometry of `entity` while keeping
its material and transform unless those fields are supplied. They accept `name`,
transforms, `material`, `parent`, `visible` and `cast_shadow`. Each mesh includes UVs,
normals and tangents for procedural PBR maps. Identical outputs share GPU geometry;
`entity_duplicate` reuses it too. Each call returns bounds, vertex/triangle counts and
a recipe. `project_save` embeds the generated meshes in the scene, so reopening or
exporting the project requires no generator or external model. Generation is limited
to 250,000 vertices and 1,500,000 indices per mesh, including all composed parts.
`mesh_compose` accepts 1–128 parts; split larger assemblies into separate meshes.

The [procedural chess test project](../../examples/projects/procedural-chess/README.md)
contains a complete 32-piece board and a separate study of all six types in both
materials. Its [MCP generation script](examples/procedural_chess.py) builds every
piece, the board, pedestals, and seeded marble/wood/brass texture maps through MCP.

## Procedural textures, material maps and sounds

These tools run in a native editor or player. They save reusable files immediately;
`project_save` is needed to persist any material assignment made to a scene entity.
Generated PNGs and WAVs also work in exported web games. With a project open, output
goes to `content/textures` or `content/audio`; `directory` overrides that location and
allows generation without a project. Names use letters, digits, `_` and `-`.
Existing files require `overwrite: true`. A JSON recipe alongside each asset records
the tool parameters; the same seed and parameters reproduce the same output.

`texture_generate` builds a periodic height field from `noise`, `bricks`, `checker`,
`wood`, `marble` or `voronoi`, then derives matching maps. Default size is 256×256,
with a maximum of 2048 pixels per axis. For example:

```json
{
  "name": "arena_bricks",
  "pattern": "bricks",
  "seed": 42,
  "size": [512, 512],
  "scale": 4,
  "colors": ["#241c28", "#805268"],
  "normal_strength": 0.04,
  "roughness": 0.75,
  "entity": "Wall"
}
```

Default maps are `base_color`, `height`, `normal`, `metallic_roughness` and `occlusion`.
Use `maps` to request a subset or add standalone `roughness` and packed `orm`.
The response includes `paths`, a map preview in `preview_order`, and a `material`
object accepted by `entity_update`. Passing `entity` applies these settings directly.
Regenerating textures already used by the scene reloads them immediately.

Normal maps use tangent-space +Y; `flip_y` reverses the green channel. Strength
expresses bump height relative to UV extent, so changing resolution preserves relief.
Data maps load as linear values, and base colour loads as sRGB. Packed channels follow
the engine's PBR layout: metallic/roughness uses G=roughness, B=metallic; ORM uses
R=occlusion, G=roughness, B=metallic. The returned material sets metallic and roughness
factors to 1 so the maps supply the intended values. Height and standalone roughness
are export maps; height does not displace the engine's geometry. Roughness and occlusion
are approximations derived from height, rather than measured material properties.

`texture_maps` derives the same maps from an existing height image:

```json
{
  "name": "stone_detail",
  "source": "content/textures/stone_height.png",
  "channel": "red",
  "maps": ["normal", "orm"],
  "normal_strength": 0.08,
  "roughness": 0.6,
  "tileable": true
}
```

Channels are `luminance` (default), `red`, `green`, `blue` and `alpha`; `invert` swaps
raised and recessed areas. `tileable` wraps edge sampling; false clamps it. Wrapping
preserves an already periodic source but cannot remove seams in an arbitrary image.
Luminance from an albedo image is only an approximate height source.

`sound_generate` produces mono 16-bit PCM WAV effects with presets `tone`, `blip`,
`laser`, `explosion`, `pickup`, `jump`, `hit` and `powerup`:

```json
{
  "name": "player_laser",
  "preset": "laser",
  "seed": 7,
  "duration": 0.22,
  "frequency": 1600,
  "end_frequency": 120,
  "noise_mix": 0.08,
  "volume": 0.7
}
```

Customize sine/square/saw/triangle/noise waveforms, logarithmic pitch sweeps, attack,
release, decay, pulse width, vibrato and low-pass filtering. Duration is 0.02–10 seconds;
sample rates range from 8 kHz to 96 kHz (44.1 kHz default). Output is peak-normalized
with clipping headroom and short endpoint fades. The response automatically includes
`sound_inspect` measurements and a waveform/spectrogram. Use `path` with `sound_play`
or Lua `play_sound`; replacing a clip clears its cache for the next playback.
`image: false` omits previews and spectrograms from any generation tool.

## Repeatable gameplay tests

`runtime_set {mode: "play", manual: true, seed: 123}` starts an explicit-step session.
Configure it while stopped, then `wait {frames: 2}` for the transition. Lua chunks and
`on_created` run once with the seeded VM; automatic frames then leave gameplay and scene
animation frozen. `runtime_step {steps: 60, dt: 0.016666666666666666}` advances one second.
Input `delay` and tap `frames` refer to simulation steps, starting at zero; queued releases
survive subsequent step calls. `input_clear` clears both frame and simulation queues.
Stop resets manual mode, the seed configuration and time scale.

Rendering, GPU particles, UI hooks, external I/O and custom application's update systems
are outside this deterministic scope. Seeds do not guarantee identical pixels or make
scripts using wall clocks, external state or their own `math.randomseed` deterministic.

`playtest` accepts `steps`, `dt`, `seed`, `setup` and `assertions`, in addition to its
existing reporting options. Fixed-step tests restart play from the editor snapshot.
`setup` runs after initialization, in `entity`'s script (or the uniquely inferred script).
Each assertion's `code` is a Lua expression that must return boolean `true`:

```json
{
  "steps": 120, "seed": 123, "entity": "Controller", "lean": true,
  "setup": "RT.god(true)",
  "input": [{"key": "A", "action": "tap", "delay": 10, "frames": 3}],
  "assertions": [
    {"name": "initialized", "at_step": 0, "code": "GAME.wave == 1"},
    {"name": "alive", "at_step": 60, "code": "GAME.lives > 0"},
    {"name": "completed", "code": "GAME.lives > 0"}
  ],
  "eval": "return GAME.stats"
}
```

An omitted `at_step` checks at the end; the first failed checkpoint ends simulation and
returns `ok: false`, `first_failure`, assertion observations, logs and the optional
screenshot before stopping. Script failures also make `ok` false. Seconds-based tests
support setup and final assertions. `save_scenario` writes the resolved test to a new JSON
file; `scenario` loads it, with explicit arguments overriding saved ones. Save paths are
relative to the repository unless absolute. Parent directories are created before
simulation begins. Existing scenario files are never replaced.

`lua_eval` without `entity` uses a persistent console without `require`/`include`;
pass a scripted entity to use its module loaders. `script_reload` queues work for the
next simulation update. In a frozen manual session, use `runtime_step` to apply it;
waiting for rendered frames alone does not advance scripts.

## Spatial editing and transactions

`scene_query {bounds: {min, max}}` finds meshes whose world boxes intersect a region;
`{center: [x,y,z], radius}` measures distance to each box. Results have world bounds,
pagination (`offset`, `limit`) and optional distance. `name_contains`, `exclude` (entity
hierarchies) and `visible_only` filter both queries and rays; hidden ancestors are respected.

`scene_raycast {origin, direction, max_distance, limit}` returns distance-sorted hits,
world positions and normals. **These are world AABB intersections**, including box-face
normals on rotated meshes; they are approximate placement queries, not triangle collision.
`entity_place {entity, origin, direction, offset, align_normal, avoid_overlap}` places a
hierarchy's support point on the nearest hit, excluding itself. Parent transforms are
respected; a rejected placement restores its transform. `scene_scatter` copies a hierarchy
into XZ `bounds: {min, max}` with `count`, `seed`, `spacing`, `on_surface`, `max_slope` and
`avoid_overlap`. All copies must fit within the attempt budget or nothing is created.

`batch {atomic: true, calls}` validates and edits a speculative scene, committing once as
one undo entry. An error restores the original scene. `dry_run: true` validates without
committing and implies atomic mode; its returned entity IDs are speculative. Give a call
an `as` alias, then reference its result using JSON-pointer syntax:

```json
{"atomic": true, "calls": [
  {"tool": "entity_spawn", "as": "root", "arguments": {"kind": "empty", "name": "Group"}},
  {"tool": "entity_spawn", "arguments": {"kind": "cube", "parent": {"$ref": "root/id"}}}
]}
```

Transactions require editor mode and accept scene/entity edits, spatial queries, placement,
scatter, procedural meshes, camera/environment/postprocessing edits and scene inspection.
They exclude scripts, files, asset generators/imports, audio, project lifecycle and runtime
operations. Normal batches retain their existing sequential behavior.

`batch {quiet: true, ...}` returns identifiers and counts for successful calls,
including geometry bounds/counts, while omitting recipes and detailed entity payloads.
It works with sequential, atomic and dry-run batches; errors and rollback details remain.
Alias resolution still uses the complete results. Call tools individually, or omit
`quiet`, when you need detailed results.

`batch {aggregate: true, ...}` goes further and drops the row per call: `summary.calls`
counts successful calls per tool, `summary.entities` lists the entities they returned
(up to 24, otherwise the count with the first and last), `summary.aliases` maps each `as`
alias to its id, and `failed_calls` keeps every failed call in full. The report's verdict
(`ok`, `failure`, `rolled_back`, `committed`) is unchanged. It overrides `quiet`.

## Prefabs, geometry and performance

`prefab_create {entity, name}` saves a hierarchy as a scene document with embedded geometry,
portable material/script paths, a zeroed root transform and fresh editor identities.
`prefab_inspect {document_id}` returns its source, linked instance entities and serialized
overrides. `prefab_update {entity, document_id}` replaces a source created by this tool.
Use `scene_instance {document_id, node_path: ["RootName"]}` for linked instances.
Source files save immediately; `project_save` persists registration. New instances load
updated sources; existing instances retain their current values until recreated or reopened.
Linked child scenes retain their source links across play/stop, transactions and project
save/reopen. Instance placement and root visibility are saved as overrides.
Updating from another selection preserves the source root name so existing links stay valid.

`mesh_sweep {path, radius, segments, caps}` sweeps a circular section along an open 3D
polyline, using transported frames. Supply `profile: [[x,y], ...]` for a simple polygonal
section. Sharp bends may intersect; reversing paths and invalid profiles are rejected.
`mesh_heightfield {resolution, size, height_scale, seed, noise_scale, octaves}` builds XZ
terrain. Alternatively provide row-major `heights` (x fastest), or a `source` image resized
to resolution and sampled as luminance. Explicit/image values multiply `height_scale`.
Both generators include UVs, normals and tangents, accept the ordinary geometry editing
fields, share the existing mesh budget, and persist through `project_save`.

`visual_capture {variants: [{name, postprocess}], frames, settle_frames}` renders one frozen
view under several post-processing variants, saves the frames and compares them. The variants are
render-only overrides (the same settings as `postprocess_set`, over the scene's own
post-processing or, with `base: "defaults"`, over the engine defaults), so no scene edit has
to be undone; the camera is locked to the current view (or `camera`) and play mode is frozen
(time scale 0) for the duration. Each variant is applied, left to settle for `settle_frames`
(temporal anti-aliasing needs about 16), then `frames` consecutive frames are written as PNGs
of the scene viewport to `dir` (default: a new folder under the system temp directory). The
reply and `manifest.json` list per image the engine frame, TAA jitter index and scene time,
and per variant the settings that differ from the baseline; `manifest.json` also holds the
full effective settings. `warnings` say when the scene was not frozen (edit mode runs tool
scripts) or the settings changed mid-capture. `postprocess_get` shows an override while one is
active (`render_override`).

The frames are compared as they are captured (`compare: false` skips it and the extra readbacks).
The first variant, `baseline` unless `include_baseline: false`, is the reference. A pixel's
difference is its largest R/G/B difference in 8-bit levels. Per variant, for the whole image and for
a `grid` of regions (default 4x4, `N` or `[columns, rows]`):

- `mean`, `max` and `over_pct` (the share of pixels over `threshold`, default 4) between its last
  image and the reference's, plus `smooth`: the mean difference of 8x8 block averages. The output is
  dithered, so any two frames of an unchanged view differ by about 0.8 levels per pixel with TAA on
  or off, which hides faint or broad changes in `mean`; averaging within blocks removes it, so
  `smooth` is the number for "did this effect shift the picture" and `over_pct` / `max` for small
  strong changes such as moved edges. Keep `threshold` above the dither (about 3 levels).
- `noise`: the same numbers between the variant's own consecutive saved frames (dither and TAA
  jitter; `frames` of 2 or more). `differs` is true when the whole image or any region has a
  `score` above 1: the larger of `smooth` and `over_pct` as a multiple of its limit (twice the
  larger noise of the variant and the reference, plus a margin). `top_regions` lists up to three
  regions above the noise, worst first, with their pixel `rect`.
- `diff_image` (`<name>_diff.png`): the per-channel difference to the reference, multiplied by
  `diff_gain` (default 16, so 3 levels show as 48; black is identical), and `noise_image`
  (`<name>_noise.png`), the same for the last two saved frames, to see what jitter looks like.
- `settle`: the last 16 settle frames of each variant are read back too (`settle_check`, default on;
  one GPU readback per frame) and the frame-to-frame difference over them and the saved images is
  judged by `smooth` and by `over_pct`: the last 8 frames (one TAA jitter period) against the 8
  before, and `converged` is true when neither is still falling (a plateau at the noise level
  counts). With 3 or more `frames`, `first_to_last` and `drifting` say whether the first saved image
  is still far from the last. A warning names a variant whose history had not settled; raise
  `settle_frames`. `converged` is null when there are fewer than 16 frame-to-frame differences to
  judge, e.g. with `settle_frames` under 16 (see the `note`).

Frames at different TAA jitter phases differ by more than consecutive frames do, so each TAA
variant's first saved image waits, as extra settle frames, up to 7 frames until its jitter phase
matches the one of the reference's first image (`sequence.jitter_phase`; the `jitter_index` of every
image is in the reply). Variants without TAA are not jittered and do not wait.

The reply keeps the whole-image numbers, `top_regions` and the settle verdict; `manifest.json` has
every region with its noise and score, the full settle series and a `meaning` string for each.
On a static scene TAA converged within one or two frames, so expect `converged: true` unless a
variant switches on something slow.

`profile_capture {frames: 300}` samples consecutive frames, returning median/p95/p99/max
frame and stage times, cumulative script-hook cost over the capture, and entity/asset
growth. Pass an earlier report as `baseline` for latency comparisons. CPU timings include
submission and display waits; they do not measure GPU execution. Use the player to measure
game costs without editor overhead. Reloading/despawning scripts during a capture can
reset or remove their cumulative counters.

## Troubleshooting

An error reading `MCP tool call requires approval, but approval policy is never` is raised by
the MCP client, never by this server or the engine: the client's approval policy forbids calls
that need confirmation. Reads can keep working while edits are refused, which looks like a
broken engine. Allow this server's tools in the client's approval settings and retry.
A real connection problem looks different: `engine_status` reports whether the engine process
is running and reachable, and `engine_output` shows its log.

## Validation and web verification

`project_validate {path}` uses the running native engine to inspect saved project/scene
JSON, missing/nonportable asset paths, camera settings, hierarchy structure, mesh indices,
duplicate identifiers/names and Lua syntax. It neither executes scripts nor migrates files.
Without `path`, it checks the active project. Dynamic script paths and gameplay need tests.

Its `browser` block is a preflight for web builds, separate from the native verdict: a project
can pass natively and still not be browser-ready. It reports as errors the asset references
that `web_build` would not serve (it stages only `project.json` and `content/`, so paths
outside `content/` or outside the project 404), and as warnings files under `content/` that
differ only in letter case and Lua calls that need a file system (`read_file`, `write_file`,
`file_exists`, `list_files`, the dialogs, `io.open`, `os.getenv`...; use `save_data` /
`load_data` instead). It also warns about script string literals under `content/` that name a
missing file. It reads source text only: paths built at run time are not seen, and GPU or
adapter differences (an adapter without bindless textures, for one) only show in a browser,
so confirm with `web_playtest`.

`web_build {project, overwrite}` validates the project and invokes `build_web.sh --project
<dir> --no-serve`, returning `target/web/<project-directory-name>`. It stops before the slow
build when the browser preflight reports errors (`ignore_browser_issues: true` packages
anyway). Existing artifacts require `overwrite: true`. `web_playtest {directory, seconds, viewport_size, browser, save_path}`
serves that artifact on a temporary loopback port and tests it in a fresh headless Chromium
profile. It checks device/canvas initialization and reports console, exception and network
failures with a screenshot. A Shift press unlocks web audio. Node 22+ and Chromium/Brave are
required; there are no npm dependencies. `ENGINE_BROWSER` selects an executable. Temporary
server/browser processes and profiles are cleaned up. Neither tool publishes anything.

All tools expose output schemas and behavior annotations. Results include structured JSON
alongside compatible text/image content; failed assertions, transactions and validation
reports set MCP `isError`. Stable query reports have typed output fields.

Regression checks: build native editor/player and the MCP binary, then run
`python3 tools/engine-mcp/examples/verify_additions.py --web`. It creates its own editor,
port and scratch project, retaining screenshots and reports under `target/engine-mcp/`.

## Talking to the engine without MCP

```sh
ENGINE_REMOTE_PORT=47811 cargo run --features egui --bin editor &
printf '%s\n' '{"id":1,"method":"engine.methods"}' | nc -q1 127.0.0.1 47811
```
