#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
SOURCE="$ROOT/shared-types/src/profitability.rs"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

if [[ "$MODE" == "--self-test" ]]; then
  PR_DD_SKIP_TESTS=1 bash "$0" >/dev/null
  backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-dd.XXXXXX")"
  cp "$SOURCE" "$backup"
  restore() {
    cp "$backup" "$SOURCE"
    rm -f "$backup"
  }
  trap restore EXIT
  python3 - "$SOURCE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = 'pub const PROFITABILITY_EVIDENCE_SOURCE: &str = "fee_schedule_registry+funding_history";'
if marker not in source:
    raise SystemExit("PR-DD self-test setup failed: source marker missing")
path.write_text(
    source.replace(
        marker,
        'pub const PROFITABILITY_EVIDENCE_SOURCE: &str = "drifted_profitability_source";',
        1,
    ),
    encoding="utf-8",
)
PY
  if PR_DD_SKIP_TESTS=1 bash "$0" >/dev/null 2>&1; then
    printf 'PR-DD completion self-test failed: drifted source registry passed\n' >&2
    exit 1
  fi
  printf 'PR-DD completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from __future__ import annotations

import csv
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])
title = "PR-DD Profitability Score Evidence & Opportunity Count Contract"
verify_anchor = "`bash scripts/check_pr_dd_completion.sh --self-test`"
evidence_contract = {
    "profitability-source-registry": "shared-types/src/profitability.rs",
    "execution-readiness": "shared-types/src/arbitrage.rs",
    "scanner-profitability": "crates/arbitrage/src/calculator.rs",
    "scanner-contract": "crates/arbitrage/src/engine_v3.rs",
    "list-projection": "crates/api/src/services/opportunity/row.rs",
    "list-contract-tests": "crates/api/src/services/opportunity/tests/list.rs",
    "ticket-cost": "crates/api/src/services/hedge_ticket/cost.rs",
    "ticket-contract-tests": "crates/api/src/services/hedge_ticket/tests/evidence.rs",
    "observation-only-browser": "test/e2e/pr_ee_profitability_evidence.spec.ts",
    "completion-governance": "scripts/check_pr_dd_completion.sh",
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-DD completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be completed with no local remainder")
if row.count(verify_anchor) != 1:
    fail("roadmap verification must name the destructive completion gate")
queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-DD\b", queue):
    fail("completed PR-DD remains in the local queue")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-DD"]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for kind, artifact in evidence_contract.items():
    if indexed[kind]["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"{kind} artifact drifted or is missing")

markers = {
    "shared-types/src/profitability.rs": (
        'pub const PROFITABILITY_EVIDENCE_SOURCE: &str = "fee_schedule_registry+funding_history";',
    ),
    "crates/arbitrage/src/calculator.rs": (
        "ProfitabilityEvidence::from_fee_snapshots(",
        "PROFITABILITY_EVIDENCE_SOURCE,",
    ),
    "crates/api/src/services/hedge_ticket/cost.rs": (
        "ticket_profitability_evidence",
        "shared_types::PROFITABILITY_EVIDENCE_SOURCE,",
    ),
    "crates/api/src/services/opportunity/row.rs": (
        "is_execution_ready(row)",
        "profitability_evidence",
    ),
}
for path, required in markers.items():
    source = (root / path).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"missing {path} marker: {marker}")

browser = (root / evidence_contract["observation-only-browser"]).read_text(encoding="utf-8")
if re.search(r"\btest\.(?:skip|fixme)\s*\(", browser):
    fail("browser fixture must not be skipped")
for marker in ("成本未验证", 'name: "观察"', 'name: "构建对冲"'):
    if marker not in browser:
        fail(f"browser fixture missing observation-only marker: {marker}")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
for path in evidence_contract.values():
    if coverage.get(path, {}).get("coverage_status") != "exact":
        fail(f"exact coverage missing for {path}")

print(f"OK PR-DD contract ({len(evidence_contract)} evidence types; shared profitability source)")
PY

bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"

if [[ "${PR_DD_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p shared-types --lib profitability_evidence --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p arbitrage --lib engine_attaches_perp_cross_side_prices --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api --bin crypto-arb-api list_row_fails_closed_when_typed_profitability_evidence_is_missing --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api --bin crypto-arb-api fee_snapshots_recompute_round_trip_cost --no-fail-fast
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-ee
fi

printf 'OK PR-DD scanner, list and HedgeTicket profitability evidence contract\n'
