#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

DEV_RELEASE_PORTS="${DEV_RELEASE_PORTS:-1}" bash "$ROOT/scripts/dev_down.sh"
bash "$ROOT/scripts/dev_up.sh"
