#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# No daemon or global-cache scan: maintenance runs at idle project checkpoints.
exec python3 -B "$ROOT/scripts/cache_hygiene.py" "$@"
