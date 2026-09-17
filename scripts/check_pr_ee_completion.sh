#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
FIXTURE="$ROOT/crates/arbitrage/fixtures/fee_schedule_registry_v2.json"
FEE_REGISTRY="$ROOT/crates/arbitrage/src/algorithms/fee_evidence.rs"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

if [[ "$MODE" == "--self-test" ]]; then
  PR_EE_SKIP_TESTS=1 bash "$0" >/dev/null
  backup_dir="$(mktemp -d "${TMPDIR:-/tmp}/crossline-pr-ee.XXXXXX")"
  cp "$FIXTURE" "$backup_dir/fee_schedule_registry_v2.json"
  cp "$FEE_REGISTRY" "$backup_dir/fee_evidence.rs"
  restore() {
    cp "$backup_dir/fee_schedule_registry_v2.json" "$FIXTURE"
    cp "$backup_dir/fee_evidence.rs" "$FEE_REGISTRY"
    rm -rf "$backup_dir"
  }
  trap restore EXIT

  perl -0pi -e \
    's/static STANDARD_FEE_REGISTRY: OnceLock</static STANDARD_FEE_REGISTRY: OnceLock<Option</' \
    "$FEE_REGISTRY"
  if PR_EE_SKIP_TESTS=1 bash "$0" >/dev/null 2>&1; then
    printf 'PR-EE completion self-test failed: Option registry erased initialization failure\n' >&2
    exit 1
  fi
  cp "$backup_dir/fee_evidence.rs" "$FEE_REGISTRY"

  python3 - "$FIXTURE" <<'PY'
from pathlib import Path
import json
import sys

path = Path(sys.argv[1])
payload = json.loads(path.read_text(encoding="utf-8"))
fingerprint = payload.get("schema", {}).get("fingerprint")
if not isinstance(fingerprint, str) or len(fingerprint) != 16:
    raise SystemExit("PR-EE self-test setup failed: registry fingerprint missing")
payload["schema"]["fingerprint"] = "0000000000000000"
path.write_text(json.dumps(payload, ensure_ascii=True, indent=2) + "\n", encoding="utf-8")
PY
  if PR_EE_SKIP_TESTS=1 bash "$0" >/dev/null 2>&1; then
    printf 'PR-EE completion self-test failed: corrupted registry fingerprint passed\n' >&2
    exit 1
  fi
  printf 'PR-EE completion self-test passed\n'
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
title = "PR-EE Profitability Evidence & Fee/Funding Source Contract"
verify_anchor = "`bash scripts/check_pr_ee_completion.sh --self-test`"
evidence_contract = {
    "shared-profitability-contract": "shared-types/src/fees.rs",
    "shared-funding-history-contract": "shared-types/src/funding.rs",
    "fee-registry-fixture": "crates/arbitrage/fixtures/fee_schedule_registry_v2.json",
    "fee-registry-consumer": "crates/arbitrage/src/algorithms/fee_evidence.rs",
    "funding-history-producer": "crates/realtime/src/history/funding_stats.rs",
    "scanner-profitability-producer": "crates/arbitrage/src/calculator.rs",
    "opportunity-list-fail-closed": "crates/api/src/services/opportunity/row.rs",
    "hedge-ticket-cost-evidence": "crates/api/src/services/hedge_ticket/cost.rs",
    "frontend-cost-consumer": "frontend/src/panels/modules/cost_profile.rs",
    "frontend-preview-consumer": "frontend/src/panels/modules/execution/data/preview/model.rs",
    "fee-registry-settings": "frontend/src/panels/modules/settings/tabs/venue_credentials/fees.rs",
    "profitability-browser": "test/e2e/pr_ee_profitability_evidence.spec.ts",
    "completion-governance": "scripts/check_pr_ee_completion.sh",
}
coverage_paths = tuple(path for path in evidence_contract.values() if not path.endswith(".json"))


def fail(message: str) -> None:
    raise SystemExit(f"PR-EE completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be completed with no local remainder")
if row.count(verify_anchor) != 1:
    fail("roadmap verification must name the destructive completion gate")
queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-EE\b", queue):
    fail("completed PR-EE remains in the local queue")
pr_bv_row = next(
    (
        line
        for line in doc.splitlines()
        if line.startswith("| `PR-BV Ranking Contract & Score Explanation`")
    ),
    None,
)
pr_bv_complete = pr_bv_row is not None and "✅ 完成" in pr_bv_row

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-EE"]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for kind, artifact in evidence_contract.items():
    if indexed[kind]["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"{kind} artifact drifted or is missing")

markers = {
    "shared-types/src/fees.rs": (
        "pub struct FeeScheduleEvidence",
        "pub struct ProfitabilityEvidence",
        "pub fn is_cost_verified",
        "PROFITABILITY_FEE_EVIDENCE_MISSING",
    ),
    "shared-types/src/funding.rs": (
        "pub struct FundingHistoryEvidence",
        "pub fn is_usable",
        "pub evidence: FundingHistoryEvidence",
    ),
    "crates/arbitrage/src/algorithms/fee_evidence.rs": (
        "include_str!(\"../../fixtures/fee_schedule_registry_v2.json\")",
        "static STANDARD_FEE_REGISTRY: OnceLock<\n    Result<FeeScheduleRegistryResponse, FeeScheduleRegistryError>",
        "standard_fee_registry_is_valid",
    ),
    "crates/realtime/src/history/funding_stats.rs": (
        "evidence: FundingHistoryEvidence",
        "sample_health",
        "problem_detail",
    ),
    "crates/arbitrage/src/calculator.rs": (
        "PROFITABILITY_EVIDENCE_SOURCE",
        "ProfitabilityEvidence::from_fee_snapshots",
    ),
    "crates/api/src/services/opportunity/row.rs": (
        "evidence.is_cost_verified()",
        "fee_evidence_complete.then_some",
    ),
    "crates/api/src/services/hedge_ticket/cost.rs": (
        "ticket_profitability_evidence",
        "PROFITABILITY_EVIDENCE_SOURCE",
    ),
    "frontend/src/panels/modules/cost_profile.rs": (
        "profitability_evidence.is_cost_verified()",
        "verified_fee_snapshot_count",
    ),
    "frontend/src/panels/modules/execution/data/preview/model.rs": (
        "profitability_status",
        "funding_history_sample_count",
    ),
    "frontend/src/panels/modules/settings/tabs/venue_credentials/fees.rs": (
        "Fee schedule fixture 注册表",
        "registry_fingerprint",
    ),
}
for path, required in markers.items():
    source = (root / path).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"missing {path} marker: {marker}")

registry = json.loads((root / evidence_contract["fee-registry-fixture"]).read_text(encoding="utf-8"))
if registry.get("schema", {}).get("version") != "fee_schedule_registry_v2":
    fail("fee registry schema version drifted")
if len(registry.get("venues", [])) != 8:
    fail("fee registry must cover eight venue families")
expected_schedule_count = 16 if pr_bv_complete else 10
schedule_count = sum(len(venue.get("schedules", [])) for venue in registry["venues"])
if schedule_count != expected_schedule_count:
    fail(
        "fee registry schedule count drifted: "
        f"expected={expected_schedule_count}, actual={schedule_count}"
    )

fingerprint = 0xCBF29CE484222325
product_names = {"spot": "Spot", "perp": "Perp", "margin": "Margin", "unknown": "Unknown"}
for venue in registry["venues"]:
    for schedule in venue["schedules"]:
        evidence = schedule["evidence"]
        fields = (
            venue["venue"], product_names[schedule["product"]],
            f'{schedule["makerFeeBps"]:.4f}', f'{schedule["takerFeeBps"]:.4f}',
            evidence.get("scheduleVersion", ""), evidence["evidenceId"],
            evidence["sourceName"], evidence["sourceUrl"], str(evidence["checkedAtMs"]),
            str(evidence.get("effectiveAtMs") or 0), evidence.get("tier", ""),
            evidence.get("scope", ""), evidence.get("problem", "") or "",
            schedule["fixtureId"], schedule["fixtureSymbol"], str(schedule["snapshotTtlMs"]),
        )
        line = ":".join(fields) + ";"
        for byte in line.encode():
            fingerprint ^= byte
            fingerprint = (fingerprint * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
expected = f"{fingerprint:016x}"
if registry["schema"].get("fingerprint") != expected:
    fail(f"fee registry fingerprint mismatch: expected={expected}")

fee_source = (root / "crates/arbitrage/src/algorithms/fee_evidence.rs").read_text(encoding="utf-8")
if re.search(r'https://(?:www\.)?(?:binance|okx|bybit|bitget|gate|htx|kucoin|hyperliquid)', fee_source):
    fail("official fee URLs must live in the registry fixture, not Rust constants")

browser = (root / evidence_contract["profitability-browser"]).read_text(encoding="utf-8")
if re.search(r"\btest\.(?:skip|fixme)\s*\(", browser):
    fail("browser fixture must not be skipped")
for marker in ("fee_schedule_registry_v2", "成本未验证", "构建对冲", "feeEvidenceComplete"):
    if marker not in browser:
        fail(f"browser fixture missing profitability evidence marker: {marker}")

package = (root / "package.json").read_text(encoding="utf-8")
if '"test:e2e:pr-ee"' not in package or "pr_ee_profitability_evidence.spec.ts" not in package:
    fail("package script or product suite wiring is missing")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
for path in coverage_paths:
    if coverage.get(path, {}).get("coverage_status") != "exact":
        fail(f"exact coverage missing for {path}")

print(
    f"OK PR-EE contract ({len(evidence_contract)} evidence types; "
    f"8 venues; {expected_schedule_count} schedules)"
)
PY

bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"

if [[ "${PR_EE_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p shared-types --lib fees::tests --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p realtime --lib funding_stats --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p arbitrage --lib fee_evidence --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p arbitrage --lib engine_attaches_perp_cross_side_prices --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api --bin crypto-arb-api profitability --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api --bin crypto-arb-api fee_snapshots_recompute_round_trip_cost --no-fail-fast
  cargo test --manifest-path "$ROOT/frontend/Cargo.toml" risk_preview --no-fail-fast
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-ee
fi

printf 'OK PR-EE profitability, fee registry and funding history evidence contract\n'
