#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
AUDIT="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
HISTORY="$ROOT/docs/audit_history/PRODUCT_AUDIT_HISTORY.md"
EVIDENCE="$ROOT/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
POSITIONS="$ROOT/crates/api/src/trading_service/positions.rs"
POSITION_CACHE="$ROOT/crates/api/src/trading_service/venue_position_cache.rs"
FAILURES="$ROOT/crates/api/src/trading_service/live_adapters/failures.rs"
ACCOUNT_POSITIONS="$ROOT/crates/api/src/services/account_positions.rs"
PORTFOLIO="$ROOT/crates/api/src/services/portfolio/outcome.rs"
FRONTEND="$ROOT/frontend/src/panels/modules/positions/components/runtime_problems.rs"
FRONTEND_TESTS="$ROOT/frontend/src/panels/modules/positions/components/runtime_problems_tests.rs"
BROWSER="$ROOT/test/e2e/pr_ay_partial_failure.spec.ts"
PACKAGE="$ROOT/package.json"
RELEASE_GATE="$ROOT/scripts/check_release_qa_contract.sh"
REPO_GATE="$ROOT/scripts/verify_repo_gates.sh"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-AZ completion gate failed: %s\n' "$1" >&2
  exit 1
}

run_static_gate() {
  PR_AZ_STATIC_ONLY=1 bash "$0" >/dev/null
}

if [[ "$MODE" == "--self-test" ]]; then
  tmp="$(mktemp -d "${TMPDIR:-/tmp}/crossline-pr-az.XXXXXX")"
  files=(
    "$AUDIT" "$EVIDENCE" "$POSITIONS" "$POSITION_CACHE" "$FAILURES"
    "$ACCOUNT_POSITIONS" "$PORTFOLIO" "$FRONTEND" "$FRONTEND_TESTS" "$BROWSER" "$PACKAGE"
    "$RELEASE_GATE" "$REPO_GATE"
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
    raise SystemExit(f"PR-AZ self-test setup failed: marker missing in {path}")
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
    '| `PR-AZ Position Fanout & Gate Schema Evidence` | ✅ 完成 |' \
    '| `PR-AZ Position Fanout & Gate Schema Evidence` | 🟡 部分完成 |' \
    'self-test accepted a downgraded roadmap row'
  assert_rejected "$EVIDENCE" $'PR-AZ\tper-venue-position-refresh\t' \
    $'PR-AZ-MISSING\tper-venue-position-refresh\t' \
    'self-test accepted incomplete exact evidence'
  assert_rejected "$POSITIONS" 'self.position_cache.invalidate(&venue);' \
    'let _ = &venue;' \
    'self-test accepted a failed venue without immediate invalidation'
  assert_rejected "$POSITIONS" 'self.position_cache.stale(&venue, epoch, now_ms)' \
    'self.position_cache.fresh(&venue, epoch, now_ms)' \
    'self-test accepted partial failure without bounded stale fallback'
  assert_rejected "$FAILURES" 'pub(crate) fn venues(&self, operation: &str)' \
    'pub(crate) fn hidden_venues(&self, operation: &str)' \
    'self-test accepted destructive failure-snapshot consumption'
  assert_rejected "$FAILURES" '"positions" => ("private_read", "account_position"),' \
    '"positions" => ("private_read", "unknown"),' \
    'self-test accepted positions detached from canonical endpoint evidence'
  assert_rejected "$ACCOUNT_POSITIONS" \
    'failure.to_api_problem(POSITION_SOURCE, POSITION_ROUTE)' \
    'failure.error.to_api_problem()' \
    'self-test accepted account positions without canonical route context'
  assert_rejected "$PORTFOLIO" 'problem.problem = Some(api_problem);' \
    'let _ = api_problem;' \
    'self-test accepted portfolio runtime problem without typed context'
  assert_rejected "$BROWSER" \
    'source gate.GET \/api\/v4\/futures\/usdt\/positions' \
    'source gate.positions' \
    'self-test accepted incomplete product endpoint evidence'
  assert_rejected "$PACKAGE" \
    '"test:e2e:pr-az": "playwright test test/e2e/pr_ay_partial_failure.spec.ts"' \
    '"test:e2e:pr-az": "true"' \
    'self-test accepted a skipped dedicated browser command'
  assert_rejected "$AUDIT" '### 🟡 6.5 下一步执行队列' \
    $'### 🟡 6.5 下一步执行队列\n1. **PR-AZ Position Fanout & Gate Schema Evidence** — stale queue fixture.' \
    'self-test accepted PR-AZ reinserted into the local queue'
  assert_rejected "$REPO_GATE" \
    'PR_AZ_STATIC_ONLY=1 bash "$ROOT/scripts/check_pr_az_completion.sh"' \
    'true # PR-AZ gate removed' \
    'self-test accepted single-scope repository wiring'

  printf 'PR-AZ completion self-test passed\n'
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
    raise SystemExit(f"PR-AZ completion gate failed: {message}")


title = "PR-AZ Position Fanout & Gate Schema Evidence"
row = next((line for line in audit.splitlines() if line.startswith(f"| `{title}` |")), None)
if row is None or "| ✅ 完成 |" not in row or "剩余：无。" not in row:
    fail("roadmap row is not complete with no local remainder")
if "`bash scripts/check_pr_az_completion.sh --self-test`" not in row:
    fail("roadmap row lacks the canonical destructive verification anchor")
if "> 本轮 PR-AZ：" not in audit:
    fail("top progress summary lacks the PR-AZ closure note")

queue = audit.split("### 🟡 6.5 下一步执行队列", 1)[-1]
numbered = re.findall(r"(?m)^\d+\. \*\*(PR-[A-Z]+)", queue)
successor_order = ("PR-BB", "PR-BC", "PR-BD")
incomplete = []
for pr_id in successor_order:
    successor = next((line for line in audit.splitlines() if line.startswith(f"| `{pr_id} ")), None)
    if successor is None:
        fail(f"successor roadmap row is missing: {pr_id}")
    if "| ✅ 完成 |" not in successor:
        incomplete.append(pr_id)
if numbered[: len(incomplete)] != incomplete or len(numbered) != len(set(numbered)) or "PR-AZ" in numbered:
    fail(f"queue handoff drifted: expected={incomplete}, actual={numbered[:len(incomplete)]}")
if "## 2026-07-19 PR-AZ Position Fanout and Gate Schema Evidence Closure" not in history:
    fail("history closure appendix is missing")

evidence_contract = {
    "per-venue-position-refresh": "crates/api/src/trading_service/positions.rs",
    "position-cache-stale": "crates/api/src/trading_service/venue_position_cache.rs",
    "route-failure-snapshot": "crates/api/src/trading_service/live_adapters/failures.rs",
    "canonical-rest-context": "crates/api/src/trading_service/live_adapters/failures.rs",
    "live-router-fanout": "crates/api/src/trading_service/live_adapters/router.rs",
    "gate-current-schema-fixture": "crates/exchange/fixtures/gate/futures_usdt_positions_v4_106_106.json",
    "gate-rate-limit-fixture": "crates/exchange/fixtures/gate/futures_usdt_positions_rate_limited.json",
    "gate-http-parser": "crates/exchange/src/adapters/gate_private_rest.rs",
    "account-position-envelope": "crates/api/src/services/account_positions.rs",
    "portfolio-runtime-problem": "crates/api/src/services/portfolio/outcome.rs",
    "frontend-degraded-venue": "frontend/src/panels/modules/positions/components/runtime_problems.rs",
    "partial-browser": "test/e2e/pr_ay_partial_failure.spec.ts",
    "product-command": "package.json",
    "release-qa": "scripts/check_release_qa_contract.sh",
    "gate-successor-authority": "scripts/check_pr_fb_completion.sh",
    "typed-partial-authority": "scripts/check_pr_ay_completion.sh",
    "repository-aggregate": "scripts/verify_repo_gates.sh",
    "completion-governance": "scripts/check_pr_az_completion.sh",
}
with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [item for item in csv.DictReader(handle, delimiter="\t") if item["pr_id"] == "PR-AZ"]
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
    if artifact == "package.json" or artifact.endswith(".json"):
        continue
    item = coverage.get(artifact)
    if item is None or item["coverage_status"] != "exact" or not item["evidence"].startswith("line:"):
        fail(f"exact coverage missing for {artifact}")

markers = {
    "crates/api/src/trading_service/positions.rs": (
        "self.route_failures.record(POSITION_OPERATION, Vec::new())",
        ".venues(POSITION_OPERATION)",
        "self.seed_successful_position_venues",
        "self.position_cache.invalidate(&venue)",
        "self.position_cache.stale(&venue, epoch, now_ms)",
        "partial_refresh_updates_success_and_keeps_failed_venue_stale",
        "failed_venue_without_stale_rows_is_not_seeded_as_fresh_empty",
    ),
    "crates/api/src/trading_service/venue_position_cache.rs": (
        "pub(super) fn stale(",
        "pub(super) fn invalidate(",
    ),
    "crates/api/src/trading_service/live_adapters/failures.rs": (
        "pub(crate) fn venues(&self, operation: &str)",
        '"positions" => ("private_read", "account_position")',
        "exchange::rest_endpoint_registry()",
        '"fixtureId"',
        '"schemaHash"',
    ),
    "crates/api/src/services/account_positions.rs": (
        "failure.to_api_problem(POSITION_SOURCE, POSITION_ROUTE)",
    ),
    "crates/api/src/services/portfolio/outcome.rs": (
        "problem.problem = Some(api_problem)",
    ),
    "frontend/src/panels/modules/positions/components/runtime_problems.rs": (
        "fn api_problem_title(problem: &ApiProblem)",
        'parts.push(format!("source {source}"))',
    ),
    "frontend/src/panels/modules/positions/components/runtime_problems_tests.rs": (
        "gate.GET /api/v4/futures/usdt/positions",
        "runtime_problem_title_preserves_typed_request_context",
    ),
}
for relative, required in markers.items():
    text = (root / relative).read_text(encoding="utf-8")
    for marker in required:
        if marker not in text:
            fail(f"runtime contract lost marker in {relative}: {marker}")

gate_rest = (root / "crates/exchange/src/adapters/gate_private_rest.rs").read_text(encoding="utf-8")
for marker in (
    "gate_positions_http_fixture_maps_current_risk_semantics",
    "gate_positions_http_rate_limit_uses_reset_timestamp",
    "futures_usdt_positions_v4_106_106.json",
    "futures_usdt_positions_rate_limited.json",
):
    if marker not in gate_rest:
        fail(f"Gate HTTP/schema contract is missing: {marker}")

browser = (root / "test/e2e/pr_ay_partial_failure.spec.ts").read_text(encoding="utf-8")
if re.search(r"\btest\.(?:skip|fixme)\(", browser) or browser.count("candidate.status() === 200") < 2:
    fail("browser contract is skipped or no longer proves HTTP 200 partial success")
if "source gate.GET" not in browser or "gate · portfolio/positions" not in browser:
    fail("browser no longer exposes the failed venue and canonical Gate endpoint")

package = json.loads((root / "package.json").read_text(encoding="utf-8"))
dedicated = "playwright test test/e2e/pr_ay_partial_failure.spec.ts"
if package["scripts"].get("test:e2e:pr-az") != dedicated:
    fail("dedicated PR-AZ browser command drifted")
if package["scripts"].get("test:e2e:product", "").count("test/e2e/pr_ay_partial_failure.spec.ts") != 1:
    fail("product browser suite must include the shared partial-failure fixture exactly once")

release = (root / "scripts/check_release_qa_contract.sh").read_text(encoding="utf-8")
if '.scripts["test:e2e:pr-az"]' not in release or "test/e2e/pr_ay_partial_failure.spec.ts" not in release:
    fail("release QA no longer locks PR-AZ browser evidence")
repo = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
wiring = 'PR_AZ_STATIC_ONLY=1 bash "$ROOT/scripts/check_pr_az_completion.sh"'
if repo.count(wiring) != 2:
    fail("completion gate must be wired into both repository scopes")

print(f"OK PR-AZ position fanout contract ({len(evidence_contract)} evidence types; queue={numbered[0]})")
PY

if [[ "${PR_AZ_STATIC_ONLY:-0}" == "1" ]]; then exit 0; fi

bash "$ROOT/scripts/check_product_audit_progress.sh"
bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_evidence_index.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"
bash "$RELEASE_GATE" --self-test
PR_FB_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_fb_completion.sh"
PR_AY_STATIC_ONLY=1 bash "$ROOT/scripts/check_pr_ay_completion.sh"

if [[ "${PR_AZ_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS=8 cargo test -p api --bin crypto-arb-api trading_service::positions::tests --no-fail-fast
  CARGO_BUILD_JOBS=8 cargo test -p api --bin crypto-arb-api live_adapters::route_tests::cases::positions --no-fail-fast
  CARGO_BUILD_JOBS=8 cargo test -p exchange gate_positions_http --lib --no-fail-fast
  CARGO_BUILD_JOBS=8 cargo test -p api --bin crypto-arb-api gate_rate_limit_envelope_keeps_rows_request_id_and_retry_after --no-fail-fast
  CARGO_BUILD_JOBS=8 cargo test -p api --bin crypto-arb-api route_failure_problem_keeps_parsed_venue_and_code --no-fail-fast
  CARGO_BUILD_JOBS=8 cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib runtime_problem_title_preserves_typed_request_context --no-fail-fast
fi

if [[ "${PR_AZ_SKIP_BROWSER:-0}" != "1" ]]; then
  CI=1 npm run test:e2e:pr-az -- --workers=1
fi

printf 'OK PR-AZ position fanout and Gate schema evidence contract\n'
