#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CHECK="$ROOT/scripts/check_crate_root_boundaries.sh"
FIXTURES="$ROOT/scripts/fixtures/crate_root_boundaries"

expect_failure() {
  local fixture="$1"
  local expected="$2"
  local output

  if output="$(bash "$CHECK" --root "$FIXTURES/$fixture" 2>&1)"; then
    printf 'crate root boundary self-test failed: %s unexpectedly passed\n' "$fixture" >&2
    exit 1
  fi
  if ! grep -Fq "$expected" <<<"$output"; then
    printf 'crate root boundary self-test failed: %s did not report %s\n' "$fixture" "$expected" >&2
    printf '%s\n' "$output" >&2
    exit 1
  fi
}

bash "$CHECK" --root "$FIXTURES/good"
expect_failure bad_logic 'unexpected_root_item'

printf 'OK crate root boundary self-test\n'
