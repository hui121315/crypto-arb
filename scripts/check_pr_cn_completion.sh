#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
DOC="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
EVIDENCE="$ROOT/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
BROWSER="$ROOT/test/e2e/pr_cn_workspace_runtime.spec.ts"
RUNTIME="$ROOT/frontend/src/panels/modules/execution/data/runtime.rs"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-CN completion gate failed: %s\n' "$1" >&2
  exit 1
}

if [[ "$MODE" == "--self-test" ]]; then
  backup_dir="$(mktemp -d "${TMPDIR:-/tmp}/crossline-pr-cn.XXXXXX")"
  for file in "$DOC" "$EVIDENCE" "$BROWSER" "$RUNTIME"; do
    cp "$file" "$backup_dir/$(basename "$file")"
  done
  restore() {
    cp "$backup_dir/$(basename "$DOC")" "$DOC"
    cp "$backup_dir/$(basename "$EVIDENCE")" "$EVIDENCE"
    cp "$backup_dir/$(basename "$BROWSER")" "$BROWSER"
    cp "$backup_dir/$(basename "$RUNTIME")" "$RUNTIME"
  }
  cleanup() {
    restore
    rm -rf "$backup_dir"
  }
  trap cleanup EXIT

  PR_CN_SKIP_TESTS=1 PR_CN_SKIP_UPSTREAM=1 bash "$0" >/dev/null

  python3 - "$DOC" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "| `PR-CN Frontend Workstation State & Navigation Runtime Contract` | ✅ 完成 |"
if source.count(marker) != 1:
    raise SystemExit("PR-CN self-test setup failed: roadmap row drifted")
path.write_text(source.replace(marker, marker.replace("✅ 完成", "🟡 部分完成"), 1), encoding="utf-8")
PY
  if PR_CN_SKIP_TESTS=1 PR_CN_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a downgraded roadmap row"
  fi
  restore

  python3 - "$EVIDENCE" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
rows = path.read_text(encoding="utf-8").splitlines()
for index, row in enumerate(rows):
    if row.startswith("PR-CN\tworkspace-runtime\t"):
        rows.pop(index)
        path.write_text("\n".join(rows) + "\n", encoding="utf-8")
        break
else:
    raise SystemExit("PR-CN self-test setup failed: evidence row missing")
PY
  if PR_CN_SKIP_TESTS=1 PR_CN_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted an incomplete evidence matrix"
  fi
  restore

  python3 - "$BROWSER" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = 'test("PR-CN preserves execution draft pending evidence and outcome across unmount"'
if source.count(marker) != 1:
    raise SystemExit("PR-CN self-test setup failed: browser marker drifted")
path.write_text(source.replace(marker, marker.replace("test(", "test.skip("), 1), encoding="utf-8")
PY
  if PR_CN_SKIP_TESTS=1 PR_CN_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a skipped browser fixture"
  fi
  restore

  python3 - "$RUNTIME" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "    draft_inputs: DraftInputs,\n"
if source.count(marker) != 1:
    raise SystemExit("PR-CN self-test setup failed: runtime marker drifted")
path.write_text(source.replace(marker, "", 1), encoding="utf-8")
PY
  if PR_CN_SKIP_TESTS=1 PR_CN_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted execution inputs outside workstation runtime"
  fi
  restore

  python3 - "$DOC" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "### 🟡 6.5 下一步执行队列"
if source.count(marker) != 1:
    raise SystemExit("PR-CN self-test setup failed: queue heading drifted")
stale = "\n\n1. **PR-CN Frontend Workstation State & Navigation Runtime Contract** — stale completed row"
path.write_text(source.replace(marker, marker + stale, 1), encoding="utf-8")
PY
  if PR_CN_SKIP_TESTS=1 PR_CN_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted completed PR-CN at the queue head"
  fi

  printf 'PR-CN completion self-test passed\n'
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
title = "PR-CN Frontend Workstation State & Navigation Runtime Contract"
verify_anchor = "`bash scripts/check_pr_cn_completion.sh --self-test`"
authorities = (
    "PR-BO Product Workflow Runtime & Feedback Loop",
    "PR-CK Frontend High-Risk ActionState Boundary",
    "PR-DL Frontend Transport Runtime & ApiProblem Contract",
    "PR-DS Realtime WS Multiplexer & Replay Contract",
    "PR-DX Opportunity Runtime Contract & Frontend ViewModel Unification",
    "PR-FT Route Surface, CI/E2E Evidence Gate & Product Boundary Contract",
)
evidence_contract = {
    "workspace-runtime": ("frontend/src/panels/workstation.rs", "workstation"),
    "typed-deep-link": ("frontend/src/panels/routing.rs", "routing::tests"),
    "module-runtime-state": ("frontend/src/state/module_runtime.rs", "module_runtime"),
    "restored-debounce": ("frontend/src/state/polling.rs", "polling"),
    "opportunity-runtime": ("frontend/src/panels/modules/opportunities/data/runtime.rs", "opportunities"),
    "futures-runtime": ("frontend/src/panels/modules/futures/data/runtime.rs", "futures"),
    "positions-runtime": ("frontend/src/panels/modules/positions/data/runtime.rs", "positions"),
    "review-runtime": ("frontend/src/panels/modules/review/data.rs", "review"),
    "execution-runtime": ("frontend/src/panels/modules/execution/data/runtime.rs", "execution"),
    "execution-draft": ("frontend/src/panels/modules/execution/draft.rs", "execution"),
    "draft-inputs": ("frontend/src/panels/modules/execution/draft/inputs.rs", "execution"),
    "draft-input-tests": ("frontend/src/panels/modules/execution/draft/inputs/tests.rs", "execution::draft::inputs::tests"),
    "preview-runtime": ("frontend/src/panels/modules/execution/data/preview.rs", "execution"),
    "confirm-action-runtime": ("frontend/src/panels/modules/execution/data/actions.rs", "execution"),
    "confirm-outcome-runtime": ("frontend/src/panels/modules/execution/data/outcome.rs", "execution"),
    "confirm-outcome-tests": ("frontend/src/panels/modules/execution/data/outcome/tests.rs", "execution::data::outcome::tests"),
    "run-route-context": ("frontend/src/panels/modules/execution/data/run/context.rs", "execution"),
    "navigation-badges": ("frontend/src/panels/nav/view.rs", "nav::view::tests"),
    "navigation-skin": ("frontend/styles/src/skin/shell.css", "product_ui_perf_gate.sh"),
    "transport-single-base": ("frontend/src/state/context.rs", "api::base::tests"),
    "rest-ws-base-derivation": ("frontend/src/api/base.rs", "api::base::tests"),
    "url-search-contract": ("frontend/Cargo.toml", "wasm32-unknown-unknown"),
    "url-search-feature-budget": ("scripts/dependency_feature_budget.tsv", "check_dependency_feature_budget.sh"),
    "workspace-browser": ("test/e2e/pr_cn_workspace_runtime.spec.ts", "test:e2e:pr-cn"),
    "workflow-authority": ("scripts/check_pr_bo_completion.sh", "check_pr_bo_completion.sh"),
    "action-state-authority": ("scripts/check_pr_ck_completion.sh", "check_pr_ck_completion.sh"),
    "transport-authority": ("scripts/check_pr_dl_completion.sh", "check_pr_dl_completion.sh"),
    "ws-multiplexer-authority": ("scripts/check_pr_ds_completion.sh", "check_pr_ds_completion.sh"),
    "repo-gate-wiring": ("scripts/verify_repo_gates.sh", "verify_repo_gates.sh"),
    "completion-governance": ("scripts/check_pr_cn_completion.sh", "check_pr_cn_completion.sh --self-test"),
}
browser_titles = (
    "PR-CN restores symbol strategy and cursor deep link across module switches",
    "PR-CN routes explicit opportunity and run context into the execution query",
    "PR-CN preserves execution draft pending evidence and outcome across unmount",
    "PR-CN exposes one typed problem inline in navigation and global toast",
)
source_markers = {
    "frontend/src/panels/workstation.rs": ("pub struct WorkspaceRuntime", "bind_runtime_problem_toasts"),
    "frontend/src/panels/routing.rs": ("pub(crate) struct WorkspaceRoute", '"symbol"', '"run"'),
    "frontend/src/state/module_runtime.rs": ("pub(crate) struct ModuleRuntimeState", "ModuleRuntimeStatus::Pending"),
    "frontend/src/panels/modules/execution/data/runtime.rs": ("    draft_inputs: DraftInputs,\n", "preview_state", "ConfirmActionRuntime"),
    "frontend/src/panels/modules/execution/data/outcome.rs": ("basic_confirm_outcome_detail", 'HedgeConfirmStatus::Submitted => "已提交"'),
    "frontend/src/panels/nav/view.rs": ("data-runtime-state", "module-tab-status"),
    "frontend/styles/src/skin/shell.css": ('data-state="pending"', "var(--color-orange)"),
    "frontend/src/state/context.rs": ("provide_ws_runtime", "with_base_signal_and_auth"),
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-CN completion gate failed: {message}")


def require_incomplete_queue_head(doc: str, queue: str) -> None:
    matched = re.search(r"(?m)^1\.\s+\*\*(PR-[A-Z]+)\b", queue)
    if not matched:
        fail("local queue must retain a PR roadmap item at its head")
    head = matched.group(1)
    row = next((line for line in doc.splitlines() if line.startswith(f"| `{head} ")), None)
    if row is None or "✅ 完成" in row:
        fail(f"local queue head must be an incomplete roadmap row: {head}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
start = doc.find("### 🟡 6.3")
end = doc.find("### 🟡 6.4", start)
if start < 0 or end < 0:
    fail("bounded roadmap section is missing")
roadmap = doc[start:end]


def roadmap_row(row_title: str) -> str:
    rows = [line for line in roadmap.splitlines() if line.startswith(f"| `{row_title}`")]
    if len(rows) != 1:
        fail(f"expected one roadmap row for {row_title}")
    return rows[0]


row = roadmap_row(title)
if "✅ 完成" not in row or "剩余：无" not in row or row.count(verify_anchor) != 1:
    fail("roadmap row must be complete, remaining-none and bound to the destructive gate")
for authority in authorities:
    if "✅ 完成" not in roadmap_row(authority):
        fail(f"successor authority is not complete: {authority}")

queue = doc[doc.find("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-CN\b", queue):
    fail("completed PR-CN remains in the local queue")
successor = roadmap_row("PR-CQ Local Runtime & Operator QA Contract")
successor_complete = "✅ 完成" in successor and "剩余：无" in successor
successor_is_head = bool(re.search(r"(?m)^1\.\s+\*\*PR-CQ\b", queue))
if successor_complete:
    if successor_is_head:
        fail("completed PR-CQ successor remains at the local queue head")
    require_incomplete_queue_head(doc, queue)
elif not successor_is_head:
    fail("PR-CQ must remain the local queue head until its completion contract closes")

history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(encoding="utf-8")
if "## 2026-07-16 PR-CN Frontend Workstation State and Navigation Runtime Closure" not in history:
    fail("history closure appendix is missing")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-CN"]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for kind, (artifact, command_anchor) in evidence_contract.items():
    evidence = indexed[kind]
    if evidence["artifact"] != artifact or command_anchor not in evidence["command"]:
        fail(f"evidence anchor drifted: {kind}")
    if not (root / artifact).is_file():
        fail(f"evidence artifact is missing: {artifact}")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
for artifact, _ in evidence_contract.values():
    item = coverage.get(artifact)
    if item is None or item["coverage_status"] != "exact" or not item["evidence"].startswith("line:"):
        fail(f"exact coverage missing for {artifact}")

for relative, markers in source_markers.items():
    source = (root / relative).read_text(encoding="utf-8")
    for marker in markers:
        if marker not in source:
            fail(f"source marker missing: {relative}:{marker}")

browser = (root / "test/e2e/pr_cn_workspace_runtime.spec.ts").read_text(encoding="utf-8")
for title in browser_titles:
    escaped = re.escape(title)
    if re.search(rf'test\.(?:skip|fixme)\(\s*["\']{escaped}["\']', browser):
        fail(f"browser anchor is skipped: {title}")
    if not re.search(rf'test\(\s*["\']{escaped}["\']', browser):
        fail(f"browser anchor is missing: {title}")

product = json.loads((root / "package.json").read_text(encoding="utf-8"))
release = json.loads((root / "scripts/fixtures/release_qa_contract/package.json").read_text(encoding="utf-8"))
path = "test/e2e/pr_cn_workspace_runtime.spec.ts"
if product.get("scripts", {}).get("test:e2e:pr-cn") != f"playwright test {path}":
    fail("dedicated PR-CN browser command drifted")
for name, package in (("product", product), ("release", release)):
    if package.get("scripts", {}).get("test:e2e:product", "").count(path) != 1:
        fail(f"{name} product suite must include PR-CN exactly once")

repo_gate = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
if repo_gate.count("check_pr_cn_completion.sh") != 2:
    fail("repo gate must execute PR-CN exactly once in docs and all scopes")

print(
    f"OK PR-CN static contract ({len(evidence_contract)} evidence types; "
    f"{len(browser_titles)} browser anchors)"
)
PY

npx playwright test "$BROWSER" --list >/dev/null

if [[ "${PR_CN_SKIP_UPSTREAM:-0}" != "1" ]]; then
  PR_BO_SKIP_TESTS=1 PR_BO_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_bo_completion.sh"
  PR_CK_SKIP_TESTS=1 PR_CK_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_ck_completion.sh"
  PR_DL_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dl_completion.sh"
  PR_DS_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_ds_completion.sh"
  bash "$ROOT/scripts/check_product_audit_progress.sh"
  bash "$ROOT/scripts/check_product_audit_evidence.sh"
  bash "$ROOT/scripts/check_product_audit_evidence_index.sh"
  bash "$ROOT/scripts/check_product_audit_coverage.sh"
  bash "$ROOT/scripts/check_release_qa_contract.sh"
fi

if [[ "${PR_CN_SKIP_TESTS:-0}" != "1" ]]; then
  jobs="${CARGO_BUILD_JOBS:-10}"
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" \
    --lib panels::routing --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" \
    --lib state::module_runtime --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" \
    --lib execution::data::outcome --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" \
    --lib execution::data::run::context --no-fail-fast
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-cn -- --workers=1
fi

printf 'PR-CN frontend workstation state and navigation runtime completion passed\n'
