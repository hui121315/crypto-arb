#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
DOC="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
EVIDENCE="$ROOT/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
HTTP="$ROOT/crates/exchange/src/http.rs"
GATE_DATA="$ROOT/crates/exchange/src/adapters/gate_private_data.rs"
GATE_REST="$ROOT/crates/exchange/src/adapters/gate_private_rest.rs"
API_PROJECTION="$ROOT/crates/api/src/services/account_positions/projection/liquidation.rs"
API_TESTS="$ROOT/crates/api/src/services/account_positions/tests/error_paths.rs"
FRONTEND_VIEW="$ROOT/frontend/src/panels/modules/positions/view.rs"
FRONTEND_TESTS="$ROOT/frontend/src/panels/modules/positions/view/tests.rs"
SUCCESS_FIXTURE="$ROOT/crates/exchange/fixtures/gate/futures_usdt_positions_v4_106_106.json"
ERROR_FIXTURE="$ROOT/crates/exchange/fixtures/gate/futures_usdt_positions_rate_limited.json"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-FB completion gate failed: %s\n' "$1" >&2
  exit 1
}

must_match() {
  local label="$1"
  local pattern="$2"
  shift 2
  rg -n -- "$pattern" "$@" >/dev/null || fail "$label"
}

require_evidence() {
  local evidence_type="$1"
  awk -F '\t' -v evidence_type="$evidence_type" '
    $1 == "PR-FB" && $2 == evidence_type { found = 1 }
    END { exit(found ? 0 : 1) }
  ' "$EVIDENCE" || fail "missing PR-FB evidence type $evidence_type"
}

if [[ "$MODE" == "--self-test" ]]; then
  backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-fb-error-fixture.XXXXXX")"
  cp "$ERROR_FIXTURE" "$backup"
  restore() {
    cp "$backup" "$ERROR_FIXTURE"
    rm -f "$backup"
  }
  trap restore EXIT

  rm "$ERROR_FIXTURE"
  if PR_FB_SKIP_TESTS=1 bash "$0"; then
    fail "self-test unexpectedly accepted a missing Gate 429 fixture"
  fi

  printf 'PR-FB completion self-test passed\n'
  exit 0
fi

[[ -s "$SUCCESS_FIXTURE" ]] || fail "current Gate positions fixture is missing"
[[ -s "$ERROR_FIXTURE" ]] || fail "Gate 429 fixture is missing"
must_match "current Gate fixture lacks position mode" '"mode": "single"' "$SUCCESS_FIXTURE"
must_match "current Gate fixture lacks current leverage" '"lever": "30"' "$SUCCESS_FIXTURE"
must_match "current Gate fixture lacks average maintenance" '"average_maintenance_rate": "0\.005"' "$SUCCESS_FIXTURE"
must_match "Gate error fixture lacks typed label" '"label": "TOO_MANY_REQUESTS"' "$ERROR_FIXTURE"
must_match "shared HTTP client ignores Gate reset header" 'x-gate-ratelimit-reset-timestamp' "$HTTP"
must_match "Gate adapter does not derive liquidation distance" 'gate_liquidation_distance_pct' "$GATE_DATA"
must_match "Gate success HTTP fixture test is missing" 'gate_positions_http_fixture_maps_current_risk_semantics' "$GATE_REST"
must_match "Gate 429 HTTP fixture test is missing" 'gate_positions_http_rate_limit_uses_reset_timestamp' "$GATE_REST"
must_match "Gate liquidation source evidence is missing" 'gate_rest_liq_price_derived_distance' "$API_PROJECTION"
must_match "API partial fanout rate-limit test is missing" 'gate_rate_limit_envelope_keeps_rows_request_id_and_retry_after' "$API_TESTS"
must_match "positions view ignores its child envelope" 'position_snapshot_section' "$FRONTEND_VIEW"
must_match "positions request context regression is missing" 'degraded_position_envelope_keeps_request_context' "$FRONTEND_TESTS"

for evidence_type in \
  gate-current-position-schema-fixture \
  gate-rate-limit-http-fixture \
  gate-liquidation-maintenance-evidence \
  positions-partial-envelope-diagnostic \
  positions-load-state-ui \
  completion-governance; do
  require_evidence "$evidence_type"
done

roadmap_row="$(rg -F '| `PR-FB Gate Position Schema, Partial Fanout & Portfolio Degrade Contract`' "$DOC" || true)"
[[ "$roadmap_row" == *"✅ 完成"* ]] || fail "roadmap row is not completed"
[[ "$roadmap_row" == *"剩余：无。"* ]] || fail "roadmap row still reports a remainder"
queue_section="$(sed -n '/^### 🟡 6\.5/,/^## /p' "$DOC")"
[[ "$queue_section" != *'**PR-FB Gate Position Schema'* ]] || fail "completed PR-FB remains in the queue"

if [[ "${PR_FB_SKIP_TESTS:-0}" == "1" ]]; then
  printf 'OK PR-FB static completion contract\n'
  exit 0
fi

JOBS="${CARGO_BUILD_JOBS:-8}"
CARGO_BUILD_JOBS="$JOBS" cargo test -p exchange gate_positions_http --lib --no-fail-fast
CARGO_BUILD_JOBS="$JOBS" cargo test -p exchange gate_reset_header --lib --no-fail-fast
CARGO_BUILD_JOBS="$JOBS" cargo test -p api --bin crypto-arb-api gate_rate_limit_envelope_keeps_rows_request_id_and_retry_after --no-fail-fast
CARGO_BUILD_JOBS="$JOBS" cargo test -p api --bin crypto-arb-api gate_rest_risk_fields_keep_actual_source_evidence --no-fail-fast
CARGO_BUILD_JOBS="$JOBS" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib position_envelope --no-fail-fast
bash "$ROOT/scripts/check_exchange_evidence_debt.sh"

printf 'OK PR-FB Gate position evidence and partial-envelope contract\n'
