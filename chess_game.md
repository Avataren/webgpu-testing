# Chess game integration findings

Investigation date: 2026-10-14.

Implementation update, 2026-10-14: the native
[chess spectator app](games/chess/README.md) is now implemented. Run `./runchess.sh`
from the engine repository. XavChess plays both sides with the isolated HCE UCI
build and a one-second search budget. The saved board has improved ivory/light
square contrast, HDR lighting, postprocessing, move and capture animation, sounds,
and automatic rematches. The investigation below records the original handoff.

## Goal and agreed direction

Build a playable chess game in this 3D engine using the procedural chess board
and pieces already created here, and reuse the chess implementation in
`../../rust_chess` (`/home/avataren/src/rust_chess`).

User preferences established in the investigation:

- Use UCI to communicate with the AI engine.
- Use **handcrafted evaluation (HCE / `classical-eval`)**. The user currently
  prefers it to their NNUE implementation. No strength comparison was performed.
- Record the findings now; implement the game in a new session.

This was a source investigation and inspection of the saved scene JSON. Neither
codebase was modified during the investigation. No compilation, UCI runtime
session, or gameplay integration test was performed. This document is the handoff.

## Recommended architecture

| Responsibility | Implementation |
| --- | --- |
| Rules, authoritative board, legal moves, history | Reuse `chess_board`, `chess_foundation`, and `move_generator` in the game |
| AI search and evaluation | Run the existing `chess_uci` executable as a child process, explicitly built for HCE |
| Board interaction, animation, game UI | New game code using this engine and the saved procedural scene |

The rules crates have no Bevy dependency. The existing graphical chess app in
`../../rust_chess/chess` uses Bevy, so its rendering, UI, and event systems should
be replaced with this engine's equivalents. Its controller code remains useful
as a reference for behavior.

UCI is a good boundary for AI search, but the current executable does not expose
a legal-move list or game-status query. The game should use the existing rules
crates for these tasks. A completely external rules backend would need an
additional interface; that is unnecessary for the recommended first version.

The game owns the authoritative board and validates both human moves and UCI
replies against its legal moves. The renderer displays that board; animated
transforms must not become the source of chess state.

## Existing procedural assets

Project: [examples/projects/procedural-chess](examples/projects/procedural-chess/README.md).

- [Project manifest](examples/projects/procedural-chess/project.json).
- [Board scene](examples/projects/procedural-chess/content/scenes/Procedural_Chess.scene.json).
- [Piece study scene](examples/projects/procedural-chess/content/scenes/Chess_Piece_Study.scene.json).
- [Generator and verification script](tools/engine-mcp/examples/procedural_chess.py).

The saved board scene was inspected and contains:

- 32 separate piece entities: 16 pawns, four rooks, four knights, four bishops,
  two queens, and two kings.
- Six shared piece mesh handles, one per piece type.
- Ivory and ebony materials, maple/walnut board squares, a frame and brass rim,
  lighting, and a game camera.
- Only a scene-label script; no chess gameplay yet.

Meshes and portable texture references are saved in the project. The assets do
not need regenerating to add gameplay. Pieces are already composed as complete
meshes and can be moved as individual entities.

Entity names have the form `Chess.White.king.5`, `Chess.Black.rook.1`, and
`Chess.White.pawn.copy-1`. Use stable entity handles and an explicit square-to-entity
map during play. Name suffixes are not square indices or a uniform numbering scheme.

### Board coordinates

Chess square indices in the existing rules use `a1 = 0`, `h1 = 7`, `a8 = 56`.
For zero-based `file = square % 8` and `rank = square / 8`, the saved scene uses:

```text
x = -4.2 + 1.2 * file
y = 0.226                  # piece base height
z =  4.2 - 1.2 * rank
```

Square spacing is 1.2 world units. White starts on the positive-Z side. Square
centers are at `y = 0.18`; their top surface is approximately `y = 0.225`.
White's h1 square is light. The game camera starts at `[10.8, 11.7, 13.8]`.

For initial entity mapping, derive the square from its starting transform and
cross-check piece type and color. Keep subsequent square changes in game state.
Reserve access to each piece-type mesh and both materials so promotion and new
games work even if the original pieces of that type have been captured.

## Reusable chess APIs

Relevant files in `../../rust_chess`:

| File | Useful functionality |
| --- | --- |
| [chess_board/src/chessboard.rs](../../rust_chess/chess_board/src/chessboard.rs) | `ChessBoard::new`, `set_from_fen`, `make_move`, `undo_move`, `get_piece_at_square`, `is_white_active`, `is_repetition`, halfmove/fullmove clocks |
| [chess_board/src/fen.rs](../../rust_chess/chess_board/src/fen.rs) | FEN loading and serialization |
| [move_generator/src/move_generator.rs](../../rust_chess/move_generator/src/move_generator.rs) | `get_legal_move_list_from_square`, `get_all_legal_moves_for_color` |
| [move_generator/src/piece_conductor.rs](../../rust_chess/move_generator/src/piece_conductor.rs) | `PieceConductor::new`, `is_king_in_check` |
| [chess_foundation/src/chess_move.rs](../../rust_chess/chess_foundation/src/chess_move.rs) | Move coordinates, capture information, castling/en-passant/promotion flags |
| [chess/src/piece_picker.rs](../../rust_chess/chess/src/piece_picker.rs) | Existing selection, validation, and human-turn behavior |
| [chess/src/chess_event_handler.rs](../../rust_chess/chess/src/chess_event_handler.rs) | Existing AI-turn, result, and animation sequencing |
| [chess/src/game_resources.rs](../../rust_chess/chess/src/game_resources.rs) | Existing clock, strength, player-color, and game-phase behavior |

Important integration details:

- Match coordinate input, including UCI replies, against generated legal moves
  and apply the matching `ChessMove`. Constructing a plain from/to move loses
  special-move flags. `make_move` alone is not a full legal-move validator.
- Castling moves two rendered entities; en passant removes a pawn from a square
  other than the destination; promotion changes the displayed piece type and
  must support queen, rook, bishop, and knight.
- Preserve move history for repetition detection and takeback.
- Determine checkmate/stalemate from legal moves and check status. The board's
  `GameState` enum/setters are not a complete automatic result service.
- The current app explicitly handles repetition and flagging. Full draw-rule
  coverage, including the 50-move rule and insufficient material, needs a deliberate
  implementation/review; the board exposes a halfmove clock but that alone does
  not implement adjudication.
- Existing strength levels also use evaluation noise and pondering. UCI exposes
  depth, timing, pondering, and Dirichlet options, but reproducing the graphical
  app's five strength presets exactly is separate work.

## Existing UCI implementation

Source: [chess_uci/src/main.rs](../../rust_chess/chess_uci/src/main.rs).
The engine identifies itself as `XavChess`.

Implemented commands include:

- `uci`, `isready`, `setoption`, `ucinewgame`.
- `position startpos moves ...` and `position fen ... moves ...`.
- `go depth ...`, `go movetime ...`, and clock-based searches using
  `wtime`, `btime`, `winc`, `binc`, and `movestogo`.
- `go ponder`, `ponderhit`, `stop`, and `quit`.
- Search progress as `info ...`; results as `bestmove ...` with an optional
  `ponder ...` move. The engine can return `bestmove 0000` when no move exists.

Search already runs on a worker thread in the executable. The 3D game still
needs nonblocking communication: read stdout on a worker and pass parsed events
to the render/game thread through a channel. Drain stderr separately so logging
cannot fill a pipe and stall the process.

### Suggested initial exchange

Send `uci` and wait for `uciok`, then configure options and send `isready`.
Wait for `readyok` before starting a game. Example option choices below are
starting points, not benchmarked settings:

```text
setoption name Threads value 2
setoption name Hash value 64
setoption name Ponder value false
isready
```

Use a modest search thread count initially so rendering has CPU time. Explicit
options avoid relying on advertised/default values. Start without pondering.

For a new game, send `ucinewgame`, synchronize readiness, and set the position.
For example, after a human opens with e2-e4:

```text
position startpos moves e2e4
go movetime 500
```

Receive, validate, and apply `bestmove`, then animate it. Supply the complete
accepted move sequence on the next `position` command, including both players'
moves. The UCI executable does not automatically advance its stored board when
it emits `bestmove`. For a custom starting position, retain its initial FEN and
append moves rather than repeatedly replacing history with the current FEN.

The current position parser silently skips unmatched move strings, so validate
before sending and do not treat command acceptance as proof of synchronization.

On restart, takeback, or side changes, stop and drain the outstanding search
before starting another. Associate local search events with a game/search
generation so an old result cannot move a piece in a new game. UCI replies have
no built-in request IDs. Handle engine exit, timeouts, errors, and clean shutdown.

## HCE selection: required configuration fix

**Do not assume `setoption name NeuralEval value false` selects HCE with the
current default build.**

Findings:

1. [The chess workspace manifest](../../rust_chess/Cargo.toml) sets
   `chess_evaluation` to `default-features = false, features = ["nn-incremental"]`.
2. [The UCI manifest](../../rust_chess/chess_uci/Cargo.toml) inherits that dependency.
   Its own `classical-eval`, `nn-incremental`, etc. features are empty marker
   features; they do not forward the choice to the evaluation dependency.
3. [board_evaluation.rs](../../rust_chess/chess_evaluation/src/board_evaluation.rs)
   prioritizes NNUE features over HCE. The direct NNUE paths do not consult the
   runtime `NeuralEval` enable flag.
4. UCI's own default `nn-incremental` feature also controls embedding the roughly
   43 MB `eval.npz` and loading it on `isready`.

A localized approach for the future implementation is to replace UCI's inherited
evaluation dependency with an explicit HCE dependency:

```toml
# ../../rust_chess/chess_uci/Cargo.toml, proposed change only
chess_evaluation = { path = "../chess_evaluation", default-features = false, features = ["classical-eval"] }
```

Then build only the UCI package with its NNUE defaults disabled:

```sh
cargo build --manifest-path ../../rust_chess/Cargo.toml \
  -p chess_uci --release --no-default-features --features classical-eval
```

**That command alone is insufficient before the dependency configuration is
corrected.** Alternatively, change the workspace evaluation selector to HCE,
but that also changes other packages that inherit it. Proper feature forwarding
would be a longer-term cleanup.

Avoid building the Bevy app in the same Cargo invocation when verifying HCE:
it explicitly enables NNUE, and Cargo feature unification can enable both backends
for a shared dependency. Verify the resolved features, HCE evaluation, and absence
of automatic NNUE weight loading in the actual executable used by the game.
These proposed changes have not been applied or built yet.

## Integration points in this 3D engine

- [src/render_application.rs](src/render_application.rs): `RenderApplication`
  provides setup, frame updates, and optional egui UI hooks.
- [src/project.rs](src/project.rs): `ProjectManifest::load_from_dir` and
  `instantiate_workspace` load the saved project and its resources.
- [src/bin/player/main.rs](src/bin/player/main.rs): reference for loading a
  project, swapping its workspace, and displaying scripted UI.
- [src/app/core/scheduler.rs](src/app/core/scheduler.rs): application contexts
  expose the scene, renderer where appropriate, and frame delta time.
- [src/scripting/lua/api/view.rs](src/scripting/lua/api/view.rs):
  `screen_to_ground`, `screen_to_world_ray`, and `world_to_screen` support board
  interaction and labels. A board-plane intersection is enough for square-based
  selection; direct clicks on tall piece silhouettes may need mesh/bounds picking.
- [src/scene/picking.rs](src/scene/picking.rs): existing entity lookup for GPU
  pick IDs. Much of the current picking orchestration lives in the editor.
- [src/scripting/lua/api/input.rs](src/scripting/lua/api/input.rs),
  [transform.rs](src/scripting/lua/api/transform.rs), and
  [material.rs](src/scripting/lua/api/material.rs): existing scripted input,
  movement, and highlighting facilities.

The old `examples/chess.rs` is a glTF rendering example (currently configured to
load Sponza), not a gameplay implementation or the procedural chess project.

### Rust application versus editor/play project

The quickest proposed first version is a small Rust game application using
`RenderApplication`. Give the game its own package/dependencies, reuse the engine
library, load the saved scene, and keep chess-specific logic outside the engine
core. This can call the rules crates and spawn a UCI process directly.

If the goal is instead a normal Lua-driven project that runs unchanged through
the generic player and editor Play button, add a rules/UCI backend bridge. The
current registered Lua APIs do not provide a UCI or asynchronous child-process
interface. The runtime does expose its Lua VM for registering APIs; registration,
ownership, resets, hot reload, and editor/player lifecycle would need integration.
Do not assume adding Lua scripts to the scene alone can access the chess crates.

The user has not selected between these application packaging choices. The Rust
application is a recommendation, not an agreed requirement.

Native desktop is the straightforward first target for a UCI child process.
Browser builds cannot spawn that native executable; browser support would need
an engine compiled to WASM in a worker or a server transport. The original chess
app has WASM search code worth inspecting if that becomes part of the scope.

## Suggested implementation sequence for the next session

1. Confirm the intended packaging against the user's next request. Start from
   this document and the existing saved scene; do not regenerate the artwork.
2. Configure and build the HCE UCI executable. Verify resolved features and a
   real handshake, search, stop, and shutdown before connecting rendering.
3. Implement a small game controller around the existing rules crates. Keep
   authoritative board state, full history, selection, legal moves, and phases
   such as human turn, AI thinking, animation, and game over.
4. Load the procedural scene, map all 32 pieces to squares, and add click-to-select
   and click-to-move with destination highlights. Initially allow local moves
   for both sides to verify the board/controller mapping.
5. Add the UCI adapter, validate its replies, and complete a human-versus-HCE game.
6. Animate moves and handle captures, both castling directions, en passant, and
   all promotion choices. Reconcile the rendered scene with the logical board
   after animation and restart.
7. Add new game, player color, promotion UI, result display, thinking indicator,
   and clocks/time controls. Adapt optional strength presets, opening labels,
   sounds, and takeback afterward as requested.

Verification should cover legal-move rejection; both player colors; normal
captures; castling; en passant; underpromotion; mate/stalemate/repetition; and
restart during an active search. Confirm input still maps correctly after camera
movement or window resizing, and that AI thinking never blocks frame updates.

No implementation has started. The assets and existing chess rules provide most
of the foundation; the remaining work is the controller, UCI transport, scene
synchronization, interaction, and UI.
