#!/usr/bin/env bash
# Usage: ./build_web.sh [example]           build an example (default: minimal), serve web/
#        ./build_web.sh --project <dir>     build the player, stage <dir> as a web game
#                                           in target/web/<name>/ and serve that (only
#                                           the content the game uses is staged)
#        ./build_web.sh --audio-worklet <dir>
#                                           only build the audio mixer's AudioWorklet
#                                           into <dir> (the editor's Build menu uses it)
# Serves on $PORT (default 8080).
set -euo pipefail

PORT="${PORT:-8080}"
NO_SERVE=0
for ARG in "$@"; do
    if [ "$ARG" = "--no-serve" ]; then NO_SERVE=1; fi
done
BUILD_DIR="${CARGO_TARGET_DIR:-target}"

# The CLI must match the wasm-bindgen crate in Cargo.lock exactly.
WANT=$(grep -A1 '^name = "wasm-bindgen"$' Cargo.lock | sed -n 's/^version = "\(.*\)"$/\1/p')
HAVE=$(wasm-bindgen --version | awk '{print $2}')
if [ "$WANT" != "$HAVE" ]; then
    echo "wasm-bindgen CLI is $HAVE but Cargo.lock wants $WANT:" >&2
    echo "  cargo install wasm-bindgen-cli --version $WANT --locked" >&2
    exit 1
fi

export RUSTFLAGS="--cfg=web_sys_unstable_apis"

# Shrink a wasm-bindgen output in place. Skipped, with a warning, without wasm-opt.
optimize_wasm() {
    if command -v wasm-opt >/dev/null; then
        wasm-opt -Os --all-features --strip-debug --strip-producers "$1" -o "$1"
    else
        echo "wasm-opt not found; $1 is unoptimized (install binaryen)" >&2
    fi
}

# The audio mixer's AudioWorklet: cubed-synth's cubed-worklet wasm and its
# processor script, which the engine loads from pkg/ next to the player. It is
# built from the cubed-synth source this build resolves (the commit Cargo.lock
# pins, or a local checkout .cargo/config.toml patches in), so the worklet and
# the engine's command encoding always match.
stage_audio_worklet() {
    local out="$1"
    local mixer
    mixer=$(env -u RUSTFLAGS cargo metadata --format-version 1 | python3 -c '
import json, sys
print(next(p["manifest_path"] for p in json.load(sys.stdin)["packages"] if p["name"] == "cubed-mixer"))')
    local worklet
    worklet="$(dirname "$mixer")/../worklet"
    env -u RUSTFLAGS cargo build --manifest-path "$worklet/Cargo.toml" \
        --target wasm32-unknown-unknown --release --target-dir "$BUILD_DIR/cubed-worklet"
    mkdir -p "$out"
    cp "$BUILD_DIR/cubed-worklet/wasm32-unknown-unknown/release/cubed_worklet.wasm" "$out/"
    cp "$worklet/js/cubed-worklet.js" "$out/"
}

if [ "${1:-}" = "--audio-worklet" ]; then
    stage_audio_worklet "${2:?usage: $0 --audio-worklet <dir>}"
    exit 0
fi

if [ "${1:-}" = "--project" ]; then
    PROJECT="${2:?usage: $0 --project <dir>}"
    [ -f "$PROJECT/project.json" ] || { echo "$PROJECT has no project.json" >&2; exit 1; }
    OUT="target/web/$(basename "$(cd "$PROJECT" && pwd)")"

    cargo build --target wasm32-unknown-unknown --profile web --bin player --features egui
    rm -rf "$OUT"
    mkdir -p "$OUT"
    wasm-bindgen --target web --out-dir "$OUT/pkg" \
        "$BUILD_DIR/wasm32-unknown-unknown/web/player.wasm"
    optimize_wasm "$OUT/pkg/player_bg.wasm"
    stage_audio_worklet "$OUT/pkg"
    # The player loads project.json and content/ relative to the page. Only the content
    # the game uses goes along (and a project.json listing only its scenes): the native
    # player works that out (built like the editor, so its build is reused).
    cp web/index.html "$OUT/"
    env -u RUSTFLAGS cargo run --quiet --release --all-features --bin player -- \
        --export-content "$PROJECT" "$OUT"

    echo "Staged $OUT; open http://localhost:$PORT/?example=player"
    if [ "$NO_SERVE" = 1 ]; then exit 0; fi
    exec python3 -m http.server --directory "$OUT" "$PORT"
fi

EXAMPLE="${1:-minimal}"
cargo build --target wasm32-unknown-unknown --profile web --example "$EXAMPLE"
wasm-bindgen --target web --out-dir web/pkg \
    "$BUILD_DIR/wasm32-unknown-unknown/web/examples/$EXAMPLE.wasm"
optimize_wasm "web/pkg/${EXAMPLE}_bg.wasm"
stage_audio_worklet web/pkg

echo "Open http://localhost:$PORT/?example=$EXAMPLE"
if [ "$NO_SERVE" = 1 ]; then exit 0; fi
exec python3 -m http.server --directory web "$PORT"
