#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
AUDIT="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
EVIDENCE="$ROOT/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
AGGREGATOR="$ROOT/crates/exchange/src/aggregator.rs"
LIVE_HEALTH="$ROOT/crates/api/src/services/hedge_preflight/live_health/credentials/part_02.rs"
EXECUTION_EVIDENCE="$ROOT/frontend/src/panels/modules/execution/components/risk_preview/evidence.rs"
BROWSER="$ROOT/test/e2e/pr_an_http_execution_evidence.spec.ts"
REPO_GATE="$ROOT/scripts/verify_repo_gates.sh"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-AN completion gate failed: %s\n' "$1" >&2
  exit 1
}

run_static_gate() {
  PR_AN_SKIP_TESTS=1 PR_AN_SKIP_UPSTREAM=1 PR_AN_SKIP_BROWSER=1 bash "$0" >/dev/null
}

expect_rejected() {
  local message="$1"
  if run_static_gate 2>/dev/null; then
    fail "$message"
  fi
}

if [[ "$MODE" == "--self-test" ]]; then
  tmp="$(mktemp -d "${TMPDIR:-/tmp}/crossline-pr-an.XXXXXX")"
  files=("$AUDIT" "$EVIDENCE" "$AGGREGATOR" "$LIVE_HEALTH" "$EXECUTION_EVIDENCE" "$BROWSER" "$REPO_GATE")
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
old = "| `PR-AN Aggregator Fanout Outcome & HTTP Runtime Telemetry` | ✅ 完成 |"
if text.count(old) != 1:
    raise SystemExit("PR-AN self-test setup failed: roadmap row drifted")
path.write_text(text.replace(old, "| `PR-AN Aggregator Fanout Outcome & HTTP Runtime Telemetry` | 🟡 部分完成 |", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted a downgraded roadmap row"
  restore_file "$AUDIT"

  python3 - "$EVIDENCE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
rows = path.read_text(encoding="utf-8").splitlines()
for index, row in enumerate(rows):
    if row.startswith("PR-AN\texecution-preflight-problem-synthesis\t"):
        del rows[index]
        break
else:
    raise SystemExit("PR-AN self-test setup failed: evidence row missing")
path.write_text("\n".join(rows) + "\n", encoding="utf-8")
PY
  expect_rejected "self-test accepted incomplete evidence"
  restore_file "$EVIDENCE"

  python3 - "$AGGREGATOR" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = "pub fn coverage(&self) -> MarketDataCoverage"
if text.count(old) != 1:
    raise SystemExit("PR-AN self-test setup failed: coverage marker drifted")
path.write_text(text.replace(old, "pub fn coverage_pct(&self) -> MarketDataCoverage", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted a legacy coverage wrapper"
  restore_file "$AGGREGATOR"

  python3 - "$LIVE_HEALTH" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = ".filter_map(live_operation_last_error)"
if text.count(old) != 1:
    raise SystemExit("PR-AN self-test setup failed: typed fallback marker drifted")
path.write_text(text.replace(old, ".filter_map(|row| row.problem.clone())", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted dropped fallback operation problems"
  restore_file "$LIVE_HEALTH"

  python3 - "$EXECUTION_EVIDENCE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = "if let Some(context) = structured_problem_context_label(problem) {"
if text.count(old) != 1:
    raise SystemExit("PR-AN self-test setup failed: execution context marker drifted")
path.write_text(text.replace(old, "if let Some(context) = None::<String> {", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted hidden execution HTTP context"
  restore_file "$EXECUTION_EVIDENCE"

  python3 - "$BROWSER" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = 'test("PR-AN execution preflight exposes HTTP and fanout problem context"'
if text.count(old) != 1:
    raise SystemExit("PR-AN self-test setup failed: browser marker drifted")
path.write_text(text.replace(old, 'test.skip("PR-AN execution preflight exposes HTTP and fanout problem context"', 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted a skipped execution product fixture"
  restore_file "$BROWSER"

  python3 - "$AUDIT" <<'PY'
from pathlib import Path
import re
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
head = re.search(r"(?m)^1\. \*\*(PR-[A-Z]+)\b", text)
if head is None:
    raise SystemExit("PR-AN self-test setup failed: successor queue head drifted")
updated = text[:head.start(1)] + "PR-AN" + text[head.end(1):]
path.write_text(updated, encoding="utf-8")
PY
  expect_rejected "self-test accepted PR-AN reinserted into the local queue"
  restore_file "$AUDIT"

  python3 - "$REPO_GATE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = 'PR_AN_SKIP_TESTS=1 PR_AN_SKIP_UPSTREAM=1 PR_AN_SKIP_BROWSER=1 bash "$ROOT/scripts/check_pr_an_completion.sh"'
if text.count(old) != 2:
    raise SystemExit("PR-AN self-test setup failed: repo wiring drifted")
path.write_text(text.replace(old, "true # PR-AN gate removed", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted single-scope repository wiring"

  printf 'PR-AN completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from pathlib import Path
import csv
import re
import sys

root = Path(sys.argv[1])
audit = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(encoding="utf-8")


def fail(message: str) -> None:
    raise SystemExit(f"PR-AN completion gate failed: {message}")


title = "PR-AN Aggregator Fanout Outcome & HTTP Runtime Telemetry"
row = next((line for line in audit.splitlines() if line.startswith(f"| `{title}` |")), None)
if row is None or "| ✅ 完成 |" not in row or "剩余：无。" not in row:
    fail("roadmap row is not complete with no local remainder")
if "`bash scripts/check_pr_an_completion.sh --self-test`" not in row:
    fail("roadmap row lacks the canonical destructive verification anchor")
if "> 本轮 PR-AN：" not in audit:
    fail("top progress summary lacks the PR-AN closure note")

queue = audit.split("### 🟡 6.5 下一步执行队列", 1)[-1]
numbered = re.findall(r"(?m)^\d+\. \*\*(PR-[A-Z]+)", queue)
if not numbered or "PR-AN" in numbered:
    fail(f"queue handoff drifted: {numbered[:5]}")
if "## 2026-07-18 PR-AN Fanout and HTTP Runtime Telemetry Closure" not in history:
    fail("history closure appendix is missing")

evidence_contract = {
    "normalized-fanout-envelope": "crates/exchange/src/aggregator.rs",
    "fanout-partial-failure-fixture": "crates/exchange/tests/aggregator_multi_test.rs",
    "fanout-runtime-consumer": "crates/api/src/services/market_data/cache/runtime.rs",
    "shared-exchange-problem": "shared-types/src/problem.rs",
    "shared-market-coverage": "shared-types/src/market.rs",
    "http-outcome-runtime": "crates/exchange/src/http_metrics.rs",
    "host-gate-runtime": "crates/exchange/src/services/host_gate.rs",
    "rate-limiter-runtime": "crates/exchange/src/services/rate_limiter.rs",
    "operation-health-projection": "crates/api/src/services/venue_operation_health/snapshot/part_16.rs",
    "execution-preflight-problem-synthesis": "crates/api/src/services/hedge_preflight/live_health/credentials/part_02.rs",
    "execution-preflight-fixture": "crates/api/src/services/hedge_preflight/tests/cases_live_a/cases/part_04.rs",
    "shared-http-context-formatter": "frontend/src/panels/modules/market_evidence.rs",
    "execution-http-context-consumer": "frontend/src/panels/modules/execution/components/risk_preview/evidence.rs",
    "execution-http-context-unit-fixture": "frontend/src/panels/modules/execution/components/risk_preview/tests/cases.rs",
    "execution-http-context-browser": "test/e2e/pr_an_http_execution_evidence.spec.ts",
    "release-qa-product-suite": "scripts/check_release_qa_contract.sh",
    "fanout-successor-gate": "scripts/check_pr_ev_completion.sh",
    "runtime-verification-successor-gate": "scripts/check_pr_dj_completion.sh",
    "status-center-successor-gate": "scripts/check_pr_eg_completion.sh",
    "completion-governance": "scripts/check_pr_an_completion.sh",
}
with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-AN"]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for evidence_type, artifact in evidence_contract.items():
    evidence = indexed[evidence_type]
    if evidence["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"evidence artifact drifted or is missing: {evidence_type}")
    if not evidence["command"].strip() or not evidence["notes"].strip():
        fail(f"evidence lacks command or notes: {evidence_type}")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
for artifact in set(evidence_contract.values()):
    item = coverage.get(artifact)
    if item is None or item["coverage_status"] != "exact" or not item["evidence"].startswith("line:"):
        fail(f"exact coverage missing for {artifact}")

markers = {
    "crates/exchange/src/aggregator.rs": (
        "pub struct FanoutReport<T>",
        "pub operation: &'static str",
        "self.succeeded() && self.rows > 0",
        "pub fn coverage(&self) -> MarketDataCoverage",
        ".with_latency_ms(Some(latency_ms))",
        "successful_empty_venue_remains_uncovered",
    ),
    "shared-types/src/problem.rs": (
        "pub symbol: Option<String>",
        "pub latency_ms: Option<u64>",
        "pub request_id: Option<String>",
    ),
    "shared-types/src/market.rs": (
        "pub struct MarketDataCoverage",
        "pub coverage_pct: f64",
    ),
    "crates/api/src/services/hedge_preflight/live_health/credentials/part_02.rs": (
        "live_operation_problem_details",
        '"symbol": live_operation_symbol(row)',
        '"latencyMs": row.latency_ms',
        ".filter_map(live_operation_last_error)",
    ),
    "frontend/src/panels/modules/market_evidence.rs": (
        "pub(crate) fn structured_problem_context_label",
        "HTTP耗时 {latency_ms}ms",
    ),
    "frontend/src/panels/modules/execution/components/risk_preview/evidence.rs": (
        "structured_problem_context_label(problem)",
    ),
}
for path, required in markers.items():
    source = (root / path).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"missing {path} marker: {marker}")

aggregator = (root / "crates/exchange/src/aggregator.rs").read_text(encoding="utf-8")
if re.search(r"pub async fn fetch_all_(?:funding_rates|tickers|spot_ticks)\(", aggregator):
    fail("legacy naked Vec fanout wrapper returned")
if "pub fn coverage_pct(" in aggregator or "pub fn into_rows(" in aggregator:
    fail("legacy fanout coverage or row escape wrapper returned")

browser = (root / "test/e2e/pr_an_http_execution_evidence.spec.ts").read_text(encoding="utf-8")
if re.search(r"\btest\.(?:skip|fixme)\s*\(", browser):
    fail("execution browser fixture must not be skipped")
for marker in ("rest_orderbooks", "BTCUSDT", "/fapi/v1/depth", "HTTP耗时 37ms", "req-pr-an-http"):
    if marker not in browser:
        fail(f"browser fixture missing structured context: {marker}")

package = (root / "package.json").read_text(encoding="utf-8")
if '"test:e2e:pr-an"' not in package or "pr_an_http_execution_evidence.spec.ts" not in package:
    fail("package script or product suite wiring is missing")
release_qa = (root / "scripts/check_release_qa_contract.sh").read_text(encoding="utf-8")
release_fixture = (root / "scripts/fixtures/release_qa_contract/package.json").read_text(encoding="utf-8")
for source, label in ((release_qa, "release QA contract"), (release_fixture, "release QA fixture")):
    if source.count("test/e2e/pr_an_http_execution_evidence.spec.ts") != 1:
        fail(f"{label} does not lock the PR-AN product fixture exactly once")
repo_gate = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
wiring = 'PR_AN_SKIP_TESTS=1 PR_AN_SKIP_UPSTREAM=1 PR_AN_SKIP_BROWSER=1 bash "$ROOT/scripts/check_pr_an_completion.sh"'
if repo_gate.count(wiring) != 2:
    fail("completion gate must be wired into both repository scopes")

print(f"OK PR-AN contract ({len(evidence_contract)} evidence types; queue={numbered[0]})")
PY

bash "$ROOT/scripts/check_product_audit_progress.sh"
bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_evidence_index.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"
bash "$ROOT/scripts/check_release_qa_contract.sh"

if [[ "${PR_AN_SKIP_UPSTREAM:-0}" != "1" ]]; then
  PR_EV_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_ev_completion.sh"
  PR_DJ_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dj_completion.sh"
  PR_EG_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_eg_completion.sh"
fi

if [[ "${PR_AN_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p shared-types market_data_coverage --lib --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p exchange aggregator --lib --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p exchange --test aggregator_multi_test --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api live_operation_health_guard --bin crypto-arb-api --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api runtime_health_records_fanout_venue_outcomes --bin crypto-arb-api --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api http_outcome_rows_use_latest_endpoint_outcome --bin crypto-arb-api --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" market_evidence --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" preflight_problem_line_surfaces_structured_http_context --no-fail-fast
fi

if [[ "${PR_AN_SKIP_BROWSER:-0}" != "1" ]]; then
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-an
fi

printf 'OK PR-AN fanout, HTTP runtime and execution evidence contract\n'
