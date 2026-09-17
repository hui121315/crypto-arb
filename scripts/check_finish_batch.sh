#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUNNER="$ROOT/scripts/finish_batch.sh"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/crossline-finish-self-test.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT

fail() {
  printf 'finish batch self-test failed: %s\n' "$1" >&2
  exit 1
}

assert_plan() {
  local files="$1"
  shift
  local output
  output="$(cd "$ROOT" && CROSSLINE_FINISH_FILES="$files" bash "$RUNNER" --plan)"
  local expected
  for expected in "$@"; do
    [[ "$output" == *"$expected"* ]] || fail "plan lacks $expected: $output"
  done
}

bash -n "$RUNNER"

assert_plan \
  $'crates/api/src/services/onchain_comparison.rs\ncrates/onchain-monitor/src/runtime.rs' \
  'source=explicit' 'backend=1' 'packages=api,onchain-monitor' \
  'frontend_rust=0' 'docs=0'

assert_plan \
  $'frontend/src/panels/modules/onchain/view.rs\nfrontend/styles/src/skin/onchain.css\nDESIGN.md' \
  'backend=0' 'frontend_rust=1' 'frontend_ui=1' 'docs=1'

assert_plan \
  $'docs/CROSSLINE_IMPLEMENTATION_GUIDE.md\nREADME.md' \
  'backend=0' 'frontend_rust=0' 'frontend_ui=0' 'docs=1'

PATHS_FILE="$TMP/paths"
HASHES_FILE="$TMP/hashes"
mkdir -p "$TMP/state"
cd "$ROOT"
git ls-files -co --exclude-standard | sort -u | while IFS= read -r file; do
  [ -f "$file" ] && printf '%s\n' "$file"
done > "$PATHS_FILE"
git hash-object --stdin-paths < "$PATHS_FILE" > "$HASHES_FILE"
paste "$PATHS_FILE" "$HASHES_FILE" > "$TMP/state/files.tsv"

cached_plan="$(CROSSLINE_FINISH_STATE_DIR="$TMP/state" bash "$RUNNER" --plan)"
for expected in \
  'source=last-success' 'backend=0' 'frontend_rust=0' 'frontend_ui=0' 'docs=0'; do
  [[ "$cached_plan" == *"$expected"* ]] || fail "cached plan lacks $expected: $cached_plan"
done

python3 - "$ROOT" <<'PY'
import json
from pathlib import Path
import sys

root = Path(sys.argv[1])
scripts = json.loads((root / "package.json").read_text(encoding="utf-8"))["scripts"]
expected = {
    "finish": "bash scripts/finish_batch.sh",
    "finish:plan": "bash scripts/finish_batch.sh --plan",
    "finish:release": "bash scripts/finish_batch.sh --release",
}
for name, command in expected.items():
    if scripts.get(name) != command:
        raise SystemExit(f"finish batch self-test failed: package script {name} drifted")

verify = (root / "scripts/verify_implementation.sh").read_text(encoding="utf-8")
if "exec bash scripts/finish_batch.sh --release" not in verify:
    raise SystemExit("finish batch self-test failed: verify entrypoint duplicates the finish path")

runner = (root / "scripts/finish_batch.sh").read_text(encoding="utf-8")
if 'CROSSLINE_RELEASE_DIST' not in runner or '--dist "$RELEASE_DIST"' not in runner:
    raise SystemExit("finish batch self-test failed: release output must be isolated from the dev server")
execution = runner[runner.index('if [ "$SHELL"'):]
if execution.index('UI static contract') > execution.index('Backend lint and tests'):
    raise SystemExit("finish batch self-test failed: cheap UI checks must precede backend tests")
if execution.index('Current documentation contracts') > execution.index('Backend lint and tests'):
    raise SystemExit("finish batch self-test failed: cheap docs checks must precede backend tests")
if execution.index('Wasm budget') > execution.index('Backend lint and tests'):
    raise SystemExit("finish batch self-test failed: release size must precede backend tests")
PY

printf 'OK change-aware finish flow self-test\n'
