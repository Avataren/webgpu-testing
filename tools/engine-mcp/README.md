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

For other clients, run `tools/engine-mcp/run.sh` (or the built
`tools/engine-mcp/target/release/engine-mcp --repo <repo>`) as a stdio MCP server.

Options and environment:

| | |
|---|---|
| `--repo DIR` / `ENGINE_REPO` | Engine repository (default: the one the binary was built in) |
| `--port PORT` / `ENGINE_REMOTE_PORT` | Port of the engine's remote control (default 47811) |
| `ENGINE_CARGO` | cargo to build the engine with (default: first toolchain satisfying `rust-version`, stable first) |
| `ENGINE_RUST_LOG` | `RUST_LOG` for launched engines (default `info`) |

## How it works

- `engine_launch` builds the target with `cargo build --all-features`, starts it with
  `ENGINE_REMOTE_PORT` set and waits for the socket. If an engine already listens on
  the port (for example an editor you started yourself), it attaches instead. The
  engine's stdout/stderr go to `target/engine-mcp/engine.log` and `engine_output`.
- Every other tool is one request to the engine (see `src/tools.rs` for the mapping),
  except `engine_info` (which combines several), `screenshot` (returns an image) and
  `lua_api_reference` (reads `scripts/LUA_SCRIPTING_README.md`).
- Closing the MCP server leaves a launched engine running, so unsaved work in the
  editor is never lost; use `engine_stop` to end it.

## Tools

| Area | Tools |
|---|---|
| Process | `engine_launch`, `engine_stop`, `engine_status`, `engine_output`, `engine_build` |
| Overview | `engine_info`, `engine_stats`, `engine_call` (any method) |
| Scene | `scene_list_entities`, `scene_tree`, `entity_get`, `assets_list` |
| Editing | `entity_spawn`, `entity_update`, `entity_remove_component`, `entity_delete`, `entity_duplicate` |
| Scripting | `lua_eval`, `script_status`, `script_reload`, `lua_api_reference` |
| Runtime | `runtime_set` (play/stop), `wait` |
| Input | `input_send`, `input_state`, `input_clear` |
| View | `screenshot`, `camera_get`, `camera_set`, `environment_get`, `environment_set`, `postprocess_get`, `postprocess_set`, `logs` |
| Editor | `editor_state`, `project_create`, `project_open`, `project_save`, `scene_new`, `scene_activate`, `scene_close`, `scene_rename`, `scene_instance`, `asset_import`, `editor_select`, `editor_undo`, `editor_redo` |

Value conventions: meters, Y up, -Z forward; rotations as Euler degrees `[x, y, z]`
applied yaw, pitch, roll (like Lua's `set_rotation`); colors as `[r, g, b(, a)]` in 0..1
or `"#rrggbb"`; entities by id (the same 64-bit handle Lua uses) or unique name.

## Talking to the engine without MCP

```sh
ENGINE_REMOTE_PORT=47811 cargo run --features egui --bin editor &
printf '%s\n' '{"id":1,"method":"engine.methods"}' | nc -q1 127.0.0.1 47811
```
