# Chess spectator game

A native chess exhibition using the engine's procedural board and pieces. XavChess
plays both sides with classical evaluation and a one-second UCI search budget per
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

The saved scene has warm ivory pieces, cooler smoked maple squares, ebony pieces,
a slate display plinth, and a blurred attic HDRI. AgX tonemapping, TAA, contact
shadows, reflections, bloom, and a subtle vignette shape the final image. The engine
MCP generated the material maps and move, capture, check, and end sounds.

The HDRI reuses `web/assets/hdr/pine_attic_4k.hdr` from the engine repository. The
launchers set the repository working directory, so the scene contains no
machine-specific HDR path and stores no duplicate HDR file.

| Control | Action |
| --- | --- |
| Space | Pause or resume |
| R | Start a new game |
| C | Toggle camera drift |
| M | Toggle sound |
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

Opening the project in the generic editor or player displays the scene. Self-play
runs in the dedicated spectator executable. Native UCI processes require desktop
execution.

To validate the game after building the HCE target:

```sh
cargo test --locked --manifest-path games/chess/Cargo.toml --target-dir target
cargo clippy --locked --manifest-path games/chess/Cargo.toml --target-dir target --all-targets --no-deps -- -D warnings
```

To recreate the visual polish through a running editor's MCP:

```sh
python3 games/chess/tools/polish_scene.py --port 47821
```

The app exposes `chess.status`, `chess.restart`, and `chess.configure` over the
engine's remote interface when `ENGINE_REMOTE_PORT` is set. Configure accepts
`paused`, `orbit`, `sound`, `speed`, and `search_millis` for accelerated verification.
