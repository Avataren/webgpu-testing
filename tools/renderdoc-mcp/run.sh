#!/usr/bin/env bash
# Start the RenderDoc MCP server (registered in .mcp.json). Creates its virtualenv on
# first use or when pyproject.toml changes. RenderDoc's Python module is not part of
# distribution packages; build_renderdoc_module.sh builds it. Only JSON-RPC may go to
# stdout.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
venv=$here/.venv
module_dir=${RENDERDOC_MODULE_PATH:-$HOME/src/renderdoc/build/lib}

if [ ! -f "$module_dir/renderdoc.so" ]; then
    echo "renderdoc-mcp: no RenderDoc Python module (renderdoc.so) in $module_dir;" \
        "run $here/build_renderdoc_module.sh or set RENDERDOC_MODULE_PATH" >&2
    exit 1
fi

if [ ! -x "$venv/bin/python" ] || [ "$here/pyproject.toml" -nt "$venv/.installed" ]; then
    python3 -m venv "$venv" >&2
    "$venv/bin/pip" install --quiet --disable-pip-version-check -e "$here[test]" >&2
    touch "$venv/.installed"
fi

export RENDERDOC_MODULE_PATH=$module_dir
# -P: nothing in the working directory (the repo) can shadow the server's imports.
exec "$venv/bin/python" -P -m renderdoc_mcp "$@"
