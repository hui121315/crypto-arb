#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
DOC="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
EVIDENCE="$ROOT/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
BROWSER="$ROOT/test/e2e/pr_bk_observability.spec.ts"
PREFLIGHT="$ROOT/crates/api/src/services/hedge_preflight/live_health/credentials/part_01.rs"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-CM completion gate failed: %s\n' "$1" >&2
  exit 1
}

if [[ "$MODE" == "--self-test" ]]; then
  backup_dir="$(mktemp -d "${TMPDIR:-/tmp}/crossline-pr-cm.XXXXXX")"
  cp "$DOC" "$backup_dir/audit.md"
  cp "$EVIDENCE" "$backup_dir/evidence.tsv"
  cp "$BROWSER" "$backup_dir/browser.ts"
  cp "$PREFLIGHT" "$backup_dir/preflight.rs"
  restore() {
    cp "$backup_dir/audit.md" "$DOC"
    cp "$backup_dir/evidence.tsv" "$EVIDENCE"
    cp "$backup_dir/browser.ts" "$BROWSER"
    cp "$backup_dir/preflight.rs" "$PREFLIGHT"
  }
  cleanup() {
    restore
    rm -rf "$backup_dir"
  }
  trap cleanup EXIT

  PR_CM_SKIP_TESTS=1 PR_CM_SKIP_UPSTREAM=1 bash "$0" >/dev/null

  python3 - "$DOC" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "| `PR-BK Observability Metrics & Runtime Diagnostics Contract` | ✅ 完成 |"
if source.count(marker) != 1:
    raise SystemExit("PR-CM self-test setup failed: PR-BK authority row drifted")
path.write_text(source.replace(marker, marker.replace("✅ 完成", "🟡 部分完成"), 1), encoding="utf-8")
PY
  if PR_CM_SKIP_TESTS=1 PR_CM_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a downgraded HTTP RTT authority"
  fi
  restore

  python3 - "$EVIDENCE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
rows = path.read_text(encoding="utf-8").splitlines()
for index, row in enumerate(rows):
    if row.startswith("PR-CM\truntime-health-dto\t"):
        rows.pop(index)
        path.write_text("\n".join(rows) + "\n", encoding="utf-8")
        break
else:
    raise SystemExit("PR-CM self-test setup failed: evidence row missing")
PY
  if PR_CM_SKIP_TESTS=1 PR_CM_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted an incomplete evidence matrix"
  fi
  restore

  python3 - "$BROWSER" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = 'test("PR-BK safe permission probes remain fail closed until live runtime proof"'
if source.count(marker) != 1:
    raise SystemExit("PR-CM self-test setup failed: permission browser marker drifted")
path.write_text(
    source.replace(marker, marker.replace("test(", "test.skip("), 1),
    encoding="utf-8",
)
PY
  if PR_CM_SKIP_TESTS=1 PR_CM_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a skipped permission-boundary browser fixture"
  fi
  restore

  python3 - "$PREFLIGHT" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "            HedgePreflightOperation::OrderFinality,\n"
if source.count(marker) != 1:
    raise SystemExit("PR-CM self-test setup failed: finality scope marker drifted")
path.write_text(source.replace(marker, "", 1), encoding="utf-8")
PY
  if PR_CM_SKIP_TESTS=1 PR_CM_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted finality outside the scoped preflight"
  fi
  restore

  python3 - "$DOC" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "### 🟡 6.5 下一步执行队列"
if source.count(marker) != 1:
    raise SystemExit("PR-CM self-test setup failed: queue heading drifted")
stale = "\n\n1. **PR-CM Venue API Status Center & Runtime Probe Contract** — stale completed row"
path.write_text(source.replace(marker, marker + stale, 1), encoding="utf-8")
PY
  if PR_CM_SKIP_TESTS=1 PR_CM_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted completed PR-CM at the local queue head"
  fi

  printf 'PR-CM completion self-test passed\n'
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
title = "PR-CM Venue API Status Center & Runtime Probe Contract"
verify_anchor = "`bash scripts/check_pr_cm_completion.sh --self-test`"
authorities = (
    "PR-EG API Operation Health & Status Center Contract",
    "PR-BK Observability Metrics & Runtime Diagnostics Contract",
    "PR-BX VenueRuntimeHealth & Settings Diagnostics",
    "PR-DV HedgeTicket Scoped Preflight & Confirm Contract",
    "PR-FF Credential Health, Secret Store & Settings API Status Contract",
)
evidence_contract = {
    "runtime-health-dto": ("shared-types/src/venues/runtime_health.rs", "runtime_health"),
    "runtime-health-projection": ("shared-types/src/venues/runtime_health_snapshot.rs", "runtime_health"),
    "readonly-runtime-api": ("crates/api/src/routers/system.rs", "routers::system::tests"),
    "operation-health-source": ("crates/api/src/services/venue_operation_health/snapshot/part_01.rs", "venue_operation_health"),
    "exchange-http-rtt-source": ("crates/exchange/src/http.rs", "transport_rtt_stops_at_response_headers_before_body_wait"),
    "exchange-http-rtt-test": ("crates/exchange/tests/http_test.rs", "transport_rtt_stops_at_response_headers_before_body_wait"),
    "safe-order-permission": ("crates/api/src/services/venue_credentials/validation/safe_order_permission.rs", "safe_order_permission_probes_do_not_grant_live_readiness"),
    "permission-runtime-boundary": ("crates/api/src/services/venue_operation_health/tests/safe_probe_endpoint_tests.rs", "safe_order_permission_probe_does_not_satisfy_live_runtime_rows"),
    "live-order-proof-runtime": ("crates/api/src/services/live_order_proof_health/transitions/part_01.rs", "live_order_proof_health"),
    "order-write-runtime-projection": ("crates/api/src/services/venue_operation_health/snapshot/part_02.rs", "venue_operation_health"),
    "run-finality-runtime": ("crates/api/src/services/run_finality_health.rs", "run_finality_health"),
    "order-finality-runtime-projection": ("crates/api/src/services/venue_operation_health/snapshot/part_06.rs", "venue_operation_health"),
    "scoped-live-preflight": ("crates/api/src/services/hedge_preflight/live_health/credentials/part_01.rs", "services::hedge_preflight::tests"),
    "confirm-time-recheck": ("crates/api/src/services/hedge_preflight/order_submission.rs", "confirm_scoped_preflight_returns_every_blocked_guard"),
    "settings-runtime-matrix": ("frontend/src/panels/modules/settings/tabs/diagnostics/runtime_matrix.rs", "runtime_health"),
    "settings-selected-venue-runtime": ("frontend/src/panels/modules/settings/tabs/venue_credentials/panels/operation_health.rs", "runtime_selection"),
    "runtime-health-browser": ("test/e2e/pr_eg_runtime_health.spec.ts", "test:e2e:pr-eg"),
    "observability-browser": ("test/e2e/pr_bk_observability.spec.ts", "test:e2e:pr-bk"),
    "runtime-separation-browser": ("test/e2e/pr_bx_runtime.spec.ts", "test:e2e:pr-bx"),
    "scoped-preflight-browser": ("test/e2e/pr_dv_scoped_preflight.spec.ts", "test:e2e:pr-dv"),
    "operation-health-authority": ("scripts/check_pr_eg_completion.sh", "check_pr_eg_completion.sh"),
    "rtt-permission-authority": ("scripts/check_pr_bk_completion.sh", "check_pr_bk_completion.sh"),
    "runtime-usability-authority": ("scripts/check_pr_bx_completion.sh", "check_pr_bx_completion.sh"),
    "scoped-preflight-authority": ("scripts/check_pr_dv_completion.sh", "check_pr_dv_completion.sh"),
    "credential-boundary-authority": ("scripts/check_pr_ff_completion.sh", "check_pr_ff_completion.sh"),
    "repo-gate-wiring": ("scripts/verify_repo_gates.sh", "verify_repo_gates.sh"),
    "completion-governance": ("scripts/check_pr_cm_completion.sh", "check_pr_cm_completion.sh --self-test"),
}
test_anchors = (
    ("shared-types/src/venues/tests_runtime_health.rs", "snapshot_groups_normalized_venues_into_all_runtime_slots"),
    ("crates/api/src/routers/system.rs", "api_projects_existing_operation_snapshot_without_external_probes"),
    ("crates/exchange/tests/http_test.rs", "transport_rtt_stops_at_response_headers_before_body_wait"),
    ("crates/api/src/services/venue_operation_health/tests/safe_probe_endpoint_tests.rs", "safe_order_permission_probe_does_not_satisfy_live_runtime_rows"),
    ("crates/api/src/services/venue_operation_health/snapshot/tests/part_04.rs", "order_write_credentials_do_not_become_live_proof"),
    ("crates/api/src/services/venue_operation_health/snapshot/tests/part_04.rs", "run_finality_failure_maps_to_problem_and_evidence"),
    ("crates/api/src/services/hedge_preflight/tests/cases_live_a/cases/part_01.rs", "live_operation_health_guard_blocks_unknown_order_permission"),
    ("crates/api/src/services/hedge_preflight/tests/cases_live_a/cases/part_02.rs", "live_operation_health_guard_blocks_missing_order_finality"),
    ("crates/api/src/services/hedge_preflight/tests/cases_live_b.rs", "live_operation_health_guard_scopes_all_evidence_to_ticket_venues"),
    ("crates/api/src/services/hedge_confirm/confirm_validate/tests.rs", "confirm_scoped_preflight_returns_every_blocked_guard"),
    ("frontend/src/panels/modules/settings/tabs/diagnostics/tests/search.rs", "operation_health_separates_configuration_from_current_usability"),
    ("frontend/src/panels/modules/settings/tabs/venue_credentials/tests_runtime/health.rs", "current_availability_requires_every_runtime_link_to_be_ok"),
)
browser_anchors = (
    ("test/e2e/pr_eg_runtime_health.spec.ts", "PR-EG Settings consumes typed venue runtime health and fails closed without evidence"),
    ("test/e2e/pr_bk_observability.spec.ts", "PR-BK HTTP RTT stays traceable across operation health, top bar, and Settings"),
    ("test/e2e/pr_bk_observability.spec.ts", "PR-BK safe permission probes remain fail closed until live runtime proof"),
    ("test/e2e/pr_bx_runtime.spec.ts", "PR-BX Settings separates configuration, capability, and current usability"),
    ("test/e2e/pr_dv_scoped_preflight.spec.ts", "PR-DV keeps confirm-time scoped preflight failure and correlation context visible"),
    ("test/e2e/pr_dv_scoped_preflight.spec.ts", "PR-DV restores submitted legs without promoting acknowledgements to Hedged"),
)
source_markers = {
    "shared-types/src/venues/runtime_health.rs": ("pub struct VenueRuntimeOperationHealth", "pub currently_usable: bool"),
    "shared-types/src/venues/runtime_health_snapshot.rs": ("pub struct VenueRuntimeHealthSnapshot", "pub currently_usable_count: usize"),
    "crates/api/src/services/venue_operation_health/snapshot/part_01.rs": ("order_write_runtime_rows(", "run_finality_runtime_rows("),
    "crates/api/src/services/venue_operation_health/snapshot/part_02.rs": ("SOURCE_LIVE_ORDER_PROOF_RUNTIME", "order_write_missing_live_proof_row"),
    "crates/api/src/services/venue_operation_health/snapshot/part_06.rs": ("SOURCE_RUN_FINALITY_RUNTIME", "run_finality_missing_row"),
    "crates/api/src/services/hedge_preflight/live_health/credentials/part_01.rs": ("HedgePreflightOperation::OrderWrite", "HedgePreflightOperation::OrderFinality"),
    "crates/api/src/services/hedge_preflight/order_submission.rs": ("order_write: OrderWriteCheck", "recheck_hedge_live_order_preflight_guards"),
    "crates/exchange/src/http.rs": ("match req.send().await", "let transport_rtt_ms = elapsed_ms(started);"),
    "frontend/src/panels/modules/settings/tabs/diagnostics/runtime_matrix.rs": ("交易运行状态中心", "runtime_operation_title"),
    "frontend/src/panels/modules/settings/tabs/venue_credentials/panels/operation_health.rs": ("当前可用性仅由运行态链路判定", "live place/cancel/finality"),
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-CM completion gate failed: {message}")


def require_incomplete_queue_head(doc: str, queue: str) -> None:
    matched = re.search(r"(?m)^1\.\s+\*\*(PR-[A-Z]+)\b", queue)
    if not matched:
        fail("local queue must retain a PR roadmap item at its head")
    head = matched.group(1)
    row = next((line for line in doc.splitlines() if line.startswith(f"| `{head} ")), None)
    if row is None or "✅ 完成" in row:
        fail(f"local queue head must be an incomplete roadmap row: {head}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
roadmap_start = doc.find("### 🟡 6.3")
roadmap_end = doc.find("### 🟡 6.4", roadmap_start)
if roadmap_start < 0 or roadmap_end < 0:
    fail("bounded roadmap section is missing")
roadmap = doc[roadmap_start:roadmap_end]


def roadmap_row(row_title: str) -> str:
    rows = [line for line in roadmap.splitlines() if line.startswith(f"| `{row_title}`")]
    if len(rows) != 1:
        fail(f"expected one roadmap row for {row_title}")
    return rows[0]


row = roadmap_row(title)
if "✅ 完成" not in row or "剩余：无" not in row or row.count(verify_anchor) != 1:
    fail("roadmap row must be complete, remaining-none and bound to the destructive gate")
for authority in authorities:
    if "✅ 完成" not in roadmap_row(authority):
        fail(f"successor authority is not complete: {authority}")

queue_start = doc.find("### 🟡 6.5")
queue = doc[queue_start:]
if re.search(r"(?m)^\d+\.\s+\*\*PR-CM\b", queue):
    fail("completed PR-CM remains in the local queue")
successor = roadmap_row("PR-CN Frontend Workstation State & Navigation Runtime Contract")
successor_complete = "✅ 完成" in successor and "剩余：无" in successor
successor_is_head = bool(re.search(r"(?m)^1\.\s+\*\*PR-CN\b", queue))
following = roadmap_row("PR-CQ Local Runtime & Operator QA Contract")
following_complete = "✅ 完成" in following and "剩余：无" in following
following_is_head = bool(re.search(r"(?m)^1\.\s+\*\*PR-CQ\b", queue))
if successor_complete:
    if successor_is_head:
        fail("completed PR-CN successor remains at the local queue head")
    if following_complete:
        if following_is_head:
            fail("completed PR-CQ following-successor remains at the local queue head")
        require_incomplete_queue_head(doc, queue)
    elif not following_is_head:
        fail("PR-CQ must become the local queue head after PR-CN completion")
elif not successor_is_head:
    fail("PR-CN must remain the local queue head until its completion contract closes")
external_marker = "**外部等待池"
if external_marker not in queue or "PR-CM" not in queue.split(external_marker, 1)[1]:
    fail("real live PR-CM evidence must remain in the external wait pool")

history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(encoding="utf-8")
if "## 2026-07-16 PR-CM Venue API Status Center & Runtime Probe Contract Closure" not in history:
    fail("history closure appendix is missing")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-CM"]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for kind, (artifact, command_anchor) in evidence_contract.items():
    evidence = indexed[kind]
    if evidence["artifact"] != artifact or command_anchor not in evidence["command"]:
        fail(f"evidence anchor drifted: {kind}")
    if not (root / artifact).is_file():
        fail(f"evidence artifact is missing: {artifact}")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
for artifact, _ in evidence_contract.values():
    row = coverage.get(artifact)
    if row is None or row["coverage_status"] != "exact" or not row["evidence"].startswith("line:"):
        fail(f"exact coverage missing for {artifact}")

for relative, markers in source_markers.items():
    source = (root / relative).read_text(encoding="utf-8")
    for marker in markers:
        if marker not in source:
            fail(f"source marker missing: {relative}:{marker}")


def uncommented(source: str) -> str:
    source = re.sub(r"/\*.*?\*/", "", source, flags=re.S)
    return re.sub(r"//[^\n]*", "", source)


for relative, function_name in test_anchors:
    source = uncommented((root / relative).read_text(encoding="utf-8"))
    pattern = re.compile(
        r"(?m)^\s*#\[(?:tokio::)?test(?:\([^\]]*\))?\]\s*\n"
        r"(?P<attrs>(?:\s*#\[[^\n]+\]\s*\n)*)"
        rf"\s*(?:async\s+)?fn\s+{re.escape(function_name)}\s*\("
    )
    match = pattern.search(source)
    if match is None:
        fail(f"runnable test anchor missing: {relative}::{function_name}")
    if "ignore" in match.group("attrs"):
        fail(f"test anchor is ignored: {relative}::{function_name}")

for relative, test_title in browser_anchors:
    source = (root / relative).read_text(encoding="utf-8")
    escaped = re.escape(test_title)
    if re.search(rf'(?m)^\s*test\.(?:skip|fixme)\(\s*["\']{escaped}["\']', source):
        fail(f"browser anchor is skipped: {test_title}")
    if not re.search(rf'(?m)^\s*test\(\s*["\']{escaped}["\']', source):
        fail(f"browser anchor is missing: {test_title}")

product = json.loads((root / "package.json").read_text(encoding="utf-8"))
release = json.loads((root / "scripts/fixtures/release_qa_contract/package.json").read_text(encoding="utf-8"))
browser_scripts = {
    "test:e2e:pr-eg": "test/e2e/pr_eg_runtime_health.spec.ts",
    "test:e2e:pr-bk": "test/e2e/pr_bk_observability.spec.ts",
    "test:e2e:pr-bx": "test/e2e/pr_bx_runtime.spec.ts",
    "test:e2e:pr-dv": "test/e2e/pr_dv_scoped_preflight.spec.ts",
}
for name, path in browser_scripts.items():
    if product.get("scripts", {}).get(name) != f"playwright test {path}":
        fail(f"dedicated browser command drifted: {name}")
    for package_name, package in (("product", product), ("release", release)):
        if package.get("scripts", {}).get("test:e2e:product", "").count(path) != 1:
            fail(f"{package_name} suite must include {path} exactly once")

repo_gate = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
if repo_gate.count("check_pr_cm_completion.sh") != 2:
    fail("repo gate must execute PR-CM exactly once in docs and all scopes")

print(
    f"OK PR-CM static contract ({len(evidence_contract)} evidence types; "
    f"{len(test_anchors)} runnable anchors; {len(browser_anchors)} browser anchors)"
)
PY

npx playwright test \
  "$ROOT/test/e2e/pr_eg_runtime_health.spec.ts" \
  "$ROOT/test/e2e/pr_bk_observability.spec.ts" \
  "$ROOT/test/e2e/pr_bx_runtime.spec.ts" \
  "$ROOT/test/e2e/pr_dv_scoped_preflight.spec.ts" \
  --list >/dev/null

if [[ "${PR_CM_SKIP_UPSTREAM:-0}" != "1" ]]; then
  PR_EG_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_eg_completion.sh"
  PR_BK_SKIP_TESTS=1 PR_BK_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_bk_completion.sh"
  bash "$ROOT/scripts/check_pr_bx_completion.sh"
  PR_DV_SKIP_TESTS=1 PR_DV_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_dv_completion.sh"
  bash "$ROOT/scripts/check_pr_ff_completion.sh"
  bash "$ROOT/scripts/check_product_audit_progress.sh"
  bash "$ROOT/scripts/check_product_audit_evidence.sh"
  bash "$ROOT/scripts/check_product_audit_evidence_index.sh"
  bash "$ROOT/scripts/check_product_audit_coverage.sh"
  bash "$ROOT/scripts/check_release_qa_contract.sh"
fi

if [[ "${PR_CM_SKIP_TESTS:-0}" != "1" ]]; then
  jobs="${CARGO_BUILD_JOBS:-10}"
  CARGO_BUILD_JOBS="$jobs" cargo test -p shared-types --lib runtime_health --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --test http_test \
    transport_rtt_stops_at_response_headers_before_body_wait --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api \
    venue_operation_health --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api \
    services::hedge_preflight::tests --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api \
    confirm_scoped_preflight_returns_every_blocked_guard --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" \
    --lib runtime_health --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" \
    --lib runtime_selection --no-fail-fast
  CI=1 npm --prefix "$ROOT" exec -- playwright test \
    test/e2e/pr_eg_runtime_health.spec.ts \
    test/e2e/pr_bk_observability.spec.ts \
    test/e2e/pr_bx_runtime.spec.ts \
    test/e2e/pr_dv_scoped_preflight.spec.ts \
    --workers=1
fi

printf 'PR-CM Venue API status center and runtime probe completion passed\n'
