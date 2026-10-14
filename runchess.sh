#!/usr/bin/env bash
set -euo pipefail
chess_repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
cd "$chess_repo_root"
cargo build --locked --release --manifest-path games/chess/uci/Cargo.toml --target-dir target/chess-uci
exec cargo run --locked --manifest-path games/chess/Cargo.toml --target-dir target
