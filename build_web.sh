#!/usr/bin/env bash
set -euo pipefail

RUSTFLAGS="--cfg=web_sys_unstable_apis" \
    cargo build --target wasm32-unknown-unknown --release --bin wgpu-cube

wasm-bindgen \
    --target web \
    --out-dir web/pkg \
    target/wasm32-unknown-unknown/release/wgpu-cube.wasm

python3 -m http.server --directory web 8080
