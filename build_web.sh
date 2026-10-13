#!/usr/bin/env bash
# Usage: ./build_web.sh [example]   (default: minimal)
# Serves web/ on $PORT (default 8080); open http://localhost:$PORT/?example=<example>.
set -euo pipefail

EXAMPLE="${1:-minimal}"
PORT="${PORT:-8080}"

# The CLI must match the wasm-bindgen crate in Cargo.lock exactly.
WANT=$(grep -A1 '^name = "wasm-bindgen"$' Cargo.lock | sed -n 's/^version = "\(.*\)"$/\1/p')
HAVE=$(wasm-bindgen --version | awk '{print $2}')
if [ "$WANT" != "$HAVE" ]; then
    echo "wasm-bindgen CLI is $HAVE but Cargo.lock wants $WANT:" >&2
    echo "  cargo install wasm-bindgen-cli --version $WANT --locked" >&2
    exit 1
fi

RUSTFLAGS="--cfg=web_sys_unstable_apis" \
    cargo build --target wasm32-unknown-unknown --release --example "$EXAMPLE"

wasm-bindgen \
    --target web \
    --out-dir web/pkg \
    "target/wasm32-unknown-unknown/release/examples/$EXAMPLE.wasm"

echo "Open http://localhost:$PORT/?example=$EXAMPLE"
python3 -m http.server --directory web "$PORT"
