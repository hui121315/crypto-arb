#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CHECK="$ROOT/scripts/check_module_size.sh"
FIXTURES="$ROOT/scripts/fixtures/module_size"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/crossline-module-size-self-test.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT

expect_failure() {
  local fixture="$1"
  local message="$2"
  local output

  if output="$(bash "$CHECK" --check-monotonic "$FIXTURES/$fixture" "$FIXTURES/baseline.tsv" 2>&1)"; then
    echo "FAIL expected monotonic check to reject ${fixture}"
    exit 1
  fi
  if ! grep -Fq "$message" <<<"$output"; then
    echo "FAIL ${fixture} did not report expected message: ${message}"
    printf '%s\n' "$output"
    exit 1
  fi
}

bash "$CHECK" --check-monotonic "$FIXTURES/reduction.tsv" "$FIXTURES/baseline.tsv"
expect_failure cap_increase.tsv "module-size debt cap increase"
expect_failure new_debt.tsv "new module-size debt entry is not allowed"

bash "$CHECK" --check-inherited-debt 318 318 300
bash "$CHECK" --check-inherited-debt 310 318 300

expect_inherited_failure() {
  local current="$1"
  local baseline="$2"
  local max="$3"
  local output

  if output="$(bash "$CHECK" --check-inherited-debt "$current" "$baseline" "$max" 2>&1)"; then
    echo "FAIL expected inherited debt check to reject ${current}/${baseline}/${max}"
    exit 1
  fi
  if ! grep -Fq "inherited module-size debt is not unchanged" <<<"$output"; then
    echo "FAIL inherited debt check did not report expected failure"
    printf '%s\n' "$output"
    exit 1
  fi
}

expect_inherited_failure 319 318 300
expect_inherited_failure 318 300 300
expect_inherited_failure 300 318 300

MODULE_SIZE_REPORT="$TMP/module-size.json" bash "$CHECK" >/dev/null
python3 - "$TMP/module-size.json" <<'PY'
import json
import sys
from pathlib import Path

report = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
assert report["schemaVersion"] == 1
assert report["generatedBy"] == "scripts/check_module_size.sh"
assert report["gatePassed"] is True
assert report["summary"]["scanned"] == len(report["modules"])
assert report["summary"]["failures"] == 0
paths = [row["path"] for row in report["modules"]]
assert paths == sorted(set(paths))
assert {row["kind"] for row in report["modules"]} == {"css", "rust"}
assert all(row["lines"] >= 0 and row["maxLines"] == 300 for row in report["modules"])
PY

echo "module-size monotonic and JSON report self-test passed"
