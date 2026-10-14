#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")"
cargo build --all-features --bin player
exec ./target/debug/player games/hollow-crown "$@"
