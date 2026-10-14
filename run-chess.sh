#!/usr/bin/env bash
set -euo pipefail
chess_repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
exec "$chess_repo_root/runchess.sh" "$@"
