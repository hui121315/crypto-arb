#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
TERMINAL_CONTRACT="$ROOT/shared-types/src/execution_ledger.rs"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

if [[ "$MODE" == "--self-test" ]]; then
  PR_DH_SKIP_TESTS=1 bash "$0" >/dev/null
  backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-dh.XXXXXX")"
  cp "$TERMINAL_CONTRACT" "$backup"
  restore() {
    cp "$backup" "$TERMINAL_CONTRACT"
    rm -f "$backup"
  }
  trap restore EXIT
  python3 - "$TERMINAL_CONTRACT" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "pub const fn supports_terminal_fill(self) -> bool"
if marker not in source:
    raise SystemExit("PR-DH self-test setup failed: terminal fill marker missing")
path.write_text(source.replace(marker, "pub const fn drifted_terminal_fill(self) -> bool", 1), encoding="utf-8")
PY
  if PR_DH_SKIP_TESTS=1 bash "$0" >/dev/null 2>&1; then
    printf 'PR-DH completion self-test failed: drifted terminal contract passed\n' >&2
    exit 1
  fi
  printf 'PR-DH completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from __future__ import annotations

import csv
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])
title = "PR-DH Review Ledger Truth Source & Estimated PnL Contract"
verify_anchor = "`bash scripts/check_pr_dh_completion.sh --self-test`"
evidence_contract = {
    "terminal-fill-contract": "shared-types/src/execution_ledger.rs",
    "review-pnl-quality-contract": "shared-types/src/review.rs",
    "review-evidence-contract": "shared-types/src/review/evidence.rs",
    "terminal-ledger-projection": "crates/review/src/realized_pnl/events.rs",
    "private-ws-terminal-integration": "crates/api/src/trading_service/private_ws_events/tests/pnl.rs",
    "pnl-quality-classifier": "crates/review/src/realized_pnl.rs",
    "sql-first-review-reader": "crates/api/src/services/review/ledger.rs",
    "typed-storage-degradation": "crates/api/src/services/review/storage_health.rs",
    "sql-storage-health": "crates/api/src/services/review/storage_health/sql.rs",
    "snapshot-cursor-budget": "crates/api/src/services/review/paging.rs",
    "review-page-client": "frontend/src/api/rest/portfolio_system/envelope.rs",
    "server-cursor-ui": "frontend/src/panels/modules/pagination/list.rs",
    "review-pnl-consumer": "frontend/src/panels/modules/review/components/executed_tab.rs",
    "review-browser-contract": "test/e2e/pr_dh_review_ledger.spec.ts",
    "wasm-release-regression": "scripts/check_wasm_budget.sh",
    "runtime-envelope-contract": "scripts/verify_runtime_contracts.sh",
    "completion-governance": "scripts/check_pr_dh_completion.sh",
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-DH completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be completed with no local remainder")
if row.count(verify_anchor) != 1:
    fail("roadmap verification must name the destructive completion gate")
queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-DH\b", queue):
    fail("completed PR-DH remains in the local queue")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-DH"]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for kind, artifact in evidence_contract.items():
    if indexed[kind]["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"{kind} artifact drifted or is missing")

markers = {
    "shared-types/src/execution_ledger.rs": (
        "pub const fn supports_terminal_fill(self) -> bool",
        "Self::VenueFill | Self::VenueOrderSnapshot | Self::OrderQuery",
    ),
    "shared-types/src/review.rs": (
        "pub actual_fields: Vec<ReviewPnlField>",
        "pub estimated_fields: Vec<ReviewPnlField>",
        "pub missing_fields: Vec<ReviewPnlField>",
    ),
    "shared-types/src/review/evidence.rs": (
        "pub estimated_slippage_fill_event_ids: Vec<String>",
        "pub fn has_complete_slippage_evidence(&self) -> bool",
    ),
    "crates/review/src/realized_pnl/events.rs": (
        "snapshot.confidence.supports_terminal_fill()",
        "ExecutionLedgerQuality::Missing",
    ),
    "crates/review/src/realized_pnl/group.rs": (
        "order.state == shared_types::LiveOrderState::Filled",
        "estimated_slippage_fill_event_ids",
    ),
    "crates/review/src/realized_pnl.rs": (
        "trade.actual_fields = fields.actual",
        "trade.estimated_fields = fields.estimated",
        "trade.missing_fields = fields.missing",
    ),
    "crates/api/src/trading_service/private_ws_events/tests/pnl.rs": (
        "mark_all_seeded_orders_filled",
        "private_fill_delta_flows_to_review_pnl_without_cumulative_double_count",
        "private_fill_delta_flows_to_portfolio_today_pnl",
        "private_funding_delta_flows_to_review_and_portfolio_pnl",
    ),
    "crates/api/src/services/review/ledger.rs": (
        "list_sql_realized_window(from_ms, to_ms).await",
        "ReviewLedgerProblemContext",
    ),
    "crates/api/src/services/review/storage_health.rs": (
        "with_review_storage_health",
        "envelope.status = ListStatus::Degraded",
        "envelope.problems.push(problem)",
    ),
    "crates/api/src/services/review/storage_health/sql.rs": (
        "sql_ledger_storage_health",
        "TRADING_SQL_LEDGER_UNAVAILABLE",
        "run_finality_append_failures",
    ),
    "crates/api/src/services/review/paging.rs": (
        "currentSnapshotId",
        'format!("rv1:{offset}:{snapshot_id}")',
        "last_cursor:",
        "review_window_days",
    ),
    "frontend/src/api/rest/portfolio_system/envelope.rs": (
        "const REVIEW_PAGE_LIMIT: usize = 50",
        "encode_query_component(value)",
    ),
    "frontend/src/panels/modules/pagination/list.rs": (
        "page.previous_cursor.as_ref()",
        "page.last_cursor.as_ref()",
    ),
    "frontend/src/panels/modules/review/components/executed_tab.rs": (
        'data-table-budget="server-page"',
        "row.actual_fields.contains(&field)",
        'badge: "缺证据"',
    ),
    "scripts/check_wasm_budget.sh": (
        "WASM_RAW_BUDGET_BYTES:-5100000",
        "WASM_BUDGET_BYTES:-1850000",
        "WASM_OZ_BUDGET_BYTES:-5100000",
    ),
    "scripts/verify_runtime_contracts.sh": (
        "SYSTEM_HEALTH_MAX_BYTES:-69632",
        '(.data|has("api")',
        '(.data.problems|type=="array")',
    ),
}
for path, required in markers.items():
    source = (root / path).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"missing {path} marker: {marker}")

browser = (root / evidence_contract["review-browser-contract"]).read_text(encoding="utf-8")
if re.search(r"\btest\.(?:skip|fixme)\s*\(", browser):
    fail("browser fixture must not be skipped")
for marker in (
    "PR-DH review ledger keeps explicit PnL quality and snapshot-bound 50-row budget",
    "const TOTAL_ROWS = 1_000",
    "const PAGE_LIMIT = 50",
    'cursor(950)',
    'hasText: "真实"',
    'hasText: "估算"',
    'hasText: "缺证据"',
):
    if marker not in browser:
        fail(f"browser fixture missing marker: {marker}")

package = (root / "package.json").read_text(encoding="utf-8")
if '"test:e2e:pr-dh"' not in package or package.count("pr_dh_review_ledger.spec.ts") < 2:
    fail("package script or product suite wiring is missing")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
for path in evidence_contract.values():
    if coverage.get(path, {}).get("coverage_status") != "exact":
        fail(f"exact coverage missing for {path}")

print(f"OK PR-DH contract ({len(evidence_contract)} evidence types; terminal ledger, PnL quality, snapshot paging)")
PY

bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"

if [[ "${PR_DH_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p shared-types execution_ledger --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p review --lib --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api --bin crypto-arb-api trading_service::private_ws_events::tests::pnl --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api --bin crypto-arb-api services::review --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib executed_tab --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib pagination::list --no-fail-fast
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-dh -- --workers=1
fi

printf 'OK PR-DH review ledger truth, PnL quality, and snapshot pagination contract\n'
