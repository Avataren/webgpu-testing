#!/usr/bin/env bash
# GPU smoke test: build the editor, player and examples, run each one briefly on the
# real GPU, and report crashes or error/validation log lines.
#
# Usage: smoke.sh [--shots] [--project DIR] [--out DIR] [--seconds N] [target...]
#   target    editor | player | <example name>   (default: a representative set)
#   --shots   also save a screenshot of the engine window (never the whole screen)
#   --project project directory for `player` (it exits immediately without one)
#   --out     where logs and screenshots go (default: target/smoke)
#   --seconds how long each target runs (default: 8; the editor gets 4 more)
set -uo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
cd "$repo" || exit 1

shots=0
project=""
out="target/smoke"
seconds=8
targets=()
while (($#)); do
    case "$1" in
        --shots) shots=1 ;;
        --project) project=$2; shift ;;
        --out) out=$2; shift ;;
        --seconds) seconds=$2; shift ;;
        -h|--help) sed -n '2,11p' "$0"; exit 0 ;;
        *) targets+=("$1") ;;
    esac
    shift
done
((${#targets[@]})) || targets=(editor pbr particle_effects game_of_life egui_demo bloom boids_optimized shadow morph animation)

# cargo is not always on PATH; pick a rustup toolchain that satisfies `rust-version`.
required=$(sed -n 's/^rust-version = "\(.*\)"/\1/p' Cargo.toml)
version_ok() { [ -z "$required" ] || [ "$(printf '%s\n%s\n' "$required" "$1" | sort -V | head -1)" = "$required" ]; }
toolchain=""
for tc in stable nightly; do
    bin="$HOME/.rustup/toolchains/$tc-x86_64-unknown-linux-gnu/bin"
    [ -x "$bin/rustc" ] || continue
    if version_ok "$("$bin/rustc" --version | awk '{print $2}' | cut -d- -f1)"; then
        toolchain=$tc
        export PATH="$bin:$HOME/.cargo/bin:$PATH"
        break
    fi
done
if [ -z "$toolchain" ]; then
    if command -v cargo >/dev/null; then
        toolchain=path
    else
        echo "No Rust toolchain >= ${required:-any} found" >&2
        exit 1
    fi
fi
# Keep nightly artifacts apart so they don't invalidate the stable build cache.
[ "$toolchain" = nightly ] && export CARGO_TARGET_DIR="$repo/target/nightly"
bin_dir="${CARGO_TARGET_DIR:-$repo/target}/debug"
echo "toolchain: $toolchain ($(rustc --version))"

if ! cargo build --all-features --bins --examples --quiet; then
    echo "BUILD FAILED" >&2
    exit 1
fi

export DISPLAY="${DISPLAY:-:0}"
mkdir -p "$out"
title="wgpu hecs Renderer"
failures=0

screenshot() {
    local name=$1 wid
    # Capture only the engine window, found by its title; never the root window.
    wid=$(xdotool search --onlyvisible --name "^$title\$" 2>/dev/null | tail -1)
    if [ -z "$wid" ]; then
        echo "  (no engine window found for screenshot)"
        return
    fi
    import -window "$wid" "$out/$name.png" && echo "  screenshot: $out/$name.png"
}

for target in "${targets[@]}"; do
    case "$target" in
        editor) exe="$bin_dir/editor"; args=(); run_for=$((seconds + 4)) ;;
        player) exe="$bin_dir/player"; args=(${project:+"$project"}); run_for=$seconds ;;
        *) exe="$bin_dir/examples/$target"; args=(); run_for=$seconds ;;
    esac
    if [ ! -x "$exe" ]; then
        echo "$target: SKIP (no binary at $exe)"
        continue
    fi

    log="$out/$target.log"
    RUST_LOG="${RUST_LOG:-warn}" "$exe" "${args[@]}" >"$log" 2>&1 &
    pid=$!
    if ((shots)); then
        sleep $((run_for - 2))
        kill -0 "$pid" 2>/dev/null && screenshot "$target"
        sleep 2
    else
        sleep "$run_for"
    fi
    if kill -0 "$pid" 2>/dev/null; then
        kill "$pid"
        wait "$pid" 2>/dev/null
        status=alive
    else
        wait "$pid"
        status="exited($?)"
    fi

    # Vulkan loader / missing validation layer / libdecor messages are environment noise.
    issues=$(grep -viE 'loader|validation layer|VK_LAYER|libdecor' "$log" | grep -iE 'error|panic|invalid' || true)
    if [ "$status" = alive ] && [ -z "$issues" ]; then
        echo "$target: OK"
    else
        failures=$((failures + 1))
        echo "$target: FAIL ($status) — $log"
        if [ -n "$issues" ]; then
            printf '%s\n' "$issues" | head -5 | cut -c1-240 | sed 's/^/    /'
        else
            tail -3 "$log" | cut -c1-240 | sed 's/^/    /'
        fi
    fi
done

exit $((failures > 0))
