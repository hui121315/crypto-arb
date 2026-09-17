#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if [ "${1:-}" = "--root" ]; then
  if [ -z "${2:-}" ] || [ "$#" -ne 2 ]; then
    printf 'usage: %s [--root <repo-root>]\n' "$0" >&2
    exit 2
  fi
  ROOT="$(cd "$2" && pwd)"
elif [ "$#" -ne 0 ]; then
  printf 'usage: %s [--root <repo-root>]\n' "$0" >&2
  exit 2
fi

MODULES="$ROOT/frontend/src/panels/modules"
WORKSTATION="$ROOT/frontend/src/panels/workstation.rs"
EXECUTION_RUNTIME="$ROOT/frontend/src/panels/modules/execution/data/runtime.rs"
FAIL=0

fail() {
  printf 'FAIL frontend module boundary: %s\n' "$*" >&2
  FAIL=1
}

require_file() {
  local file="$1"
  [ -f "$file" ] || {
    fail "required anchor missing: ${file#$ROOT/}"
    return 1
  }
}

require_dir() {
  local dir="$1"
  [ -d "$dir" ] || {
    fail "required directory missing: ${dir#$ROOT/}"
    return 1
  }
}

scan_module_root() {
  local file="$1"
  local line line_no in_pub_use=0
  local mod_re='^[[:space:]]*(pub(\([^)]*\))?[[:space:]]+)?mod[[:space:]]+[A-Za-z_][A-Za-z0-9_]*[[:space:]]*;[[:space:]]*$'
  local pub_use_re='^[[:space:]]*pub(\([^)]*\))?[[:space:]]+use[[:space:]]+'

  line_no=0
  while IFS= read -r line || [ -n "$line" ]; do
    line_no=$((line_no + 1))

    if [ "$in_pub_use" -eq 1 ]; then
      [[ "$line" == *';'* ]] && in_pub_use=0
      continue
    fi

    [[ "$line" =~ ^[[:space:]]*$ ]] && continue
    [[ "$line" =~ ^[[:space:]]*// ]] && continue
    [[ "$line" =~ ^[[:space:]]*#\[cfg\(test\)\][[:space:]]*$ ]] && continue
    [[ "$line" =~ $mod_re ]] && continue
    if [[ "$line" =~ $pub_use_re ]]; then
      [[ "$line" != *';'* ]] && in_pub_use=1
      continue
    fi

    fail "${file#$ROOT/}:$line_no module roots may only declare modules or re-export items"
  done < "$file"
}

check_module_roots() {
  local file
  while IFS= read -r file; do
    scan_module_root "$file"
  done < <(find "$MODULES" -mindepth 2 -maxdepth 2 -type f -name mod.rs -print | sort)
}

check_component_client_calls() {
  local matches
  matches="$(rg -n \
    'use_global\(\)\.(client|settings_client)|\.(client|settings_client)\.[A-Za-z_][A-Za-z0-9_]*[[:space:]]*\(' \
    "$MODULES" --glob 'view.rs' --glob 'components/**/*.rs' 2>/dev/null || true)"
  if [ -n "$matches" ]; then
    printf '%s\n' "$matches" >&2
    fail 'components and views must route client calls through data.rs'
  fi
}

check_dto_mirrors() {
  local matches
  matches="$(rg -n \
    '^\s*(pub(\([^)]*\))?\s+)?(struct|enum|type)\s+(PortfolioSummary|PositionRow|RiskSnapshot|SystemHealth|VenueQuality|StrategyPerformance|ExecutedTrade|MissedOpportunity)\b' \
    "$ROOT/frontend/src" 2>/dev/null || true)"
  if [ -n "$matches" ]; then
    printf '%s\n' "$matches" >&2
    fail 'frontend must not mirror core shared DTOs'
  fi
}

match_count() {
  local pattern="$1"
  shift
  rg -n "$pattern" "$@" 2>/dev/null | wc -l | tr -d '[:space:]'
}

check_execution_selection_owner() {
  local futures="$MODULES/futures/view.rs"
  local opportunities="$MODULES/opportunities"
  local owners

  require_file "$WORKSTATION" || return
  require_file "$EXECUTION_RUNTIME" || return
  require_file "$futures" || return
  require_dir "$opportunities" || return

  owners="$(match_count 'RwSignal::new\(\s*ExecutionSelection::empty\(\)\s*\)' --multiline "$ROOT/frontend/src")"
  if [ "$owners" -ne 1 ] || ! rg -q -U 'RwSignal::new\(\s*ExecutionSelection::empty\(\)\s*\)' "$EXECUTION_RUNTIME"; then
    fail 'ExecutionRuntime must be the sole ExecutionSelection signal seed owner'
  fi
  if ! rg -q 'create_execution_runtime\(\)' "$WORKSTATION"; then
    fail 'Workstation must create the canonical ExecutionRuntime'
  fi
  if ! rg -q 'execution_runtime\.seed_selection\(opp\.execution_seed\(\)\)' "$futures"; then
    fail 'Futures must seed ExecutionRuntime through its list-row adapter'
  fi
  if ! rg -q 'execution_runtime\.seed_selection\(ExecutionSelectionSeed::from_opportunities\(row\.as_ref\(\)\)\)' "$opportunities"; then
    fail 'Opportunities must seed ExecutionRuntime from the canonical list view'
  fi
}

require_dir "$MODULES" || exit 1
check_module_roots
check_component_client_calls
check_dto_mirrors
check_execution_selection_owner

if [ "$FAIL" -ne 0 ]; then
  exit 1
fi

printf 'OK frontend module boundary gate\n'
