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
- Every other tool is one request to the engine (see `src/tools.rs` for the mapping),
  except `engine_info` (which combines several), `screenshot` (returns an image),
  `lua_api_reference` (reads `scripts/LUA_SCRIPTING_README.md`; `outline`, `section` and
  `search` return just a part of it), `batch` (runs several tools in one call) and
  `playtest` (play, input, wait, report, screenshot, stop; `lean` for soak tests).
  `engine_info` also reports whether this server is older than its sources
  (`mcp_server.restart_needed`).
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
| Procedural assets | `mesh_lathe`, `mesh_extrude`, `mesh_compose`, `texture_generate`, `texture_maps`, `sound_generate` |
| Input | `input_send`, `input_state`, `input_clear` |
| View | `screenshot`, `camera_get`, `camera_set`, `environment_get`, `environment_set`, `postprocess_get`, `postprocess_set`, `logs` |
| Editor | `editor_state`, `project_create`, `project_open`, `project_save`, `scene_new`, `scene_activate`, `scene_close`, `scene_rename`, `scene_instance`, `asset_import`, `asset_reload`, `editor_select`, `editor_undo`, `editor_redo` |

Value conventions: meters, Y up, -Z forward; rotations as Euler degrees `[x, y, z]`
applied yaw, pitch, roll (like Lua's `set_rotation`); colors as `[r, g, b(, a)]` in 0..1
or `"#rrggbb"`; entities by id (the same 64-bit handle Lua uses) or unique name.

## Procedural geometry

`mesh_lathe` revolves `[radius, height]` profile points around Y. Points run bottom to
top; equal heights form ledges and radius zero makes a pole. `segments` controls radial
resolution (64 default), `caps` closes nonzero end radii, and `crease_angle` (45 degrees
default) preserves sharp profile corners while smoothing the rest. For example:

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

## Talking to the engine without MCP

```sh
ENGINE_REMOTE_PORT=47811 cargo run --features egui --bin editor &
printf '%s\n' '{"id":1,"method":"engine.methods"}' | nc -q1 127.0.0.1 47811
```
