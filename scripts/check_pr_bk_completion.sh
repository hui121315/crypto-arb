#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
STATUS_BAR="$ROOT/frontend/src/panels/status_bar/slots/operation.rs"
RUNTIME="$ROOT/scripts/verify_runtime_contracts.sh"
BROWSER="$ROOT/test/e2e/pr_bk_observability.spec.ts"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-BK completion gate failed: %s\n' "$1" >&2
  exit 1
}

if [[ "$MODE" == "--self-test" ]]; then
  backup_dir="$(mktemp -d "${TMPDIR:-/tmp}/crossline-pr-bk.XXXXXX")"
  cp "$STATUS_BAR" "$backup_dir/operation.rs"
  cp "$RUNTIME" "$backup_dir/verify_runtime_contracts.sh"
  cp "$BROWSER" "$backup_dir/pr_bk_observability.spec.ts"
  restore() {
    cp "$backup_dir/operation.rs" "$STATUS_BAR"
    cp "$backup_dir/verify_runtime_contracts.sh" "$RUNTIME"
    cp "$backup_dir/pr_bk_observability.spec.ts" "$BROWSER"
    rm -rf "$backup_dir"
  }
  trap restore EXIT

  PR_BK_SKIP_TESTS=1 PR_BK_SKIP_UPSTREAM=1 bash "$0" >/dev/null

  perl -0pi -e 's/"HTTP RTT"/"HTTP request elapsed"/' "$STATUS_BAR"
  if PR_BK_SKIP_TESTS=1 PR_BK_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a detached HTTP RTT product label"
  fi
  cp "$backup_dir/operation.rs" "$STATUS_BAR"

  perl -0pi -e 's/^probe_metrics_contract\n//m' "$RUNTIME"
  if PR_BK_SKIP_TESTS=1 PR_BK_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted runtime contracts without the Prometheus probe"
  fi
  cp "$backup_dir/verify_runtime_contracts.sh" "$RUNTIME"

  perl -0pi -e 's/test\("PR-BK safe permission/test.skip("PR-BK safe permission/' "$BROWSER"
  if PR_BK_SKIP_TESTS=1 PR_BK_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a skipped permission-boundary browser fixture"
  fi

  printf 'PR-BK completion self-test passed\n'
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
title = "PR-BK Observability Metrics & Runtime Diagnostics Contract"
verify_anchor = "`bash scripts/check_pr_bk_completion.sh --self-test`"
evidence = {
    "exchange-http-rtt-source": "crates/exchange/src/http.rs",
    "exchange-http-rtt-deterministic-test": "crates/exchange/tests/http_test.rs",
    "operation-health-rtt-projection": "crates/api/src/services/venue_operation_health/snapshot/part_03.rs",
    "prometheus-http-rtt-contract": "crates/api/src/routers/metrics.rs",
    "permission-probe-matrix": "crates/api/src/services/venue_credentials/validation/order_permission_matrix_tests.rs",
    "permission-probe-runtime-boundary": "crates/api/src/services/venue_operation_health/tests/safe_probe_endpoint_tests.rs",
    "settings-rtt-contract": "frontend/src/panels/modules/settings/tabs/diagnostics/ws_rtt.rs",
    "status-bar-rtt-contract": "frontend/src/panels/status_bar/slots/operation.rs",
    "real-api-runtime-smoke": "scripts/verify_runtime_contracts.sh",
    "non-skipping-browser-smoke": "test/e2e/pr_bk_observability.spec.ts",
    "product-suite-contract": "package.json",
    "release-suite-contract": "scripts/fixtures/release_qa_contract/package.json",
    "release-suite-gate": "scripts/check_release_qa_contract.sh",
    "completion-governance": "scripts/check_pr_bk_completion.sh",
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-BK completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be completed with no local remainder")
if row.count(verify_anchor) != 1:
    fail("roadmap verification must name the destructive completion gate")
queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-BK\b", queue):
    fail("completed PR-BK remains in the local queue")

history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(encoding="utf-8")
if "## 2026-07-15 PR-BK Observability Metrics & Runtime Diagnostics Contract Closure" not in history:
    fail("history closure appendix is missing")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-BK"]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence):
    fail(f"evidence type drift: expected={sorted(evidence)}, actual={sorted(indexed)}")
for kind, artifact in evidence.items():
    if indexed[kind]["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"{kind} artifact drifted or is missing")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
coverage_exceptions = {"package.json", "scripts/fixtures/release_qa_contract/package.json"}
for artifact in evidence.values():
    if artifact in coverage_exceptions:
        continue
    if coverage.get(artifact, {}).get("coverage_status") != "exact":
        fail(f"exact coverage missing for {artifact}")

markers = {
    "crates/exchange/src/http.rs": (
        "let started = Instant::now();",
        "match req.send().await",
        "let transport_rtt_ms = elapsed_ms(started);",
    ),
    "crates/exchange/tests/http_test.rs": (
        "transport_rtt_stops_at_response_headers_before_body_wait",
        "http_outcome_metrics_snapshot()",
        "HTTP send boundary must finish before response body is released",
    ),
    "crates/api/src/services/venue_operation_health/snapshot/part_03.rs": (
        "latency_ms: Some(snapshot.last_latency_ms)",
        "latency_p95_ms: snapshot.latency_p95_ms",
    ),
    "crates/api/src/routers/metrics.rs": (
        "crypto_arb_http_request_last_latency_ms",
        "crypto_arb_http_request_last_retry_after_ms",
        "crypto_arb_http_request_last_observed_at_ms",
        "crypto_arb_http_request_latency_ms_p95_bucket",
    ),
    "crates/api/src/services/venue_credentials/validation/order_permission_matrix_tests.rs": (
        "save_time_order_permission_probe_matrix_matches_validator_wiring",
    ),
    "crates/api/src/services/venue_credentials/validation/order_permission_matrix_tests/endpoints.rs": (
        "save_time_order_permission_probe_sources_have_endpoint_specs",
    ),
    "crates/api/src/services/venue_operation_health/tests/safe_probe_endpoint_tests.rs": (
        "safe_order_permission_probe_matrix_uses_registered_endpoint_evidence_without_live_write",
        "safe_order_permission_probe_does_not_satisfy_live_runtime_rows",
        "does_not_grant_live_write=true",
    ),
    "frontend/src/panels/modules/settings/tabs/diagnostics/ws_rtt.rs": (
        "交易所 HTTP RTT = 后端 outbound request send() 到响应头返回的耗时",
        "不含 HostGate/singleflight/RateLimiter 本地等待和响应体处理",
    ),
    "frontend/src/panels/status_bar/slots/operation.rs": ('"HTTP RTT"',),
    "scripts/verify_runtime_contracts.sh": (
        "probe_metrics_contract()",
        "probe_metrics_contract",
        "crypto_arb_http_request_last_latency_ms gauge",
        "crypto_arb_http_request_latency_ms_p95_bucket gauge",
    ),
    "test/e2e/pr_bk_observability.spec.ts": (
        "PR-BK HTTP RTT stays traceable across operation health, top bar, and Settings",
        "PR-BK safe permission probes remain fail closed until live runtime proof",
        "does_not_grant_live_write=true",
    ),
    "scripts/check_release_qa_contract.sh": ("test/e2e/pr_bk_observability.spec.ts",),
}
for relative, required_markers in markers.items():
    source = (root / relative).read_text(encoding="utf-8")
    for marker in required_markers:
        if marker not in source:
            fail(f"{relative} lost marker: {marker}")

runtime_contract = (root / "scripts/verify_runtime_contracts.sh").read_text(encoding="utf-8")
if runtime_contract.count("probe_metrics_contract") != 2:
    fail("runtime contract must define and execute the Prometheus probe exactly once")

browser = (root / "test/e2e/pr_bk_observability.spec.ts").read_text(encoding="utf-8")
if re.search(r"\btest\.(?:skip|fixme)\s*\(", browser):
    fail("browser evidence may not be skipped or marked fixme")
if len(re.findall(r"(?m)^test\(", browser)) != 2:
    fail("browser fixture must expose exactly two runnable PR-BK scenarios")

product = json.loads((root / "package.json").read_text(encoding="utf-8"))
release = json.loads(
    (root / "scripts/fixtures/release_qa_contract/package.json").read_text(encoding="utf-8")
)
expected = "playwright test test/e2e/pr_bk_observability.spec.ts"
if product.get("scripts", {}).get("test:e2e:pr-bk") != expected:
    fail("package.json lost the dedicated PR-BK browser command")
for name, package in (("product", product), ("release", release)):
    if "test/e2e/pr_bk_observability.spec.ts" not in package.get("scripts", {}).get(
        "test:e2e:product", ""
    ):
        fail(f"{name} product suite lost the PR-BK browser fixture")

repo_gate = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
if repo_gate.count("check_pr_bk_completion.sh") < 2:
    fail("repo gate must run the PR-BK completion contract in docs and all scopes")
if "pr_bk_observability.spec.ts\" --list" not in repo_gate:
    fail("repo gate must enumerate the PR-BK browser fixture")

print(f"OK PR-BK static completion contract ({len(evidence)} evidence types)")
PY

bash -n "$RUNTIME"
npx playwright test "$BROWSER" --list >/dev/null

if [[ "${PR_BK_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-10}" \
    cargo test -p exchange --test http_test \
      transport_rtt_stops_at_response_headers_before_body_wait --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-10}" \
    cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib --no-fail-fast
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-bk
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-10}" \
    RUNTIME_STARTUP_TIMEOUT_SECS="${RUNTIME_STARTUP_TIMEOUT_SECS:-180}" \
    bash "$ROOT/scripts/verify_runtime_contracts_with_api.sh"
fi

if [[ "${PR_BK_SKIP_UPSTREAM:-0}" != "1" ]]; then
  bash "$ROOT/scripts/check_product_audit_progress.sh"
  bash "$ROOT/scripts/check_product_audit_evidence.sh"
  bash "$ROOT/scripts/check_product_audit_evidence_index.sh"
  bash "$ROOT/scripts/check_product_audit_coverage.sh"
  bash "$ROOT/scripts/check_release_qa_contract.sh"
fi

printf 'PR-BK completion gate passed\n'
