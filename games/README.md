# Game demos

Games and interactive scenes built with the engine. Run these commands from the
repository root, or open a saved project directory in the editor and press Play.

| Demo | Run |
| --- | --- |
| [Tide Observatory](tide-observatory/README.md) — kinetic brass sculpture, reflecting basin, and orbital camera | `./run-tide-observatory.sh` |
| [Hollow Crown](hollow-crown/README.md) | `./run-hollow-crown.sh` |
| [Chess](chess/README.md) — AI spectator | `./run-chess.sh` |
| [Robotron](robotron/README.md) | `cargo run --all-features --bin player -- games/robotron` |
| [Breakout](breakout/project.json) | `cargo run --all-features --bin player -- games/breakout` |
| [Neon Rift](neon-rift/project.json) | `cargo run --all-features --bin player -- games/neon-rift` |
| [RT Lab](rt-lab/README.md) — test room for ray-traced reflections | `cargo run --release --all-features --bin player -- games/rt-lab` |

Package a saved project for the web using
`./build_web.sh --project games/<demo-name>`. Chess uses its own native spectator
launcher; see its README for details.
