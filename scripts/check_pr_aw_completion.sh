#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
AUDIT="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
HISTORY="$ROOT/docs/audit_history/PRODUCT_AUDIT_HISTORY.md"
EVIDENCE="$ROOT/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
BOUNDARY="$ROOT/scripts/check_frontend_module_boundaries.sh"
BOUNDARY_SELF_TEST="$ROOT/scripts/check_frontend_module_boundaries_self_test.sh"
COMPONENT_GATE="$ROOT/scripts/check_pr_et_completion.sh"
READINESS_GATE="$ROOT/scripts/check_pr_db_completion.sh"
CI_WORKFLOW="$ROOT/.github/workflows/ci.yml"
RUNTIME_WRAPPER="$ROOT/scripts/verify_runtime_contracts_with_api.sh"
BROWSER_RUNTIME="$ROOT/test/e2e/data_pipeline.spec.ts"
BROWSER_RECOVERY="$ROOT/test/e2e/pr_cq_operator_qa.spec.ts"
MOCK_API="$ROOT/test/e2e/mock_api.mjs"
MATRIX="$ROOT/scripts/exchange_operation_evidence_matrix.tsv"
PACKAGE="$ROOT/package.json"
REPO_GATE="$ROOT/scripts/verify_repo_gates.sh"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-AW completion gate failed: %s\n' "$1" >&2
  exit 1
}

run_static_gate() {
  PR_AW_STATIC_ONLY=1 bash "$0" >/dev/null
}

if [[ "$MODE" == "--self-test" ]]; then
  tmp="$(mktemp -d "${TMPDIR:-/tmp}/crossline-pr-aw.XXXXXX")"
  files=(
    "$AUDIT"
    "$EVIDENCE"
    "$COMPONENT_GATE"
    "$READINESS_GATE"
    "$CI_WORKFLOW"
    "$RUNTIME_WRAPPER"
    "$BROWSER_RUNTIME"
    "$MOCK_API"
    "$MATRIX"
    "$PACKAGE"
    "$REPO_GATE"
  )
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

  mutate_once() {
    local file="$1"
    local old="$2"
    local new="$3"
    python3 - "$file" "$old" "$new" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
old, new = sys.argv[2:]
text = path.read_text(encoding="utf-8")
if old not in text:
    raise SystemExit(f"PR-AW self-test setup failed: marker missing in {path}")
path.write_text(text.replace(old, new, 1), encoding="utf-8")
PY
  }

  assert_rejected() {
    local file="$1"
    local old="$2"
    local new="$3"
    local message="$4"
    mutate_once "$file" "$old" "$new"
    if run_static_gate 2>/dev/null; then
      fail "$message"
    fi
    restore_file "$file"
  }

  run_static_gate
  assert_rejected "$AUDIT" \
    '| `PR-AW Verification Gate Coverage & Drift Control` | ✅ 完成 |' \
    '| `PR-AW Verification Gate Coverage & Drift Control` | 🟡 部分完成 |' \
    'self-test accepted a downgraded roadmap row'
  assert_rejected "$EVIDENCE" \
    $'PR-AW\tdeep-component-client-parser\t' \
    $'PR-AW-MISSING\tdeep-component-client-parser\t' \
    'self-test accepted incomplete exact evidence'
  assert_rejected "$COMPONENT_GATE" \
    'def require_no_direct_client_calls(root: Path) -> None:' \
    'def removed_direct_client_scan(root: Path) -> None:' \
    'self-test accepted a disabled component client parser'
  assert_rejected "$READINESS_GATE" \
    'route_status(router, "/api/trading/live-readiness")' \
    'route_status(router, "/api/trading/readiness")' \
    'self-test accepted a detached legacy readiness 404 assertion'
  assert_rejected "$CI_WORKFLOW" \
    'run: CI=1 npm run test:e2e:product' \
    'run: echo browser-smoke-disabled' \
    'self-test accepted CI without product browser smoke'
  assert_rejected "$RUNTIME_WRAPPER" \
    'ALLOW_RUNTIME_SKIP=0' \
    'ALLOW_RUNTIME_SKIP=1' \
    'self-test accepted a skippable runtime contract'
  assert_rejected "$BROWSER_RUNTIME" \
    'test("mock runtime contract smoke uses backend envelopes"' \
    'test.skip("mock runtime contract smoke uses backend envelopes"' \
    'self-test accepted a skipped runtime envelope fixture'
  assert_rejected "$MOCK_API" \
    'url.pathname === "/api/system/venue-runtime-health"' \
    'url.pathname === "/api/system/venue-runtime-health-disabled"' \
    'self-test accepted a missing mock runtime-health route'
  assert_rejected "$MATRIX" \
    $'Binance\trecorded\trecorded\trecorded\trecorded\t' \
    $'Binance\tunrecorded\trecorded\trecorded\trecorded\t' \
    'self-test accepted an unrecorded operation evidence bucket'
  assert_rejected "$PACKAGE" \
    '"test:e2e:pr-aw"' \
    '"test:e2e:pr-aw-disabled"' \
    'self-test accepted a missing focused browser command'
  assert_rejected "$AUDIT" \
    '1. **PR-AX Frontend Wasm & Module Debt Burn-down**' \
    '1. **PR-AW Verification Gate Coverage & Drift Control**' \
    'self-test accepted PR-AW reinserted into the local queue'
  assert_rejected "$REPO_GATE" \
    'PR_AW_STATIC_ONLY=1 bash "$ROOT/scripts/check_pr_aw_completion.sh"' \
    'true # PR-AW gate removed' \
    'self-test accepted single-scope repository wiring'

  printf 'PR-AW completion self-test passed\n'
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
    raise SystemExit(f"PR-AW completion gate failed: {message}")


title = "PR-AW Verification Gate Coverage & Drift Control"
row = next((line for line in audit.splitlines() if line.startswith(f"| `{title}` |")), None)
if row is None or "| ✅ 完成 |" not in row or "剩余：无。" not in row:
    fail("roadmap row is not complete with no local remainder")
if "`bash scripts/check_pr_aw_completion.sh --self-test`" not in row:
    fail("roadmap row lacks the canonical destructive verification anchor")
if "> 本轮 PR-AW：" not in audit:
    fail("top progress summary lacks the PR-AW closure note")

queue = audit.split("### 🟡 6.5 下一步执行队列", 1)[-1]
numbered = re.findall(r"(?m)^\d+\. \*\*(PR-[A-Z]+)", queue)
successor_order = ("PR-AX", "PR-AY", "PR-AZ", "PR-BB", "PR-BC")
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
    or "PR-AW" in numbered
):
    fail(f"queue handoff drifted: expected={incomplete_successors}, actual={numbered[:len(incomplete_successors)]}")
if "## 2026-07-19 PR-AW Verification Coverage and Drift Control Closure" not in history:
    fail("history closure appendix is missing")

evidence_contract = {
    "repository-aggregate": "scripts/verify_repo_gates.sh",
    "ci-runtime-browser": ".github/workflows/ci.yml",
    "release-qa-contract": "scripts/check_release_qa_contract.sh",
    "component-boundary": "scripts/check_frontend_module_boundaries.sh",
    "component-boundary-negative-fixture": "scripts/check_frontend_module_boundaries_self_test.sh",
    "deep-component-client-parser": "scripts/check_pr_et_completion.sh",
    "legacy-readiness-removal": "scripts/check_pr_db_completion.sh",
    "legacy-copy-boundary": "scripts/product_copy_gate.sh",
    "route-inventory-boundary": "scripts/check_route_inventory.sh",
    "runtime-required-wrapper": "scripts/verify_runtime_contracts_with_api.sh",
    "runtime-contract-probes": "scripts/verify_runtime_contracts.sh",
    "runtime-no-skip-governance": "scripts/check_pr_aa_completion.sh",
    "runtime-envelope-browser": "test/e2e/data_pipeline.spec.ts",
    "operator-recovery-browser": "test/e2e/pr_cq_operator_qa.spec.ts",
    "mock-runtime-fixture": "test/e2e/mock_api.mjs",
    "package-focused-browser": "package.json",
    "operation-evidence-matrix": "scripts/exchange_operation_evidence_matrix.tsv",
    "operation-matrix-gate": "scripts/check_exchange_operation_evidence_matrix.sh",
    "schema-evidence-governance": "scripts/check_pr_bz_completion.sh",
    "evidence-ledger-validator": "scripts/check_product_audit_evidence.sh",
    "coverage-ledger-validator": "scripts/check_product_audit_coverage.sh",
    "successor-aware-queue-aq": "scripts/check_pr_aq_completion.sh",
    "successor-aware-queue-ar": "scripts/check_pr_ar_completion.sh",
    "successor-aware-queue-as": "scripts/check_pr_as_completion.sh",
    "successor-aware-queue-au": "scripts/check_pr_au_completion.sh",
    "successor-aware-queue-av": "scripts/check_pr_av_completion.sh",
    "completion-governance": "scripts/check_pr_aw_completion.sh",
}
with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [item for item in csv.DictReader(handle, delimiter="\t") if item["pr_id"] == "PR-AW"]
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

component = (root / "scripts/check_pr_et_completion.sh").read_text(encoding="utf-8")
for marker in (
    "def require_no_direct_client_calls(root: Path) -> None:",
    r"use_global\(\)\.(?:client|settings_client)",
    r"\b(?:client|settings_client)\.[A-Za-z_]",
    r"\bspawn_local\s*\(",
):
    if marker not in component:
        fail(f"deep component/client parser lost marker: {marker}")
boundary_self_test = (root / "scripts/check_frontend_module_boundaries_self_test.sh").read_text(encoding="utf-8")
if "expect_failure bad_component_client" not in boundary_self_test:
    fail("component/client negative fixture is detached")

readiness = (root / "scripts/check_pr_db_completion.sh").read_text(encoding="utf-8")
if 'route_status(router, "/api/trading/live-readiness")' not in readiness or "StatusCode::NOT_FOUND" not in readiness:
    fail("legacy live-readiness 404 contract is detached")
if "live-readiness" not in (root / "scripts/product_copy_gate.sh").read_text(encoding="utf-8"):
    fail("product copy gate no longer rejects legacy live-readiness")
if "live-readiness" not in (root / "scripts/check_route_inventory.sh").read_text(encoding="utf-8"):
    fail("route inventory no longer rejects legacy live-readiness")

workflow = (root / ".github/workflows/ci.yml").read_text(encoding="utf-8")
for marker in (
    "  runtime-contracts:",
    "RUNTIME_BUILD_API=0 bash scripts/verify_runtime_contracts_with_api.sh",
    "  browser-smoke:",
    "run: CI=1 npm run test:e2e:product",
):
    if marker not in workflow:
        fail(f"CI verification contract lost marker: {marker}")
runtime_wrapper = (root / "scripts/verify_runtime_contracts_with_api.sh").read_text(encoding="utf-8")
if runtime_wrapper.count("ALLOW_RUNTIME_SKIP=0") != 2 or "ALLOW_RUNTIME_SKIP=1" in runtime_wrapper:
    fail("runtime-required wrapper became skippable")
runtime_contracts = (root / "scripts/verify_runtime_contracts.sh").read_text(encoding="utf-8")
for marker in ("probe_json_budget", "probe_gzip_budget", "opportunity_rate_limit_error", "accountBindings"):
    if marker not in runtime_contracts:
        fail(f"runtime contract probe lost marker: {marker}")

browser_contracts = {
    "test/e2e/data_pipeline.spec.ts": (
        "mock runtime contract smoke uses backend envelopes",
        "PR-CO visual smoke keeps core tables stable across viewports",
    ),
    "test/e2e/pr_cq_operator_qa.spec.ts": (
        "PR-CQ credential save failure exits pending and records operator-visible evidence",
        "PR-CQ execution submit failure exits pending and remains retryable",
    ),
}
for relative, titles in browser_contracts.items():
    source = (root / relative).read_text(encoding="utf-8")
    for browser_title in titles:
        if f'test("{browser_title}"' not in source:
            fail(f"browser fixture is missing or skipped: {browser_title}")
        if re.search(rf"test\.(?:skip|fixme)\(\"{re.escape(browser_title)}\"", source):
            fail(f"browser fixture is skipped: {browser_title}")
mock_api = (root / "test/e2e/mock_api.mjs").read_text(encoding="utf-8")
for marker in (
    'url.pathname === "/api/system/venue-runtime-health"',
    "function venueRuntimeHealth(scenario)",
    "currentlyUsable:",
):
    if marker not in mock_api:
        fail(f"mock runtime-health fixture lost marker: {marker}")

focused = (
    'playwright test test/e2e/data_pipeline.spec.ts test/e2e/pr_cq_operator_qa.spec.ts --grep '
    '"mock runtime contract smoke uses backend envelopes|PR-CO visual smoke keeps core tables stable across viewports|'
    'PR-CQ credential save failure exits pending and records operator-visible evidence|'
    'PR-CQ execution submit failure exits pending and remains retryable"'
)
package = json.loads((root / "package.json").read_text(encoding="utf-8"))
scripts = package.get("scripts", {})
if scripts.get("test:e2e:pr-aw") != focused:
    fail("package PR-AW focused command is missing")
for browser_path in browser_contracts:
    if scripts.get("test:e2e:product", "").count(browser_path) != 1:
        fail(f"product suite must include {browser_path} exactly once")
    if browser_path not in (root / "scripts/check_release_qa_contract.sh").read_text(encoding="utf-8"):
        fail(f"release QA command lock drifted: {browser_path}")

with (root / "scripts/exchange_operation_evidence_matrix.tsv").open(encoding="utf-8", newline="") as handle:
    matrix = list(csv.DictReader(handle, delimiter="\t"))
expected_venues = {"Binance", "Okx", "Bybit", "Bitget", "Gate", "Htx", "Kucoin", "Hyperliquid"}
if len(matrix) != 8 or {item["venue"] for item in matrix} != expected_venues:
    fail("operation evidence matrix must cover exactly eight venues")
rest_columns = (
    "rest_trade_write_order_ack",
    "rest_private_order_status",
    "rest_private_account_balance",
    "rest_private_account_position",
)
for item in matrix:
    if any(item[column] != "recorded" for column in rest_columns):
        fail(f"operation evidence matrix has an unrecorded REST bucket: {item['venue']}")
    if item["ws_private_stream_evidence"] != "recorded":
        fail(f"private stream evidence is not recorded: {item['venue']}")
    if item["close_position_boundary"] != "display_only_without_operation_evidence":
        fail(f"close-position boundary drifted: {item['venue']}")
    if item["finality_boundary"] != "ack_not_final":
        fail(f"ACK/finality boundary drifted: {item['venue']}")
matrix_gate = (root / "scripts/check_exchange_operation_evidence_matrix.sh").read_text(encoding="utf-8")
for marker in ("matrix must contain exactly 8 venue rows", "WS evidence tests locked", "--self-test"):
    if marker not in matrix_gate:
        fail(f"operation matrix gate lost marker: {marker}")

repo_gate = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
wiring = 'PR_AW_STATIC_ONLY=1 bash "$ROOT/scripts/check_pr_aw_completion.sh"'
if repo_gate.count(wiring) != 2:
    fail("completion gate must be wired into both repository scopes")

print(f"OK PR-AW verification contract ({len(evidence_contract)} evidence types; queue={numbered[0]})")
PY

if [[ "${PR_AW_STATIC_ONLY:-0}" == "1" ]]; then
  exit 0
fi

bash "$ROOT/scripts/check_product_audit_progress.sh"
bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_evidence_index.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"
bash "$ROOT/scripts/check_release_qa_contract.sh"
node --check "$MOCK_API"

if [[ "${PR_AW_SKIP_UPSTREAM:-0}" != "1" ]]; then
  bash "$BOUNDARY"
  bash "$BOUNDARY_SELF_TEST"
  bash "$COMPONENT_GATE"
  PR_DB_SKIP_TESTS=1 PR_DB_SKIP_UPSTREAM=1 PR_DB_SKIP_BROWSER_LIST=1 bash "$READINESS_GATE"
  PR_AA_SKIP_TESTS=1 PR_AA_SKIP_UPSTREAM=1 PR_AA_SKIP_BROWSER_LIST=1 \
    bash "$ROOT/scripts/check_pr_aa_completion.sh"
  PR_BZ_SKIP_TESTS=1 PR_BZ_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_bz_completion.sh"
  bash "$ROOT/scripts/check_exchange_operation_evidence_matrix.sh" --self-test
  bash "$ROOT/scripts/check_exchange_operation_evidence_matrix.sh"
fi

if [[ "${PR_AW_SKIP_RUNTIME:-0}" != "1" ]]; then
  bash "$RUNTIME_WRAPPER"
fi

if [[ "${PR_AW_SKIP_BROWSER:-0}" != "1" ]]; then
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-aw
fi

printf 'OK PR-AW verification coverage and drift-control contract\n'
