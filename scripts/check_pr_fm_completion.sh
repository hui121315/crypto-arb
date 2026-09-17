#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
DOC="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
EVIDENCE="$ROOT/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
INVENTORY="$ROOT/docs/API_ROUTE_INVENTORY.tsv"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-FM completion gate failed: %s\n' "$1" >&2
  exit 1
}

must_not_match() {
  local label="$1"
  local pattern="$2"
  shift 2
  if rg -n -- "$pattern" "$@"; then
    fail "$label"
  fi
}

must_match() {
  local label="$1"
  local pattern="$2"
  shift 2
  if ! rg -n -- "$pattern" "$@" >/dev/null; then
    fail "$label"
  fi
}

require_evidence() {
  local evidence_type="$1"
  if ! awk -F '\t' -v evidence_type="$evidence_type" '
    $1 == "PR-FM" && $2 == evidence_type { found = 1 }
    END { exit(found ? 0 : 1) }
  ' "$EVIDENCE"; then
    fail "missing PR-FM evidence type $evidence_type"
  fi
}

if [[ "$MODE" == "--self-test" ]]; then
  api_target="$ROOT/crates/api/Cargo.toml"
  root_target="$ROOT/Cargo.toml"
  api_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-fm-api.XXXXXX")"
  root_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-fm-root.XXXXXX")"
  cp "$api_target" "$api_backup"
  cp "$root_target" "$root_backup"
  restore() {
    cp "$api_backup" "$api_target"
    cp "$root_backup" "$root_target"
    rm -f "$api_backup" "$root_backup"
  }
  trap restore EXIT
  printf '\nlegacy-simulation = []\n' >>"$api_target"
  if bash "$0"; then
    fail "self-test unexpectedly accepted a restored legacy simulation feature"
  fi
  cp "$api_backup" "$api_target"
  printf '\nsimulation = { path = "crates/simulation" }\n' >>"$root_target"
  if bash "$0"; then
    fail "self-test unexpectedly accepted a shared workspace simulation dependency"
  fi
  printf 'PR-FM completion self-test passed\n'
  exit 0
fi

for removed in \
  "$ROOT/crates/api/src/routers/simulation.rs" \
  "$ROOT/frontend/src/api/rest/options.rs"; do
  [[ ! -e "$removed" ]] || fail "removed surface still exists: ${removed#$ROOT/}"
done

must_not_match \
  "legacy simulation dependency or feature remains in api" \
  'legacy-simulation|dep:simulation|simulation = \{ workspace = true, optional = true \}' \
  "$ROOT/crates/api/Cargo.toml"
must_not_match \
  "simulation remains a shared workspace dependency" \
  '^simulation[[:space:]]*=' \
  "$ROOT/Cargo.toml"
must_not_match \
  "legacy simulation wiring remains in the API runtime" \
  'legacy-simulation|sim_portfolio|RouteGate::Simulation|SIMULATION_ENDPOINTS|routers::simulation|append_simulation_stores' \
  "$ROOT/crates/api/src/app.rs" \
  "$ROOT/crates/api/src/state.rs" \
  "$ROOT/crates/api/src/route_specs.rs" \
  "$ROOT/crates/api/src/routers/mod.rs" \
  "$ROOT/crates/api/src/services/runtime_state.rs"
must_not_match \
  "simulation api-surface config remains" \
  'api_surface\.simulation|APP_API_SURFACE__SIMULATION' \
  "$ROOT/crates/common/src/config.rs" \
  "$ROOT/.env.example"
must_not_match \
  "simulation route remains in the inventory" \
  '/api/simulation|api_surface\.simulation' \
  "$INVENTORY"
must_match \
  "frontend options/simulation boundary regression test is missing" \
  'legacy_options_and_simulation_cannot_reenter_the_main_rest_client' \
  "$ROOT/frontend/src/api/rest.rs"

must_match \
  "options positions no longer fail closed" \
  'OPTIONS_POSITIONS_UNSUPPORTED' \
  "$ROOT/crates/api/src/routers/options.rs" \
  "$ROOT/crates/api/src/app.rs"
must_match \
  "OptionsPerpBasis no longer has a diagnostics-only policy" \
  'is_diagnostic_only_strategy' \
  "$ROOT/shared-types/src/strategy.rs"
must_match \
  "OptionsPerpBasis can fall back into a P0 market rule" \
  'Some\(kind\) if !is_p0_executable_strategy\(kind\) => None' \
  "$ROOT/crates/arbitrage/src/strategy_registry.rs"
must_match \
  "deleted simulation route lacks an all-surfaces smoke test" \
  'deleted_simulation_routes_stay_absent_with_all_remaining_surfaces_enabled' \
  "$ROOT/crates/api/src/app.rs"

if ! awk -F '\t' '
  $1 ~ /^\/api\/options\// {
    count += 1
    if ($11 != "none") {
      bad = 1
      printf "unexpected frontend client for %s: %s\n", $1, $11 > "/dev/stderr"
    }
  }
  END { exit(count == 4 && !bad ? 0 : 1) }
' "$INVENTORY"; then
  fail "options calculator inventory must have four clientless diagnostic routes"
fi

roadmap_row="$(rg -F '| `PR-FM Options Simulation Surface, Unsupported Semantics & Legacy Route Contract`' "$DOC" || true)"
[[ "$roadmap_row" == *"✅ 完成"* ]] || fail "roadmap row is not completed"
[[ "$roadmap_row" == *"剩余：无。"* ]] || fail "roadmap row still reports a remainder"
[[ "$roadmap_row" == *"check_pr_fm_completion.sh"* ]] || fail "roadmap row lacks the completion gate"
if rg -q '^1\. \*\*PR-FM ' "$DOC"; then
  fail "completed PR-FM remains at the queue head"
fi

for evidence_type in \
  simulation-http-surface-deletion \
  options-perp-basis-hard-gate \
  frontend-options-wrapper-removal \
  completion-governance-gate; do
  require_evidence "$evidence_type"
done

bash "$ROOT/scripts/check_dependency_budget.sh"
bash "$ROOT/scripts/check_api_cargo_tree_budget.sh"
bash "$ROOT/scripts/check_route_inventory.sh"
cargo test -p shared-types --lib options_perp_basis_is_diagnostic_only --no-fail-fast
cargo test -p arbitrage --lib options_perp_basis_cannot_fall_back_to_a_p0_market_rule --no-fail-fast
cargo test -p api --bin crypto-arb-api deleted_simulation_routes_stay_absent_with_all_remaining_surfaces_enabled --no-fail-fast
cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib legacy_options_and_simulation_cannot_reenter_the_main_rest_client --no-fail-fast

printf 'OK PR-FM Options/Simulation surface completion gate\n'
