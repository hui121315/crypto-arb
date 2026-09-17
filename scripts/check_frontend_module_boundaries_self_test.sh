#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CHECK="$ROOT/scripts/check_frontend_module_boundaries.sh"
FIXTURES="$ROOT/scripts/fixtures/frontend_module_boundaries"

expect_failure() {
  local fixture="$1"
  local expected="$2"
  local output

  if output="$(bash "$CHECK" --root "$FIXTURES/$fixture" 2>&1)"; then
    printf 'frontend module boundary self-test failed: %s unexpectedly passed\n' "$fixture" >&2
    exit 1
  fi
  if ! grep -Fq "$expected" <<<"$output"; then
    printf 'frontend module boundary self-test failed: %s did not report %s\n' "$fixture" "$expected" >&2
    printf '%s\n' "$output" >&2
    exit 1
  fi
}

bash "$CHECK" --root "$FIXTURES/good"
expect_failure bad_module_root 'module roots may only declare modules or re-export items'
expect_failure bad_component_client 'components and views must route client calls through data.rs'
expect_failure bad_dto_mirror 'frontend must not mirror core shared DTOs'
expect_failure bad_selection_owner 'ExecutionRuntime must be the sole ExecutionSelection signal seed owner'

printf 'OK frontend module boundary self-test\n'
