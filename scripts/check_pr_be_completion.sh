#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
AUDIT="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
EVIDENCE="$ROOT/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
AUTH_EDGE="$ROOT/crates/api/src/middleware/auth/denial.rs"
AUTH_DENIAL="$ROOT/crates/api/src/services/action_runs/auth_denial.rs"
ACTION_AUDIT="$ROOT/crates/api/src/services/action_runs/audit_log.rs"
WRITER="$ROOT/crates/api/src/middleware/audit/writer.rs"
BROWSER="$ROOT/test/e2e/pr_dt_request_correlation.spec.ts"
FIXTURE="$ROOT/test/e2e/fixtures/route_runtime_policy.mjs"
SECURITY_SMOKE="$ROOT/scripts/verify_api_security_runtime_smoke.sh"
REPO_GATE="$ROOT/scripts/verify_repo_gates.sh"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-BE completion gate failed: %s\n' "$1" >&2
  exit 1
}

run_static_gate() {
  PR_BE_STATIC_ONLY=1 bash "$0" >/dev/null
}

if [[ "$MODE" == "--self-test" ]]; then
  tmp="$(mktemp -d "${TMPDIR:-/tmp}/crossline-pr-be.XXXXXX")"
  files=("$AUDIT" "$EVIDENCE" "$AUTH_EDGE" "$AUTH_DENIAL" "$ACTION_AUDIT" "$WRITER" "$BROWSER" "$FIXTURE" "$SECURITY_SMOKE" "$REPO_GATE")
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
    raise SystemExit(f"PR-BE self-test setup failed: marker missing in {path}")
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
    '| `PR-BE High-Risk Audit Trail & Request Correlation` | ✅ 完成 |' \
    '| `PR-BE High-Risk Audit Trail & Request Correlation` | 🟡 部分完成 |' \
    'self-test accepted a downgraded roadmap row'
  assert_rejected "$EVIDENCE" $'PR-BE\tauth-denial-service\t' \
    $'PR-BE-MISSING\tauth-denial-service\t' \
    'self-test accepted missing exact evidence'
  assert_rejected "$AUTH_EDGE" \
    'action_runs::record_auth_denial(request.method(), request.uri().path(), &problem)?;' \
    'let _ = problem;' \
    'self-test accepted an auth edge that bypasses denial auditing'
  assert_rejected "$AUTH_DENIAL" 'audit::record_durable(&event)' 'audit::record(&event)' \
    'self-test accepted non-durable pre-handler denial auditing'
  assert_rejected "$AUTH_DENIAL" 'assert_eq!(route_count, 20);' 'assert_eq!(route_count, 19);' \
    'self-test accepted incomplete high-risk route coverage'
  assert_rejected "$ACTION_AUDIT" \
    'ActionRunKind::TradingKillSwitch => "trading.kill_switch.set"' \
    'ActionRunKind::TradingKillSwitch => "trading.kill_switch.changed"' \
    'self-test accepted audit action drift'
  assert_rejected "$WRITER" '.and_then(|()| file.sync_data())' '.and_then(|()| file.flush())' \
    'self-test accepted terminal events without durable sync'
  assert_rejected "$BROWSER" \
    'test("PR-BE auth-denied high-risk mutations keep typed audit correlation"' \
    'test.skip("PR-BE auth-denied high-risk mutations keep typed audit correlation"' \
    'self-test accepted a skipped product fixture'
  assert_rejected "$FIXTURE" \
    'recordDeniedAudit({ request, route: routePolicy("high_risk_action") });' \
    'return unauthorized(request, response); // audit removed' \
    'self-test accepted a runtime fixture without denied audit evidence'
  assert_rejected "$SECURITY_SMOKE" \
    'AUTH_DENIAL_EVENT_COUNT="$protected_route_count"' \
    'AUTH_DENIAL_EVENT_COUNT=0 # denied audit accounting removed' \
    'self-test accepted a real API smoke without auth-denial accounting'
  assert_rejected "$AUDIT" $'### 🟡 6.5 下一步执行队列\n' \
    $'### 🟡 6.5 下一步执行队列\n\n1. **PR-BE High-Risk Audit Trail & Request Correlation** — stale\n' \
    'self-test accepted PR-BE reinserted at the queue head'
  assert_rejected "$REPO_GATE" \
    'PR_BE_STATIC_ONLY=1 bash "$ROOT/scripts/check_pr_be_completion.sh"' \
    'true # PR-BE gate removed' \
    'self-test accepted single-scope repository wiring'

  printf 'PR-BE completion self-test passed\n'
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
    raise SystemExit(f"PR-BE completion gate failed: {message}")


title = "PR-BE High-Risk Audit Trail & Request Correlation"
row = next((line for line in audit.splitlines() if line.startswith(f"| `{title}` |")), None)
if row is None or "| ✅ 完成 |" not in row or "剩余：无。" not in row:
    fail("roadmap row is not complete with no local remainder")
if "`bash scripts/check_pr_be_completion.sh --self-test`" not in row:
    fail("roadmap row lacks the destructive verification anchor")
if "> 本轮 PR-BE：" not in audit:
    fail("top progress summary lacks the PR-BE closure note")
if "## 2026-07-19 PR-BE High-Risk Audit and Request Correlation Closure" not in history:
    fail("history closure appendix is missing")

queue = audit.split("### 🟡 6.5 下一步执行队列", 1)[-1]
if re.search(r"(?m)^\d+\. \*\*PR-BE\b", queue):
    fail("completed PR-BE returned to the queue")

contract = {
    "route-policy-authority": "scripts/check_pr_dk_completion.sh",
    "durable-action-authority": "scripts/check_pr_fi_completion.sh",
    "action-boundary-authority": "scripts/check_pr_ck_completion.sh",
    "typed-correlation-authority": "scripts/check_pr_dt_completion.sh",
    "route-registry": "crates/api/src/route_specs.rs",
    "route-inventory": "docs/API_ROUTE_INVENTORY.tsv",
    "mutation-audit-matrix": "scripts/check_mutation_audit_contract.sh",
    "trace-request-id": "crates/api/src/middleware/trace.rs",
    "verified-actor-and-sink-health": "crates/api/src/middleware/audit.rs",
    "auth-denial-edge": "crates/api/src/middleware/auth/denial.rs",
    "auth-denial-service": "crates/api/src/services/action_runs/auth_denial.rs",
    "action-run-lifecycle": "crates/api/src/services/action_runs/lifecycle.rs",
    "action-run-terminal": "crates/api/src/services/action_runs/mutate.rs",
    "audit-context": "crates/api/src/services/action_runs/audit_context.rs",
    "audit-correlation": "crates/api/src/services/action_runs/audit_log.rs",
    "bounded-durable-writer": "crates/api/src/middleware/audit/writer.rs",
    "durable-replay": "crates/api/src/middleware/audit/replay.rs",
    "live-audit-fail-closed": "crates/api/src/trading_service/helpers.rs",
    "shared-action-contract": "shared-types/src/actions.rs",
    "frontend-action-state": "frontend/src/state/action_state.rs",
    "route-runtime-fixture": "test/e2e/fixtures/route_runtime_policy.mjs",
    "audit-runtime-fixture": "test/e2e/fixtures/route_runtime_audit.mjs",
    "product-browser": "test/e2e/pr_dt_request_correlation.spec.ts",
    "security-runtime-smoke": "scripts/verify_api_security_runtime_smoke.sh",
    "repository-aggregate": "scripts/verify_repo_gates.sh",
    "completion-governance": "scripts/check_pr_be_completion.sh",
}
with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [item for item in csv.DictReader(handle, delimiter="\t") if item["pr_id"] == "PR-BE"]
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
    if artifact.startswith("docs/"):
        continue
    item = coverage.get(artifact)
    if item is None or item["coverage_status"] != "exact" or not item["evidence"].startswith("line:"):
        fail(f"exact coverage missing for {artifact}")

markers = {
    "crates/api/src/middleware/auth.rs": (
        "Err(error) => return denial::reject(&request, error)",
        'denial::reject(&request, AppError::Unauthorized("token mismatch".into()))',
    ),
    "crates/api/src/middleware/auth/denial.rs": (
        "action_runs::record_auth_denial(request.method(), request.uri().path(), &problem)?;",
    ),
    "crates/api/src/services/action_runs/auth_denial.rs": (
        "audit::record_durable(&event)",
        "auth_denial_matrix_covers_every_high_risk_action_route",
        "assert_eq!(route_count, 20);",
        "authentication denial could not be durably audited",
    ),
    "crates/api/src/services/action_runs/lifecycle.rs": (
        'record_audit(&run, "accepted")?',
        "request_id: common::request_id::current()",
    ),
    "crates/api/src/services/action_runs/mutate.rs": (
        "record_audit(&updated, audit_outcome(&updated))?",
    ),
    "crates/api/src/services/action_runs/audit_log.rs": (
        ".with_context(action_event_context(run))",
        ".with_correlation(action_correlation(run))",
        "with_order_ids(order_ids)",
        "with_run_ids(run_ids)",
        'ActionRunKind::TradingKillSwitch => "trading.kill_switch.set"',
    ),
    "crates/api/src/middleware/audit/writer.rs": (
        "mpsc::sync_channel(AUDIT_WRITER_QUEUE_CAPACITY)",
        ".and_then(|()| file.sync_data())",
        "queue_full: capacity={AUDIT_WRITER_QUEUE_CAPACITY}",
    ),
    "crates/api/src/middleware/audit/replay.rs": (
        "validate_correlation(&entry, &run",
        "recover_interrupted_run",
    ),
    "scripts/check_mutation_audit_contract.sh": (
        "high-risk routes with runtime and auth-denial audit actions",
    ),
    "test/e2e/fixtures/route_runtime_policy.mjs": (
        'actionKind: "trading_kill_switch"',
        'actionKind: "venue_credentials_update"',
        'actionKind: "hedge_confirm"',
        'recordDeniedAudit({ request, route: routePolicy("high_risk_action") });',
        'recordDeniedAudit({ request, route: routePolicy("high_risk_secret") });',
        'recordDeniedAudit({ request, route: routePolicy("high_risk_execution") });',
    ),
    "test/e2e/fixtures/route_runtime_audit.mjs": (
        "function recordDeniedAudit",
        'problemCode: "UNAUTHORIZED"',
        'actorKind: "unknown"',
    ),
    "scripts/verify_api_security_runtime_smoke.sh": (
        "assert_auth_denial_event",
        'AUTH_DENIAL_EVENT_COUNT="$protected_route_count"',
        "expected_audit_lines_before_replay=$((AUTH_DENIAL_EVENT_COUNT + 18))",
        'head -n "$audit_lines_before_replay"',
    ),
}
for relative, required in markers.items():
    source = (root / relative).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"runtime contract lost marker in {relative}: {marker}")

browser = (root / "test/e2e/pr_dt_request_correlation.spec.ts").read_text(encoding="utf-8")
if re.search(r'test\.(?:skip|fixme)\("PR-BE ', browser):
    fail("PR-BE browser fixture is skipped")
if browser.count('test("PR-BE ') != 1:
    fail("browser contract must keep one focused PR-BE scenario")
for marker in (
    "trading.kill_switch.set",
    "venue_credentials.update",
    "hedge.confirm",
    'expect(audit.body.events).toHaveLength(3)',
    'not.toContain("wrong-bearer-secret")',
):
    if marker not in browser:
        fail(f"browser evidence marker missing: {marker}")

repo = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
static = 'PR_BE_STATIC_ONLY=1 bash "$ROOT/scripts/check_pr_be_completion.sh"'
if repo.count(static) != 2:
    fail("completion gate must be wired into both repository scopes")
listing = 'npx playwright test "$ROOT/test/e2e/pr_dt_request_correlation.spec.ts" --grep "PR-BE" --list'
if repo.count(listing) != 1:
    fail("repository browser fixture listing is missing")

print(f"OK PR-BE audit correlation contract ({len(contract)} evidence types; excluded from active queue)")
PY

if [[ "${PR_BE_STATIC_ONLY:-0}" == "1" ]]; then exit 0; fi

bash "$ROOT/scripts/check_product_audit_progress.sh"
bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_evidence_index.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"
bash "$ROOT/scripts/check_mutation_audit_contract.sh"
PR_DK_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dk_completion.sh"
bash "$ROOT/scripts/check_pr_fi_completion.sh"
PR_CK_SKIP_TESTS=1 PR_CK_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_ck_completion.sh"
PR_DT_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dt_completion.sh"

if [[ "${PR_BE_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS=8 cargo test -p api --bin crypto-arb-api auth_denial --no-fail-fast
  CARGO_BUILD_JOBS=8 cargo test -p api --bin crypto-arb-api services::action_runs --no-fail-fast
fi
if [[ "${PR_BE_SKIP_BROWSER:-0}" != "1" ]]; then
  CI=1 npx playwright test "$ROOT/test/e2e/pr_dt_request_correlation.spec.ts" \
    --grep "PR-BE" --workers=1
fi

printf 'OK PR-BE high-risk audit and request correlation contract\n'
