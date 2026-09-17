#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
AUDIT="$ROOT/crates/api/src/middleware/audit.rs"
AUDIT_EVENT="$ROOT/crates/api/src/middleware/audit/event.rs"
RESPONSE="$ROOT/crates/common/src/error/response.rs"
APP="$ROOT/crates/api/src/app.rs"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-DT completion gate failed: %s\n' "$1" >&2
  exit 1
}

if [[ "$MODE" == "--self-test" ]]; then
  audit_event_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-dt-audit-event.XXXXXX")"
  response_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-dt-response.XXXXXX")"
  app_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-dt-app.XXXXXX")"
  cp "$AUDIT_EVENT" "$audit_event_backup"
  cp "$RESPONSE" "$response_backup"
  cp "$APP" "$app_backup"
  restore() {
    cp "$audit_event_backup" "$AUDIT_EVENT"
    cp "$response_backup" "$RESPONSE"
    cp "$app_backup" "$APP"
    rm -f "$audit_event_backup" "$response_backup" "$app_backup"
  }
  trap restore EXIT

  python3 - "$AUDIT_EVENT" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "    #[serde(flatten)]\n    pub correlation: AuditCorrelation,"
if source.count(marker) != 1:
    raise SystemExit("PR-DT self-test setup failed: audit flatten marker drifted")
path.write_text(source.replace(marker, "    pub correlation: AuditCorrelation,"), encoding="utf-8")
PY
  if PR_DT_SKIP_TESTS=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a nested audit correlation object"
  fi
  cp "$audit_event_backup" "$AUDIT_EVENT"

  python3 - "$RESPONSE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "header::WWW_AUTHENTICATE"
if source.count(marker) != 2:
    raise SystemExit("PR-DT self-test setup failed: auth challenge marker drifted")
path.write_text(source.replace(marker, "header::AUTHORIZATION"), encoding="utf-8")
PY
  if PR_DT_SKIP_TESTS=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a disconnected WWW-Authenticate challenge"
  fi
  cp "$response_backup" "$RESPONSE"

  python3 - "$APP" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "        header::WWW_AUTHENTICATE,\n"
if source.count(marker) != 1:
    raise SystemExit("PR-DT self-test setup failed: auth CORS marker drifted")
path.write_text(source.replace(marker, ""), encoding="utf-8")
PY
  if PR_DT_SKIP_TESTS=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a browser-invisible WWW-Authenticate challenge"
  fi

  printf 'PR-DT completion self-test passed\n'
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
title = "PR-DT ApiProblem & Request Correlation Contract"
verify_anchor = "`bash scripts/check_pr_dt_completion.sh --self-test`"
evidence_contract = {
    "retry-after-parser": "frontend/src/api/rest.rs",
    "internal-error-redaction": "crates/common/src/error.rs",
    "auth-typed-response": "crates/common/src/error/response.rs",
    "auth-cors-exposure": "crates/api/src/app.rs",
    "auth-middleware-contract": "crates/api/src/middleware/auth.rs",
    "request-id-trace": "crates/api/src/middleware/trace.rs",
    "audit-correlation-schema": "crates/api/src/middleware/audit/correlation.rs",
    "audit-event-projection": "crates/api/src/middleware/audit.rs",
    "action-run-correlation": "crates/api/src/services/action_runs/audit_log.rs",
    "audit-replay-integrity": "crates/api/src/middleware/audit/replay.rs",
    "runtime-retry-saturation": "crates/api/src/services/runtime_problem.rs",
    "loadstate-no-swallow-governance": "scripts/verify_repo_gates.sh",
    "browser-contract-fixture": "test/e2e/fixtures/route_runtime_policy.mjs",
    "product-browser": "test/e2e/pr_dt_request_correlation.spec.ts",
    "product-suite-contract": "package.json",
    "completion-governance": "scripts/check_pr_dt_completion.sh",
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-DT completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be completed with no local remainder")
if row.count(verify_anchor) != 1:
    fail("roadmap verification must name the destructive completion gate")
queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-DT\b", queue):
    fail("completed PR-DT remains in the local queue")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-DT"]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for kind, artifact in evidence_contract.items():
    if indexed[kind]["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"{kind} artifact drifted or is missing")

markers = {
    "crates/common/src/error.rs": (
        "AUTHENTICATION_REQUIRED_MESSAGE",
        "provide_valid_bearer_token",
        "saturating_mul(1_000)",
    ),
    "crates/common/src/error/response.rs": (
        "header::WWW_AUTHENTICATE",
        "unauthorized_response_is_stable_typed_and_recoverable",
        "rate_limit_retry_after_saturates_instead_of_overflowing",
    ),
    "crates/api/src/app.rs": (
        "let response_headers = [\n        header::RETRY_AFTER,\n        header::WWW_AUTHENTICATE,\n        request_id_header,\n    ];",
        "header::ACCESS_CONTROL_EXPOSE_HEADERS",
        "prod_like_security_contract_rejects_no_auth_and_evil_origin",
    ),
    "crates/api/src/middleware/auth.rs": (
        "unauthorized_carries_request_id_behind_trace_layer",
        "header::WWW_AUTHENTICATE",
        "provide_valid_bearer_token",
    ),
    "crates/api/src/middleware/trace.rs": (
        "request.extensions_mut().insert(request_id)",
        "common::request_id::scope",
        "x-request-id",
    ),
    "crates/api/src/middleware/audit/correlation.rs": (
        "pub(crate) struct AuditCorrelation",
        "with_order_ids",
        "with_run_ids",
    ),
    "crates/api/src/middleware/audit.rs": (
        "mod event;",
        "pub(crate) use event::{AuditEvent, AuditEventContext, AuditResourceKind};",
        "event_serializes_correlation_as_top_level_fields",
    ),
    "crates/api/src/middleware/audit/event.rs": (
        "#[serde(flatten)]\n    pub correlation: AuditCorrelation,",
    ),
    "crates/api/src/services/action_runs/audit_log.rs": (
        "action_correlation(run)",
        '"/executionRun/runId"',
        '"/costReconciliation/evidenceOrderIds"',
    ),
    "crates/api/src/services/action_runs/audit_log/tests.rs": (
        "audit_detail_summarizes_close_run_cost_evidence",
        "audit_detail_summarizes_hedge_confirm_execution_evidence",
        "order_action_correlation_keeps_internal_client_and_exchange_ids",
    ),
    "crates/api/src/middleware/audit/replay.rs": (
        "validate_correlation(&entry, &run",
        "replay_rejects_top_level_request_or_action_identity_drift",
        "replay_accepts_matching_top_level_correlation",
    ),
    "crates/api/src/services/runtime_problem.rs": (
        "saturating_mul(1_000)",
        "rate_limit_retry_after_saturates",
    ),
    "scripts/verify_repo_gates.sh": (
        "frontend async API errors must not be erased with .await.ok()",
        "frontend resources must expose LoadState instead of module-local LocalResource<Option>",
        "check_pr_dt_completion.sh",
    ),
    "test/e2e/fixtures/route_runtime_policy.mjs": (
        'label: "high_risk_execution"',
        '"www-authenticate"',
        "orderIds",
        "runIds",
    ),
    "test/e2e/pr_dt_request_correlation.spec.ts": (
        "PR-DT auth 401 is typed, recoverable, and request-correlated",
        "PR-DT audit pairs expose action, order, and execution run identities",
        "PR-DT LoadState keeps auth failure visible without executable fallback rows",
    ),
}
for path, required in markers.items():
    source = (root / path).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"missing {path} marker: {marker}")

browser_path = evidence_contract["product-browser"]
browser = (root / browser_path).read_text(encoding="utf-8")
if re.search(r"\btest\.(?:skip|fixme)\s*\(", browser):
    fail("browser fixture must not be skipped")
if browser.count('test("PR-DT ') != 3:
    fail("browser fixture must keep all three non-skipping scenarios")

package = json.loads((root / "package.json").read_text(encoding="utf-8"))
scripts = package.get("scripts", {})
dedicated = "playwright test test/e2e/pr_dt_request_correlation.spec.ts"
if scripts.get("test:e2e:pr-dt") != dedicated:
    fail("dedicated browser command is missing")
if scripts.get("test:e2e:product", "").count(browser_path) != 1:
    fail("product suite must include the PR-DT fixture exactly once")
release_fixture = json.loads(
    (root / "scripts/fixtures/release_qa_contract/package.json").read_text(encoding="utf-8")
)
if release_fixture.get("scripts", {}).get("test:e2e:product", "").count(browser_path) != 1:
    fail("release QA fixture must include the PR-DT browser fixture exactly once")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
for path in evidence_contract.values():
    if path == "package.json":
        continue
    if coverage.get(path, {}).get("coverage_status") != "exact":
        fail(f"exact coverage missing for {path}")

print(
    f"OK PR-DT static contract ({len(evidence_contract)} evidence types; "
    "typed auth + request/action/order/run correlation + LoadState/Chromium closure)"
)
PY

bash "$ROOT/scripts/check_mutation_audit_contract.sh"
PR_EH_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_eh_completion.sh"
PR_DL_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dl_completion.sh"
bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"
bash "$ROOT/scripts/check_release_qa_contract.sh"

if [[ "${PR_DT_SKIP_TESTS:-0}" != "1" ]]; then
  jobs="${CARGO_BUILD_JOBS:-8}"
  CARGO_BUILD_JOBS="$jobs" cargo test -p common --features http error::response --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api audit --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api auth --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api runtime_problem --no-fail-fast
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-dt -- --workers=1
fi

printf 'OK PR-DT ApiProblem and request correlation completion contract\n'
