#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
FEE_REGISTRY="$ROOT/crates/arbitrage/src/algorithms/fee_evidence.rs"
COUNTDOWN_CONTRACT="$ROOT/shared-types/src/arbitrage.rs"
SQL_LEDGER="$ROOT/crates/trading/src/sql_ledger.rs"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-BP completion gate failed: %s\n' "$1" >&2
  exit 1
}

if [[ "$MODE" == "--self-test" ]]; then
  backup_dir="$(mktemp -d "${TMPDIR:-/tmp}/crossline-pr-bp.XXXXXX")"
  cp "$FEE_REGISTRY" "$backup_dir/fee_evidence.rs"
  cp "$COUNTDOWN_CONTRACT" "$backup_dir/arbitrage.rs"
  cp "$SQL_LEDGER" "$backup_dir/sql_ledger.rs"
  restore() {
    cp "$backup_dir/fee_evidence.rs" "$FEE_REGISTRY"
    cp "$backup_dir/arbitrage.rs" "$COUNTDOWN_CONTRACT"
    cp "$backup_dir/sql_ledger.rs" "$SQL_LEDGER"
    rm -rf "$backup_dir"
  }
  trap restore EXIT

  PR_BP_SKIP_TESTS=1 PR_BP_SKIP_UPSTREAM=1 bash "$0" >/dev/null

  perl -0pi -e \
    's/static STANDARD_FEE_REGISTRY: OnceLock</static STANDARD_FEE_REGISTRY: OnceLock<Option</' \
    "$FEE_REGISTRY"
  if PR_BP_SKIP_TESTS=1 PR_BP_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a fee registry that no longer retains initialization failure"
  fi
  cp "$backup_dir/fee_evidence.rs" "$FEE_REGISTRY"

  perl -0pi -e \
    's/pub settlement_countdown_seconds: Option<i64>/pub settlement_countdown_seconds: i64/' \
    "$COUNTDOWN_CONTRACT"
  if PR_BP_SKIP_TESTS=1 PR_BP_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a settlement countdown that fabricates missing evidence"
  fi
  cp "$backup_dir/arbitrage.rs" "$COUNTDOWN_CONTRACT"

  perl -0pi -e \
    's/collect_realized_run_finality_events\(finality_events\)\?/finality_events.filter_map(Result::ok).collect()/' \
    "$SQL_LEDGER"
  if PR_BP_SKIP_TESTS=1 PR_BP_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted per-row SQL finality decode failure erasure"
  fi

  printf 'PR-BP completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from __future__ import annotations

import csv
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])
title = "PR-BP Panic/Allow Debt Gate & User-Reachable Result Hygiene"
verify_anchor = "`bash scripts/check_pr_bp_completion.sh --self-test`"
evidence = {
    "review-evidence": "crates/review/src/executed.rs",
    "exchange-http": "crates/exchange/src/http.rs",
    "execution-preview": "frontend/src/panels/modules/execution/data/preview/build.rs",
    "ws-runtime": "crates/exchange/src/ws/manager.rs",
    "frontend-apr-evidence": "frontend/src/panels/modules/opportunity_view_model/model.rs",
    "fee-registry-result": "crates/arbitrage/src/algorithms/fee_evidence.rs",
    "fee-registry-http": "crates/api/src/routers/trading.rs",
    "fee-registry-http-test": "crates/api/src/routers/trading/tests/cases_fee_registry.rs",
    "capability-matrix-result": "crates/api/src/services/hedge_preview/guards.rs",
    "opportunity-current-index": "crates/api/src/services/opportunity_index.rs",
    "ticket-plan-timeline": "crates/api/src/services/execution_runs/timeline.rs",
    "ticket-plan-submit": "crates/api/src/services/execution_orchestrator.rs",
    "ticket-plan-submission-context": "crates/api/src/services/execution_orchestrator/submission.rs",
    "confirm-context-preservation": "crates/api/src/services/execution_orchestrator/context.rs",
    "confirm-partial-outcome": "crates/api/src/services/hedge_confirm/confirm.rs",
    "retry-after-clock": "frontend/src/api/rest/retry_after.rs",
    "funding-journal-result": "crates/trading/src/journal/projection/part_04.rs",
    "keychain-result": "crates/api/src/services/venue_credentials/keychain.rs",
    "credential-storage-health": "crates/api/src/services/venue_credentials/storage.rs",
    "credential-storage-backend-health": "crates/api/src/services/venue_credentials/storage/health.rs",
    "account-mode-result": "crates/api/src/services/hedge_preflight/account_mode.rs",
    "mutation-result-boundary": "crates/api/src/routers/trading/account.rs",
    "settlement-countdown-contract": "shared-types/src/arbitrage.rs",
    "settlement-countdown-product": "frontend/src/panels/modules/opportunity_view_model/format.rs",
    "action-run-decode": "frontend/src/panels/modules/settings/tabs/action_runs/hedge_confirm.rs",
    "sql-finality-result": "crates/trading/src/sql_ledger.rs",
    "hedge-timing-evidence": "crates/api/src/services/hedge_ticket/quote.rs",
    "completion-governance": "scripts/check_pr_bp_completion.sh",
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-BP completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be completed with no local remainder")
if row.count(verify_anchor) != 1:
    fail("roadmap verification must name the destructive completion gate exactly once")
queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-BP\b", queue):
    fail("completed PR-BP remains in the local queue")

history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(encoding="utf-8")
if "## 2026-07-15 PR-BP Panic/Allow Debt Gate & User-Reachable Result Hygiene Closure" not in history:
    fail("history closure appendix is missing")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-BP"]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence):
    fail(f"evidence type drift: expected={sorted(evidence)}, actual={sorted(indexed)}")
for kind, artifact in evidence.items():
    if indexed[kind]["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"{kind} artifact drifted or is missing")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
for artifact in evidence.values():
    if coverage.get(artifact, {}).get("coverage_status") != "exact":
        fail(f"exact coverage missing for {artifact}")

markers = {
    "crates/arbitrage/src/algorithms/fee_evidence.rs": (
        "static STANDARD_FEE_REGISTRY: OnceLock<\n    Result<FeeScheduleRegistryResponse, FeeScheduleRegistryError>",
        "pub enum FeeScheduleRegistryError",
        ".get_or_init(|| load_standard_fee_registry(STANDARD_FEE_REGISTRY_FIXTURE))",
    ),
    "crates/api/src/routers/trading.rs": (
        "codes::FEE_SCHEDULE_REGISTRY_UNAVAILABLE",
        "fee_schedule_registry_error",
    ),
    "crates/api/src/routers/trading/tests/cases_fee_registry.rs": (
        "fee_schedule_registry_failure_maps_to_typed_service_unavailable",
    ),
    "crates/api/src/services/hedge_preview/guards.rs": (
        ") -> Result<(), AppError>",
        "trading.exchange_capability_matrix(&plan.exchange)?",
        "attach_order_capability_options(\n        input.state",
    ),
    "crates/api/src/services/opportunity_index.rs": (
        "pub(crate) fn current(&self, id: &str) -> Option<ArbitrageOpportunityDto>",
    ),
    "crates/api/src/services/opportunity_detail.rs": (
        ".current(id)\n        .ok_or_else(|| expired_error(id))",
    ),
    "crates/api/src/services/execution_runs/timeline.rs": (
        "codes::HEDGE_TICKET_ORDER_PLAN_EVIDENCE_INVALID",
        "append_ticket_plan_problem(run, problem)",
    ),
    "crates/api/src/services/execution_orchestrator.rs": (
        "ticket_submission_context(",
    ),
    "crates/api/src/services/execution_orchestrator/submission.rs": (
        "invalid_ticket_order_plan_problem(",
    ),
    "crates/api/src/services/execution_orchestrator/context.rs": (
        '"originalDetails".to_owned()',
        "attach_problem_preserves_non_object_details",
    ),
    "crates/api/src/services/hedge_confirm/confirm.rs": (
        "codes::HEDGE_CONFIRM_PARTIAL_OUTCOME_ENCODE_FAILED",
        '"originalDetails".to_owned()',
    ),
    "frontend/src/api/rest/retry_after.rs": (
        'parse_retry_after_ms_with_clock("7", None)',
        "retry_after_numeric_does_not_depend_on_wall_clock",
    ),
    "crates/api/src/services/venue_credentials/keychain.rs": (
        "Result<Option<String>, CredentialUpdateError>",
        "ERR_SEC_ITEM_NOT_FOUND",
    ),
    "crates/api/src/services/venue_credentials/storage.rs": (
        "record_backend_error(backend, &error);",
        "secret_for_current_backend",
    ),
    "crates/api/src/services/venue_credentials/storage/health.rs": (
        "static BACKEND_READ_ERRORS: OnceLock<DashMap<SecretBackend, String>>",
        "Secret backend 读取异常，凭证字段已按未配置处理。",
    ),
    "crates/api/src/services/hedge_preflight/account_mode.rs": (
        "fn successful_account_mode<'a>(",
    ),
    "shared-types/src/arbitrage.rs": (
        "pub settlement_countdown_seconds: Option<i64>",
        "opportunity_list_metrics_preserve_missing_settlement_countdown",
    ),
    "frontend/src/panels/modules/opportunity_view_model/format.rs": (
        'return "结算时间缺证据".into();',
    ),
    "frontend/src/panels/modules/settings/tabs/action_runs/hedge_confirm.rs": (
        "Result<HedgeConfirmResponse, serde_json::Error>",
        '"确认结果解码失败"',
    ),
    "crates/trading/src/sql_ledger.rs": (
        "collect_realized_run_finality_events(finality_events)?",
        "sql_realized_close_runs_reject_any_invalid_finality_row",
    ),
    "crates/api/src/services/hedge_ticket/quote.rs": (
        'None => format!("{exchange} {symbol} orderbook 触发限频退避，重试时间未知")',
    ),
}
for relative, required_markers in markers.items():
    source = (root / relative).read_text(encoding="utf-8")
    for marker in required_markers:
        if marker not in source:
            fail(f"{relative} lost marker: {marker}")

for relative in (
    "crates/api/src/routers/trading/account.rs",
    "crates/api/src/routers/trading/adapters.rs",
    "crates/api/src/routers/trading/kill_switch.rs",
):
    if "let mutation = match &result" not in (root / relative).read_text(encoding="utf-8"):
        fail(f"{relative} erased explicit mutation result matching")

forbidden = {
    "crates/api/src/services/hedge_preview/guards.rs": r"exchange_capability_matrix\([^;]+\)\.ok\(\)",
    "crates/api/src/services/opportunity_detail.rs": r"\.get\(id,\s*None\)\s*\.ok\(\)\s*\.flatten\(\)",
    "crates/trading/src/journal/projection/part_04.rs": r"pub fn record_funding_by_venue_symbol\(",
    "crates/api/src/services/venue_credentials/keychain.rs": r"get_generic_password\([^;]+\)\.ok\(\)\?",
    "shared-types/src/arbitrage.rs": r"pub settlement_countdown_seconds: i64",
    "frontend/src/panels/modules/settings/tabs/action_runs/hedge_confirm.rs": r"serde_json::from_value\(result\.clone\(\)\)\s*\.ok\(\)",
    "crates/trading/src/sql_ledger.rs": r"sql_run_finality_replay_event\(&row\)\.ok\(\)",
}
for relative, pattern in forbidden.items():
    source = (root / relative).read_text(encoding="utf-8")
    if re.search(pattern, source, re.DOTALL):
        fail(f"{relative} reintroduced a forbidden result-erasure pattern")

repo_gate = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
if repo_gate.count("check_pr_bp_completion.sh") < 2:
    fail("repo gate must run the PR-BP completion contract in docs and all scopes")

print(f"OK PR-BP static completion contract ({len(evidence)} evidence types)")
PY

bash -n "$0"
bash "$ROOT/scripts/check_allow_debt.sh"
bash "$ROOT/scripts/check_panic_result_debt.sh"

if [[ "${PR_BP_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-10}" \
    cargo test -p arbitrage fee_evidence --lib --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-10}" \
    cargo test -p trading funding --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-10}" \
    cargo test -p trading sql_realized_close_runs_reject_any_invalid_finality_row \
      --lib --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-10}" \
    cargo test -p shared-types \
      opportunity_list_metrics_preserve_missing_settlement_countdown --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-10}" \
    cargo test -p api --bin crypto-arb-api ticket_order_plan --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-10}" \
    cargo test -p api --bin crypto-arb-api venue_credentials --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-10}" \
    cargo test -p api --bin crypto-arb-api \
      orderbook_blockers_do_not_present_unknown_timings_as_zero --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-10}" \
    cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib --no-fail-fast
  bash "$ROOT/scripts/check_allow_debt_self_test.sh"
fi

if [[ "${PR_BP_SKIP_UPSTREAM:-0}" != "1" ]]; then
  bash "$ROOT/scripts/check_product_audit_progress.sh"
  bash "$ROOT/scripts/check_product_audit_evidence.sh"
  bash "$ROOT/scripts/check_product_audit_evidence_index.sh"
  bash "$ROOT/scripts/check_product_audit_coverage.sh"
fi

printf 'PR-BP completion gate passed\n'
