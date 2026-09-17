#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
PREVIEW_CONTRACT="$ROOT/shared-types/src/arbitrage.rs"
RUN_FEED="$ROOT/frontend/src/panels/modules/execution/data/run.rs"
BROWSER="$ROOT/test/e2e/pr_bo_workflow_recovery.spec.ts"
AUDIT_DOC="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-BO completion gate failed: %s\n' "$1" >&2
  exit 1
}

if [[ "$MODE" == "--self-test" ]]; then
  preview_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-bo-preview.XXXXXX")"
  feed_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-bo-feed.XXXXXX")"
  browser_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-bo-browser.XXXXXX")"
  audit_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-bo-audit.XXXXXX")"
  cp "$PREVIEW_CONTRACT" "$preview_backup"
  cp "$RUN_FEED" "$feed_backup"
  cp "$BROWSER" "$browser_backup"
  cp "$AUDIT_DOC" "$audit_backup"
  restore() {
    cp "$preview_backup" "$PREVIEW_CONTRACT"
    cp "$feed_backup" "$RUN_FEED"
    cp "$browser_backup" "$BROWSER"
    cp "$audit_backup" "$AUDIT_DOC"
    rm -f "$preview_backup" "$feed_backup" "$browser_backup" "$audit_backup"
  }
  trap restore EXIT

  PR_BO_SKIP_TESTS=1 PR_BO_SKIP_UPSTREAM=1 bash "$0" >/dev/null

  python3 - "$PREVIEW_CONTRACT" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "pub workflow_view: crate::workflow::HedgeTicketView,"
if source.count(marker) != 1:
    raise SystemExit("PR-BO self-test setup failed: preview workflow marker drifted")
path.write_text(source.replace(marker, "pub workflow_view_removed: crate::workflow::HedgeTicketView,", 1), encoding="utf-8")
PY
  if PR_BO_SKIP_TESTS=1 PR_BO_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a preview response without workflowView"
  fi
  cp "$preview_backup" "$PREVIEW_CONTRACT"

  python3 - "$RUN_FEED" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "apply_run_candidate(workflow, candidate, WS_RUN_SOURCE);"
if source.count(marker) != 1:
    raise SystemExit("PR-BO self-test setup failed: WS projection marker drifted")
path.write_text(source.replace(marker, "apply_run_candidate(workflow, candidate, REST_RUN_SOURCE);", 1), encoding="utf-8")
PY
  if PR_BO_SKIP_TESTS=1 PR_BO_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a WS delta mislabeled as a REST snapshot"
  fi
  cp "$feed_backup" "$RUN_FEED"

  python3 - "$BROWSER" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = 'test("PR-BO restores ticket health, seeds REST, and applies execution WS delta"'
if source.count(marker) != 1:
    raise SystemExit("PR-BO self-test setup failed: browser test marker drifted")
path.write_text(source.replace(marker, 'test.skip("PR-BO restores ticket health, seeds REST, and applies execution WS delta"', 1), encoding="utf-8")
PY
  if PR_BO_SKIP_TESTS=1 PR_BO_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a skipped recovery fixture"
  fi
  cp "$browser_backup" "$BROWSER"

  python3 - "$AUDIT_DOC" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
updated, replacements = __import__("re").subn(
    r"(?m)^1\. \*\*PR-[A-Z]+",
    "1. **PR-BS",
    source,
    count=1,
)
if replacements != 1:
    raise SystemExit("PR-BO self-test setup failed: current queue head is missing")
path.write_text(updated, encoding="utf-8")
PY
  if PR_BO_SKIP_TESTS=1 PR_BO_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a completed successor re-entering the local queue"
  fi
  cp "$audit_backup" "$AUDIT_DOC"

  python3 - "$AUDIT_DOC" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
lines = path.read_text(encoding="utf-8").splitlines()
matches = [index for index, line in enumerate(lines) if line.startswith("| `PR-BS Frontend Product Semantics & Copy Gate`")]
if len(matches) != 1:
    raise SystemExit("PR-BO self-test setup failed: successor roadmap row drifted")
index = matches[0]
if "✅ 完成" not in lines[index]:
    raise SystemExit("PR-BO self-test setup failed: successor is not complete")
lines[index] = lines[index].replace("✅ 完成", "🟡 部分完成", 1)
path.write_text("\n".join(lines) + "\n", encoding="utf-8")
PY
  if PR_BO_SKIP_TESTS=1 PR_BO_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted an unfinished successor outside the local queue head"
  fi
  cp "$audit_backup" "$AUDIT_DOC"

  python3 - "$AUDIT_DOC" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
lines = path.read_text(encoding="utf-8").splitlines()
matches = [
    index
    for index, line in enumerate(lines)
    if "机会搜索/详情/预览错误被空态吞没" in line
]
if len(matches) != 1:
    raise SystemExit("PR-BO self-test setup failed: preview finding drifted")
index = matches[0]
if "✅ 完成" not in lines[index] or "PR-BY" not in lines[index]:
    raise SystemExit("PR-BO self-test setup failed: PR-BY preview closure is missing")
lines[index] = lines[index].replace("✅ 完成", "🟡 部分完成", 1)
path.write_text("\n".join(lines) + "\n", encoding="utf-8")
PY
  if PR_BO_SKIP_TESTS=1 PR_BO_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted preview debt after the completed PR-BY successor"
  fi

  printf 'PR-BO completion self-test passed\n'
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
pr_id = "PR-BO"
title = "PR-BO Product Workflow Runtime & Feedback Loop"
verify_anchor = "`bash scripts/check_pr_bo_completion.sh --self-test`"
evidence_contract = {
    "shared-workflow-contract": "shared-types/src/workflow.rs",
    "shared-run-projection": "shared-types/src/workflow/projection.rs",
    "preview-api-contract": "shared-types/src/arbitrage.rs",
    "run-evidence-contract": "shared-types/src/execution_run.rs",
    "backend-workflow-projection": "crates/api/src/services/hedge_preview/workflow_view.rs",
    "backend-health-projection": "crates/api/src/services/hedge_preview/workflow_view/health.rs",
    "backend-projection-tests": "crates/api/src/services/hedge_preview/workflow_view/tests.rs",
    "preview-projection-wiring": "crates/api/src/services/hedge_preview.rs",
    "durable-run-projection": "crates/api/src/services/execution_runs/query.rs",
    "run-delta-projection": "crates/api/src/services/execution_runs/project.rs",
    "frontend-workflow-runtime": "frontend/src/panels/modules/execution/data/workflow.rs",
    "frontend-context-recovery": "frontend/src/panels/modules/execution/data/run/context.rs",
    "frontend-rest-ws-merge": "frontend/src/panels/modules/execution/data/run.rs",
    "frontend-workflow-status": "frontend/src/panels/modules/execution/components/workflow_status.rs",
    "frontend-workflow-style": "frontend/styles/src/skin/execution-workflow.css",
    "product-browser": "test/e2e/pr_bo_workflow_recovery.spec.ts",
    "completion-governance": "scripts/check_pr_bo_completion.sh",
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-BO completion gate failed: {message}")


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
if re.search(r"(?m)^\d+\.\s+\*\*PR-BO\b", queue):
    fail("completed PR-BO remains in the local queue")
successor_title = "PR-BS Frontend Product Semantics & Copy Gate"
successor_row = next(
    (line for line in doc.splitlines() if line.startswith(f"| `{successor_title}`")),
    None,
)
if successor_row is None:
    fail("PR-BS successor roadmap row is missing")
successor_complete = "✅ 完成" in successor_row
successor_queued = re.search(r"(?m)^\d+\.\s+\*\*PR-BS\b", queue) is not None
successor_is_head = re.search(r"(?m)^1\.\s+\*\*PR-BS\b", queue) is not None
if successor_complete and successor_queued:
    fail("completed PR-BS successor remains in the local queue")
if not successor_complete and not successor_is_head:
    fail("unfinished PR-BS successor must remain the local queue head")

for finding_title in (
    "主产品闭环缺可恢复 Workflow 状态",
    "ExecutionRun / orders 缺 REST snapshot fallback",
    "持仓/复盘不是执行后的强反馈事实源",
):
    finding = next((line for line in reversed(fact_lines) if finding_title in line), None)
    if finding is None or "✅" not in finding:
        fail(f"absorbed finding remains incomplete: {finding_title}")

preview_finding = next(
    (line for line in reversed(fact_lines) if "机会搜索/详情/预览错误被空态吞没" in line),
    None,
)
problem_successor_title = "PR-BY Frontend ApiProblem & LoadState Contract"
problem_successor_row = next(
    (line for line in doc.splitlines() if line.startswith(f"| `{problem_successor_title}`")),
    None,
)
problem_successor_complete = (
    problem_successor_row is not None and "✅ 完成" in problem_successor_row
)
if problem_successor_complete:
    if preview_finding is None or "✅ 完成" not in preview_finding or "PR-BY" not in preview_finding:
        fail("completed PR-BY successor must own the closed preview envelope finding")
elif preview_finding is None or "🟡 部分完成" not in preview_finding or "PR-BO" in preview_finding:
    fail("remaining preview envelope work must stay partial without PR-BO ownership")

if "## 2026-07-15 PR-BO Product Workflow Runtime and Feedback Loop Closure" not in history:
    fail("history closure appendix is missing")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == pr_id]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(
        "evidence type drift: "
        f"expected={sorted(evidence_contract)}, actual={sorted(indexed)}"
    )
for evidence_type, artifact in evidence_contract.items():
    evidence = indexed[evidence_type]
    if evidence["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"{evidence_type} artifact drifted or is missing")
    if not evidence["command"].strip() or not evidence["notes"].strip():
        fail(f"{evidence_type} lacks command or notes")

markers = {
    "shared-types/src/workflow.rs": (
        "pub struct WorkflowEvidenceHealth",
        "pub struct HedgeTicketLegView",
        "pub market: WorkflowEvidenceHealth",
        "pub capability: WorkflowEvidenceHealth",
    ),
    "shared-types/src/workflow/projection.rs": (
        "pub fn apply_execution_run",
        "pub fn from_execution_run",
    ),
    "shared-types/src/arbitrage.rs": ("pub workflow_view: crate::workflow::HedgeTicketView,",),
    "shared-types/src/execution_run.rs": (
        "pub const EXECUTION_RUN_EVIDENCE_SCHEMA_VERSION: u8 = 2;",
        "pub hedge_ticket_view: Option<HedgeTicketView>",
    ),
    "crates/api/src/services/hedge_preview/workflow_view.rs": (
        "pub(super) fn project(",
        "market: market_health(quote)",
        "balance: preflight_health(",
        "capability: capability_health(",
    ),
    "crates/api/src/services/hedge_preview/workflow_view/health.rs": (
        "pub(super) fn market_health",
        "pub(super) fn fee_health",
        "pub(super) fn preflight_health",
        "pub(super) fn capability_health",
    ),
    "crates/api/src/services/hedge_preview.rs": (
        "workflow_view::project(&ticket, &ticket_order_plans",
        "workflow_view,",
    ),
    "crates/api/src/services/execution_runs/query.rs": (
        "refresh_workflow_view(&mut run)",
        ".max(shared_types::EXECUTION_RUN_EVIDENCE_SCHEMA_VERSION);",
        "HedgeTicketView::from_execution_run(run)",
    ),
    "crates/api/src/services/execution_runs/project.rs": ("refresh_workflow_view(run)",),
    "frontend/src/panels/modules/execution/data/workflow.rs": (
        'const WORKFLOW_VIEW_KEY: &str = "crossline.execution.hedgeTicketView";',
        "pub(super) const REST_RUN_SOURCE: WorkflowViewSource = WorkflowViewSource::RestRunSnapshot;",
        "pub(super) const WS_RUN_SOURCE: WorkflowViewSource = WorkflowViewSource::WsRunDelta;",
        'Self::RestRunSnapshot => "REST 运行单快照"',
        'Self::WsRunDelta => "WS 运行单增量"',
    ),
    "frontend/src/panels/modules/execution/data/run/context.rs": (
        "empty_selection_restores_durable_run_context",
    ),
    "frontend/src/panels/modules/execution/data/run.rs": (
        "apply_run_candidate(workflow, candidate, REST_RUN_SOURCE);",
        "apply_run_candidate(workflow, candidate, WS_RUN_SOURCE);",
    ),
    "frontend/src/panels/modules/execution/components/workflow_status.rs": (
        'data-testid="hedge-workflow-status"',
        "data-workflow-source=",
        "data-health-status=",
    ),
}
for path, required in markers.items():
    source = (root / path).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"missing {path} marker: {marker}")

browser = (root / "test/e2e/pr_bo_workflow_recovery.spec.ts").read_text(encoding="utf-8")
if re.search(r"\btest\.(?:skip|fixme)\s*\(", browser):
    fail("workflow recovery browser fixture must not be skipped")
for marker in (
    "page.routeWebSocket",
    'event: "execution_run_updated"',
    '"本地票据快照"',
    '"REST 运行单快照"',
    '"WS 运行单增量"',
    'for (const kind of ["market", "fee", "balance", "capability"]',
):
    if marker not in browser:
        fail(f"product browser marker drifted: {marker}")

package = json.loads((root / "package.json").read_text(encoding="utf-8"))
expected_script = "playwright test test/e2e/pr_bo_workflow_recovery.spec.ts"
if package.get("scripts", {}).get("test:e2e:pr-bo") != expected_script:
    fail("package test:e2e:pr-bo script drifted")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
for artifact in evidence_contract.values():
    if coverage.get(artifact, {}).get("coverage_status") != "exact":
        fail(f"exact coverage missing for {artifact}")

print(
    f"OK PR-BO static contract ({len(evidence_contract)} evidence rows; "
    "ticket health, durable REST/WS recovery and non-skipping product evidence)"
)
PY

bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"

if [[ "${PR_BO_SKIP_UPSTREAM:-0}" != "1" ]]; then
  PR_EA_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_ea_completion.sh"
  PR_DF_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_df_completion.sh"
  PR_DW_SKIP_TESTS=1 PR_DW_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_dw_completion.sh"
fi

if [[ "${PR_BO_SKIP_TESTS:-0}" != "1" ]]; then
  jobs="${CARGO_BUILD_JOBS:-10}"
  CARGO_BUILD_JOBS="$jobs" cargo test -p shared-types workflow --lib --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api workflow_view --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib workflow --no-fail-fast
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-bo -- --workers=1
fi

printf 'PR-BO Product Workflow Runtime and Feedback Loop completion gate passed\n'
