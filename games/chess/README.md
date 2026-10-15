# Chess spectator game

A native chess exhibition in a path-traced study. XavChess plays both sides with classical evaluation and a one-second UCI search budget per
move. Finished games pause briefly, then restart automatically.

Both colors share the same full-strength settings: sixteen search threads (capped
by the engine to available CPUs), a 128 MB transposition table, and disabled root
move noise. The engine returns its best searched move without a difficulty handicap.

Run from the repository root:

```sh
./runchess.sh
```

The launcher builds the HCE engine and the spectator app. Rust 1.95 or newer and a
checkout of [rust_chess](https://github.com/Avataren/rust_chess) at `../../rust_chess`
relative to this engine repository are required. The implementation was validated
against revision `ad4d427164288145550d33c323a08d6077059dc1`.

The rules crates and UCI source are reused through path dependencies. The local UCI manifest
selects only `classical-eval`; it does not change the sibling workspace's NNUE setup.
`CHESS_UCI_PATH` can override the executable used by the app.

## The study

The game plays in [`study/`](study), a real-scale room modelled in Blender:

- Staunton pieces for 55 mm squares (king 95 mm) in boxwood and ebony. The turned
  pieces are exact lathe profiles; the knight's head is a signed distance field with
  carved mane locks, eyes and nostrils, meshed through OpenVDB.
- A walnut and maple board with maple stringing and inlaid coordinates: a-h along
  both rank edges and 1-8 along both file edges, each turned to the nearer player.
- A mahogany games table with a tooled leather top, an analog chess clock (domed
  crystals, brass plungers, red flags) facing the board, and two bentwood chairs.
- The room: oak floor, Persian rug, green panelled wainscot, two curtained sash
  windows, a marble fireplace with a painting, bookcases, a floor lamp, and a crystal
  decanter on a side table in the sun.

It is lit for path tracing: a low sun through the window bars falls across the
board and the decanter, sky light comes in through the windows, and the fire and
the lamp warm the back of the room. The spectator turns path tracing on at start
where the GPU supports it; P switches to the raster lighting and back.

The sky is a copy of `web/assets/hdr/kloppenheim_06_puresky_4k.hdr` in
`content/environment`, so the project is self-contained.

The app reads the board from the scene rather than assuming its size: the square
size comes from the white rooks and the centre from the 32 starting pieces
(`Chess.<White|Black>.<kind>.<square>`, white at +Z in their parent's space).
Captured pieces stand in lines of four from the `Chess.Captures.White` and
`Chess.Captures.Black` anchors, and `Move.Inlay.0`-`7` mark the last move. Camera
drift swings the scene's game camera around the board and back.

### Rebuilding the assets

Sources live in [`tools/study/`](tools/study): `textures.py` generates the wood,
leather, rug, book, marble, painting and dial maps with numpy; the Blender scripts
build and export the GLBs (`pieces.py`, `chess_set.py`, `furniture.py`, `room.py`).

```sh
python3 games/chess/tools/study/textures.py
blender -b --python games/chess/tools/study/build_assets.py
```

Then import the GLBs from `games/chess/tools/study/build/glb` into the study with the
engine MCP's `asset_import` and place them as in the saved scene.

| Control | Action |
| --- | --- |
| Space | Pause or resume |
| R | Start a new game |
| C | Toggle camera drift |
| M | Toggle sound |
| P | Toggle path tracing (spectator) |
| Escape | Close the app |

The side panel shows recent moves, the evaluation from Ivory's perspective, and
session results in win/draw/loss order. Pace changes the animation and viewing
delay; AI thinking remains one second per move.

Chess logic lives in `src/rules.rs`, asynchronous UCI transport in `src/uci.rs`, and
scene animation and the spectator interface in `src/main.rs`. Rules state is
authoritative. Engine replies must match generated legal moves. Castling, en
passant, all promotion types, checkmate, stalemate, repetition, the fifty-move rule,
and common insufficient-material positions are handled. A 400-ply exhibition limit
ensures a session always progresses to another game. Search generations prevent
old replies from moving pieces after a restart.

`CHESS_PATH_TRACING=0` starts with raster lighting, and `CHESS_WINDOW=960x540` sets the
window size. Settings are saved per user under the `chess-spectator` name.

## Playing from the editor

Press Play on `games/chess/study` in the editor (or run it in the player) and the same
exhibition runs from Lua: `content/scripts/chess_game.lua` on the `Chess Director`
entity starts the XavChess HCE build over UCI with the engine's process API
(`process_spawn`, `process_write`, `process_read_lines`) and the same strength settings.
Build the engine first with `./runchess.sh` (or its first `cargo build` line). The
engine still chooses every move; `chess_rules.lua` only keeps the position for
castling, en passant, promotion, check, mate and draws. A camera director glides
between slow shots around the board. Keys: Space pause, R new game, C camera, M
sound. The native spectator strips the scene's scripts and plays the game itself.

UCI needs a desktop build: the process API does not exist on the web.

## Sounds

`tools/study/sounds.py` synthesises the sounds from simple physical models: a felted
piece set down on wood (wood modes, contact click, the table's body), a capture (clack,
landing, the taken piece set aside), a soft low tone for check, a two-note mantel-clock
chime at the end, and the clock plunger pressed after each move. Several takes of each
keep repeated moves from sounding identical.

To validate the game after building the HCE target:

```sh
cargo test --locked --manifest-path games/chess/Cargo.toml --target-dir target
cargo clippy --locked --manifest-path games/chess/Cargo.toml --target-dir target --all-targets --no-deps -- -D warnings
```

The original procedural board lives on in `examples/projects/procedural-chess`. To
recreate its visual polish through a running editor's MCP:

```sh
python3 games/chess/tools/polish_scene.py --port 47821
```

The app exposes `chess.status`, `chess.restart`, and `chess.configure` over the
engine's remote interface when `ENGINE_REMOTE_PORT` is set. Configure accepts
`paused`, `orbit`, `sound`, `speed`, and `search_millis` for accelerated verification.
