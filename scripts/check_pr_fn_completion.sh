#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
DOC="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
EVIDENCE="$ROOT/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
INVENTORY="$ROOT/docs/API_ROUTE_INVENTORY.tsv"
QUERY="$ROOT/crates/api/src/services/spot/query.rs"
ROUTE_SPECS="$ROOT/crates/api/src/route_specs.rs"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-FN completion gate failed: %s\n' "$1" >&2
  exit 1
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
    $1 == "PR-FN" && $2 == evidence_type { found = 1 }
    END { exit(found ? 0 : 1) }
  ' "$EVIDENCE"; then
    fail "missing PR-FN evidence type $evidence_type"
  fi
}

if [[ "$MODE" == "--self-test" ]]; then
  query_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-fn-query.XXXXXX")"
  route_specs_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-fn-route-specs.XXXXXX")"
  cp "$QUERY" "$query_backup"
  cp "$ROUTE_SPECS" "$route_specs_backup"
  restore() {
    cp "$query_backup" "$QUERY"
    cp "$route_specs_backup" "$ROUTE_SPECS"
    rm -f "$query_backup" "$route_specs_backup"
  }
  trap restore EXIT

  perl -0pi -e 's/pub\(super\) const SPOT_TICKS_MAX_LIMIT: usize = 128;/pub(super) const SPOT_TICKS_MAX_LIMIT: usize = 1024;/' "$QUERY"
  if bash "$0"; then
    fail "self-test unexpectedly accepted an unbounded spot diagnostic page"
  fi

  cp "$query_backup" "$QUERY"
  cp "$route_specs_backup" "$ROUTE_SPECS"
  perl -0pi -e 's/(const SPOT_ENDPOINTS:[\s\S]*?"\/api\/v1\/spot\/ticks",\n\s*)"diagnostic"/${1}"legacy"/' "$ROUTE_SPECS"
  if bash "$0"; then
    fail "self-test unexpectedly accepted a legacy-classified spot diagnostic route"
  fi

  printf 'PR-FN completion self-test passed\n'
  exit 0
fi

must_match \
  "shared spot query DTO is missing" \
  'pub struct SpotTicksQuery' \
  "$ROOT/shared-types/src/spot.rs"
must_match \
  "shared bounded spot page DTO is missing" \
  'pub struct SpotTicksPage' \
  "$ROOT/shared-types/src/spot.rs"
must_match \
  "spot router reintroduced a local request DTO" \
  'Query<SpotTicksQuery>' \
  "$ROOT/crates/api/src/routers/spot.rs"
must_match \
  "spot diagnostic page limit is no longer bounded at 128" \
  'pub\(super\) const SPOT_TICKS_MAX_LIMIT: usize = 128;' \
  "$QUERY"
must_match \
  "spot query no longer carries fresh-only row evidence" \
  'fresh_only|is_fresh\(row\)' \
  "$QUERY"
must_match \
  "spot query lost cursor/limit diagnostics" \
  'LIST_CURSOR_INVALID|LIST_LIMIT_CLAMPED' \
  "$QUERY"
must_match \
  "spot diagnostic lost request correlation" \
  'common::request_id::current' \
  "$QUERY"
must_match \
  "spot diagnostic lost official base listing coverage" \
  'base_listing_coverage' \
  "$ROOT/crates/api/src/services/spot.rs" \
  "$QUERY"
must_match \
  "spot diagnostics no longer render venue outcome and listing evidence" \
  'fanout_row|listing_row|base_listing_coverage' \
  "$ROOT/frontend/src/panels/modules/settings/tabs/diagnostics/spot_debug.rs"
must_match \
  "non-P0 strategy fallback regression test is missing" \
  'diagnostic_non_p0_kinds_cannot_fall_back_to_p0_market_rules' \
  "$ROOT/crates/arbitrage/src/strategy_registry.rs"

if ! awk '
  /const SPOT_ENDPOINTS/ { in_spot = 1 }
  in_spot && /"\/api\/v1\/spot\/ticks"/ { found_path = 1 }
  in_spot && found_path && /"diagnostic"/ { found = 1; exit }
  END { exit(found ? 0 : 1) }
' "$ROUTE_SPECS"; then
  fail "spot route is not classified as a default-off diagnostic surface"
fi

if ! awk -F '\t' '
  $1 == "/api/v1/spot/ticks" && $4 == "diagnostic" && $5 == "default_off" \
    && $9 == "api_surface.spot_v1" && $10 == "shared" \
    && $11 == "ApiClient::spot_ticks+ApiClient::spot_ticks_query" { found = 1 }
  END { exit(found ? 0 : 1) }
' "$INVENTORY"; then
  fail "spot route inventory does not preserve the diagnostic runtime contract"
fi

must_match \
  "route browser fixture does not retain the spot diagnostic gate" \
  'disabled_spot_diagnostic' \
  "$ROOT/test/e2e/fixtures/route_runtime_policy.mjs" \
  "$ROOT/test/e2e/route_registry.spec.ts"

roadmap_row="$(rg -F '| `PR-FN Spot v1 Market Data Envelope, Coverage & Diagnostics Contract`' "$DOC" || true)"
[[ "$roadmap_row" == *"✅ 完成"* ]] || fail "roadmap row is not completed"
[[ "$roadmap_row" == *"剩余：无。"* ]] || fail "roadmap row still reports a remainder"
[[ "$roadmap_row" == *"check_pr_fn_completion.sh"* ]] || fail "roadmap row lacks the completion gate"
if rg -q '^1\. \*\*PR-FN ' "$DOC"; then
  fail "completed PR-FN remains at the queue head"
fi

for evidence_type in \
  shared-spot-query-page-contract \
  venue-listing-runtime-evidence \
  diagnostic-p0-boundary \
  settings-spot-diagnostics \
  route-browser-diagnostic-gate \
  completion-governance-gate; do
  require_evidence "$evidence_type"
done

bash "$ROOT/scripts/check_route_inventory.sh"
bash "$ROOT/scripts/check_route_runtime_policy.sh"
cargo test -p shared-types --lib spot --no-fail-fast
cargo test -p arbitrage --lib diagnostic_non_p0_kinds_cannot_fall_back_to_p0_market_rules --no-fail-fast
cargo test -p api --bin crypto-arb-api spot --no-fail-fast
cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib spot_debug --no-fail-fast
CI=1 npx playwright test "$ROOT/test/e2e/route_registry.spec.ts" --grep 'gated diagnostic routes' --reporter=list

printf 'OK PR-FN Spot diagnostics completion gate\n'
