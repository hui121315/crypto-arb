#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
POSTGRES_GATE="$ROOT/scripts/verify_postgres_ledger_contract.sh"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

if [[ "$MODE" == "--self-test" ]]; then
  PR_DM_SKIP_TESTS=1 bash "$0" >/dev/null
  backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-dm.XXXXXX")"
  cp "$POSTGRES_GATE" "$backup"
  restore() {
    cp "$backup" "$POSTGRES_GATE"
    chmod +x "$POSTGRES_GATE"
    rm -f "$backup"
  }
  trap restore EXIT
  perl -0pi -e 's/  --ignored/  --list/' "$POSTGRES_GATE"
  if PR_DM_SKIP_TESTS=1 bash "$0" >/dev/null 2>&1; then
    printf 'PR-DM completion self-test failed: skipped PostgreSQL contract passed\n' >&2
    exit 1
  fi
  printf 'PR-DM completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from __future__ import annotations

import csv
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])
title = "PR-DM Storage Health & Ledger Persistence Contract"
verify_anchor = "`bash scripts/check_pr_dm_completion.sh --self-test`"
evidence_contract = {
    "migration-source": "crates/trading/src/sql_ledger/migrations.rs",
    "normalized-ledger-schema": "crates/trading/migrations/20260601_orders.sql",
    "transactional-writer": "crates/trading/src/sql_ledger/writer.rs",
    "projection-jobs": "crates/trading/src/sql_ledger/projection_jobs.rs",
    "run-cost-facts": "crates/trading/src/sql_ledger/run_cost.rs",
    "run-cost-rebuild": "crates/trading/src/sql_ledger/run_cost_rebuild.rs",
    "funding-runtime": "crates/api/src/lifecycle/funding_payments.rs",
    "execution-unwind-cost": "crates/api/src/services/execution_runs/ledger_event.rs",
    "close-run-cost": "crates/api/src/services/close_run_costs.rs",
    "review-portfolio-consumer": "crates/api/src/services/portfolio_pnl.rs",
    "storage-runtime-health": "crates/api/src/services/venue_operation_health/snapshot/part_12.rs",
    "postgres-live-smoke": "crates/trading/tests/sql_ledger_postgres.rs",
    "postgres-self-starting-gate": "scripts/verify_postgres_ledger_contract.sh",
    "upstream-ledger-governance": "scripts/check_pr_fz_completion.sh",
    "completion-governance": "scripts/check_pr_dm_completion.sh",
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-DM completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be completed with no local remainder")
if row.count(verify_anchor) != 1:
    fail("roadmap verification must name the destructive completion gate")
queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-DM\b", queue):
    fail("completed PR-DM remains in the local queue")
for upstream in ("PR-C ExecutionRun Finality", "PR-E CloseRun", "PR-H Review Ledger", "PR-FZ Runtime Artifact"):
    upstream_row = next(
        (line for line in doc.splitlines() if line.startswith(f"| `{upstream}")),
        None,
    )
    if upstream_row is None or "✅ 完成" not in upstream_row:
        fail(f"upstream completed contract drifted: {upstream}")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(
    encoding="utf-8", newline=""
) as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-DM"]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for kind, artifact in evidence_contract.items():
    if indexed[kind]["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"{kind} artifact drifted or is missing")

markers = {
    "crates/trading/migrations/20260601_orders.sql": (
        "CREATE TABLE IF NOT EXISTS fills",
        "CREATE TABLE IF NOT EXISTS fees",
        "CREATE TABLE IF NOT EXISTS funding_payments",
        "CREATE TABLE IF NOT EXISTS slippage_events",
        "CREATE TABLE IF NOT EXISTS run_finality_events",
    ),
    "crates/trading/migrations/20260710_run_cost_sources.sql": (
        "run_cost_facts_exactly_one_source_check",
        "CREATE TABLE IF NOT EXISTS run_finality_source_links",
        "CREATE TABLE IF NOT EXISTS run_cost_rebuild_receipts",
    ),
    "crates/trading/src/sql_ledger/writer.rs": (
        "protocol::persist_event_group",
        "project_order_event_transaction",
    ),
    "crates/trading/src/sql_ledger/projection_jobs.rs": (
        "FOR UPDATE SKIP LOCKED",
        "claim_token",
    ),
    "crates/trading/src/sql_ledger/run_cost.rs": (
        "ExecutionLedgerPayload::Slippage",
        "ExecutionLedgerPayload::FundingPayment",
        "run_kind: \"close_run\"",
    ),
    "crates/trading/src/sql_ledger/run_cost_rebuild.rs": (
        "run_cost_rebuild_receipts",
        "order_high_water",
        "finality_high_water",
    ),
    "crates/api/src/services/execution_runs/ledger_event.rs": (
        "actual_unwind_slippage_usd",
        "actual_unwind_fee_usd",
        "actual_funding_usd",
    ),
    "crates/api/src/services/close_run_costs.rs": (
        "CloseRunCostReconciliation",
        "funding_event_ids",
        "compensation_slippage_event_ids",
    ),
    "crates/api/src/services/portfolio_pnl.rs": (
        "realized_pnl_by_group_with_close_runs",
    ),
    "crates/api/src/services/venue_operation_health/snapshot/part_12.rs": (
        "Trading SQL ledger writer/replay 可用",
        "trading_sql_ledger_storage_row",
    ),
    "crates/trading/tests/sql_ledger_postgres.rs": (
        "stored_normalized_cost_sources",
        "FROM slippage_events WHERE event_id = $1",
        "FROM funding_payments WHERE event_id = $1",
        "sql_ledger_postgres_roundtrip_restarts_from_committed_facts",
    ),
}
for path, required in markers.items():
    source = (root / path).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"missing {path} marker: {marker}")

postgres_gate = (root / "scripts/verify_postgres_ledger_contract.sh").read_text(
    encoding="utf-8"
)
for marker in (
    "mktemp -d /tmp/crossline-pr-dm-postgres.",
    "trap cleanup EXIT",
    "--test sql_ledger_postgres",
    "run_cost_facts_rebuild_execution_and_close_reconciliation",
    "startup_projection_worker_catches_pending_fill_once",
    "OK PR-DM disposable PostgreSQL ledger contract",
):
    if marker not in postgres_gate:
        fail(f"self-starting PostgreSQL gate marker drifted: {marker}")
if postgres_gate.count("  --ignored") != 3:
    fail("all three PostgreSQL cargo invocations must explicitly run ignored live tests")
if postgres_gate.count("  --test-threads=1") != 3:
    fail("all PostgreSQL live tests must run serially against the disposable database")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(
    encoding="utf-8", newline=""
) as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
for path in evidence_contract.values():
    if coverage.get(path, {}).get("coverage_status") != "exact":
        fail(f"exact coverage missing for {path}")

print(
    f"OK PR-DM contract ({len(evidence_contract)} evidence types; "
    "normalized funding/slippage + restart/rebuild + storage health closure)"
)
PY

bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"
bash "$ROOT/scripts/check_pr_c_completion.sh"
bash "$ROOT/scripts/check_pr_fz_completion.sh"

if [[ "${PR_DM_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test \
    --manifest-path "$ROOT/Cargo.toml" \
    -p trading \
    sql_ledger \
    --lib \
    --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test \
    --manifest-path "$ROOT/Cargo.toml" \
    -p api \
    --bin crypto-arb-api \
    execution_runs \
    --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test \
    --manifest-path "$ROOT/Cargo.toml" \
    -p api \
    --bin crypto-arb-api \
    close_run_costs \
    --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test \
    --manifest-path "$ROOT/Cargo.toml" \
    -p review \
    realized_pnl \
    --lib \
    --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" bash "$POSTGRES_GATE"
fi

printf 'OK PR-DM storage health and ledger persistence contract\n'
