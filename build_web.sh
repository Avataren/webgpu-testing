#!/usr/bin/env bash
# Usage: ./build_web.sh [example]           build an example (default: minimal), serve web/
#        ./build_web.sh --project <dir>     build the player, stage <dir> as a web game
#                                           in target/web/<name>/ and serve that
# Serves on $PORT (default 8080).
set -euo pipefail

PORT="${PORT:-8080}"

# The CLI must match the wasm-bindgen crate in Cargo.lock exactly.
WANT=$(grep -A1 '^name = "wasm-bindgen"$' Cargo.lock | sed -n 's/^version = "\(.*\)"$/\1/p')
HAVE=$(wasm-bindgen --version | awk '{print $2}')
if [ "$WANT" != "$HAVE" ]; then
    echo "wasm-bindgen CLI is $HAVE but Cargo.lock wants $WANT:" >&2
    echo "  cargo install wasm-bindgen-cli --version $WANT --locked" >&2
    exit 1
fi

export RUSTFLAGS="--cfg=web_sys_unstable_apis"

if [ "${1:-}" = "--project" ]; then
    PROJECT="${2:?usage: $0 --project <dir>}"
    [ -f "$PROJECT/project.json" ] || { echo "$PROJECT has no project.json" >&2; exit 1; }
    OUT="target/web/$(basename "$(cd "$PROJECT" && pwd)")"

    cargo build --target wasm32-unknown-unknown --release --bin player --features egui
    rm -rf "$OUT"
    mkdir -p "$OUT"
    wasm-bindgen --target web --out-dir "$OUT/pkg" \
        target/wasm32-unknown-unknown/release/player.wasm
    # The player loads project.json and content/ relative to the page.
    cp web/index.html "$PROJECT/project.json" "$OUT/"
    cp -r "$PROJECT/content" "$OUT/"

    echo "Staged $OUT; open http://localhost:$PORT/?example=player"
    exec python3 -m http.server --directory "$OUT" "$PORT"
fi

EXAMPLE="${1:-minimal}"
cargo build --target wasm32-unknown-unknown --release --example "$EXAMPLE"
wasm-bindgen --target web --out-dir web/pkg \
    "target/wasm32-unknown-unknown/release/examples/$EXAMPLE.wasm"

echo "Open http://localhost:$PORT/?example=$EXAMPLE"
exec python3 -m http.server --directory web "$PORT"
