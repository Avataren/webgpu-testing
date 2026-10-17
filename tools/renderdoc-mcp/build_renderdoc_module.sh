#!/usr/bin/env bash
# Build RenderDoc's Python replay module (renderdoc.so) for the installed RenderDoc.
# Distribution packages (Arch's included) ship only qrenderdoc/renderdoccmd, but the
# MCP server replays captures through this module, and it must come from the same
# RenderDoc version that made the captures. Builds the module alone (no Qt UI) from
# the commit `renderdoccmd version` reports, into $RENDERDOC_SRC/build/lib.
#
#   RENDERDOC_SRC   checkout location (default ~/src/renderdoc)
#   JOBS            parallel compile jobs (default 16)
set -euo pipefail

src=${RENDERDOC_SRC:-$HOME/src/renderdoc}
jobs=${JOBS:-16}

if ! command -v renderdoccmd >/dev/null; then
    echo "renderdoccmd not found; install RenderDoc first" >&2
    exit 1
fi
read -r tag commit < <(renderdoccmd version | sed -nE 's/.* (v[0-9.]+) built from ([0-9a-f]+).*/\1 \2/p')
if [ -z "${tag:-}" ]; then
    echo "could not read the RenderDoc version from 'renderdoccmd version'" >&2
    exit 1
fi
echo "Installed RenderDoc: $tag ($commit)" >&2

if [ ! -d "$src/.git" ]; then
    git clone --depth 1 --branch "$tag" https://github.com/baldurk/renderdoc.git "$src"
else
    git -C "$src" fetch --depth 1 origin tag "$tag"
    git -C "$src" checkout --quiet "$tag"
fi
if [ "$(git -C "$src" rev-parse HEAD)" != "$commit" ]; then
    echo "warning: $tag in $src is $(git -C "$src" rev-parse --short HEAD), the installed build says $commit" >&2
fi

python_version=$(python3 -c 'import sys; print(f"{sys.version_info[0]}.{sys.version_info[1]}")')
nice -n 10 cmake -S "$src" -B "$src/build" -G Ninja -DCMAKE_BUILD_TYPE=Release \
    -DENABLE_QRENDERDOC=OFF -DENABLE_PYRENDERDOC=ON -DFORCE_PY_VERSION="$python_version" >&2
nice -n 10 cmake --build "$src/build" -j "$jobs" >&2
echo "$src/build/lib/renderdoc.so"
