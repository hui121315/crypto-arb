#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
AGGREGATOR="$ROOT/crates/exchange/src/aggregator.rs"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

if [[ "$MODE" == "--self-test" ]]; then
  PR_EV_SKIP_TESTS=1 bash "$0" >/dev/null
  backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-ev.XXXXXX")"
  cp "$AGGREGATOR" "$backup"
  restore() {
    cp "$backup" "$AGGREGATOR"
    rm -f "$backup"
  }
  trap restore EXIT
  python3 - "$AGGREGATOR" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "self.succeeded() && self.rows > 0"
if marker not in source:
    raise SystemExit("PR-EV self-test setup failed: coverage marker missing")
path.write_text(source.replace(marker, "self.succeeded()", 1), encoding="utf-8")
PY
  if PR_EV_SKIP_TESTS=1 bash "$0" >/dev/null 2>&1; then
    printf 'PR-EV completion self-test failed: empty success passed coverage gate\n' >&2
    exit 1
  fi
  printf 'PR-EV completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from __future__ import annotations

import csv
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])
title = "PR-EV Exchange HTTP Telemetry, Error Context & Fanout Envelope"
verify_anchor = "`bash scripts/check_pr_ev_completion.sh --self-test`"
evidence_contract = {
    "normalized-fanout-envelope": "crates/exchange/src/aggregator.rs",
    "typed-fanout-runtime-context": "crates/api/src/services/market_data/cache/runtime.rs",
    "symbol-scoped-market-problem": "crates/api/src/services/market_data/cache/orderbook_ops.rs",
    "http-operation-problem-details": "crates/api/src/services/venue_operation_health/snapshot/part_16.rs",
    "operation-finality-health": "crates/api/src/services/venue_operation_health/snapshot/part_06.rs",
    "http-rtt-status-consumer": "frontend/src/panels/status_bar/slots/operation.rs",
    "execution-problem-consumer": "frontend/src/panels/modules/market_evidence.rs",
    "shared-exchange-problem": "shared-types/src/problem.rs",
    "execution-problem-browser": "test/e2e/pr_ev_exchange_problem.spec.ts",
    "completion-governance": "scripts/check_pr_ev_completion.sh",
}
coverage_paths = (
    "shared-types/src/problem.rs",
    "crates/exchange/src/aggregator.rs",
    "crates/exchange/tests/aggregator_multi_test.rs",
    "crates/api/src/data_source.rs",
    "crates/api/src/lifecycle/funding.rs",
    "crates/api/src/services/market_data/cache/fetch.rs",
    "crates/api/src/services/market_data/cache/orderbook_ops.rs",
    "crates/api/src/services/market_data/cache/runtime.rs",
    "crates/api/src/services/market_data/cache/tests/cases_2.rs",
    "crates/api/src/services/market_data/cache/tests/cases_4.rs",
    "crates/api/src/services/venue_operation_health/snapshot/part_06.rs",
    "crates/api/src/services/venue_operation_health/snapshot/part_16.rs",
    "crates/api/src/services/venue_operation_health/snapshot/tests/part_04.rs",
    "crates/api/src/services/venue_operation_health/snapshot/tests/part_05.rs",
    "frontend/src/panels/status_bar/slots/operation.rs",
    "frontend/src/panels/modules/market_evidence.rs",
    "test/e2e/pr_ev_exchange_problem.spec.ts",
    "scripts/check_pr_ev_completion.sh",
    "scripts/check_release_qa_contract.sh",
    "scripts/verify_repo_gates.sh",
)


def fail(message: str) -> None:
    raise SystemExit(f"PR-EV completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be completed with no local remainder")
if row.count(verify_anchor) != 1:
    fail("roadmap verification must name the destructive completion gate")
queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-EV\b", queue):
    fail("completed PR-EV remains in the local queue")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-EV"]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for kind, artifact in evidence_contract.items():
    if indexed[kind]["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"{kind} artifact drifted or is missing")

markers = {
    "crates/exchange/src/aggregator.rs": (
        "pub operation: &'static str",
        "self.succeeded() && self.rows > 0",
        "pub fn coverage(&self) -> MarketDataCoverage",
        ".with_latency_ms(Some(latency_ms))",
        "successful_empty_venue_remains_uncovered",
    ),
    "crates/api/src/services/market_data/cache/runtime.rs": (
        "record_runtime_symbol_error",
        "problem.latency_ms.or(Some(outcome.latency_ms))",
    ),
    "crates/api/src/services/venue_operation_health/snapshot/part_16.rs": (
        '"operation": format!("http_rest:{} {}", snapshot.method, snapshot.path)',
        '"symbol": request_context_value(&snapshot.last_request_context, "symbol")',
        '"lastLatencyMs": snapshot.last_latency_ms',
    ),
    "crates/api/src/services/venue_operation_health/snapshot/part_06.rs": (
        "operation: OP_ORDER_FINALITY.to_owned()",
        "run_finality_runtime_evidence",
    ),
    "frontend/src/panels/status_bar/slots/operation.rs": (
        '"HTTP RTT"',
        "row.latency_ms",
    ),
    "frontend/src/panels/modules/market_evidence.rs": (
        "problem_context_label",
        'format!("HTTP耗时 {latency_ms}ms")',
        "exposes_structured_exchange_problem_context_for_execution_evidence",
    ),
}
for path, required in markers.items():
    source = (root / path).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"missing {path} marker: {marker}")

aggregator = (root / "crates/exchange/src/aggregator.rs").read_text(encoding="utf-8")
if "pub fn coverage_pct(" in aggregator or "pub fn into_rows(" in aggregator:
    fail("legacy coverage or naked Vec wrapper returned")

browser = (root / "test/e2e/pr_ev_exchange_problem.spec.ts").read_text(encoding="utf-8")
if re.search(r"\btest\.(?:skip|fixme)\s*\(", browser):
    fail("browser fixture must not be skipped")
for marker in ("rest_orderbooks", "HTTP耗时 35ms", "请求 req-pr-ev"):
    if marker not in browser:
        fail(f"browser fixture missing structured context: {marker}")

package = (root / "package.json").read_text(encoding="utf-8")
if '"test:e2e:pr-ev"' not in package or "pr_ev_exchange_problem.spec.ts" not in package:
    fail("package script or product suite wiring is missing")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
for path in coverage_paths:
    if coverage.get(path, {}).get("coverage_status") != "exact":
        fail(f"exact coverage missing for {path}")

print(f"OK PR-EV contract ({len(evidence_contract)} evidence types; {len(coverage_paths)} exact paths)")
PY

bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"

if [[ "${PR_EV_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p exchange aggregator --lib --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api runtime_health_records_fanout_venue_outcomes --bin crypto-arb-api --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api orderbook_fetch_error_records_per_venue_runtime_health --bin crypto-arb-api --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api http_outcome_rows_use_latest_endpoint_outcome --bin crypto-arb-api --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api run_finality_failure_maps_to_problem_and_evidence --bin crypto-arb-api --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" market_evidence --no-fail-fast
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-ev
fi

printf 'OK PR-EV HTTP telemetry, fanout envelope and execution evidence contract\n'
