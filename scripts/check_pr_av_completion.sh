#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
AUDIT="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
HISTORY="$ROOT/docs/audit_history/PRODUCT_AUDIT_HISTORY.md"
EVIDENCE="$ROOT/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
SHARED="$ROOT/shared-types/src/problem.rs"
SHARED_CODES="$ROOT/shared-types/src/problem/codes.rs"
COMMON="$ROOT/crates/common/src/error.rs"
FRONTEND="$ROOT/frontend/src/state/section.rs"
BROWSER="$ROOT/test/e2e/pr_av_problem_recovery.spec.ts"
RELEASE_QA="$ROOT/scripts/check_release_qa_contract.sh"
REPO_GATE="$ROOT/scripts/verify_repo_gates.sh"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-AV completion gate failed: %s\n' "$1" >&2
  exit 1
}

run_static_gate() {
  PR_AV_STATIC_ONLY=1 PR_AV_SKIP_TESTS=1 PR_AV_SKIP_UPSTREAM=1 PR_AV_SKIP_BROWSER=1 bash "$0" >/dev/null
}

expect_rejected() {
  local message="$1"
  if run_static_gate 2>/dev/null; then
    fail "$message"
  fi
}

if [[ "$MODE" == "--self-test" ]]; then
  tmp="$(mktemp -d "${TMPDIR:-/tmp}/crossline-pr-av.XXXXXX")"
  files=("$AUDIT" "$EVIDENCE" "$SHARED" "$SHARED_CODES" "$COMMON" "$FRONTEND" "$BROWSER" "$RELEASE_QA" "$REPO_GATE")
  for file in "${files[@]}"; do
    cp "$file" "$tmp/$(basename "$file").$(printf '%s' "$file" | shasum | cut -c1-12)"
  done
  restore_file() {
    local file="$1"
    cp "$tmp/$(basename "$file").$(printf '%s' "$file" | shasum | cut -c1-12)" "$file"
  }
  restore_all() {
    for file in "${files[@]}"; do
      restore_file "$file"
    done
    rm -rf "$tmp"
  }
  trap restore_all EXIT

  run_static_gate

  python3 - "$AUDIT" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = "| `PR-AV API Problem Contract & Error Taxonomy` | ✅ 完成 |"
if text.count(old) != 1:
    raise SystemExit("PR-AV self-test setup failed: roadmap row drifted")
path.write_text(text.replace(old, "| `PR-AV API Problem Contract & Error Taxonomy` | 🟡 部分完成 |", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted a downgraded roadmap row"
  restore_file "$AUDIT"

  python3 - "$EVIDENCE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
rows = path.read_text(encoding="utf-8").splitlines()
for index, row in enumerate(rows):
    if row.startswith("PR-AV\tshared-recovery-taxonomy\t"):
        del rows[index]
        break
else:
    raise SystemExit("PR-AV self-test setup failed: evidence row missing")
path.write_text("\n".join(rows) + "\n", encoding="utf-8")
PY
  expect_rejected "self-test accepted incomplete exact evidence"
  restore_file "$EVIDENCE"

  python3 - "$SHARED" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = "    pub recovery_action: Option<ApiRecoveryAction>,\n"
if text.count(old) != 1:
    raise SystemExit("PR-AV self-test setup failed: recovery field drifted")
path.write_text(text.replace(old, "    // recovery action removed\n", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted a problem contract without typed recovery"
  restore_file "$SHARED"

  python3 - "$SHARED_CODES" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = 'pub const ADAPTER_MISSING_CREDENTIALS: &str = "ADAPTER_MISSING_CREDENTIALS";'
if text.count(old) != 1:
    raise SystemExit("PR-AV self-test setup failed: shared problem code drifted")
path.write_text(text.replace(old, "// auth problem code removed", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted an incomplete shared problem code registry"
  restore_file "$SHARED_CODES"

  python3 - "$COMMON" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = "            .with_recovery_action(self.recovery_action());\n"
if text.count(old) != 1:
    raise SystemExit("PR-AV self-test setup failed: AppError recovery projection drifted")
path.write_text(text.replace(old, "            ;\n", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted AppError projection without recovery taxonomy"
  restore_file "$COMMON"

  python3 - "$FRONTEND" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = "pub(crate) fn recovery_action_context(problem: &ApiProblem) -> Option<String> {"
if text.count(old) != 1:
    raise SystemExit("PR-AV self-test setup failed: frontend recovery formatter drifted")
path.write_text(text.replace(old, "fn removed_recovery_action_context(problem: &ApiProblem) -> Option<String> {", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted hidden product recovery context"
  restore_file "$FRONTEND"

  python3 - "$BROWSER" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = 'test("PR-AV opportunity failure exposes structured recovery without fake rows"'
if text.count(old) != 1:
    raise SystemExit("PR-AV self-test setup failed: browser marker drifted")
path.write_text(text.replace(old, 'test.skip("PR-AV opportunity failure exposes structured recovery without fake rows"', 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted a skipped product fixture"
  restore_file "$BROWSER"

  python3 - "$AUDIT" <<'PY'
from pathlib import Path
import re
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
head = re.search(r"(?m)^1\. \*\*(PR-[A-Z]+)\b", text)
if head is None:
    raise SystemExit("PR-AV self-test setup failed: queue head drifted")
path.write_text(text[:head.start(1)] + "PR-AV" + text[head.end(1):], encoding="utf-8")
PY
  expect_rejected "self-test accepted PR-AV reinserted into the local queue"
  restore_file "$AUDIT"

  python3 - "$REPO_GATE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = 'PR_AV_SKIP_TESTS=1 PR_AV_SKIP_UPSTREAM=1 PR_AV_SKIP_BROWSER=1 bash "$ROOT/scripts/check_pr_av_completion.sh"'
if text.count(old) != 2:
    raise SystemExit("PR-AV self-test setup failed: repository wiring drifted")
path.write_text(text.replace(old, "true # PR-AV gate removed", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted single-scope repository wiring"

  printf 'PR-AV completion self-test passed\n'
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
    raise SystemExit(f"PR-AV completion gate failed: {message}")


title = "PR-AV API Problem Contract & Error Taxonomy"
row = next((line for line in audit.splitlines() if line.startswith(f"| `{title}` |")), None)
if row is None or "| ✅ 完成 |" not in row or "剩余：无。" not in row:
    fail("roadmap row is not complete with no local remainder")
if "`bash scripts/check_pr_av_completion.sh --self-test`" not in row:
    fail("roadmap row lacks the canonical destructive verification anchor")
if "> 本轮 PR-AV：" not in audit:
    fail("top progress summary lacks the PR-AV closure note")

queue = audit.split("### 🟡 6.5 下一步执行队列", 1)[-1]
numbered = re.findall(r"(?m)^\d+\. \*\*(PR-[A-Z]+)", queue)
successor_order = ("PR-AW", "PR-AX", "PR-AY", "PR-AZ", "PR-BB", "PR-BC")
incomplete_successors = []
for pr_id in successor_order:
    successor_row = next((line for line in audit.splitlines() if line.startswith(f"| `{pr_id} ")), None)
    if successor_row is None:
        fail(f"successor roadmap row is missing: {pr_id}")
    if "| ✅ 完成 |" not in successor_row:
        incomplete_successors.append(pr_id)
if (
    numbered[: len(incomplete_successors)] != incomplete_successors
    or len(numbered) != len(set(numbered))
    or "PR-AV" in numbered
):
    fail(f"queue handoff drifted: expected={incomplete_successors}, actual={numbered[:len(incomplete_successors)]}")
if "## 2026-07-19 PR-AV API Problem and Recovery Taxonomy Closure" not in history:
    fail("history closure appendix is missing")

evidence_contract = {
    "successor-request-correlation": "scripts/check_pr_dl_completion.sh",
    "successor-audit-correlation": "scripts/check_pr_dt_completion.sh",
    "successor-loadstate-contract": "scripts/check_pr_by_completion.sh",
    "successor-exchange-context": "scripts/check_pr_an_completion.sh",
    "shared-recovery-taxonomy": "shared-types/src/problem.rs",
    "shared-problem-codes": "shared-types/src/problem/codes.rs",
    "common-problem-projection": "crates/common/src/error.rs",
    "common-http-fixtures": "crates/common/src/error/response.rs",
    "position-problem-projection": "crates/api/src/services/account_positions.rs",
    "balance-problem-projection": "crates/api/src/services/account_balances/problems.rs",
    "open-order-problem-projection": "crates/api/src/services/account_open_orders.rs",
    "action-run-problem-projection": "crates/api/src/services/action_runs/mutate.rs",
    "portfolio-action-problem-projection": "crates/api/src/services/portfolio_actions/problems.rs",
    "portfolio-envelope-projection": "crates/api/src/services/portfolio_snapshot_envelope.rs",
    "audit-problem-contract": "crates/api/src/services/action_runs/audit_context/tests.rs",
    "frontend-problem-renderer": "frontend/src/state/section.rs",
    "settings-problem-surface": "frontend/src/panels/modules/settings/tabs/state_view.rs",
    "execution-problem-surface": "frontend/src/panels/modules/execution/problem.rs",
    "opportunity-problem-surface": "frontend/src/panels/modules/opportunities/data/detail_format.rs",
    "opportunity-problem-fixtures": "frontend/src/panels/modules/opportunities/data/tests/detail.rs",
    "opportunity-stream-surface": "frontend/src/state/arbitrage_stream.rs",
    "review-problem-surface": "frontend/src/panels/modules/review/view/derive.rs",
    "product-browser": "test/e2e/pr_av_problem_recovery.spec.ts",
    "package-product-suite": "package.json",
    "release-qa-product-suite": "scripts/check_release_qa_contract.sh",
    "completion-governance": "scripts/check_pr_av_completion.sh",
}
with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [item for item in csv.DictReader(handle, delimiter="\t") if item["pr_id"] == "PR-AV"]
indexed = {item["evidence_type"]: item for item in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for evidence_type, artifact in evidence_contract.items():
    evidence = indexed[evidence_type]
    if evidence["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"evidence artifact drifted or is missing: {evidence_type}")
    if not evidence["command"].strip() or not evidence["notes"].strip():
        fail(f"evidence lacks command or notes: {evidence_type}")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {item["file"]: item for item in csv.DictReader(handle, delimiter="\t")}
for artifact in set(evidence_contract.values()):
    if artifact == "package.json":
        continue
    item = coverage.get(artifact)
    if item is None or item["coverage_status"] != "exact" or not item["evidence"].startswith("line:"):
        fail(f"exact coverage missing for {artifact}")

markers = {
    "shared-types/src/problem.rs": (
        "pub mod codes;",
        "pub enum ApiRecoveryAction",
        "pub recovery_action: Option<ApiRecoveryAction>",
        "pub fn effective_recovery_action",
        "with_recovery_action",
    ),
    "shared-types/src/problem/codes.rs": (
        'pub const ADAPTER_MISSING_CREDENTIALS: &str = "ADAPTER_MISSING_CREDENTIALS";',
        'pub const HEDGE_TICKET_REQUIRED: &str = "HEDGE_TICKET_REQUIRED";',
        'pub const UPSTREAM_HTTP: &str = "UPSTREAM_HTTP";',
    ),
    "crates/common/src/error.rs": (
        "pub fn to_api_problem",
        ".with_recovery_action(self.recovery_action())",
        "fn recovery_action(&self) -> ApiRecoveryAction",
    ),
    "crates/common/src/error/response.rs": (
        "domain_statuses_map_to_stable_recovery_taxonomy",
        "recoveryAction",
        "authenticate",
    ),
    "frontend/src/state/section.rs": (
        "pub(crate) fn prefixed_problem_message",
        "pub(crate) fn problem_detail_context",
        "pub(crate) fn recovery_action_context",
        'format!("下一步 {}"',
    ),
    "frontend/src/panels/modules/settings/tabs/state_view.rs": ("prefixed_problem_message(prefix, problem)",),
    "frontend/src/panels/modules/execution/problem.rs": ("prefixed_problem_message(prefix, problem)",),
    "frontend/src/panels/modules/opportunities/data/detail_format.rs": ("problem_context(problem)",),
    "frontend/src/state/arbitrage_stream.rs": ("problem_detail_context, recovery_action_context",),
}
for relative, required in markers.items():
    source = (root / relative).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"{relative} lost marker: {marker}")

manual_projection = re.compile(r"ApiProblem::new\(\s*error\.code\(\),\s*error\.to_string\(\)", re.S)
for path in (root / "crates/api/src").rglob("*.rs"):
    if manual_projection.search(path.read_text(encoding="utf-8")):
        fail(f"manual AppError projection returned: {path.relative_to(root)}")

browser_path = "test/e2e/pr_av_problem_recovery.spec.ts"
browser = (root / browser_path).read_text(encoding="utf-8")
if re.search(r"\btest\.(?:skip|fixme)\s*\(", browser):
    fail("PR-AV product fixture must not be skipped")
if len(re.findall(r"(?m)^test\(", browser)) != 4:
    fail("PR-AV browser fixture must expose exactly four runnable scenarios")
for marker in (
    "PR-AV opportunity failure exposes structured recovery without fake rows",
    "PR-AV settings failure exposes operation context and operator recovery",
    "PR-AV execution preview exposes scoped recovery and remains blocked",
    "PR-AV review failure exposes ledger context without fake execution",
    "下一步 等待后重试",
    "下一步 检查运行状态",
    "下一步 联系操作员",
):
    if marker not in browser:
        fail(f"PR-AV browser fixture missing: {marker}")

package = json.loads((root / "package.json").read_text(encoding="utf-8"))
focused = "playwright test test/e2e/pr_av_problem_recovery.spec.ts"
if package.get("scripts", {}).get("test:e2e:pr-av") != focused:
    fail("package PR-AV focused command is missing")
if package.get("scripts", {}).get("test:e2e:product", "").count(browser_path) != 1:
    fail("product suite must include the PR-AV browser exactly once")
for path in ("scripts/check_release_qa_contract.sh", "scripts/fixtures/release_qa_contract/package.json"):
    if (root / path).read_text(encoding="utf-8").count(browser_path) != 1:
        fail(f"release QA command lock drifted: {path}")

repo_gate = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
wiring = 'PR_AV_SKIP_TESTS=1 PR_AV_SKIP_UPSTREAM=1 PR_AV_SKIP_BROWSER=1 bash "$ROOT/scripts/check_pr_av_completion.sh"'
if repo_gate.count(wiring) != 2:
    fail("completion gate must be wired into both repository scopes")
if repo_gate.count(browser_path) != 3:
    fail("repository gate must lock the focused command, product suite and browser list")

print(f"OK PR-AV contract ({len(evidence_contract)} evidence types; queue={numbered[0]})")
PY

if [[ "${PR_AV_STATIC_ONLY:-0}" == "1" ]]; then
  exit 0
fi

bash "$ROOT/scripts/check_product_audit_progress.sh"
bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_evidence_index.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"
bash "$ROOT/scripts/check_release_qa_contract.sh"

if [[ "${PR_AV_SKIP_UPSTREAM:-0}" != "1" ]]; then
  PR_DL_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dl_completion.sh"
  PR_DT_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dt_completion.sh"
  PR_BY_SKIP_TESTS=1 PR_BY_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_by_completion.sh"
  PR_AN_SKIP_TESTS=1 PR_AN_SKIP_UPSTREAM=1 PR_AN_SKIP_BROWSER=1 bash "$ROOT/scripts/check_pr_an_completion.sh"
fi

if [[ "${PR_AV_SKIP_TESTS:-0}" != "1" ]]; then
  jobs="${CARGO_BUILD_JOBS:-8}"
  CARGO_BUILD_JOBS="$jobs" cargo test -p shared-types problem --lib --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p common --features http error::response --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib problem_context --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib state::section --no-fail-fast
fi

if [[ "${PR_AV_SKIP_BROWSER:-0}" != "1" ]]; then
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-av
fi

printf 'OK PR-AV API problem taxonomy and product recovery contract\n'
