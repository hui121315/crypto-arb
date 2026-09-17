#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
DETAIL="$ROOT/frontend/src/panels/modules/review/components/executed_ledger_detail.rs"
REALIZED_EVENTS="$ROOT/crates/review/src/realized_pnl/events.rs"
FUNDING_ROUTER="$ROOT/crates/api/src/trading_service/live_adapters/router/funding_payments.rs"
POSTGRES_GATE="$ROOT/scripts/verify_postgres_ledger_contract.sh"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-BL completion gate failed: %s\n' "$1" >&2
  exit 1
}

if [[ "$MODE" == "--self-test" ]]; then
  detail_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-bl-detail.XXXXXX")"
  events_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-bl-events.XXXXXX")"
  router_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-bl-router.XXXXXX")"
  postgres_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-bl-postgres.XXXXXX")"
  cp "$DETAIL" "$detail_backup"
  cp "$REALIZED_EVENTS" "$events_backup"
  cp "$FUNDING_ROUTER" "$router_backup"
  cp "$POSTGRES_GATE" "$postgres_backup"
  restore() {
    cp "$detail_backup" "$DETAIL"
    cp "$events_backup" "$REALIZED_EVENTS"
    cp "$router_backup" "$FUNDING_ROUTER"
    cp "$postgres_backup" "$POSTGRES_GATE"
    chmod +x "$POSTGRES_GATE"
    rm -f "$detail_backup" "$events_backup" "$router_backup" "$postgres_backup"
  }
  trap restore EXIT

  PR_BL_SKIP_TESTS=1 PR_BL_SKIP_UPSTREAM=1 bash "$0" >/dev/null

  python3 - "$DETAIL" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = 'event.order.run_id.as_deref().unwrap_or("-")'
if source.count(marker) != 1:
    raise SystemExit("PR-BL self-test setup failed: run identity marker drifted")
path.write_text(source.replace(marker, 'Some("hidden").unwrap_or("-")', 1), encoding="utf-8")
PY
  if PR_BL_SKIP_TESTS=1 PR_BL_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted Review detail without ledger run identity"
  fi
  cp "$detail_backup" "$DETAIL"

  python3 - "$REALIZED_EVENTS" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "&& snapshot.confidence.supports_terminal_fill()"
if source.count(marker) != 1:
    raise SystemExit("PR-BL self-test setup failed: terminal confidence marker drifted")
path.write_text(source.replace(marker, "", 1), encoding="utf-8")
PY
  if PR_BL_SKIP_TESTS=1 PR_BL_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted non-terminal fill evidence"
  fi
  cp "$events_backup" "$REALIZED_EVENTS"

  python3 - "$FUNDING_ROUTER" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "let mut tasks = FuturesUnordered::new();"
if source.count(marker) != 1:
    raise SystemExit("PR-BL self-test setup failed: funding fanout marker drifted")
path.write_text(source.replace(marker, "let mut tasks = Vec::new();", 1), encoding="utf-8")
PY
  if PR_BL_SKIP_TESTS=1 PR_BL_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted funding ingestion without bounded route fanout"
  fi
  cp "$router_backup" "$FUNDING_ROUTER"

  python3 - "$POSTGRES_GATE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "  --ignored"
if source.count(marker) != 3:
    raise SystemExit("PR-BL self-test setup failed: PostgreSQL live invocations drifted")
path.write_text(source.replace(marker, "  --list", 1), encoding="utf-8")
PY
  if PR_BL_SKIP_TESTS=1 PR_BL_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a skipped durable-ledger acceptance"
  fi

  printf 'PR-BL completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from __future__ import annotations

import csv
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])
verify_anchor = "`bash scripts/check_pr_bl_completion.sh --self-test`"
roadmap_titles = (
    "PR-BL Review PnL Evidence & Estimated Field Contract",
    "PR-CH Review Ledger & PnL Evidence Contract",
)
evidence_contract = {
    "PR-BL": {
        "current-pnl-evidence": "scripts/check_pr_dw_completion.sh",
        "terminal-fill-contract": "shared-types/src/execution_ledger.rs",
        "ledger-identity-contract": "shared-types/src/review/evidence.rs",
        "review-field-quality-contract": "shared-types/src/review.rs",
        "realized-pnl-projection": "crates/review/src/realized_pnl/events.rs",
        "close-run-cost-projection": "crates/review/src/realized_pnl/close_run_costs.rs",
        "sql-first-review": "crates/api/src/services/review/ledger.rs",
        "shared-portfolio-consumer": "crates/api/src/services/portfolio_pnl.rs",
        "cross-venue-funding-lifecycle": "crates/api/src/lifecycle/funding_payments.rs",
        "cross-venue-funding-router": "crates/api/src/trading_service/live_adapters/router/funding_payments.rs",
        "durable-run-cost-facts": "crates/trading/src/sql_ledger/run_cost.rs",
        "review-request-correlation": "crates/api/src/routers/review.rs",
        "review-paging": "crates/api/src/services/review/paging.rs",
        "review-storage-health": "crates/api/src/services/review/storage_health.rs",
        "frontend-field-quality": "frontend/src/panels/modules/review/components/executed_tab.rs",
        "frontend-ledger-identity": "frontend/src/panels/modules/review/components/executed_ledger_detail.rs",
        "product-browser": "test/e2e/pr_dw_review_runtime.spec.ts",
        "postgres-live-contract": "scripts/verify_postgres_ledger_contract.sh",
        "terminal-fill-authority": "scripts/check_pr_dh_completion.sh",
        "runtime-evidence-authority": "scripts/check_pr_dw_completion.sh",
        "durable-ledger-authority": "scripts/check_pr_dm_completion.sh",
        "funding-route-authority": "scripts/check_pr_fz_completion.sh",
        "close-run-authority": "scripts/check_pr_dz_completion.sh",
        "execution-finality-authority": "scripts/check_pr_ea_completion.sh",
        "completion-governance": "scripts/check_pr_bl_completion.sh",
    },
    "PR-CH": {
        "current-review-ledger": "scripts/check_pr_dw_completion.sh",
        "terminal-fill-authority": "scripts/check_pr_dh_completion.sh",
        "funding-route-authority": "scripts/check_pr_fz_completion.sh",
        "durable-ledger-authority": "scripts/check_pr_dm_completion.sh",
        "runtime-evidence-authority": "scripts/check_pr_dw_completion.sh",
        "finality-authority": "scripts/check_pr_ea_completion.sh",
        "close-run-authority": "scripts/check_pr_dz_completion.sh",
        "review-projection": "crates/review/src/realized_pnl.rs",
        "frontend-evidence": "frontend/src/panels/modules/review/components/executed_evidence.rs",
        "product-browser": "test/e2e/pr_dw_review_runtime.spec.ts",
        "completion-governance": "scripts/check_pr_bl_completion.sh",
    },
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-BL completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(
    encoding="utf-8"
)
audit_sources = (doc, history)
for title in roadmap_titles:
    row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
    if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
        fail(f"roadmap row must be complete with no local remainder: {title}")
    if row.count(verify_anchor) != 1:
        fail(f"roadmap verification must name the destructive gate once: {title}")

queue = doc[doc.index("### 🟡 6.5"):]
for pr_id in ("PR-BL", "PR-CH"):
    if re.search(rf"(?m)^\d+\.\s+\*\*{pr_id}\b", queue):
        fail(f"completed {pr_id} remains in the local queue")

finding_titles = (
    "复盘模块 / 执行账本 / 策略绩效口径",
    "复盘模块 / PnL 账本 / 策略绩效口径二次深挖",
    "历史存储 / Funding 历史 / Metrics / 审计日志",
    "复盘/历史收益事实源、PnL 与策略绩效口径",
    "Review PnL / portfolio PnL / fee-funding-slippage evidence",
    "历史存储 / SQL migration / runtime artifact 边界",
    "Review runtime evidence / VenueQuality / 前端复盘 LoadState 复扫",
    "复盘/执行账本事实源、PnL、费用、Funding 与策略绩效口径复扫",
    "复盘/Review、Portfolio PnL、VenueQuality 与前端复盘运行时复扫",
    "交易事实源可证明成交/费用/funding 终态",
    "复盘 PnL ledger evidence 完整闭环",
    "Funding/Slippage evidence contract",
    "ExecutionRun/CloseRun actual cost 已闭环",
    "Review PnL 使用完整 ledger resolver",
    "Review DTO/UI 字段与运行证据完整",
    "Portfolio PnL funding payment ledger 已闭环",
)
for title in finding_titles:
    completed_findings = [
        line
        for source in audit_sources
        for line in source.splitlines()
        if title in line and "✅" in line
    ]
    if not completed_findings:
        fail(f"absorbed Review finding is missing or remains incomplete: {title}")

shared_resolver = [
    line
    for source in audit_sources
    for line in source.splitlines()
    if "Review 与 Portfolio PnL 共用" in line and "✅" in line
]
if len(shared_resolver) < 2:
    fail("all duplicated Review/Portfolio resolver findings must be complete")

readiness = next(
    (
        line
        for source in audit_sources
        for line in source.splitlines()
        if line.startswith("| 复盘 | ✅")
    ),
    None,
)
if readiness is None:
    fail("Review product-readiness row remains incomplete")

upstream_titles = (
    "PR-FZ Runtime Artifact, Storage Path & Data Hygiene Contract",
    "PR-DH Review Ledger Truth Source & Estimated PnL Contract",
    "PR-DM Storage Health & Ledger Persistence Contract",
    "PR-DW Review Runtime Evidence & VenueQuality LoadState Contract",
    "PR-DZ Portfolio AccountState & CloseRun Contract",
    "PR-EA ExecutionRun Finality & ActionState Contract",
)
for title in upstream_titles:
    row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
    if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
        fail(f"completed upstream authority drifted: {title}")

if "## 2026-07-15 PR-BL and PR-CH Review PnL Evidence Closure" not in history:
    fail("history closure appendix is missing")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    all_rows = list(csv.DictReader(handle, delimiter="\t"))
for pr_id, expected in evidence_contract.items():
    rows = [row for row in all_rows if row["pr_id"] == pr_id]
    indexed = {row["evidence_type"]: row for row in rows}
    if len(indexed) != len(rows) or set(indexed) != set(expected):
        fail(
            f"{pr_id} evidence type drift: "
            f"expected={sorted(expected)}, actual={sorted(indexed)}"
        )
    for evidence_type, artifact in expected.items():
        row = indexed[evidence_type]
        if row["artifact"] != artifact or not (root / artifact).is_file():
            fail(f"{pr_id} {evidence_type} artifact drifted or is missing")
        if not row["command"].strip() or not row["notes"].strip():
            fail(f"{pr_id} {evidence_type} lacks command or notes")

markers = {
    "shared-types/src/execution_ledger.rs": (
        "pub const fn supports_terminal_fill(self) -> bool",
        "Self::VenueFill | Self::VenueOrderSnapshot | Self::OrderQuery",
    ),
    "shared-types/src/review.rs": (
        "pub actual_fields: Vec<ReviewPnlField>",
        "pub estimated_fields: Vec<ReviewPnlField>",
        "pub missing_fields: Vec<ReviewPnlField>",
        "pub request_id: Option<String>",
        "pub storage_health: Option<VenueOperationHealth>",
    ),
    "shared-types/src/review/evidence.rs": (
        "pub source: OrderUpdateSource",
        "pub run_id: Option<String>",
        "pub ticket_id: Option<String>",
        "pub close_run_evidence: Vec<ReviewCloseRunEvidence>",
    ),
    "crates/review/src/realized_pnl/events.rs": (
        "snapshot.confidence.supports_terminal_fill()",
        "snapshot.quality != shared_types::ExecutionLedgerQuality::Missing",
    ),
    "crates/review/src/realized_pnl.rs": (
        "trade.actual_fields = fields.actual",
        "trade.estimated_fields = fields.estimated",
        "trade.missing_fields = fields.missing",
    ),
    "crates/review/src/realized_pnl/close_run_costs.rs": (
        "close_run_reconciled_cost_delta",
        "cost.funding_event_ids.as_slice()",
        "cost.manual_handling_event_ids.as_slice()",
        "close_run_cost_missing",
    ),
    "crates/api/src/services/review/ledger.rs": (
        "list_sql_realized_window(from_ms, to_ms).await",
        "ReviewLedgerProblemContext",
    ),
    "crates/api/src/services/portfolio_pnl.rs": (
        "list_sql_realized_window(from_ms, to_ms).await",
        "realized_pnl_by_group_with_close_runs",
    ),
    "crates/api/src/lifecycle/funding_payments.rs": (
        "ingest_configured_private_funding_payments",
        "persist_then_publish_ledger_projected_runs",
        "MissedTickBehavior::Skip",
    ),
    "crates/api/src/trading_service/live_adapters/router/funding_payments.rs": (
        "let mut tasks = FuturesUnordered::new();",
        "collect_funding_payment_result",
        'self.failures.record("funding_payments", failures)',
    ),
    "crates/trading/src/sql_ledger/run_cost.rs": (
        "INSERT INTO run_cost_facts",
        'component: "slippage"',
        'component: "funding"',
        'run_kind: "close_run"',
    ),
    "crates/api/src/services/review/paging.rs": (
        'format!("rv1:{offset}:{snapshot_id}")',
        "review_window_days",
        "last_cursor:",
    ),
    "crates/api/src/services/review/storage_health.rs": (
        "with_review_storage_health",
        "envelope.status = ListStatus::Degraded",
    ),
    "frontend/src/panels/modules/review/components/executed_tab.rs": (
        'data-table-budget="server-page"',
        "row.actual_fields.contains(&field)",
        'badge: "缺证据"',
    ),
    "frontend/src/panels/modules/review/components/executed_evidence.rs": (
        'format!("终态 {filled}/{} Filled via {sources}", orders.len())',
        'format!("真实 {actual} · 估算 {estimated} · 缺证据 {missing}")',
    ),
    "frontend/src/panels/modules/review/components/executed_ledger_detail.rs": (
        'event.order.run_id.as_deref().unwrap_or("-")',
        'event.order.ticket_id.as_deref().unwrap_or("-")',
        "order_update_source_label(event.source)",
    ),
}
for path, required in markers.items():
    source = (root / path).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"missing {path} marker: {marker}")

review_router = (root / "crates/api/src/routers/review.rs").read_text(encoding="utf-8")
if review_router.count(".with_request_id(common::request_id::current())") != 3:
    fail("all Review envelopes must carry body request correlation")

browser = (root / "test/e2e/pr_dw_review_runtime.spec.ts").read_text(encoding="utf-8")
if re.search(r"\btest\.(?:skip|fixme)\s*\(", browser):
    fail("Review runtime browser fixture must not be skipped")
for marker in (
    "PR-DW keeps durable review, funding, close, unwind, storage, and body request evidence visible",
    "req-review-body-pr-dw",
    "事件 fill:2 fee:2 funding:1 slip:2 book:2",
    "close-pr-dw compensated run:run-pr-dw ticket:ticket-pr-dw",
):
    if marker not in browser:
        fail(f"Review runtime browser marker drifted: {marker}")

postgres_gate = (root / "scripts/verify_postgres_ledger_contract.sh").read_text(encoding="utf-8")
if postgres_gate.count("  --ignored") != 3 or postgres_gate.count("  --test-threads=1") != 3:
    fail("all three durable-ledger PostgreSQL acceptances must run serially and non-skipping")
if "  --list" in postgres_gate:
    fail("durable-ledger PostgreSQL acceptance became list-only")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
for expected in evidence_contract.values():
    for artifact in expected.values():
        if coverage.get(artifact, {}).get("coverage_status") != "exact":
            fail(f"exact coverage missing for {artifact}")

print(
    "OK PR-BL/PR-CH static contract "
    f"({sum(len(items) for items in evidence_contract.values())} evidence rows; "
    "terminal fill, cross-venue costs, durable ledger, runtime and product evidence)"
)
PY

bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"

if [[ "${PR_BL_SKIP_UPSTREAM:-0}" != "1" ]]; then
  bash "$ROOT/scripts/check_pr_fz_completion.sh"
  PR_DH_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dh_completion.sh"
  PR_DM_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dm_completion.sh"
  PR_DW_SKIP_TESTS=1 PR_DW_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_dw_completion.sh"
  PR_DZ_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dz_completion.sh"
  PR_EA_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_ea_completion.sh"
fi

if [[ "${PR_BL_SKIP_TESTS:-0}" != "1" ]]; then
  jobs="${CARGO_BUILD_JOBS:-10}"
  CARGO_BUILD_JOBS="$jobs" cargo test -p shared-types review --lib --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p review realized_pnl --lib --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange funding_payments --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api funding_payments --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api services::review --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api portfolio_pnl --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib review --no-fail-fast
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-dw -- --workers=1
  CARGO_BUILD_JOBS="$jobs" bash "$POSTGRES_GATE"
fi

printf 'PR-BL and PR-CH Review PnL evidence completion gate passed\n'
