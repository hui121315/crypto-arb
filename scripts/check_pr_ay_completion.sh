#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
AUDIT="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
HISTORY="$ROOT/docs/audit_history/PRODUCT_AUDIT_HISTORY.md"
EVIDENCE="$ROOT/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
SHARED="$ROOT/shared-types/src/system.rs"
SYSTEM_PROBLEMS="$ROOT/crates/api/src/services/system_health/problems.rs"
RESOURCE_POLLING="$ROOT/frontend/src/state/resource_polling.rs"
STATUS_DATA="$ROOT/frontend/src/panels/status_bar/data.rs"
BROWSER="$ROOT/test/e2e/pr_ay_partial_failure.spec.ts"
PACKAGE="$ROOT/package.json"
RELEASE_GATE="$ROOT/scripts/check_release_qa_contract.sh"
REPO_GATE="$ROOT/scripts/verify_repo_gates.sh"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-AY completion gate failed: %s\n' "$1" >&2
  exit 1
}

run_static_gate() {
  PR_AY_STATIC_ONLY=1 bash "$0" >/dev/null
}

if [[ "$MODE" == "--self-test" ]]; then
  tmp="$(mktemp -d "${TMPDIR:-/tmp}/crossline-pr-ay.XXXXXX")"
  files=(
    "$AUDIT" "$EVIDENCE" "$SHARED" "$SYSTEM_PROBLEMS" "$RESOURCE_POLLING"
    "$STATUS_DATA" "$BROWSER" "$PACKAGE" "$RELEASE_GATE" "$REPO_GATE"
  )
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
    raise SystemExit(f"PR-AY self-test setup failed: marker missing in {path}")
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
    '| `PR-AY Runtime Partial Failure & Browser Smoke` | ✅ 完成 |' \
    '| `PR-AY Runtime Partial Failure & Browser Smoke` | 🟡 部分完成 |' \
    'self-test accepted a downgraded roadmap row'
  assert_rejected "$EVIDENCE" $'PR-AY\truntime-problem-typed-context\t' \
    $'PR-AY-MISSING\truntime-problem-typed-context\t' \
    'self-test accepted incomplete exact evidence'
  assert_rejected "$SHARED" 'pub problem: Option<crate::ApiProblem>,' \
    'pub problem: Option<String>,' \
    'self-test accepted typed runtime problem erasure'
  assert_rejected "$RESOURCE_POLLING" '*state = LoadState::Stale { value, problem }' \
    '*state = LoadState::Ready(value)' \
    'self-test accepted degraded data without a typed stale problem'
  assert_rejected "$STATUS_DATA" '.system_health_envelope()' '.system_health()' \
    'self-test accepted system envelope problem erasure'
  assert_rejected "$BROWSER" \
    'test("PR-AY degraded system envelope keeps usable scalar data isolated"' \
    'test.skip("PR-AY degraded system envelope keeps usable scalar data isolated"' \
    'self-test accepted a skipped browser contract'
  assert_rejected "$BROWSER" 'candidate.status() === 200' 'candidate.status() === 502' \
    'self-test accepted a browser contract that no longer proves partial success'
  assert_rejected "$PACKAGE" \
    '"test:e2e:pr-ay": "playwright test test/e2e/pr_ay_partial_failure.spec.ts"' \
    '"test:e2e:pr-ay": "true"' \
    'self-test accepted a skipped dedicated browser command'
  assert_rejected "$AUDIT" '### 🟡 6.5 下一步执行队列' \
    $'### 🟡 6.5 下一步执行队列\n1. **PR-AY Runtime Partial Failure & Browser Smoke** — stale queue fixture.' \
    'self-test accepted PR-AY reinserted into the local queue'
  assert_rejected "$REPO_GATE" \
    'PR_AY_STATIC_ONLY=1 bash "$ROOT/scripts/check_pr_ay_completion.sh"' \
    'true # PR-AY gate removed' \
    'self-test accepted single-scope repository wiring'

  printf 'PR-AY completion self-test passed\n'
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
    raise SystemExit(f"PR-AY completion gate failed: {message}")


title = "PR-AY Runtime Partial Failure & Browser Smoke"
row = next((line for line in audit.splitlines() if line.startswith(f"| `{title}` |")), None)
if row is None or "| ✅ 完成 |" not in row or "剩余：无。" not in row:
    fail("roadmap row is not complete with no local remainder")
if "`bash scripts/check_pr_ay_completion.sh --self-test`" not in row:
    fail("roadmap row lacks the canonical destructive verification anchor")
if "> 本轮 PR-AY：" not in audit:
    fail("top progress summary lacks the PR-AY closure note")

queue = audit.split("### 🟡 6.5 下一步执行队列", 1)[-1]
numbered = re.findall(r"(?m)^\d+\. \*\*(PR-[A-Z]+)", queue)
successor_order = ("PR-AZ", "PR-BB", "PR-BC", "PR-BD")
incomplete = []
for pr_id in successor_order:
    successor = next((line for line in audit.splitlines() if line.startswith(f"| `{pr_id} ")), None)
    if successor is None:
        fail(f"successor roadmap row is missing: {pr_id}")
    if "| ✅ 完成 |" not in successor:
        incomplete.append(pr_id)
if numbered[: len(incomplete)] != incomplete or len(numbered) != len(set(numbered)) or "PR-AY" in numbered:
    fail(f"queue handoff drifted: expected={incomplete}, actual={numbered[:len(incomplete)]}")
if "## 2026-07-19 PR-AY Runtime Partial Failure and Browser Closure" not in history:
    fail("history closure appendix is missing")

evidence_contract = {
    "runtime-problem-typed-context": "shared-types/src/system.rs",
    "system-health-typed-projection": "crates/api/src/services/system_health/problems.rs",
    "system-health-resource-envelope": "crates/api/src/services/system_health.rs",
    "portfolio-partial-envelope": "crates/api/src/services/portfolio_snapshot_envelope.rs",
    "account-state-partial-contract": "crates/api/src/services/account_state.rs",
    "market-fanout-isolation": "crates/exchange/tests/aggregator_multi_test.rs",
    "market-fanout-envelope": "crates/api/src/services/market_data/envelope/health.rs",
    "frontend-resource-polling": "frontend/src/state/resource_polling.rs",
    "status-bar-degraded-consumer": "frontend/src/panels/status_bar/data.rs",
    "positions-problem-consumer": "frontend/src/panels/modules/positions/components/runtime_problems.rs",
    "partial-failure-browser": "test/e2e/pr_ay_partial_failure.spec.ts",
    "product-suite": "package.json",
    "release-qa-product-suite": "scripts/check_release_qa_contract.sh",
    "ci-browser": ".github/workflows/ci.yml",
    "fanout-successor-authority": "scripts/check_pr_an_completion.sh",
    "loadstate-successor-authority": "scripts/check_pr_by_completion.sh",
    "repository-aggregate": "scripts/verify_repo_gates.sh",
    "completion-governance": "scripts/check_pr_ay_completion.sh",
}
with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [item for item in csv.DictReader(handle, delimiter="\t") if item["pr_id"] == "PR-AY"]
indexed = {item["evidence_type"]: item for item in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for evidence_type, artifact in evidence_contract.items():
    item = indexed[evidence_type]
    if item["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"evidence artifact drifted or is missing: {evidence_type}")
    if not item["command"].strip() or not item["notes"].strip():
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
    "shared-types/src/system.rs": (
        "pub problem: Option<crate::ApiProblem>",
        "pub fn to_api_problem(&self) -> crate::ApiProblem",
    ),
    "crates/api/src/services/system_health/problems.rs": (
        "problem: Some(problem.clone())",
        "problem.to_api_problem(code)",
    ),
    "frontend/src/state/resource_polling.rs": (
        "LoadState::Stale { value, problem }",
        "RESOURCE_DEGRADED_WITHOUT_PROBLEM",
        "use_conditional_resource_load_state",
    ),
    "frontend/src/panels/status_bar/data.rs": (
        "use_conditional_resource_load_state",
        ".system_health_envelope()",
    ),
}
for relative, required in markers.items():
    text = (root / relative).read_text(encoding="utf-8")
    for marker in required:
        if marker not in text:
            fail(f"runtime contract lost marker in {relative}: {marker}")

browser = (root / "test/e2e/pr_ay_partial_failure.spec.ts").read_text(encoding="utf-8")
browser_titles = (
    "PR-AY degraded system envelope keeps usable scalar data isolated",
    "PR-AY portfolio account and market fanout isolate one venue failure",
)
if any(f'test("{title}"' not in browser for title in browser_titles):
    fail("browser titles drifted")
if re.search(r"\btest\.(?:skip|fixme)\(", browser) or browser.count("candidate.status() === 200") < 2:
    fail("browser contract is skipped or no longer proves HTTP 200 partial success")
for marker in ("request_id", "accountStatus", "funding.fanout", "balance-row"):
    if marker not in browser:
        fail(f"browser partial-failure assertion is missing: {marker}")

package = json.loads((root / "package.json").read_text(encoding="utf-8"))
dedicated = "playwright test test/e2e/pr_ay_partial_failure.spec.ts"
if package["scripts"].get("test:e2e:pr-ay") != dedicated:
    fail("dedicated product browser command drifted")
if package["scripts"].get("test:e2e:product", "").count("test/e2e/pr_ay_partial_failure.spec.ts") != 1:
    fail("product browser suite must include PR-AY exactly once")

release = (root / "scripts/check_release_qa_contract.sh").read_text(encoding="utf-8")
if '.scripts["test:e2e:pr-ay"]' not in release or "test/e2e/pr_ay_partial_failure.spec.ts" not in release:
    fail("release QA no longer locks PR-AY browser evidence")
ci = (root / ".github/workflows/ci.yml").read_text(encoding="utf-8")
if "browser-smoke:" not in ci or "npm run test:e2e:product" not in ci:
    fail("official CI browser product suite is missing")
repo = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
wiring = 'PR_AY_STATIC_ONLY=1 bash "$ROOT/scripts/check_pr_ay_completion.sh"'
if repo.count(wiring) != 2:
    fail("completion gate must be wired into both repository scopes")

print(f"OK PR-AY partial-failure contract ({len(evidence_contract)} evidence types; queue={numbered[0]})")
PY

if [[ "${PR_AY_STATIC_ONLY:-0}" == "1" ]]; then exit 0; fi

bash "$ROOT/scripts/check_product_audit_progress.sh"
bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_evidence_index.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"
bash "$RELEASE_GATE" --self-test
PR_EV_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_ev_completion.sh"
PR_AN_SKIP_TESTS=1 PR_AN_SKIP_UPSTREAM=1 PR_AN_SKIP_BROWSER=1 bash "$ROOT/scripts/check_pr_an_completion.sh"
PR_BY_SKIP_TESTS=1 PR_BY_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_by_completion.sh"
PR_EW_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_ew_completion.sh"

if [[ "${PR_AY_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS=8 cargo test -p shared-types runtime_problem_preserves_typed_request_context --lib --no-fail-fast
  CARGO_BUILD_JOBS=8 cargo test -p api --bin crypto-arb-api operation_health_problem_keeps_typed_request_context --no-fail-fast
  CARGO_BUILD_JOBS=8 cargo test -p api --bin crypto-arb-api partial_fanout --no-fail-fast
  CARGO_BUILD_JOBS=8 cargo test -p exchange --test aggregator_multi_test --no-fail-fast
  CARGO_BUILD_JOBS=8 cargo test --manifest-path "$ROOT/frontend/Cargo.toml" resource_polling --lib --no-fail-fast
  CARGO_BUILD_JOBS=8 cargo test --manifest-path "$ROOT/frontend/Cargo.toml" runtime_problem_title_preserves_typed_request_context --lib --no-fail-fast
fi

if [[ "${PR_AY_SKIP_BROWSER:-0}" != "1" ]]; then
  CI=1 npm run test:e2e:pr-ay -- --workers=1
fi

printf 'OK PR-AY runtime partial-failure and browser contract\n'
