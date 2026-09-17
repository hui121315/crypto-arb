#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
AUDIT="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
HISTORY="$ROOT/docs/audit_history/PRODUCT_AUDIT_HISTORY.md"
EVIDENCE="$ROOT/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
SHARED="$ROOT/shared-types/src/actions/evidence.rs"
POSITIONS="$ROOT/frontend/src/panels/modules/positions/data/actions.rs"
COMPENSATION="$ROOT/frontend/src/panels/modules/positions/data/actions/compensation.rs"
BROWSER="$ROOT/test/e2e/pr_bd_action_scope.spec.ts"
PACKAGE="$ROOT/package.json"
REPO_GATE="$ROOT/scripts/verify_repo_gates.sh"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-BD completion gate failed: %s\n' "$1" >&2
  exit 1
}

run_static_gate() {
  PR_BD_STATIC_ONLY=1 bash "$0" >/dev/null
}

if [[ "$MODE" == "--self-test" ]]; then
  tmp="$(mktemp -d "${TMPDIR:-/tmp}/crossline-pr-bd.XXXXXX")"
  files=("$AUDIT" "$EVIDENCE" "$SHARED" "$POSITIONS" "$COMPENSATION" "$BROWSER" "$PACKAGE" "$REPO_GATE")
  for file in "${files[@]}"; do
    cp "$file" "$tmp/$(basename "$file").$(printf '%s' "$file" | shasum | cut -c1-12)"
  done
  restore_file() {
    local file="$1"
    cp "$tmp/$(basename "$file").$(printf '%s' "$file" | shasum | cut -c1-12)" "$file"
  }
  restore_all() {
    for file in "${files[@]}"; do restore_file "$file"; done
    rm -rf "$tmp"
  }
  trap restore_all EXIT

  mutate_once() {
    local file="$1" old="$2" new="$3"
    python3 - "$file" "$old" "$new" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
old, new = sys.argv[2:]
text = path.read_text(encoding="utf-8")
if old not in text:
    raise SystemExit(f"PR-BD self-test setup failed: marker missing in {path}")
path.write_text(text.replace(old, new, 1), encoding="utf-8")
PY
  }
  assert_rejected() {
    local file="$1" old="$2" new="$3" message="$4"
    mutate_once "$file" "$old" "$new"
    if run_static_gate 2>/dev/null; then fail "$message"; fi
    restore_file "$file"
  }

  run_static_gate
  assert_rejected "$AUDIT" \
    '| `PR-BD Frontend ActionState & High-Risk Button Boundary` | ✅ 完成 |' \
    '| `PR-BD Frontend ActionState & High-Risk Button Boundary` | 🟡 部分完成 |' \
    'self-test accepted a downgraded roadmap row'
  assert_rejected "$EVIDENCE" $'PR-BD\tshared-action-evidence\t' \
    $'PR-BD-MISSING\tshared-action-evidence\t' \
    'self-test accepted missing exact evidence'
  assert_rejected "$SHARED" 'pub action_kind: Option<ActionRunKind>' \
    'pub removed_action_kind: Option<ActionRunKind>' \
    'self-test accepted missing typed action scope'
  assert_rejected "$SHARED" 'pub venues: Vec<String>' 'pub removed_venues: Vec<String>' \
    'self-test accepted hidden venue scope'
  assert_rejected "$POSITIONS" 'ActionRunKind::PortfolioClosePair' \
    'ActionRunKind::PortfolioClosePosition' \
    'self-test accepted wrong pair action kind'
  assert_rejected "$POSITIONS" 'ActionRunKind::PortfolioCloseAll' \
    'ActionRunKind::PortfolioClosePair' \
    'self-test accepted wrong close-all scope'
  assert_rejected "$COMPENSATION" 'ActionRunKind::PortfolioCloseCompensation' \
    'ActionRunKind::PortfolioClosePosition' \
    'self-test accepted lost compensation identity'
  assert_rejected "$BROWSER" 'test("PR-BD keeps typed scope and identity for partial paired close"' \
    'test.skip("PR-BD keeps typed scope and identity for partial paired close"' \
    'self-test accepted a skipped product fixture'
  assert_rejected "$PACKAGE" '"test:e2e:pr-bd": "playwright test test/e2e/pr_bd_action_scope.spec.ts"' \
    '"test:e2e:pr-bd": "true"' \
    'self-test accepted a skipped browser command'
  assert_rejected "$AUDIT" $'### 🟡 6.5 下一步执行队列\n' \
    $'### 🟡 6.5 下一步执行队列\n\n1. **PR-BD Frontend ActionState & High-Risk Button Boundary** — stale\n' \
    'self-test accepted PR-BD reinserted at queue head'
  assert_rejected "$AUDIT" \
    '| `PR-BE High-Risk Audit Trail & Request Correlation` | ✅ 完成 |' \
    '| `PR-BE High-Risk Audit Trail & Request Correlation` | 🟡 部分完成 |' \
    'self-test accepted a downgraded successor outside the local queue'
  assert_rejected "$REPO_GATE" 'PR_BD_STATIC_ONLY=1 bash "$ROOT/scripts/check_pr_bd_completion.sh"' \
    'true # PR-BD gate removed' \
    'self-test accepted single-scope repository wiring'

  printf 'PR-BD completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from pathlib import Path
import csv
import json
import re
import sys

root = Path(sys.argv[1])
audit = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(encoding="utf-8")


def fail(message: str) -> None:
    raise SystemExit(f"PR-BD completion gate failed: {message}")


title = "PR-BD Frontend ActionState & High-Risk Button Boundary"
row = next((line for line in audit.splitlines() if line.startswith(f"| `{title}` |")), None)
if row is None or "| ✅ 完成 |" not in row or "剩余：无。" not in row:
    fail("roadmap row is not complete with no local remainder")
if "`bash scripts/check_pr_bd_completion.sh --self-test`" not in row:
    fail("roadmap row lacks the destructive verification anchor")
if "> 本轮 PR-BD：" not in audit:
    fail("top progress summary lacks the PR-BD closure note")
queue = audit.split("### 🟡 6.5 下一步执行队列", 1)[-1]
numbered_lines = re.findall(r"(?m)^\d+\. \*\*(PR-[A-Z]+)([^\n]*)", queue)
numbered = [pr_id for pr_id, _ in numbered_lines]
if "PR-BD" in numbered or len(numbered) != len(set(numbered)):
    fail(f"queue handoff drifted: {numbered}")
successor_title = "PR-BE High-Risk Audit Trail & Request Correlation"
successor_row = next(
    (line for line in audit.splitlines() if line.startswith(f"| `{successor_title}` |")),
    None,
)
successor_complete = successor_row is not None and "| ✅ 完成 |" in successor_row
if successor_complete:
    if "PR-BE" in numbered or any("外部等待" not in line for _, line in numbered_lines):
        fail(f"completed successor left a local queue item: {numbered}")
    queue_state = "external-only" if numbered else "empty"
elif not numbered or numbered[0] != "PR-BE":
    fail(f"unfinished successor is not the local queue head: {numbered}")
else:
    queue_state = numbered[0]
if "## 2026-07-19 PR-BD Action Scope and High-Risk Boundary Closure" not in history:
    fail("history closure appendix is missing")

contract = {
    "action-boundary-authority": "scripts/check_pr_ck_completion.sh",
    "durable-action-authority": "scripts/check_pr_fi_completion.sh",
    "close-run-authority": "scripts/check_pr_dz_completion.sh",
    "typed-state-authority": "scripts/check_pr_by_completion.sh",
    "shared-action-kind": "shared-types/src/actions.rs",
    "shared-action-evidence": "shared-types/src/actions/evidence.rs",
    "shared-evidence-conversions": "shared-types/src/actions/evidence/conversions.rs",
    "shared-evidence-tests": "shared-types/src/actions/evidence/tests.rs",
    "shared-execution-partial": "shared-types/src/hedge.rs",
    "frontend-action-state": "frontend/src/state/action_state.rs",
    "mutation-request-context": "frontend/src/api/rest/transport.rs",
    "positions-action-hooks": "frontend/src/panels/modules/positions/data/actions.rs",
    "positions-compensation-hooks": "frontend/src/panels/modules/positions/data/actions/compensation.rs",
    "positions-request-contract": "frontend/src/panels/modules/positions/data/requests/close.rs",
    "positions-close-run-state": "frontend/src/panels/modules/positions/data/runs.rs",
    "positions-recovery": "frontend/src/panels/modules/positions/data/runs/recovery.rs",
    "positions-scope-tests": "frontend/src/panels/modules/positions/data/tests/close/action_scope.rs",
    "positions-view-boundary": "frontend/src/panels/modules/positions/view.rs",
    "positions-risk-boundary": "frontend/src/panels/modules/positions/components/kill_switch_bar.rs",
    "execution-partial-outcome": "frontend/src/panels/modules/execution/data/actions/outcome.rs",
    "settings-action-boundary": "frontend/src/panels/modules/settings/data/actions.rs",
    "product-browser": "test/e2e/pr_bd_action_scope.spec.ts",
    "package-product-suite": "package.json",
    "release-qa-product-suite": "scripts/check_release_qa_contract.sh",
    "release-qa-fixture": "scripts/fixtures/release_qa_contract/package.json",
    "repository-aggregate": "scripts/verify_repo_gates.sh",
    "completion-governance": "scripts/check_pr_bd_completion.sh",
}
with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [item for item in csv.DictReader(handle, delimiter="\t") if item["pr_id"] == "PR-BD"]
indexed = {item["evidence_type"]: item for item in rows}
if len(indexed) != len(rows) or set(indexed) != set(contract):
    fail(f"evidence type drift: expected={sorted(contract)}, actual={sorted(indexed)}")
for evidence_type, artifact in contract.items():
    item = indexed[evidence_type]
    if item["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"evidence artifact drifted or missing: {evidence_type}")
    if not item["command"].strip() or not item["notes"].strip():
        fail(f"evidence lacks command or notes: {evidence_type}")
with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {item["file"]: item for item in csv.DictReader(handle, delimiter="\t")}
for artifact in set(contract.values()):
    if artifact.endswith(".json"):
        continue
    item = coverage.get(artifact)
    if item is None or item["coverage_status"] != "exact" or not item["evidence"].startswith("line:"):
        fail(f"exact coverage missing for {artifact}")

markers = {
    "shared-types/src/actions.rs": ("pub const fn as_str", "PortfolioCloseManualTerminal"),
    "shared-types/src/actions/evidence.rs": ("pub action_kind: Option<ActionRunKind>", "pub venues: Vec<String>", "pub symbols: Vec<String>"),
    "shared-types/src/actions/evidence/conversions.rs": ("with_action_kind(close_run_action_kind(run))", "ActionRunKind::HedgeConfirm"),
    "frontend/src/panels/modules/positions/data/actions.rs": (
        "position_scope_evidence(&context, ActionRunKind::PortfolioClosePosition, &row)",
        "position_scope_evidence(&context, ActionRunKind::PortfolioClosePair, &row)",
        ".with_action_kind(ActionRunKind::PortfolioCloseAll)",
        "ActionRunKind::TradingKillSwitch",
    ),
    "frontend/src/panels/modules/positions/data/actions/compensation.rs": ("ActionRunKind::PortfolioCloseCompensation", "ActionRunKind::PortfolioCloseManualTerminal"),
    "frontend/src/panels/modules/positions/data/runs.rs": ("ActionEvidence::from_close_run", "kill_switch_response_evidence"),
    "frontend/src/panels/modules/positions/data/runs/recovery.rs": ("recover_position_close_state", "recover_close_all_state"),
}
for relative, required in markers.items():
    text = (root / relative).read_text(encoding="utf-8")
    for marker in required:
        if marker not in text:
            fail(f"runtime contract lost marker in {relative}: {marker}")
for relative in ("frontend/src/panels/modules/positions/view.rs", "frontend/src/panels/modules/positions/components/kill_switch_bar.rs"):
    text = (root / relative).read_text(encoding="utf-8")
    if re.search(r"client\.(?:close|set_|kill)|spawn_local", text):
        fail(f"component boundary bypassed data hooks: {relative}")

browser = (root / "test/e2e/pr_bd_action_scope.spec.ts").read_text(encoding="utf-8")
if re.search(r"\btest\.(?:skip|fixme)\(", browser) or browser.count('test("PR-BD') != 2:
    fail("browser contract is skipped or no longer has exactly two product fixtures")
for marker in ("action_kind portfolio_close_pair", "action_kind portfolio_close_all", "action_kind trading_kill_switch", "venue binance,okx"):
    if marker not in browser:
        fail(f"browser evidence marker missing: {marker}")
package = json.loads((root / "package.json").read_text(encoding="utf-8"))
if package["scripts"].get("test:e2e:pr-bd") != "playwright test test/e2e/pr_bd_action_scope.spec.ts":
    fail("dedicated PR-BD browser command drifted")
if package["scripts"].get("test:e2e:product", "").count("test/e2e/pr_bd_action_scope.spec.ts") != 1:
    fail("product browser suite must include PR-BD exactly once")
release = (root / "scripts/check_release_qa_contract.sh").read_text(encoding="utf-8")
if '.scripts["test:e2e:pr-bd"]' not in release or "test/e2e/pr_bd_action_scope.spec.ts" not in release:
    fail("release QA no longer locks PR-BD browser evidence")
repo = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
if repo.count('PR_BD_STATIC_ONLY=1 bash "$ROOT/scripts/check_pr_bd_completion.sh"') != 2:
    fail("completion gate must be wired into both repository scopes")
if repo.count('npx playwright test "$ROOT/test/e2e/pr_bd_action_scope.spec.ts" --list') != 1:
    fail("repository browser fixture listing is missing")

print(f"OK PR-BD action scope contract ({len(contract)} evidence types; queue={queue_state})")
PY

if [[ "${PR_BD_STATIC_ONLY:-0}" == "1" ]]; then exit 0; fi

bash "$ROOT/scripts/check_product_audit_progress.sh"
bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_evidence_index.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"
bash "$ROOT/scripts/check_release_qa_contract.sh" --self-test
PR_CK_SKIP_TESTS=1 PR_CK_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_ck_completion.sh"
bash "$ROOT/scripts/check_pr_fi_completion.sh"
PR_DZ_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dz_completion.sh"
PR_BY_SKIP_TESTS=1 PR_BY_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_by_completion.sh"

if [[ "${PR_BD_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS=8 cargo test -p shared-types actions --lib --no-fail-fast
  CARGO_BUILD_JOBS=8 cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib positions --no-fail-fast
fi
if [[ "${PR_BD_SKIP_BROWSER:-0}" != "1" ]]; then
  CI=1 npm run test:e2e:pr-bd -- --workers=1
fi

printf 'OK PR-BD ActionState and high-risk boundary contract\n'
