#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
SIZING_CONTRACT="$ROOT/shared-types/src/execution_sizing.rs"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

if [[ "$MODE" == "--self-test" ]]; then
  PR_EB_SKIP_TESTS=1 bash "$0" >/dev/null
  backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-eb.XXXXXX")"
  cp "$SIZING_CONTRACT" "$backup"
  restore() {
    cp "$backup" "$SIZING_CONTRACT"
    rm -f "$backup"
  }
  trap restore EXIT
  python3 - "$SIZING_CONTRACT" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "pub type OrderSizingPlan = ExecutionSizingPlan;"
if marker not in source:
    raise SystemExit("PR-EB self-test setup failed: sizing alias missing")
path.write_text(source.replace(marker, "pub type DriftedSizingPlan = ExecutionSizingPlan;", 1), encoding="utf-8")
PY
  if PR_EB_SKIP_TESTS=1 bash "$0" >/dev/null 2>&1; then
    printf 'PR-EB completion self-test failed: drifted sizing contract passed\n' >&2
    exit 1
  fi
  printf 'PR-EB completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from __future__ import annotations

import csv
import json
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])
title = "PR-EB Venue InstrumentSpec & Order Sizing Contract"
verify_anchor = "`bash scripts/check_pr_eb_completion.sh --self-test`"
evidence_contract = {
    "shared-instrument-spec": "shared-types/src/instrument_registry.rs",
    "shared-order-sizing-contract": "shared-types/src/execution_sizing.rs",
    "ticket-compile-binding": "shared-types/src/hedge.rs",
    "eight-venue-instrument-interface": "crates/exchange/src/adapter.rs",
    "eight-venue-instrument-fixtures": "crates/exchange/src/adapters/kucoin_instruments_tests.rs",
    "registry-lifecycle": "crates/api/src/lifecycle/instruments.rs",
    "registry-fail-closed": "crates/api/src/services/instrument_registry.rs",
    "preview-contract-binding": "crates/api/src/services/hedge_preview/guards/runtime.rs",
    "live-confirm-revalidation": "crates/api/src/services/hedge_confirm/confirm_validate.rs",
    "submission-native-compile": "crates/api/src/trading_service/submit/submission_contract.rs",
    "frontend-live-gate": "frontend/src/panels/modules/execution/data/preview/response.rs",
    "frontend-product-evidence": "frontend/src/panels/modules/execution/components/risk_preview/evidence/order_plan.rs",
    "product-browser": "test/e2e/pr_eb_instrument_sizing.spec.ts",
    "completion-governance": "scripts/check_pr_eb_completion.sh",
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-EB completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be completed with no local remainder")
if row.count(verify_anchor) != 1:
    fail("roadmap verification must name the destructive completion gate")
queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-EB\b", queue):
    fail("completed PR-EB remains in the local queue")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-EB"]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for kind, artifact in evidence_contract.items():
    if indexed[kind]["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"{kind} artifact drifted or is missing")

markers = {
    "shared-types/src/instrument_registry.rs": (
        "pub type InstrumentSpec = VenueInstrument;",
        "pub fn is_hedge_constructible(&self) -> bool",
        "pub native_symbol: String",
        "pub contract_size: Option<f64>",
        "pub price_tick: Option<f64>",
        "pub qty_step: Option<f64>",
    ),
    "shared-types/src/execution_sizing.rs": (
        "pub type OrderSizingPlan = ExecutionSizingPlan;",
        "pub fn validate_order_sizing_contract(",
        "pub raw_contracts: f64",
        "pub rounded_base_qty: f64",
        "pub rounding_loss_bps: f64",
        "OrderSizingContractError::PlanMismatch",
    ),
    "shared-types/src/hedge.rs": (
        "pub instrument_spec: Option<InstrumentSpec>",
        "pub sizing_plan: Option<OrderSizingPlan>",
        "pub fn validate_sizing_contract(&self)",
        "pub fn submission_context(&self)",
    ),
    "crates/api/src/lifecycle/instruments.rs": (
        "from_secs(15 * 60)",
        "MissedTickBehavior::Skip",
        "refresh_all(&aggregator, &registry).await",
        "adapter.fetch_instruments().await",
        "registry.replace_venue(venue, instruments)",
    ),
    "crates/api/src/services/instrument_registry.rs": (
        "pub(crate) const SUPPORTED_VENUES",
        "pub(crate) fn supports_venue(&self, venue: &str)",
        "pub(crate) fn resolve_hedge_instrument(",
        "pub(crate) fn plan_leg_sizing(",
    ),
    "crates/api/src/services/hedge_preview/guards/runtime.rs": (
        "fn attach_instrument_contract(",
        'push_plan_blocker(plan, "INSTRUMENT_SPEC_MISSING")',
        "live_preview_instrument_contract_is_fail_closed_then_attached",
    ),
    "crates/api/src/services/hedge_confirm/confirm_validate.rs": (
        "fn validate_live_sizing_contracts(",
        "plan.validate_sizing_contract()",
    ),
    "crates/api/src/services/hedge_confirm/confirm_validate/tests.rs": (
        "live_confirm_requires_ticket_bound_instrument_sizing_contracts",
    ),
    "crates/api/src/trading_service/submit/submission_contract.rs": (
        "pub(super) fn compile_submission_intent(",
        "intent.symbol.clone_from(&instrument.native_symbol)",
        "intent.quantity = sizing.rounded_base_qty",
        "ticket_contract_rejects_tampered_sizing",
    ),
    "frontend/src/panels/modules/execution/data/preview/response.rs": (
        "fn order_sizing_blockers(",
        "plan.validate_sizing_contract()",
        "shared_types::ExecutionMode::Live",
    ),
    "frontend/src/panels/modules/execution/components/risk_preview/evidence/order_plan.rs": (
        "提交 {}：数量 {} / {} 张",
        "价格刻度 {}",
        "rounding_loss_bps",
        "contract_status",
    ),
}
for path, required in markers.items():
    source = (root / path).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"missing {path} marker: {marker}")

adapter_files = (
    "binance.rs",
    "okx.rs",
    "bybit.rs",
    "bitget.rs",
    "gate.rs",
    "kucoin.rs",
    "htx.rs",
    "hyperliquid.rs",
)
for name in adapter_files:
    source = (root / "crates/exchange/src/adapters" / name).read_text(encoding="utf-8")
    if "async fn fetch_instruments" not in source:
        fail(f"{name} does not implement fetch_instruments")

fixture_anchors = {
    "crates/exchange/src/adapters/binance_exchange_info_tests.rs": "registry_projection_uses_compiled_usdt_and_usdc_specs",
    "crates/exchange/src/adapters/okx_instruments_tests.rs": "into_venue_instrument_maps_min_qty_from_min_sz",
    "crates/exchange/src/adapters/bybit_instruments_tests.rs": "into_venue_instrument_maps_base_coin_sizing",
    "crates/exchange/src/adapters/bitget_instruments_tests.rs": "into_venue_instrument_maps_base_coin_sizing",
    "crates/exchange/src/adapters/gate_instruments_tests.rs": "into_venue_instrument_maps_quanto_and_min_qty",
    "crates/exchange/src/adapters/kucoin_instruments_tests.rs": "official_matrix_maps_usdt_usdc_and_verified_equity_contracts",
    "crates/exchange/src/adapters/htx_instruments_tests.rs": "into_venue_instrument_maps_quanto_and_min_qty",
    "crates/exchange/src/adapters/hyperliquid_instruments_tests.rs": "into_venue_instrument_derives_ticks_from_sz_decimals",
}
for path, anchor in fixture_anchors.items():
    source = (root / path).read_text(encoding="utf-8")
    match = re.search(rf"#\[(?:tokio::)?test\]\s*fn\s+{re.escape(anchor)}\b", source)
    if match is None:
        fail(f"non-skipping fixture anchor missing: {path}:{anchor}")
    prefix = source[max(0, match.start() - 160):match.start()]
    if "#[ignore" in prefix or "should_panic" in prefix:
        fail(f"fixture anchor is skipped or panic-expected: {path}:{anchor}")

browser = (root / evidence_contract["product-browser"]).read_text(encoding="utf-8")
if re.search(r"\btest\.(?:skip|fixme)\s*\(", browser):
    fail("browser fixture must not be skipped")
for marker in (
    "PR-EB renders ticket-bound native symbol, sizing and rounding evidence",
    "PR-EB live preview disables submit when instrument sizing evidence is absent",
    'toContainText("提交 XBTUSDCM：数量 1.128 / 1128 张")',
    'toHaveAttribute("title", /舍入损耗 3.9648 基点/)',
    'toBeDisabled()',
):
    if marker not in browser:
        fail(f"browser fixture missing marker: {marker}")

package = json.loads((root / "package.json").read_text(encoding="utf-8"))
scripts = package.get("scripts", {})
if scripts.get("test:e2e:pr-eb") != "playwright test test/e2e/pr_eb_instrument_sizing.spec.ts":
    fail("dedicated browser script is missing")
if scripts.get("test:e2e:product", "").count("pr_eb_instrument_sizing.spec.ts") != 1:
    fail("product suite wiring is missing or duplicated")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
coverage_paths = set(evidence_contract.values()) | {
    "crates/api/src/services/hedge_confirm/confirm_validate/tests.rs"
}
for path in coverage_paths:
    if coverage.get(path, {}).get("coverage_status") != "exact":
        fail(f"exact coverage missing for {path}")

print(
    f"OK PR-EB contract ({len(evidence_contract)} evidence types; "
    "8 venue fixtures; preview-confirm-submit-browser closure)"
)
PY

bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"

if [[ "${PR_EB_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p shared-types execution_sizing --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p exchange --lib instrument --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api instrument_registry --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api instrument_contract --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api submission_contract --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib instrument_sizing --no-fail-fast
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-eb -- --workers=1
fi

printf 'OK PR-EB venue instrument specification and order sizing contract\n'
