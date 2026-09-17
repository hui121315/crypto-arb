#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
AUDIT="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
EVIDENCE="$ROOT/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
EVENTS="$ROOT/crates/review/src/realized_pnl/events.rs"
COSTS="$ROOT/crates/review/src/realized_pnl/close_run_costs.rs"
STORAGE="$ROOT/crates/api/src/services/review/storage_health.rs"
PAGING="$ROOT/crates/api/src/services/review/paging.rs"
DETAIL="$ROOT/frontend/src/panels/modules/review/components/executed_ledger_detail.rs"
BROWSER="$ROOT/test/e2e/pr_bc_review_closure.spec.ts"
PACKAGE="$ROOT/package.json"
RELEASE="$ROOT/scripts/check_release_qa_contract.sh"
REPO_GATE="$ROOT/scripts/verify_repo_gates.sh"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-BC completion gate failed: %s\n' "$1" >&2
  exit 1
}

run_static_gate() {
  PR_BC_STATIC_ONLY=1 bash "$0" >/dev/null
}

if [[ "$MODE" == "--self-test" ]]; then
  tmp="$(mktemp -d "${TMPDIR:-/tmp}/crossline-pr-bc.XXXXXX")"
  files=("$AUDIT" "$EVIDENCE" "$EVENTS" "$COSTS" "$STORAGE" "$PAGING" "$DETAIL" "$BROWSER" "$PACKAGE" "$RELEASE" "$REPO_GATE")
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
    raise SystemExit(f"PR-BC self-test setup failed: marker missing in {path}")
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
    '| `PR-BC Review Fact Source & PnL Semantics` | ✅ 完成 |' \
    '| `PR-BC Review Fact Source & PnL Semantics` | 🟡 部分完成 |' \
    'self-test accepted a downgraded roadmap row'
  assert_rejected "$EVIDENCE" $'PR-BC\tterminal-fill-authority\t' \
    $'PR-BC-MISSING\tterminal-fill-authority\t' \
    'self-test accepted missing exact evidence'
  assert_rejected "$EVENTS" '&& snapshot.confidence.supports_terminal_fill()' \
    '&& true' \
    'self-test accepted non-terminal fill projection'
  assert_rejected "$DETAIL" 'close_fee:{} close_slip:{} comp_fee:{} comp_slip:{}' \
    'cost_total:{}' \
    'self-test accepted hidden field-level close and unwind costs'
  assert_rejected "$STORAGE" 'if sql_ledger_storage_configured(&sql_snapshot) {' \
    'if false && sql_ledger_storage_configured(&sql_snapshot) {' \
    'self-test accepted disabled SQL storage health'
  assert_rejected "$PAGING" 'max_limit: REVIEW_MAX_LIMIT,' \
    'max_limit: usize::MAX,' \
    'self-test accepted an unbounded review page contract'
  assert_rejected "$BROWSER" 'test("PR-BC explains terminal review facts and every close-unwind cost component"' \
    'test.skip("PR-BC explains terminal review facts and every close-unwind cost component"' \
    'self-test accepted a skipped product fixture'
  assert_rejected "$PACKAGE" '"test:e2e:pr-bc": "playwright test test/e2e/pr_bc_review_closure.spec.ts"' \
    '"test:e2e:pr-bc": "true"' \
    'self-test accepted a skipped browser command'
  assert_rejected "$AUDIT" $'### 🟡 6.5 下一步执行队列\n' \
    $'### 🟡 6.5 下一步执行队列\n\n1. **PR-BC Review Fact Source & PnL Semantics** — stale\n' \
    'self-test accepted PR-BC reinserted at queue head'
  assert_rejected "$AUDIT" \
    '| `PR-BE High-Risk Audit Trail & Request Correlation` | ✅ 完成 |' \
    '| `PR-BE High-Risk Audit Trail & Request Correlation` | 🟡 部分完成 |' \
    'self-test accepted a downgraded terminal successor outside the local queue'
  assert_rejected "$REPO_GATE" 'PR_BC_STATIC_ONLY=1 bash "$ROOT/scripts/check_pr_bc_completion.sh"' \
    'true # PR-BC gate removed' \
    'self-test accepted single-scope repository wiring'

  printf 'PR-BC completion self-test passed\n'
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
    raise SystemExit(f"PR-BC completion gate failed: {message}")


title = "PR-BC Review Fact Source & PnL Semantics"
row = next((line for line in audit.splitlines() if line.startswith(f"| `{title}` |")), None)
if row is None or "| ✅ 完成 |" not in row or "剩余：无。" not in row:
    fail("roadmap row is not complete with no local remainder")
if "`bash scripts/check_pr_bc_completion.sh --self-test`" not in row:
    fail("roadmap row lacks the destructive verification anchor")
if "> 本轮 PR-BC：" not in audit:
    fail("top progress summary lacks the PR-BC closure note")

queue = audit.split("### 🟡 6.5 下一步执行队列", 1)[-1]
numbered_lines = re.findall(r"(?m)^\d+\. \*\*(PR-[A-Z]+)([^\n]*)", queue)
numbered = [pr_id for pr_id, _ in numbered_lines]
bd_title = "PR-BD Frontend ActionState & High-Risk Button Boundary"
bd_row = next((line for line in audit.splitlines() if line.startswith(f"| `{bd_title}` |")), "")
bd_complete = "| ✅ 完成 |" in bd_row
be_title = "PR-BE High-Risk Audit Trail & Request Correlation"
be_row = next((line for line in audit.splitlines() if line.startswith(f"| `{be_title}` |")), "")
be_complete = "| ✅ 完成 |" in be_row
if "PR-BC" in numbered or len(numbered) != len(set(numbered)):
    fail(f"queue handoff drifted: {numbered}")
if not bd_complete:
    if not numbered or numbered[0] != "PR-BD":
        fail(f"unfinished PR-BD is not the queue head: {numbered}")
    queue_state = numbered[0]
elif not be_complete:
    if "PR-BD" in numbered or not numbered or numbered[0] != "PR-BE":
        fail(f"unfinished PR-BE is not the queue head: {numbered}")
    queue_state = numbered[0]
else:
    if "PR-BD" in numbered or "PR-BE" in numbered:
        fail(f"completed successors remain in the queue: {numbered}")
    if any("外部等待" not in line for _, line in numbered_lines):
        fail(f"local queue item remains after PR-BE completion: {numbered}")
    queue_state = "external-only" if numbered else "empty"
if "## 2026-07-19 PR-BC Review Fact and PnL Closure" not in history:
    fail("history closure appendix is missing")

contract = {
    "terminal-fill-authority": "scripts/check_pr_dh_completion.sh",
    "runtime-review-authority": "scripts/check_pr_dw_completion.sh",
    "close-run-authority": "scripts/check_pr_dz_completion.sh",
    "execution-finality-authority": "scripts/check_pr_ea_completion.sh",
    "pnl-evidence-authority": "scripts/check_pr_bl_completion.sh",
    "shared-review-envelope": "shared-types/src/review.rs",
    "shared-ledger-evidence": "shared-types/src/review/evidence.rs",
    "terminal-fill-projection": "crates/review/src/realized_pnl/events.rs",
    "pnl-field-quality": "crates/review/src/realized_pnl.rs",
    "close-unwind-cost-projection": "crates/review/src/realized_pnl/close_run_costs.rs",
    "sql-first-review-reader": "crates/api/src/services/review/ledger.rs",
    "review-storage-health": "crates/api/src/services/review/storage_health.rs",
    "review-server-pagination": "crates/api/src/services/review/paging.rs",
    "review-request-correlation": "crates/api/src/routers/review.rs",
    "frontend-field-quality": "frontend/src/panels/modules/review/components/executed_tab.rs",
    "frontend-ledger-drilldown": "frontend/src/panels/modules/review/components/executed_ledger_detail.rs",
    "frontend-page-runtime": "frontend/src/panels/modules/review/data.rs",
    "product-browser": "test/e2e/pr_bc_review_closure.spec.ts",
    "package-product-suite": "package.json",
    "release-qa-product-suite": "scripts/check_release_qa_contract.sh",
    "release-qa-fixture": "scripts/fixtures/release_qa_contract/package.json",
    "repository-aggregate": "scripts/verify_repo_gates.sh",
    "completion-governance": "scripts/check_pr_bc_completion.sh",
}
with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [item for item in csv.DictReader(handle, delimiter="\t") if item["pr_id"] == "PR-BC"]
indexed = {item["evidence_type"]: item for item in rows}
if len(indexed) != len(rows) or set(indexed) != set(contract):
    fail(f"evidence type drift: expected={sorted(contract)}, actual={sorted(indexed)}")
for evidence_type, artifact in contract.items():
    item = indexed[evidence_type]
    if item["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"evidence artifact drifted or is missing: {evidence_type}")
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
    "shared-types/src/review.rs": (
        "pub request_id: Option<String>", "pub storage_health: Option<VenueOperationHealth>",
        "pub actual_fields: Vec<ReviewPnlField>", "pub missing_fields: Vec<ReviewPnlField>",
    ),
    "shared-types/src/review/evidence.rs": (
        "pub run_id: Option<String>", "pub ticket_id: Option<String>",
        "pub source: OrderUpdateSource", "pub close_run_evidence: Vec<ReviewCloseRunEvidence>",
    ),
    "crates/review/src/realized_pnl/events.rs": ("supports_terminal_fill()",),
    "crates/review/src/realized_pnl.rs": (
        "ReviewPnlField::Fee", "ReviewPnlField::Funding", "ReviewPnlField::Slippage", "close_run_cost_missing",
    ),
    "crates/review/src/realized_pnl/close_run_costs.rs": (
        "apply_close_run_costs", "CloseRunCostComponent::ManualHandling", "close_run_cost_missing",
    ),
    "crates/api/src/services/review/ledger.rs": ("list_sql_realized_window", "ReviewLedgerStatus::LedgerBacked"),
    "crates/api/src/services/review/storage_health.rs": (
        "if sql_ledger_storage_configured(&sql_snapshot) {", "with_execution_ledger_storage_health",
    ),
    "crates/api/src/services/review/paging.rs": (
        "max_limit: REVIEW_MAX_LIMIT", "previous_cursor", "next_cursor", "last_cursor",
    ),
    "crates/api/src/routers/review.rs": ("with_request_id(common::request_id::current())",),
    "frontend/src/panels/modules/review/components/executed_tab.rs": (
        'data-table-budget="server-page"', "ReviewPnlField::Funding", '"真实"',
    ),
    "frontend/src/panels/modules/review/components/executed_ledger_detail.rs": (
        "run:{} ticket:{} via {}", "close_fee:{} close_slip:{} comp_fee:{} comp_slip:{}",
        "funding:{} manual:{} total:{} missing:{}",
    ),
    "frontend/src/panels/modules/review/data.rs": ("review_executed_page", "load_cursor"),
}
for relative, required in markers.items():
    text = (root / relative).read_text(encoding="utf-8")
    for marker in required:
        if marker not in text:
            fail(f"runtime contract lost marker in {relative}: {marker}")

browser = (root / "test/e2e/pr_bc_review_closure.spec.ts").read_text(encoding="utf-8")
if re.search(r"\btest\.(?:skip|fixme)\(", browser) or browser.count('test("PR-BC') != 1:
    fail("browser contract is skipped or no longer has exactly one product fixture")
for marker in (
    "终态 2/2 Filled via 私有 WS/订单回查", "run:run-pr-bc ticket:ticket-pr-bc",
    "close_fee:$0.2/1 close_slip:$0.3/1", "total:$1.1 missing:none", 'get("limit") === "50"',
):
    if marker not in browser:
        fail(f"browser evidence marker missing: {marker}")

package = json.loads((root / "package.json").read_text(encoding="utf-8"))
dedicated = "playwright test test/e2e/pr_bc_review_closure.spec.ts"
if package["scripts"].get("test:e2e:pr-bc") != dedicated:
    fail("dedicated PR-BC browser command drifted")
if package["scripts"].get("test:e2e:product", "").count("test/e2e/pr_bc_review_closure.spec.ts") != 1:
    fail("product browser suite must include PR-BC exactly once")
release = (root / "scripts/check_release_qa_contract.sh").read_text(encoding="utf-8")
if '.scripts["test:e2e:pr-bc"]' not in release or "test/e2e/pr_bc_review_closure.spec.ts" not in release:
    fail("release QA no longer locks PR-BC browser evidence")
repo = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
if repo.count('PR_BC_STATIC_ONLY=1 bash "$ROOT/scripts/check_pr_bc_completion.sh"') != 2:
    fail("completion gate must be wired into both repository scopes")
if repo.count('npx playwright test "$ROOT/test/e2e/pr_bc_review_closure.spec.ts" --list') != 1:
    fail("repository browser fixture listing is missing")

print(f"OK PR-BC review fact and PnL contract ({len(contract)} evidence types; queue={queue_state})")
PY

if [[ "${PR_BC_STATIC_ONLY:-0}" == "1" ]]; then exit 0; fi

bash "$ROOT/scripts/check_product_audit_progress.sh"
bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_evidence_index.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"
bash "$ROOT/scripts/check_release_qa_contract.sh" --self-test
PR_DH_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dh_completion.sh"
PR_DW_SKIP_TESTS=1 PR_DW_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_dw_completion.sh"
PR_DZ_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dz_completion.sh"
PR_EA_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_ea_completion.sh"
PR_BL_SKIP_TESTS=1 PR_BL_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_bl_completion.sh"

if [[ "${PR_BC_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS=8 cargo test -p shared-types review --lib --no-fail-fast
  CARGO_BUILD_JOBS=8 cargo test -p review realized_pnl --lib --no-fail-fast
  CARGO_BUILD_JOBS=8 cargo test -p api --bin crypto-arb-api services::review --no-fail-fast
  CARGO_BUILD_JOBS=8 cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib review --no-fail-fast
fi
if [[ "${PR_BC_SKIP_BROWSER:-0}" != "1" ]]; then
  CI=1 npm run test:e2e:pr-bc -- --workers=1
fi

printf 'OK PR-BC review fact and PnL contract\n'
