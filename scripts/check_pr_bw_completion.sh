#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
RUN_STATE="$ROOT/crates/api/src/services/execution_runs/state.rs"
UNWIND="$ROOT/crates/api/src/services/execution_orchestrator/unwind.rs"
WS_PUBLISH="$ROOT/crates/api/src/services/ws_publish.rs"
STATUS_TESTS="$ROOT/frontend/src/panels/modules/execution/components/execution_status_bar/testing.rs"
ACTIONS="$ROOT/frontend/src/panels/modules/execution/data/actions.rs"
AUDIT_DOC="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-BW completion gate failed: %s\n' "$1" >&2
  exit 1
}

if [[ "$MODE" == "--self-test" ]]; then
  temp="$(mktemp -d "${TMPDIR:-/tmp}/crossline-pr-bw.XXXXXX")"
  cp "$RUN_STATE" "$temp/run-state.rs"
  cp "$UNWIND" "$temp/unwind.rs"
  cp "$WS_PUBLISH" "$temp/ws-publish.rs"
  cp "$STATUS_TESTS" "$temp/status-tests.rs"
  cp "$ACTIONS" "$temp/actions.rs"
  cp "$AUDIT_DOC" "$temp/audit.md"
  restore() {
    cp "$temp/run-state.rs" "$RUN_STATE"
    cp "$temp/unwind.rs" "$UNWIND"
    cp "$temp/ws-publish.rs" "$WS_PUBLISH"
    cp "$temp/status-tests.rs" "$STATUS_TESTS"
    cp "$temp/actions.rs" "$ACTIONS"
    cp "$temp/audit.md" "$AUDIT_DOC"
    rm -rf "$temp"
  }
  trap restore EXIT

  PR_BW_SKIP_TESTS=1 PR_BW_SKIP_UPSTREAM=1 bash "$0" >/dev/null

  python3 - "$RUN_STATE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "    if both_filled(run) {"
if source.count(marker) != 1:
    raise SystemExit("PR-BW self-test setup failed: terminal fill marker drifted")
path.write_text(source.replace(marker, "    if true {", 1), encoding="utf-8")
PY
  if PR_BW_SKIP_TESTS=1 PR_BW_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted ACK or partial state as Hedged"
  fi
  cp "$temp/run-state.rs" "$RUN_STATE"

  python3 - "$UNWIND" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "    run.unwind_problem = Some(problem.clone());"
if source.count(marker) != 1:
    raise SystemExit("PR-BW self-test setup failed: unwind problem marker drifted")
path.write_text(source.replace(marker, "    run.unwind_problem = None;", 1), encoding="utf-8")
PY
  if PR_BW_SKIP_TESTS=1 PR_BW_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted an erased unwind failure"
  fi
  cp "$temp/unwind.rs" "$UNWIND"

  python3 - "$WS_PUBLISH" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = ".publish(channels::EXECUTION, execution_run_message(event, run)?);"
if source.count(marker) != 1:
    raise SystemExit("PR-BW self-test setup failed: execution channel marker drifted")
path.write_text(
    source.replace(
        marker,
        ".publish(channels::ORDERS, execution_run_message(event, run)?);",
        1,
    ),
    encoding="utf-8",
)
PY
  if PR_BW_SKIP_TESTS=1 PR_BW_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted an ExecutionRun delta on the orders channel"
  fi
  cp "$temp/ws-publish.rs" "$WS_PUBLISH"

  python3 - "$STATUS_TESTS" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "fn notice_surfaces_partial_unwind_failed_and_closed_states() {"
if source.count(marker) != 1:
    raise SystemExit("PR-BW self-test setup failed: status matrix test marker drifted")
path.write_text(source.replace(marker, "#[ignore]\n" + marker, 1), encoding="utf-8")
PY
  if PR_BW_SKIP_TESTS=1 PR_BW_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a skipped partial/unwind/closed fixture"
  fi
  cp "$temp/status-tests.rs" "$STATUS_TESTS"

  python3 - "$ACTIONS" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "ActionState::Idle | ActionState::Accepted { .. } | ActionState::Succeeded { .. }"
if source.count(marker) != 1:
    raise SystemExit("PR-BW self-test setup failed: action restore marker drifted")
path.write_text(
    source.replace(marker, marker + " | ActionState::Failed { .. }", 1),
    encoding="utf-8",
)
PY
  if PR_BW_SKIP_TESTS=1 PR_BW_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted stale run snapshots overwriting a local confirm failure"
  fi
  cp "$temp/actions.rs" "$ACTIONS"

  python3 - "$AUDIT_DOC" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "### 🟡 6.5 下一步执行队列"
if source.count(marker) != 1:
    raise SystemExit("PR-BW self-test setup failed: queue heading drifted")
stale = "\n\n1. **PR-BY Frontend ApiProblem & LoadState Contract** — stale completed row"
path.write_text(
    source.replace(marker, marker + stale, 1),
    encoding="utf-8",
)
PY
  if PR_BW_SKIP_TESTS=1 PR_BW_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a completed PR-BY successor in the local queue"
  fi

  printf 'PR-BW completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from __future__ import annotations

import csv
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])
pr_id = "PR-BW"
title = "PR-BW ExecutionRun Finality & ActionState Snapshot"
verify_anchor = "`bash scripts/check_pr_bw_completion.sh --self-test`"
evidence_contract = {
    "shared-finality-snapshot": "shared-types/src/execution_run.rs",
    "shared-run-state": "shared-types/src/hedge.rs",
    "shared-confirm-partial-outcome": "shared-types/src/arbitrage.rs",
    "finality-watcher": "crates/api/src/services/run_finality/reconciliation/part_01.rs",
    "terminal-state-projector": "crates/api/src/services/execution_runs/state.rs",
    "typed-unwind-result": "crates/api/src/services/execution_orchestrator/unwind.rs",
    "exact-run-query": "crates/api/src/services/execution_runs/query.rs",
    "durable-run-ledger": "crates/api/src/services/execution_run_store.rs",
    "execution-ws-delta": "crates/api/src/services/ws_publish.rs",
    "execution-ws-replay": "crates/api/src/services/ws_replay.rs",
    "scoped-confirm-preflight": "crates/api/src/services/hedge_confirm/confirm_validate.rs",
    "scoped-balance-recheck": "crates/api/src/services/hedge_margin.rs",
    "frontend-run-recovery": "frontend/src/panels/modules/execution/data/run.rs",
    "frontend-action-state": "frontend/src/panels/modules/execution/data/actions.rs",
    "frontend-status-matrix": "frontend/src/panels/modules/execution/components/execution_status_bar/state.rs",
    "frontend-status-tests": "frontend/src/panels/modules/execution/components/execution_status_bar/testing.rs",
    "kucoin-finality-fee-fixture": "crates/exchange/src/adapters/kucoin_private_data_tests.rs",
    "gate-finality-fee-fixture": "crates/exchange/src/adapters/gate_fill_evidence_tests.rs",
    "htx-finality-fee-fixture": "crates/api/src/lifecycle/private_ws/tests/htx_finality.rs",
    "normalized-run-cost-facts": "crates/trading/src/sql_ledger/run_cost.rs",
    "review-ledger-consumer": "crates/api/src/services/review/ledger.rs",
    "portfolio-ledger-consumer": "crates/api/src/services/portfolio_pnl.rs",
    "upstream-finality-governance": "scripts/check_pr_ea_completion.sh",
    "completion-governance": "scripts/check_pr_bw_completion.sh",
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-BW completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(
    encoding="utf-8"
)
fact_lines = history.split("## 附录 I：", 1)[0].splitlines() + doc.splitlines()
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be complete with no local remainder")
if row.count(verify_anchor) != 1:
    fail("roadmap verification must name the destructive gate once")

queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-BW\b", queue):
    fail("completed PR-BW remains in the local queue")
successor_title = "PR-BY Frontend ApiProblem & LoadState Contract"
successor_row = next(
    (line for line in doc.splitlines() if line.startswith(f"| `{successor_title}`")),
    None,
)
if successor_row is None:
    fail("PR-BY successor roadmap row is missing")
successor_complete = "✅ 完成" in successor_row
successor_queued = re.search(r"(?m)^\d+\.\s+\*\*PR-BY\b", queue) is not None
successor_is_head = re.search(r"(?m)^1\.\s+\*\*PR-BY\b", queue) is not None
if successor_complete and successor_queued:
    fail("completed PR-BY successor remains in the local queue")
if not successor_complete and not successor_is_head:
    fail("unfinished PR-BY successor must remain the local queue head")

absorbed_findings = (
    "ExecutionRun 聚合层 finality projector 仍不完整",
    "8 家交易所终态/成交/资金费能力未进入统一 adapter",
    "对冲执行页缺 typed ActionState / LoadState / run 回放",
    "前端执行页 runtime 已有 REST seed 但缺精确回放",
    "private WS disconnect/circuit/parse health 不可查询",
    "Gate 单订单查询已减少 open-order-only 风险",
    "订单和 ExecutionRun 没有 snapshot replay",
    "执行页 ExecutionRun scoped runtime truth",
    "Private WS/reconciliation 已有基础 ExecutionRun 投影，缺完整 finality 证据",
    "ExecutionRun 与 OrderJournal 终态未形成单一事实源",
    "写单能力测试缺 8 家一致矩阵",
    "ExecutionRun 把 ACK/Accepted 当成 Hedged",
    "Unwind 失败被 `.ok()` 吞掉",
    "执行 preview 错误被前端显示为等待",
    "Live router order backfill 扫所有 route",
    "Accepted/Submitted 双腿会被标成 Hedged",
    "Execution confirm 错误被组件丢弃",
)
for finding_title in absorbed_findings:
    finding = next((line for line in reversed(fact_lines) if finding_title in line), None)
    if finding is None or "✅ 完成" not in finding:
        fail(f"absorbed finding remains incomplete: {finding_title}")

for audit_id in ("AUD-121", "AUD-222", "AUD-223", "AUD-224", "AUD-228", "AUD-229", "AUD-249", "AUD-257"):
    audit = next((line for line in reversed(fact_lines) if f"`{audit_id}`" in line), None)
    if audit is None or "✅ 完成" not in audit:
        fail(f"absorbed audit row remains incomplete: {audit_id}")

preview_finding = next(
    (line for line in reversed(fact_lines) if "对冲 preview / 机会详情 / symbol search 吞错" in line),
    None,
)
if preview_finding is None or "PR-BW" in preview_finding:
    fail("frontend load-state finding still delegates work to PR-BW")
if successor_complete and "✅ 完成" not in preview_finding:
    fail("completed PR-BY successor did not close the frontend load-state finding")
if not successor_complete and "🟡 部分完成" not in preview_finding:
    fail("unfinished PR-BY successor lost its remaining frontend load-state finding")

for upstream in ("PR-DH", "PR-DM", "PR-DF", "PR-DV", "PR-EA", "PR-EO", "PR-EP", "PR-ER", "PR-FD"):
    upstream_row = next((line for line in doc.splitlines() if line.startswith(f"| `{upstream} ")), None)
    if upstream_row is None or "✅ 完成" not in upstream_row:
        fail(f"required successor authority is not complete: {upstream}")

if "## 2026-07-15 PR-BW ExecutionRun Finality and ActionState Closure" not in history:
    fail("history closure appendix is missing")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == pr_id]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for evidence_type, artifact in evidence_contract.items():
    evidence = indexed[evidence_type]
    if evidence["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"{evidence_type} artifact drifted or is missing")
    if not evidence["command"].strip() or not evidence["notes"].strip():
        fail(f"{evidence_type} lacks command or notes")

markers = {
    "shared-types/src/execution_run.rs": (
        "pub struct ExecutionRunTimelineEvent",
        "pub finality_confidence: ExecutionFillConfidence",
        "pub last_reconciled_at_ms: Option<i64>",
    ),
    "shared-types/src/hedge.rs": (
        "pub enum ExecutionRunState",
        "FirstLegPartial",
        "pub unwind_problem: Option<ApiProblem>",
        "pub finality_problem: Option<ApiProblem>",
    ),
    "shared-types/src/arbitrage.rs": (
        "pub struct HedgeConfirmPartialOutcome",
        "pub unwind_status: HedgeConfirmUnwindStatus",
        "pub manual_review_required: bool",
    ),
    "crates/api/src/services/run_finality/reconciliation/part_01.rs": (
        "pub(crate) async fn refresh_pending_runs",
        ".refresh_order_state(&target.internal_order_id)",
        "handle_refresh_result(state, &target, result, outcome);",
    ),
    "crates/api/src/services/execution_runs/state.rs": (
        "if both_filled(run) {",
        "run.state = ExecutionRunState::Hedged;",
        "LiveOrderState::PartiallyFilled | LiveOrderState::Filled",
    ),
    "crates/api/src/services/execution_orchestrator/unwind.rs": (
        "pub(super) fn mark_unwind_result",
        "run.unwind_problem = Some(problem.clone());",
        "run.recovery_action = Some(RecoveryAction::ManualReview);",
    ),
    "crates/api/src/services/execution_runs/query.rs": (
        "pub(crate) fn recent_by_query",
        "optional_id_matches(&self.run_id, &run.run_id)",
        "optional_id_matches(&self.ticket_id, &run.ticket_id)",
    ),
    "crates/api/src/services/execution_run_store.rs": (
        "pub(crate) fn append_projected",
        "file.sync_data()?;",
        "replay_execution_runs",
    ),
    "crates/api/src/services/ws_publish.rs": (
        ".publish(channels::EXECUTION, execution_run_message(event, run)?);",
        'publish_execution_run_event(state, "execution_run_updated", &run)?;',
    ),
    "crates/api/src/services/ws_replay.rs": (
        "channel: channels::EXECUTION",
        'event: "execution_run_updated".to_owned()',
    ),
    "crates/api/src/services/hedge_confirm/confirm_validate.rs": (
        "validate_confirm_scoped_preflight",
        "recheck_hedge_live_order_preflight_guards",
        "live_operation_health_guard",
    ),
    "crates/api/src/services/hedge_margin.rs": (
        "pub(crate) async fn ensure_final_margin",
        ".refresh_scoped_balances(&venues)",
        "if !intents.iter().any(|intent| requires_live_margin(intent))",
    ),
    "frontend/src/panels/modules/execution/data/run.rs": (
        "execution_runs_for_context(",
        "start_execution_stream_with_state(",
        "apply_fallback_result(",
    ),
    "frontend/src/panels/modules/execution/data/actions.rs": (
        "if !allows_execution_run_restore(&state.get_untracked())",
        "const fn allows_execution_run_restore(state: &ActionState) -> bool",
        "ActionState::Idle | ActionState::Accepted { .. } | ActionState::Succeeded { .. }",
    ),
    "frontend/src/panels/modules/execution/components/action_bar/labels.rs": (
        'ExecutionRunState::SecondLegSubmitted => "第二腿已提交，等待成交确认"',
        'ExecutionRunState::UnwindRequired => "需要反向处理"',
        'ExecutionRunState::Closed => "已关闭"',
    ),
    "frontend/src/panels/modules/execution/components/execution_status_bar/state.rs": (
        "ExecutionRunState::FirstLegPartial => Some(format!(",
        "ExecutionRunState::Unwinding => Some(format!(",
        "ExecutionRunState::FailedSafe => Some(format!(",
        'ExecutionRunState::Closed => Some("双腿执行已关闭',
    ),
    "crates/api/src/services/review/ledger.rs": ("list_sql_realized_window",),
    "crates/api/src/services/portfolio_pnl.rs": ("realized_ledger_from_sql_window",),
}
for path, required in markers.items():
    source = (root / path).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"missing {path} marker: {marker}")

actions_source = (root / "frontend/src/panels/modules/execution/data/actions.rs").read_text(
    encoding="utf-8"
)
restore_policy = re.search(
    r"const fn allows_execution_run_restore\(state: &ActionState\) -> bool \{(?P<body>.*?)\n\}",
    actions_source,
    flags=re.DOTALL,
)
if restore_policy is None or "ActionState::Failed" in restore_policy.group("body"):
    fail("local confirm failure must not be restorable from a stale run snapshot")

test_anchors = {
    "crates/api/src/services/execution_orchestrator/tests/unwind.rs": (
        "unwind_failure_keeps_run_visible_for_manual_review",
        "unwind_failure_problem_preserves_request_id_retry_after",
    ),
    "crates/api/src/services/execution_runs/tests/cases_a.rs": (
        "execution_run_query_filters_by_exact_context",
        "order_update_projects_fills_into_run",
        "private_ws_fill_event_projects_into_run_leg_finality",
    ),
    "crates/api/src/services/run_finality/tests.rs": ("terminal_order_state_only_skips_final_states",),
    "crates/api/src/services/ws_replay/tests.rs": ("execution_channel_replays_recent_runs",),
    "crates/api/src/routers/trading/tests/cases_list.rs": ("list_execution_runs_filters_exact_context",),
    "frontend/src/panels/modules/execution/components/execution_status_bar/testing.rs": (
        "notice_surfaces_partial_unwind_failed_and_closed_states",
        "hedged_label_requires_both_legs_filled",
    ),
    "frontend/src/panels/modules/execution/data/actions_tests.rs": (
        "local_confirm_failure_is_not_overwritten_by_a_stale_run_snapshot",
    ),
    "crates/exchange/src/adapters/kucoin_private_data_tests.rs": (
        "kucoin_fills_parse_official_fixture_without_defaulting_fee",
    ),
    "crates/exchange/src/adapters/gate_fill_evidence_tests.rs": (
        "parses_official_my_trades_fixture_without_combining_fee_units",
    ),
    "crates/api/src/lifecycle/private_ws/tests/htx_finality.rs": (
        "htx_terminal_fill_projects_execution_run_once_after_durable_ack",
    ),
}
for path, anchors in test_anchors.items():
    source = (root / path).read_text(encoding="utf-8")
    if "#[ignore]" in source:
        fail(f"target fixture contains an ignored test: {path}")
    for anchor in anchors:
        if not re.search(rf"(?m)^\s*(?:async\s+)?fn\s+{re.escape(anchor)}\s*\(", source):
            fail(f"missing runnable test anchor: {path}:{anchor}")

verify_source = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
if verify_source.count("check_pr_bw_completion.sh") != 2:
    fail("repo docs/full gate wiring drifted")
predecessor = (root / "scripts/check_pr_bv_completion.sh").read_text(encoding="utf-8")
for marker in ("successor_title = \"PR-BW ExecutionRun Finality & ActionState Snapshot\"", "successor_complete", "successor_is_head"):
    if marker not in predecessor:
        fail(f"PR-BV predecessor is not successor-aware: {marker}")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
for artifact in evidence_contract.values():
    if coverage.get(artifact, {}).get("coverage_status") != "exact":
        fail(f"exact coverage missing for {artifact}")

print(
    f"OK PR-BW static contract ({len(evidence_contract)} evidence rows; "
    "typed finality/unwind, scoped replay, three venue fixtures and SQL-first consumers)"
)
PY

bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"

if [[ "${PR_BW_SKIP_UPSTREAM:-0}" != "1" ]]; then
  PR_BV_SKIP_TESTS=1 PR_BV_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_bv_completion.sh"
  PR_DV_SKIP_TESTS=1 PR_DV_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_dv_completion.sh"
  PR_EA_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_ea_completion.sh"
  PR_DF_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_df_completion.sh"
  bash "$ROOT/scripts/check_pr_fd_completion.sh"
  PR_DM_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dm_completion.sh"
  bash "$ROOT/scripts/check_pr_eo_completion.sh"
  bash "$ROOT/scripts/check_pr_ep_completion.sh"
  bash "$ROOT/scripts/check_pr_er_completion.sh"
fi

if [[ "${PR_BW_SKIP_TESTS:-0}" != "1" ]]; then
  jobs="${CARGO_BUILD_JOBS:-10}"
  CARGO_BUILD_JOBS="$jobs" cargo test -p shared-types execution_run --lib --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p shared-types hedge_confirm --lib --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api execution_orchestrator --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api execution_runs --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api run_finality --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api list_execution_runs_filters_exact_context --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api execution_channel_replays_recent_runs --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --lib kucoin_fills_parse_official_fixture_without_defaulting_fee --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --lib parses_official_my_trades_fixture_without_combining_fee_units --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api htx_terminal_fill_projects_execution_run_once_after_durable_ack --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p trading non_usd_funding_keeps_native_amount_without_fabricated_usd --lib --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib execution_status_bar --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib actions --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib action_bar --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib execution_run --no-fail-fast
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-ea -- --workers=1
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-dv -- --workers=1
fi

printf 'PR-BW ExecutionRun Finality and ActionState completion gate passed\n'
