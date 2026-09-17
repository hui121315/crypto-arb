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
import json
import re
import shutil
import sys
import tempfile
from pathlib import Path


TITLE = "PR-CG HedgeTicket Scoped Preflight & Balance Evidence"
VERIFY = "`bash scripts/check_pr_cg_completion.sh --self-test`"
EVIDENCE = {
    "successor-pr-dv": ("scripts/check_pr_dv_completion.sh", "check_pr_dv_completion.sh"),
    "successor-pr-cm": ("scripts/check_pr_cm_completion.sh", "check_pr_cm_completion.sh"),
    "successor-pr-eb": ("scripts/check_pr_eb_completion.sh", "check_pr_eb_completion.sh"),
    "successor-pr-ed": ("scripts/check_pr_ed_completion.sh", "check_pr_ed_completion.sh"),
    "successor-pr-eg": ("scripts/check_pr_eg_completion.sh", "check_pr_eg_completion.sh"),
    "confirm-context-authority": ("scripts/check_pr_df_completion.sh", "check_pr_df_completion.sh"),
    "run-finality-authority": ("scripts/check_pr_ea_completion.sh", "check_pr_ea_completion.sh"),
    "shared-margin-outcome": ("shared-types/src/hedge.rs", "cargo test -p shared-types hedge"),
    "scoped-margin-preview-final": ("crates/api/src/services/hedge_margin.rs", "cargo test -p api --bin crypto-arb-api hedge_margin"),
    "margin-problem-evidence": ("crates/api/src/services/hedge_margin/evidence.rs", "final_margin_error_embeds_preflight_outcome_details"),
    "margin-request-retry-freshness": ("crates/api/src/services/hedge_margin/health.rs", "margin_evidence_merges_scoped_operation_health_request_and_retry"),
    "account-state-margin-facts": ("crates/api/src/services/hedge_margin/venues.rs", "margin_rows_prefer_verified_account_summary_available_facts"),
    "final-margin-guard-projection": ("crates/api/src/services/hedge_preview/final_margin.rs", "final_margin_evidence_replaces_preview_guard_and_workflow_health"),
    "workflow-margin-refresh": ("crates/api/src/services/hedge_preview/workflow_view.rs", "final_margin_refresh_updates_both_legs_without_erasing_other_health"),
    "preview-state-persistence": ("crates/api/src/services/hedge_preview.rs", "persist_preview"),
    "confirm-router-persistence": ("crates/api/src/services/hedge_confirm/confirm.rs", "persist_preview"),
    "execution-run-persistence": ("crates/api/src/services/execution_orchestrator/run_model.rs", "hedge_ticket_view = Some(preview.workflow_view.clone())"),
    "scoped-live-operation-matrix": ("crates/api/src/services/hedge_preflight/live_health/credentials/part_01.rs", "services::hedge_preflight::tests"),
    "confirm-account-order-recheck": ("crates/api/src/services/hedge_preflight/order_submission.rs", "services::hedge_preflight::tests"),
    "confirm-all-blockers": ("crates/api/src/services/hedge_confirm/confirm_validate/tests.rs", "confirm_scoped_preflight_returns_every_blocked_guard"),
    "frontend-preflight-evidence": ("frontend/src/panels/modules/execution/components/risk_preview/preflight.rs", "cargo test --manifest-path frontend/Cargo.toml --lib risk_preview"),
    "frontend-workflow-evidence": ("frontend/src/panels/modules/execution/components/workflow_status.rs", "health_title_retains_evidence_and_problem_identity"),
    "product-browser": ("test/e2e/pr_dv_scoped_preflight.spec.ts", "test:e2e:pr-dv"),
    "product-suite-contract": ("package.json", "test:e2e:pr-dv"),
    "repo-gate-wiring": ("scripts/verify_repo_gates.sh", "verify_repo_gates.sh"),
    "completion-governance": ("scripts/check_pr_cg_completion.sh", "check_pr_cg_completion.sh --self-test"),
}
RUNNABLE = (
    ("crates/api/src/services/hedge_preview/final_margin.rs", "final_margin_evidence_replaces_preview_guard_and_workflow_health"),
    ("crates/api/src/services/hedge_preview/workflow_view/tests.rs", "final_margin_refresh_updates_both_legs_without_erasing_other_health"),
    ("crates/api/src/services/hedge_margin/tests/outcome.rs", "margin_outcome_carries_scoped_balance_evidence"),
    ("crates/api/src/services/hedge_margin/tests/evidence.rs", "final_margin_error_embeds_preflight_outcome_details"),
    ("crates/api/src/services/hedge_margin/tests/account_summary.rs", "margin_rows_prefer_verified_account_summary_available_facts"),
    ("crates/api/src/services/hedge_confirm/confirm_validate/tests.rs", "confirm_scoped_preflight_returns_every_blocked_guard"),
    ("frontend/src/panels/modules/execution/components/workflow_status.rs", "health_title_retains_evidence_and_problem_identity"),
)


def fail(message: str) -> None:
    raise ValueError(message)


def rows(path: Path) -> list[dict[str, str]]:
    with path.open(encoding="utf-8", newline="") as handle:
        return list(csv.DictReader(handle, delimiter="\t"))


def require_markers(root: Path, relative: str, markers: tuple[str, ...]) -> None:
    source = (root / relative).read_text(encoding="utf-8")
    for marker in markers:
        if marker not in source:
            fail(f"missing {relative} marker: {marker}")


def check(root: Path) -> None:
    doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
    row = next((line for line in doc.splitlines() if line.startswith(f"| `{TITLE}`")), None)
    if row is None or "✅ 完成" not in row or "剩余：无。" not in row or row.count(VERIFY) != 1:
        fail("roadmap row must be complete, remaining-none and bound to the destructive gate")
    queue = doc[doc.index("### 🟡 6.5"):]
    if re.search(r"(?m)^\d+\.\s+\*\*PR-CG\b", queue):
        fail("completed PR-CG remains in the local queue")
    da_row = next((line for line in doc.splitlines() if line.startswith("| `PR-DA ")), "")
    if "🟡 部分完成" in da_row:
        if not re.search(r"(?m)^1\.\s+\*\*PR-DA\b", queue):
            fail("PR-DA must be the next incomplete queue head")
    elif "✅ 完成" in da_row:
        head = re.search(r"(?m)^1\.\s+\*\*(PR-[A-Z]+)\b", queue)
        head_row = next((line for line in doc.splitlines() if head and line.startswith(f"| `{head.group(1)} ")), "")
        if not head_row or "✅ 完成" in head_row:
            fail("completed PR-DA must hand off to an incomplete roadmap row")
    else:
        fail("PR-DA successor must be complete or partial")
    queue_items = re.findall(r"(?m)^\d+\.\s+\*\*", queue)
    if not queue_items:
        fail("local queue plus external pool must not be empty")

    history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(encoding="utf-8")
    if "## 2026-07-16 PR-CG HedgeTicket Scoped Preflight and Balance Evidence Closure" not in history:
        fail("history closure appendix is missing")

    selected = [item for item in rows(root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv") if item["pr_id"] == "PR-CG"]
    indexed = {item["evidence_type"]: item for item in selected}
    if len(indexed) != len(selected) or set(indexed) != set(EVIDENCE):
        fail(f"evidence type drift: expected={sorted(EVIDENCE)}, actual={sorted(indexed)}")
    for kind, (artifact, command_anchor) in EVIDENCE.items():
        item = indexed[kind]
        if item["artifact"] != artifact or command_anchor not in item["command"]:
            fail(f"evidence anchor drifted: {kind}")
        if not (root / artifact).is_file() or not item["notes"].strip():
            fail(f"evidence artifact or note is missing: {artifact}")

    coverage = {item["file"]: item for item in rows(root / "docs/PRODUCT_AUDIT_COVERAGE.tsv")}
    for artifact, _ in EVIDENCE.values():
        if artifact == "package.json" or Path(artifact).suffix in {".md", ".json"}:
            continue
        item = coverage.get(artifact)
        if item is None or item["coverage_status"] != "exact" or not item["evidence"].startswith("line:"):
            fail(f"exact coverage missing for {artifact}")

    marker_contract = {
        "crates/api/src/services/hedge_margin.rs": (
            "list_scoped_balances(&venues)",
            "refresh_scoped_balances(&venues)",
            "确认终检通过",
            "apply_final_margin_guard(preview, guard)",
        ),
        "crates/api/src/services/hedge_preview/final_margin.rs": (
            "record_final_margin_evidence",
            "workflow_view::refresh_margin_evidence",
            "ticket is missing preview-time margin evidence",
        ),
        "crates/api/src/services/hedge_preview.rs": (
            "pub(crate) fn persist_preview",
            ".hedge_tickets()",
            ".hedge_previews()",
        ),
        "crates/api/src/services/hedge_confirm/confirm.rs": (
            "ensure_final_margin(state, &mut preview).await",
            "hedge_preview::persist_preview(state, &preview)",
        ),
        "crates/api/src/services/execution_orchestrator/run_model.rs": (
            "run.evidence.hedge_ticket_view = Some(preview.workflow_view.clone())",
            "refresh_workflow_view(&mut run)",
        ),
        "crates/api/src/services/hedge_margin/health.rs": (
            "evidence.freshness_ms",
            "evidence.retry_after_ms",
            "evidence.request_id",
            "data_health.request_id",
        ),
        "crates/api/src/services/hedge_preflight/live_health/credentials/part_01.rs": (
            "HedgePreflightOperation::PrivateRead",
            "HedgePreflightOperation::MarginBalance",
            "HedgePreflightOperation::Positions",
            "HedgePreflightOperation::OpenOrders",
            "HedgePreflightOperation::PrivateWs",
            "HedgePreflightOperation::OrderFinality",
            "HedgePreflightOperation::Orderbook",
            "live_operation_request_id(plans, rows)",
            "live_operation_row_health(plans, rows)",
        ),
        "crates/api/src/services/hedge_preflight/order_submission.rs": (
            "recheck_hedge_live_order_preflight_guards",
            "tokio::join!(",
            "account_mode_guard",
            "order_write_guard",
        ),
    }
    for relative, markers in marker_contract.items():
        require_markers(root, relative, markers)

    ignored = re.compile(r"#\[\s*(?:ignore|should_panic)|#\[\s*cfg_attr[^\]]*ignore")
    for relative, test_name in RUNNABLE:
        source = (root / relative).read_text(encoding="utf-8")
        match = re.search(rf"#\[(?:tokio::)?test\][\s\S]{{0,180}}?(?:async\s+)?fn\s+{re.escape(test_name)}\b", source)
        if match is None or ignored.search(source[max(0, match.start() - 160):match.end()]):
            fail(f"runnable anchor missing, skipped or panic-expected: {relative}:{test_name}")

    browser_path = "test/e2e/pr_dv_scoped_preflight.spec.ts"
    browser = (root / browser_path).read_text(encoding="utf-8")
    if re.search(r"\btest\.(?:skip|fixme)\s*\(", browser):
        fail("browser fixture must not be skipped")
    for marker in (
        "PR-DV restores submitted legs without promoting acknowledgements to Hedged",
        "req-pr-cg-final-margin",
        "account_state.margin_facts",
        "data-health=\"balance\"",
    ):
        if marker not in browser:
            fail(f"browser final-margin marker missing: {marker}")
    if browser.count("req-pr-cg-final-margin") < 3:
        fail("browser must prove final margin evidence before and after reload")

    package = json.loads((root / "package.json").read_text(encoding="utf-8"))
    scripts = package.get("scripts", {})
    if scripts.get("test:e2e:pr-dv") != "playwright test test/e2e/pr_dv_scoped_preflight.spec.ts":
        fail("dedicated browser command is missing")
    if scripts.get("test:e2e:product", "").count(browser_path) != 1:
        fail("product suite must include the browser fixture exactly once")

    repo_gate = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
    if repo_gate.count("check_pr_cg_completion.sh") != 2:
        fail("repo gate must execute PR-CG once in docs and all scopes")


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
        "crates/api/src/services/hedge_margin/tests/outcome.rs",
        "crates/api/src/services/hedge_margin/tests/evidence.rs",
        "crates/api/src/services/hedge_margin/tests/account_summary.rs",
        "crates/api/src/services/hedge_preview/workflow_view/tests.rs",
    }
    paths.update(artifact for artifact, _ in EVIDENCE.values())
    paths.update(relative for relative, _ in RUNNABLE)
    with tempfile.TemporaryDirectory(prefix="crossline-pr-cg-completion-") as temp:
        root = Path(temp) / "repo"
        for relative in paths:
            target = root / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source_root / relative, target)
        check(root)
        assert_rejected(root, "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md", lambda text: text.replace(f"| `{TITLE}` | ✅ 完成 |", f"| `{TITLE}` | 🟡 部分完成 |", 1), "a downgraded roadmap row")
        assert_rejected(root, "docs/PRODUCT_AUDIT_EVIDENCE.tsv", lambda text: "\n".join(line for line in text.splitlines() if not line.startswith("PR-CG\tfinal-margin-guard-projection\t")) + "\n", "an incomplete evidence matrix")
        assert_rejected(root, "crates/api/src/services/hedge_margin.rs", lambda text: text.replace("refresh_scoped_balances(&venues)", "list_scoped_balances(&venues)", 1), "a non-refreshing final balance check")
        assert_rejected(root, "crates/api/src/services/hedge_preview/final_margin.rs", lambda text: text.replace("workflow_view::refresh_margin_evidence", "workflow_view::removed_margin_evidence", 1), "a detached workflow projection")
        assert_rejected(root, "crates/api/src/services/hedge_confirm/confirm.rs", lambda text: text.replace("    crate::services::hedge_preview::persist_preview(state, &preview);\n", "", 1), "discarded successful final margin evidence")
        assert_rejected(root, "crates/api/src/services/hedge_preflight/live_health/credentials/part_01.rs", lambda text: text.replace("            HedgePreflightOperation::PrivateWs,\n", "", 1), "private WS outside the ticket scope")
        assert_rejected(root, "test/e2e/pr_dv_scoped_preflight.spec.ts", lambda text: text.replace('test("PR-DV restores submitted legs', 'test.skip("PR-DV restores submitted legs', 1), "a skipped restore fixture")
        assert_rejected(root, "test/e2e/pr_dv_scoped_preflight.spec.ts", lambda text: text.replace("req-pr-cg-final-margin", "removed-final-margin-request", 3), "missing browser request identity")
        assert_rejected(root, "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md", lambda text: text.replace("### 🟡 6.5 下一步执行队列", "### 🟡 6.5 下一步执行队列\n\n1. **PR-CG stale**", 1), "completed PR-CG returned to the queue")
        assert_rejected(root, "docs/PRODUCT_AUDIT_COVERAGE.tsv", lambda text: "\n".join(line for line in text.splitlines() if not line.startswith("crates/api/src/services/hedge_preview/final_margin.rs\t")) + "\n", "missing exact final-margin coverage")
        assert_rejected(root, "scripts/verify_repo_gates.sh", lambda text: text.replace('  PR_CG_SKIP_TESTS=1 PR_CG_SKIP_UPSTREAM=1 PR_CG_SKIP_BROWSER_LIST=1 bash "$ROOT/scripts/check_pr_cg_completion.sh"\n', "", 1), "single-scope repository wiring")
    print("PR-CG completion self-test passed")


root = Path(sys.argv[1])
mode = sys.argv[2]
if mode == "--self-test":
    self_test(root)
else:
    check(root)
    print(f"OK PR-CG static contract ({len(EVIDENCE)} evidence types; {len(RUNNABLE)} runnable anchors; non-skipping final-margin restore)")
PY

if [[ "$MODE" == "--self-test" ]]; then
  exit 0
fi

if [[ "${PR_CG_SKIP_BROWSER_LIST:-0}" != "1" ]]; then
  npx playwright test "$ROOT/test/e2e/pr_dv_scoped_preflight.spec.ts" --list >/dev/null
fi

if [[ "${PR_CG_SKIP_UPSTREAM:-0}" != "1" ]]; then
  PR_DV_SKIP_TESTS=1 PR_DV_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_dv_completion.sh"
  PR_CM_SKIP_TESTS=1 PR_CM_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_cm_completion.sh"
  PR_EB_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_eb_completion.sh"
  PR_ED_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_ed_completion.sh"
  PR_EG_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_eg_completion.sh"
  PR_DF_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_df_completion.sh"
  PR_EA_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_ea_completion.sh"
  bash "$ROOT/scripts/check_product_audit_progress.sh"
  bash "$ROOT/scripts/check_product_audit_evidence.sh"
  bash "$ROOT/scripts/check_product_audit_evidence_index.sh"
  bash "$ROOT/scripts/check_product_audit_coverage.sh"
  bash "$ROOT/scripts/check_release_qa_contract.sh"
fi

if [[ "${PR_CG_SKIP_TESTS:-0}" != "1" ]]; then
  jobs="${CARGO_BUILD_JOBS:-10}"
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api hedge_margin --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api final_margin --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api services::hedge_preflight::tests --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api confirm_scoped_preflight_returns_every_blocked_guard --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib workflow_status --no-fail-fast
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-dv -- --workers=1
fi

printf 'OK PR-CG HedgeTicket scoped preflight and balance evidence completion contract\n'
