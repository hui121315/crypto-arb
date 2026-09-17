#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

python3 - "$ROOT" "$MODE" <<'PY'
from __future__ import annotations

import csv
import re
import shutil
import sys
import tempfile
from pathlib import Path


PR_TITLE = "PR-DI Portfolio AccountState & CloseRun Contract"
VERIFY_ANCHOR = "`bash scripts/check_pr_di_completion.sh --self-test`"

EVIDENCE = {
    "successor-pr-cd": ("scripts/check_pr_cd_completion.sh", "check_pr_cd_completion.sh"),
    "successor-pr-ed": ("scripts/check_pr_ed_completion.sh", "check_pr_ed_completion.sh"),
    "successor-pr-dz": ("scripts/check_pr_dz_completion.sh", "check_pr_dz_completion.sh"),
    "successor-pr-dh": ("scripts/check_pr_dh_completion.sh", "check_pr_dh_completion.sh"),
    "successor-pr-ea": ("scripts/check_pr_ea_completion.sh", "check_pr_ea_completion.sh"),
    "successor-pr-bw": ("scripts/check_pr_bw_completion.sh", "check_pr_bw_completion.sh"),
    "balance-runtime-pr-dc": ("crates/api/src/services/account_balances.rs", "balance_row_health"),
    "account-state-contract": ("shared-types/src/orders.rs", "account_state"),
    "portfolio-envelope-contract": ("shared-types/src/portfolio/snapshot.rs", "portfolio_snapshot_envelope"),
    "close-run-contract": ("shared-types/src/portfolio/close.rs", "close_run_serializes"),
    "account-state-runtime": ("crates/api/src/services/account_state.rs", "account_state"),
    "balance-row-health": ("crates/api/src/services/account_balances/health.rs", "balance_row_health"),
    "position-partial-fanout": ("crates/api/src/services/account_positions/tests/error_paths.rs", "partial_position_fanout"),
    "open-order-partial-fanout": ("crates/api/src/services/account_open_orders/tests.rs", "partial_open_order_fanout"),
    "portfolio-lifecycle-envelope": ("crates/api/src/lifecycle/portfolio.rs", "lifecycle::portfolio"),
    "wallet-nav-source": ("crates/api/src/services/portfolio/risk.rs", "nav_breakdown"),
    "portfolio-pnl-quality": ("crates/api/src/services/portfolio_pnl/tests.rs", "empty_realized_window"),
    "funding-direction": ("crates/portfolio/src/risk.rs", "funding_cluster"),
    "auto-compensation-worker": ("crates/api/src/services/close_runs/auto_compensation.rs", "auto_compensation_worker"),
    "auto-compensation-policy": ("crates/api/src/services/close_runs/auto_compensation/retry_policy.rs", "auto_compensation_worker_skips_live_candidates"),
    "auto-compensation-tests": ("crates/api/src/services/close_runs/tests/cases_c/compensation.rs", "auto_compensation_worker_retries_single_failed_candidate_once"),
    "close-run-replay": ("crates/api/src/services/close_runs/tests/cases_c/recovery.rs", "close_run_store_replays_submitted_snapshot_after_restart"),
    "manual-terminal": ("crates/api/src/services/close_runs/tests/cases_b/manual_terminal.rs", "manual_terminal_ack_success_commits_once_with_manual_source"),
    "frontend-load-state": ("frontend/src/panels/modules/positions/data/snapshot.rs", "degraded_snapshot_envelope_keeps_snapshot_as_stale"),
    "frontend-section-state": ("frontend/src/panels/modules/positions/view/tests.rs", "snapshot_section_error_keeps_problem_visible"),
    "frontend-risk-quality": ("frontend/src/panels/modules/positions/components/positions_table/testing/pr_dz.rs", "unknown_liquidation_severity_is_visibly_distinct_from_ok"),
    "frontend-close-run-state": ("frontend/src/panels/modules/positions/data/tests/close.rs", "compensation_submitted_close_run_maps_to_accepted_action_state"),
    "frontend-close-run-recovery": ("frontend/src/panels/modules/positions/components/close_runs_panel/testing/finality.rs", "close_run_rows_keep_only_actionable_runs_newest_first"),
    "portfolio-evidence-browser": ("test/e2e/pr_cd_portfolio_evidence.spec.ts", "test:e2e:pr-cd"),
    "portfolio-truth-browser": ("test/e2e/pr_dz_portfolio_truth.spec.ts", "test:e2e:pr-dz"),
    "completion-governance": ("scripts/check_pr_di_completion.sh", "check_pr_di_completion.sh --self-test"),
}

RUNNABLE = (
    ("shared-types/src/portfolio/tests/snapshot.rs", "portfolio_snapshot_envelope_can_carry_error_without_fake_snapshot"),
    ("shared-types/src/portfolio/tests/close.rs", "close_run_serializes_submitted_without_claiming_final_success"),
    ("crates/api/src/services/account_state/tests.rs", "account_state_includes_open_order_rows_and_degrades"),
    ("crates/api/src/services/account_state/tests/reconciliation.rs", "account_state_merges_child_evidence_and_marks_unknown_equity"),
    ("crates/api/src/services/account_balances/tests.rs", "balance_row_health_keeps_source_freshness_and_retry_context"),
    ("crates/api/src/services/account_positions/tests/error_paths.rs", "partial_position_fanout_keeps_rows_and_surfaces_route_health"),
    ("crates/api/src/services/account_open_orders/tests.rs", "partial_open_order_fanout_keeps_rows_and_surfaces_route_health"),
    ("crates/api/src/lifecycle/portfolio.rs", "stale_degraded_snapshot_keeps_last_business_values_and_problem_source"),
    ("crates/api/src/lifecycle/portfolio.rs", "cold_start_failure_publishes_error_envelope_without_fake_snapshot"),
    ("crates/api/src/services/portfolio/tests/cases_a/nav.rs", "nav_breakdown_keeps_wallet_position_cash_and_unrealized_quality"),
    ("crates/api/src/services/portfolio/tests/cases_b.rs", "missing_or_invalid_liquidation_distance_is_never_marked_ok"),
    ("crates/api/src/services/portfolio_pnl/tests.rs", "empty_realized_window_is_an_actual_zero_with_source"),
    ("crates/portfolio/src/risk/tests.rs", "funding_cluster_excludes_long_receiving_leg"),
    ("crates/portfolio/src/risk/tests.rs", "funding_cluster_skips_unverified_rate"),
    ("crates/api/src/services/close_runs/tests/cases_c/compensation.rs", "auto_compensation_worker_submits_single_candidate_with_action_run"),
    ("crates/api/src/services/close_runs/tests/cases_c/compensation.rs", "auto_compensation_worker_retries_single_failed_candidate_once"),
    ("crates/api/src/services/close_runs/tests/cases_c/recovery.rs", "auto_compensation_worker_skips_live_candidates"),
    ("crates/api/src/services/close_runs/tests/cases_c/recovery.rs", "close_run_store_replays_submitted_snapshot_after_restart"),
    ("crates/api/src/services/close_runs/tests/cases_b/manual_terminal.rs", "manual_terminal_ack_success_commits_once_with_manual_source"),
    ("crates/api/src/services/close_runs/tests/cases_b/manual_terminal.rs", "manual_terminal_ack_failure_leaves_hot_and_durable_state_unmutated"),
    ("frontend/src/panels/modules/positions/data/tests/snapshot.rs", "degraded_snapshot_envelope_keeps_snapshot_as_stale"),
    ("frontend/src/panels/modules/positions/view/tests.rs", "snapshot_section_error_keeps_problem_visible"),
    ("frontend/src/panels/modules/positions/view/tests.rs", "close_runs_surface_hides_all_empty_states"),
    ("frontend/src/panels/modules/positions/view/tests.rs", "close_runs_surface_hides_ready_empty_state"),
    ("frontend/src/panels/modules/positions/components/positions_table/testing/pr_dz.rs", "unknown_liquidation_severity_is_visibly_distinct_from_ok"),
    ("frontend/src/panels/modules/positions/data/tests/close.rs", "compensation_submitted_close_run_maps_to_accepted_action_state"),
    ("frontend/src/panels/modules/positions/components/close_runs_panel/testing/finality.rs", "close_run_rows_keep_only_actionable_runs_newest_first"),
)

BROWSERS = {
    "test/e2e/pr_cd_portfolio_evidence.spec.ts": "PR-CD keeps NAV components PnL ledger quality and liquidation provenance visible",
    "test/e2e/pr_dz_portfolio_truth.spec.ts": "PR-DZ portfolio envelope keeps wallet NAV, position evidence, partial state, and durable compensation visible",
}

SUCCESSORS = (
    "PR-CD Portfolio AccountState Evidence & Risk LoadState Contract",
    "PR-DC Balance Runtime Health & Request Coalescing Contract",
    "PR-DH Review Ledger Truth Source & Estimated PnL Contract",
    "PR-DZ Portfolio AccountState & CloseRun Contract",
    "PR-EA ExecutionRun Finality & ActionState Contract",
    "PR-ED AccountState Evidence & Unified Margin Contract",
    "PR-BW ExecutionRun Finality & ActionState Snapshot",
)


def fail(message: str) -> None:
    raise ValueError(message)


def table_rows(path: Path) -> list[dict[str, str]]:
    with path.open(encoding="utf-8", newline="") as handle:
        return list(csv.DictReader(handle, delimiter="\t"))


def require_markers(root: Path, relative: str, *markers: str) -> str:
    source = (root / relative).read_text(encoding="utf-8")
    for marker in markers:
        if marker not in source:
            fail(f"missing {relative} marker: {marker}")
    return source


def check(root: Path) -> None:
    doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
    row = next((line for line in doc.splitlines() if line.startswith(f"| `{PR_TITLE}`")), None)
    if row is None or "✅ 完成" not in row or "剩余：无。" not in row or row.count(VERIFY_ANCHOR) != 1:
        fail("roadmap row must be complete, remaining-none and bound to the destructive gate")

    queue = doc[doc.index("### 🟡 6.5"):]
    if re.search(r"(?m)^\d+\.\s+\*\*PR-DI\b", queue):
        fail("completed PR-DI remains in the queue")
    successor_row = next(
        (line for line in doc.splitlines() if line.startswith("| `PR-CF CEX Credential Validation & Account Mode Evidence Matrix`")),
        None,
    )
    if successor_row is None or "✅ 完成" not in successor_row or "剩余：无。" not in successor_row:
        fail("successor PR-CF completion drifted")
    if re.search(r"(?m)^\d+\.\s+\*\*PR-CF\b", queue):
        fail("completed successor PR-CF remains in the queue")
    queue_items = re.findall(r"(?m)^\d+\.\s+\*\*", queue)
    if not queue_items:
        fail("local queue plus external pool must not be empty")

    for successor in SUCCESSORS:
        successor_row = next((line for line in doc.splitlines() if line.startswith(f"| `{successor}`")), None)
        if successor_row is None or "✅ 完成" not in successor_row or "剩余：无。" not in successor_row:
            fail(f"completed successor authority drifted: {successor}")

    history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(encoding="utf-8")
    if "## 2026-07-17 PR-DI Portfolio AccountState and CloseRun Closure" not in history:
        fail("PR-DI closure appendix is missing")

    selected = [item for item in table_rows(root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv") if item["pr_id"] == "PR-DI"]
    indexed = {item["evidence_type"]: item for item in selected}
    if len(indexed) != len(selected) or set(indexed) != set(EVIDENCE):
        fail(f"evidence type drift: expected={sorted(EVIDENCE)}, actual={sorted(indexed)}")
    for evidence_type, (artifact, command_anchor) in EVIDENCE.items():
        item = indexed[evidence_type]
        if item["artifact"] != artifact or command_anchor not in item["command"] or not item["notes"].strip():
            fail(f"evidence anchor drifted: {evidence_type}")
        if not (root / artifact).is_file():
            fail(f"evidence artifact missing: {artifact}")

    coverage = {item["file"]: item for item in table_rows(root / "docs/PRODUCT_AUDIT_COVERAGE.tsv")}
    for artifact, _ in EVIDENCE.values():
        item = coverage.get(artifact)
        if item is None or item["coverage_status"] != "exact" or not item["evidence"].startswith("line:"):
            fail(f"exact coverage missing for {artifact}")

    account_contract = require_markers(
        root,
        "shared-types/src/orders.rs",
        "pub struct AccountStateSnapshot",
        "pub balances: VenueBalanceEnvelope",
        "pub positions: VenuePositionEnvelope",
        "pub open_orders: VenueOpenOrdersEnvelope",
        "pub problems: Vec<ApiProblem>",
        "pub operation_health: Vec<VenueOperationHealth>",
        "pub field_quality: Vec<AccountFieldQuality>",
        "pub account_bindings: Vec<AccountBindingEvidence>",
    )
    shared_contracts = "\n".join(
        path.read_text(encoding="utf-8") for path in (root / "shared-types/src").rglob("*.rs")
    )
    if shared_contracts.count("pub struct AccountStateSnapshot") != 1:
        fail("AccountStateSnapshot must remain a single shared contract")
    if "status: ListStatus::Degraded" not in account_contract:
        fail("default AccountState must remain fail-closed")

    require_markers(
        root,
        "shared-types/src/portfolio/snapshot.rs",
        "pub enum PortfolioSnapshotStatus",
        "Degraded,",
        "Stale,",
        "\n    Error,\n",
        "pub snapshot: Option<PortfolioSnapshot>",
        "pub problems: Vec<ApiProblem>",
        "pub retry_after_ms: Option<u64>",
    )
    require_markers(
        root,
        "shared-types/src/portfolio/close.rs",
        "PartiallySubmitted",
        "UnwindRequired",
        "CompensationSubmitted",
        "CompensationFailed",
        "ManuallyResolved",
        "pub naked_exposure_usd: f64",
        "pub finality_problem: Option<ApiProblem>",
        "pub unwind_plan: Option<CloseRunUnwindPlan>",
    )
    require_markers(
        root,
        "crates/api/src/services/account_state.rs",
        "account_summary_problems",
        "account_equity_unknown_quality",
        "account_state_status",
        "ListStatus::Degraded",
    )
    require_markers(
        root,
        "crates/api/src/services/account_balances/health.rs",
        "data_health.freshness_ms",
        "data_health.retry_after_ms",
        "evidence.request_id.clone()",
    )
    require_markers(
        root,
        "crates/api/src/services/portfolio/risk.rs",
        "AccountFieldQualityStatus::Actual",
        "AccountFieldQualityStatus::Missing",
        'source: "account_state.account_summaries.total_equity_usd"',
    )
    require_markers(
        root,
        "crates/api/src/services/portfolio/pricing.rs",
        "PositionSeverity::Unknown",
        '"fundingRate8h"',
        '"nextFundingMs"',
    )
    require_markers(
        root,
        "crates/api/src/services/account_positions/projection/liquidation.rs",
        '"liquidationPrice"',
        '"liquidationDistancePct"',
        "AccountFieldQualityStatus::Missing",
        "estimated_position_field",
    )
    require_markers(root, "crates/portfolio/src/risk.rs", "funding_payment_usd(row).max(0.0)")
    require_markers(
        root,
        "crates/api/src/services/close_runs/auto_compensation.rs",
        "auto_submit_compensation_once",
        "begin_auto_compensation_action_run",
        "auto_compensation_key(submit)",
    )
    require_markers(
        root,
        "crates/api/src/services/close_runs/auto_compensation/retry_policy.rs",
        "AutoCompensationSkip::LiveMode",
        "compensation_retry_allowed_for_candidate",
    )
    require_markers(
        root,
        "crates/api/src/services/close_runs/unwind.rs",
        "MAX_COMPENSATION_ATTEMPTS_PER_CANDIDATE: usize = 2",
        "matched_count < MAX_COMPENSATION_ATTEMPTS_PER_CANDIDATE",
    )
    require_markers(
        root,
        "frontend/src/panels/modules/positions/data/snapshot.rs",
        "snapshot.set(LoadState::Stale",
        "merge_close_run_update",
    )
    require_markers(
        root,
        "frontend/src/panels/modules/positions/components/positions_table/quality.rs",
        '"liquidationPrice"',
        '"fundingRate8h" | "nextFundingMs"',
        '"nextFundingMs" => "结算时间缺证据"',
    )

    invalid_test = re.compile(r"#\[\s*(?:ignore|should_panic)|#\[\s*cfg_attr[^\]]*ignore")
    for relative, test_name in RUNNABLE:
        source = (root / relative).read_text(encoding="utf-8")
        match = re.search(rf"#\[(?:tokio::)?test\][\s\S]{{0,180}}?(?:async\s+)?fn\s+{re.escape(test_name)}\b", source)
        if match is None or invalid_test.search(source[max(0, match.start() - 160):match.end()]):
            fail(f"runnable test is missing or skipped: {relative}:{test_name}")

    for relative, title in BROWSERS.items():
        source = (root / relative).read_text(encoding="utf-8")
        escaped = re.escape(title)
        if re.search(rf'test\.(?:skip|fixme)\(\s*["\']{escaped}["\']', source):
            fail(f"browser proof is skipped: {title}")
        if not re.search(rf'test\(\s*["\']{escaped}["\']', source):
            fail(f"browser proof is missing: {title}")

    repo_gate = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
    if repo_gate.count("check_pr_di_completion.sh") != 2:
        fail("repo gate must execute PR-DI in docs and full scopes")


def assert_rejected(root: Path, relative: str, transform, label: str) -> None:
    path = root / relative
    baseline = path.read_text(encoding="utf-8")
    changed = transform(baseline)
    if changed == baseline:
        fail(f"self-test setup drifted: {label}")
    path.write_text(changed, encoding="utf-8")
    try:
        check(root)
    except ValueError:
        pass
    else:
        fail(f"self-test accepted {label}")
    finally:
        path.write_text(baseline, encoding="utf-8")


def self_test(source_root: Path) -> None:
    paths = {
        "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md",
        "docs/PRODUCT_AUDIT_EVIDENCE.tsv",
        "docs/PRODUCT_AUDIT_COVERAGE.tsv",
        "docs/audit_history/PRODUCT_AUDIT_HISTORY.md",
        "scripts/verify_repo_gates.sh",
        "shared-types/src/portfolio/tests/snapshot.rs",
        "shared-types/src/portfolio/tests/close.rs",
        "crates/api/src/services/account_state/tests.rs",
        "crates/api/src/services/account_state/tests/reconciliation.rs",
        "crates/api/src/services/account_balances/tests.rs",
        "crates/api/src/services/account_positions/projection/liquidation.rs",
        "crates/api/src/lifecycle/portfolio.rs",
        "crates/api/src/services/close_runs/unwind.rs",
        "crates/api/src/services/portfolio/pricing.rs",
        "crates/api/src/services/portfolio/tests/cases_a/nav.rs",
        "crates/api/src/services/portfolio/tests/cases_b.rs",
        "crates/portfolio/src/risk/tests.rs",
        "frontend/src/panels/modules/positions/data/tests/snapshot.rs",
        "frontend/src/panels/modules/positions/components/positions_table/quality.rs",
        "frontend/src/panels/modules/positions/view/tests.rs",
    }
    paths.update(artifact for artifact, _ in EVIDENCE.values())
    paths.update(relative for relative, _ in RUNNABLE)
    paths.update(BROWSERS)
    with tempfile.TemporaryDirectory(prefix="crossline-pr-di-completion-") as temp:
        root = Path(temp) / "repo"
        for relative in paths:
            target = root / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source_root / relative, target)
        check(root)
        assert_rejected(root, "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md", lambda text: text.replace(f"| `{PR_TITLE}` | ✅ 完成 |", f"| `{PR_TITLE}` | 🟡 部分完成 |", 1), "a downgraded roadmap row")
        assert_rejected(root, "docs/PRODUCT_AUDIT_EVIDENCE.tsv", lambda text: "\n".join(line for line in text.splitlines() if not line.startswith("PR-DI\taccount-state-contract\t")) + "\n", "an incomplete evidence matrix")
        assert_rejected(root, "shared-types/src/orders.rs", lambda text: text.replace("pub struct AccountStateSnapshot", "pub struct RemovedAccountStateSnapshot", 1), "a detached AccountState contract")
        assert_rejected(root, "shared-types/src/portfolio/snapshot.rs", lambda text: text.replace("    Error,", "    HiddenError,", 1), "a Portfolio envelope without error state")
        assert_rejected(root, "crates/api/src/services/portfolio/risk.rs", lambda text: text.replace('source: "account_state.account_summaries.total_equity_usd"', 'source: "position_margin_proxy"'), "a proxy NAV source")
        assert_rejected(root, "crates/portfolio/src/risk.rs", lambda text: text.replace("funding_payment_usd(row).max(0.0)", "funding_payment_usd(row).abs()", 1), "funding receive legs counted as outflow")
        assert_rejected(root, "crates/api/src/services/close_runs/auto_compensation/retry_policy.rs", lambda text: text.replace("AutoCompensationSkip::LiveMode", "AutoCompensationSkip::NoCandidate", 1), "automatic live compensation")
        assert_rejected(root, "crates/api/src/services/close_runs/tests/cases_c/compensation.rs", lambda text: text.replace("#[tokio::test]\nasync fn auto_compensation_worker_retries_single_failed_candidate_once", "#[tokio::test]\n#[ignore]\nasync fn auto_compensation_worker_retries_single_failed_candidate_once", 1), "a skipped bounded retry fixture")
        assert_rejected(root, "frontend/src/panels/modules/positions/view/tests.rs", lambda text: text.replace("#[test]\nfn close_runs_surface_hides_all_empty_states", "#[test]\n#[ignore]\nfn close_runs_surface_hides_all_empty_states", 1), "a skipped empty CloseRun surface fixture")
        assert_rejected(root, "test/e2e/pr_dz_portfolio_truth.spec.ts", lambda text: text.replace('test("PR-DZ portfolio envelope', 'test.skip("PR-DZ portfolio envelope', 1), "a skipped portfolio truth browser proof")
        assert_rejected(root, "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md", lambda text: text.replace("### 🟡 6.5 下一步执行队列", "### 🟡 6.5 下一步执行队列\n\n1. **PR-DI stale**", 1), "completed PR-DI returned to the queue")
        assert_rejected(root, "scripts/verify_repo_gates.sh", lambda text: text.replace("check_pr_di_completion.sh", "removed_pr_di_completion.sh", 1), "single-scope repo wiring")
    print("PR-DI completion destructive self-test passed")


try:
    root = Path(sys.argv[1])
    if sys.argv[2] == "--self-test":
        self_test(root)
    else:
        check(root)
        print(
            f"OK PR-DI static contract ({len(EVIDENCE)} evidence types; "
            f"{len(RUNNABLE)} runnable anchors; {len(BROWSERS)} browser anchors)"
        )
except (OSError, KeyError, ValueError) as exc:
    raise SystemExit(f"PR-DI completion gate failed: {exc}") from exc
PY

if [[ "$MODE" == "--self-test" ]]; then
  exit 0
fi

if [[ "${PR_DI_SKIP_BROWSER_LIST:-0}" != "1" ]]; then
  npx playwright test \
    "$ROOT/test/e2e/pr_cd_portfolio_evidence.spec.ts" \
    "$ROOT/test/e2e/pr_dz_portfolio_truth.spec.ts" \
    --list >/dev/null
fi

if [[ "${PR_DI_SKIP_UPSTREAM:-0}" != "1" ]]; then
  PR_CD_SKIP_TESTS=1 PR_CD_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_cd_completion.sh"
  PR_ED_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_ed_completion.sh"
  PR_DZ_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dz_completion.sh"
  PR_DH_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dh_completion.sh"
  PR_EA_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_ea_completion.sh"
  PR_BW_SKIP_TESTS=1 PR_BW_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_bw_completion.sh"
  bash "$ROOT/scripts/check_product_audit_progress.sh"
  bash "$ROOT/scripts/check_product_audit_evidence.sh"
  bash "$ROOT/scripts/check_product_audit_evidence_index.sh"
  bash "$ROOT/scripts/check_product_audit_coverage.sh"
fi

if [[ "${PR_DI_SKIP_TESTS:-0}" != "1" ]]; then
  jobs="${CARGO_BUILD_JOBS:-10}"
  CARGO_BUILD_JOBS="$jobs" cargo test -p shared-types --lib portfolio --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p portfolio --lib funding_cluster --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api account_state --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api partial_position_fanout_keeps_rows_and_surfaces_route_health --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api partial_open_order_fanout_keeps_rows_and_surfaces_route_health --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api lifecycle::portfolio --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api services::portfolio --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api services::close_runs --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib positions --no-fail-fast
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-cd -- --workers=1
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-dz -- --workers=1
fi

printf 'PR-DI Portfolio AccountState and CloseRun completion passed\n'
