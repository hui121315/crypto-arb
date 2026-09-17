#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
DOC="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
EVIDENCE="$ROOT/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
STRATEGY="$ROOT/shared-types/src/strategy.rs"
STRATEGY_CAPABILITIES="$ROOT/shared-types/src/strategy_capabilities.rs"
MARKET="$ROOT/crates/arbitrage/src/algorithms/market.rs"
PERP_SCANNER="$ROOT/crates/arbitrage/src/algorithms/perp_price_spread.rs"
SPOT_SCANNER="$ROOT/crates/arbitrage/src/algorithms/spot_cross.rs"
REGISTRY="$ROOT/crates/arbitrage/src/strategy_registry.rs"
CALCULATOR="$ROOT/crates/arbitrage/src/calculator.rs"
FEES="$ROOT/crates/api/src/services/hedge_ticket/fees.rs"
INVENTORY="$ROOT/crates/api/src/services/hedge_preview/guards/spot_inventory.rs"
PREVIEW="$ROOT/crates/api/src/services/hedge_preview/pricing.rs"
INSTRUMENT_GATE="$ROOT/crates/api/src/services/instrument_registry/coverage/execution_gate.rs"
FRONTEND_FILTER="$ROOT/frontend/src/panels/modules/futures/data/runtime.rs"
FRONTEND_COLUMNS="$ROOT/frontend/src/panels/modules/futures/columns.rs"
FRONTEND_PREVIEW="$ROOT/frontend/src/panels/modules/execution/data/preview/response.rs"
MARKET_VERIFY="$ROOT/scripts/verify_market_apis.sh"
README="$ROOT/README.md"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-GC completion gate failed: %s\n' "$1" >&2
  exit 1
}

must_match() {
  local label="$1"
  local pattern="$2"
  shift 2
  rg -n -- "$pattern" "$@" >/dev/null || fail "$label"
}

must_not_match() {
  local label="$1"
  local pattern="$2"
  shift 2
  if rg -n -- "$pattern" "$@" >/dev/null; then
    fail "$label"
  fi
}

require_evidence() {
  local evidence_type="$1"
  local artifact="$2"
  if ! awk -F '\t' -v evidence_type="$evidence_type" -v artifact="$artifact" '
    $1 == "PR-GC" && $2 == evidence_type && $3 == artifact { found = 1 }
    END { exit(found ? 0 : 1) }
  ' "$EVIDENCE"; then
    fail "missing PR-GC evidence $evidence_type -> $artifact"
  fi
}

if [[ "$MODE" == "--self-test" ]]; then
  backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-gc-strategy.XXXXXX")"
  cp "$STRATEGY" "$backup"
  restore() {
    cp "$backup" "$STRATEGY"
    rm -f "$backup"
  }
  trap restore EXIT
  python3 - "$STRATEGY" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "    PerpPriceSpread,\n"
if source.count(marker) != 1:
    raise SystemExit("PR-GC self-test setup failed: strategy marker drifted")
path.write_text(source.replace(marker, "    PerpPriceSpreadRemoved,\n", 1), encoding="utf-8")
PY
  if bash "$0"; then
    fail "self-test unexpectedly accepted a missing PerpPriceSpread strategy"
  fi
  printf 'PR-GC completion self-test passed\n'
  exit 0
fi

must_match \
  "shared strategy enum lost PerpPriceSpread" \
  '^    PerpPriceSpread,$' \
  "$STRATEGY"
must_match \
  "shared strategy enum lost SpotCross" \
  '^    SpotCross,$' \
  "$STRATEGY"
must_match \
  "P0 strategy allowlist is not five strategies" \
  'P0_EXECUTABLE_STRATEGY_KINDS: \[StrategyKind; 5\]' \
  "$STRATEGY_CAPABILITIES"
must_match \
  "Live strategy capability no longer fails closed to perpetual-only strategies" \
  'LIVE_EXECUTABLE_STRATEGY_KINDS: \[StrategyKind; 2\]' \
  "$STRATEGY_CAPABILITIES"
must_match \
  "strategy contract lost the separate Live execution capability" \
  'live_execution_supported|is_main_p0_live_executable' \
  "$STRATEGY"
must_match \
  "cross-quote or unknown-quote rows are no longer rejected" \
  'fn canonical_quote_symbol|fn quote_assets_match' \
  "$MARKET"
must_match \
  "perpetual price-spread scanner lost executable bid/ask regression" \
  'emits_executable_bid_ask_spread_with_funding_evidence' \
  "$PERP_SCANNER"
must_match \
  "perpetual price-spread scanner lost funding schedule blocker" \
  'missing_funding_keeps_diagnostic_row_fail_closed' \
  "$PERP_SCANNER"
must_match \
  "spot-cross scanner lost prefunded inventory regression" \
  'emits_prefunded_same_quote_spot_spread' \
  "$SPOT_SCANNER"
must_match \
  "spot-cross scanner lost FX and same-venue rejection" \
  'rejects_fx_exposure_and_same_venue_aliases' \
  "$SPOT_SCANNER"
must_match \
  "one-shot strategies no longer reserve a full cycle exactly once" \
  'one_shot_price_spreads_must_cover_the_full_cycle_cost_once' \
  "$REGISTRY"
must_match \
  "funding cap is no longer fail-closed without per-contract evidence" \
  'funding_cap_distance_bps: None' \
  "$CALCULATOR"
must_not_match \
  "a universal funding cap constant re-entered the product" \
  'FUNDING_CAP_BPS' \
  "$ROOT/crates/arbitrage/src"
must_match \
  "price-spread legs lost their actual Spot/Perp fee products" \
  'StrategyKind::SpotCross|StrategyKind::PerpPriceSpread' \
  "$FEES"
must_match \
  "spot-cross Live preview lost explicit quote/base inventory requirements" \
  '买入端计价币|卖出端基础币库存' \
  "$INVENTORY"
must_match \
  "spot-cross missing balance no longer names exact configuration" \
  'missing_assets_name_the_exact_configuration' \
  "$INVENTORY"
must_match \
  "preview no longer separates one-shot gross edge from costs" \
  'fn gross_edge_usd|estimated_gross_edge_usd' \
  "$PREVIEW"
must_match \
  "execution gate lost economic-identity or product-specific checks" \
  'listing_gate_blocks_same_symbol_with_different_asset_classes|spot_strategy_requires_official_spot_execution_specs' \
  "$ROOT/crates/api/src/services/instrument_registry/coverage/tests"
must_match \
  "execution gate lost incompatible price-unit protection" \
  'MAX_EXECUTABLE_PRICE_SCALE_RATIO|price_scale_ratio' \
  "$INSTRUMENT_GATE"
must_match \
  "frontend strategy filter lost PerpPriceSpread or SpotCross" \
  'PerpPriceSpread|SpotCross' \
  "$FRONTEND_FILTER"
must_match \
  "frontend one-shot table contract is missing" \
  'one_shot_spreads_hide_funding_and_annualized_columns' \
  "$FRONTEND_COLUMNS"
must_match \
  "frontend preview can no longer consume the gross-edge contract" \
  'estimated_gross_edge_usd' \
  "$FRONTEND_PREVIEW"
must_match \
  "README lost the price-spread product boundary" \
  '^## 价差套利$' \
  "$README"
must_match \
  "market API verification lost the five-strategy capability contract" \
  'LIVE_KINDS = \{"perp_cross", "perp_price_spread"\}' \
  "$MARKET_VERIFY"
must_match \
  "market API verification lost the Spot v1 data envelope" \
  'isinstance\(data\.get\("ticks"\), list\)' \
  "$MARKET_VERIFY"
must_match \
  "market API verification lost the default-off Spot route boundary" \
  'default_off enable=APP_API_SURFACE__SPOT_V1=true' \
  "$MARKET_VERIFY"

roadmap_row="$(rg -F '| `PR-GC Price Spread Arbitrage Product Contract`' "$DOC" || true)"
[[ "$roadmap_row" == *"✅ 完成"* ]] || fail "roadmap row is not completed"
[[ "$roadmap_row" == *"剩余：无。"* ]] || fail "roadmap row still reports a local remainder"
[[ "$roadmap_row" == *"check_pr_gc_completion.sh"* ]] || fail "roadmap row lacks this completion gate"
if rg -q '^1\. \*\*PR-GC\*\*' "$DOC"; then
  fail "completed PR-GC remains at the local queue head"
fi

require_evidence shared-price-spread-strategy shared-types/src/strategy.rs
require_evidence strict-same-quote-fx-boundary crates/arbitrage/src/algorithms/market.rs
require_evidence perp-price-spread-scanner crates/arbitrage/src/algorithms/perp_price_spread.rs
require_evidence prefunded-spot-cross-scanner crates/arbitrage/src/algorithms/spot_cross.rs
require_evidence full-cycle-price-spread-cost crates/arbitrage/src/strategy_registry.rs
require_evidence per-contract-funding-cap-boundary crates/arbitrage/src/calculator.rs
require_evidence spot-cross-inventory-preflight crates/api/src/services/hedge_preview/guards/spot_inventory.rs
require_evidence gross-edge-preview-contract crates/api/src/services/hedge_preview/pricing.rs
require_evidence economic-identity-and-product-gate crates/api/src/services/instrument_registry/coverage/execution_gate.rs
require_evidence economic-identity-gate-regression crates/api/src/services/instrument_registry/coverage/tests/execution_gate.rs
require_evidence frontend-five-strategy-query frontend/src/api/rest/arbitrage/paths.rs
require_evidence frontend-one-shot-spread-contract frontend/src/panels/modules/futures/data/runtime.rs
require_evidence frontend-gross-edge-preview frontend/src/panels/modules/execution/data/preview/response.rs
require_evidence completion-governance-gate scripts/check_pr_gc_completion.sh
require_evidence strategy-live-execution-capability shared-types/src/strategy_capabilities.rs
require_evidence market-api-runtime-contract scripts/verify_market_apis.sh

CARGO_BUILD_JOBS=8 cargo test -p shared-types p0_query_values_are_stable --no-fail-fast
CARGO_BUILD_JOBS=8 cargo test -p arbitrage algorithms::perp_price_spread --no-fail-fast
CARGO_BUILD_JOBS=8 cargo test -p arbitrage algorithms::spot_cross --no-fail-fast
CARGO_BUILD_JOBS=8 cargo test -p arbitrage strategy_registry --no-fail-fast
CARGO_BUILD_JOBS=8 cargo test -p api --bin crypto-arb-api hedge_preview --no-fail-fast
CARGO_BUILD_JOBS=8 cargo test -p api --bin crypto-arb-api instrument_registry::coverage --no-fail-fast
CARGO_BUILD_JOBS=8 cargo test -p api --bin crypto-arb-api services::opportunity --no-fail-fast
CARGO_BUILD_JOBS=8 cargo test --manifest-path "$ROOT/frontend/Cargo.toml" p0_strategy --no-fail-fast
CARGO_BUILD_JOBS=8 cargo test --manifest-path "$ROOT/frontend/Cargo.toml" one_shot --no-fail-fast
CARGO_BUILD_JOBS=8 cargo test --manifest-path "$ROOT/frontend/Cargo.toml" preview --no-fail-fast
bash "$MARKET_VERIFY" --self-test
bash "$ROOT/scripts/check_product_audit_progress.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"
bash "$ROOT/scripts/check_product_audit_evidence_index.sh"

printf 'OK PR-GC price-spread arbitrage completion gate\n'
