#!/usr/bin/env bash
# Start the engine MCP server (registered in .mcp.json). Builds it first when the
# binary is missing or older than its sources. Only JSON-RPC may go to stdout.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
repo=$(cd "$here/../.." && pwd)
bin="$here/target/release/engine-mcp"

if [ ! -x "$bin" ] || [ -n "$(find "$here/src" "$here/Cargo.toml" "$here/web_playtest.mjs" -newer "$bin" -print -quit)" ]; then
    # cargo is not always on PATH; fall back to rustup toolchains (stable first).
    if ! command -v cargo >/dev/null; then
        for tc in "$HOME"/.rustup/toolchains/stable-* "$HOME"/.rustup/toolchains/nightly-*; do
            if [ -x "$tc/bin/cargo" ]; then
                export PATH="$tc/bin:$HOME/.cargo/bin:$PATH"
                break
            fi
        done
    fi
    cargo build --release --quiet --manifest-path "$here/Cargo.toml" >&2
fi

exec "$bin" --repo "$repo" "$@"
