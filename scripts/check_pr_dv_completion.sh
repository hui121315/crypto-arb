#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
CONFIRM="$ROOT/crates/api/src/services/hedge_confirm/confirm.rs"
LIVE_SCOPE="$ROOT/crates/api/src/services/hedge_preflight/live_health/credentials/part_01.rs"
BROWSER="$ROOT/test/e2e/pr_dv_scoped_preflight.spec.ts"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-DV completion gate failed: %s\n' "$1" >&2
  exit 1
}

if [[ "$MODE" == "--self-test" ]]; then
  confirm_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-dv-confirm.XXXXXX")"
  scope_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-dv-scope.XXXXXX")"
  browser_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-dv-browser.XXXXXX")"
  cp "$CONFIRM" "$confirm_backup"
  cp "$LIVE_SCOPE" "$scope_backup"
  cp "$BROWSER" "$browser_backup"
  restore() {
    cp "$confirm_backup" "$CONFIRM"
    cp "$scope_backup" "$LIVE_SCOPE"
    cp "$browser_backup" "$BROWSER"
    rm -f "$confirm_backup" "$scope_backup" "$browser_backup"
  }
  trap restore EXIT

  PR_DV_SKIP_TESTS=1 PR_DV_SKIP_UPSTREAM=1 bash "$0" >/dev/null

  python3 - "$CONFIRM" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "validate_confirm_scoped_preflight(state, &preview).await"
if source.count(marker) != 1:
    raise SystemExit("PR-DV self-test setup failed: confirm recheck marker drifted")
path.write_text(source.replace(marker, "Ok(())", 1), encoding="utf-8")
PY
  if PR_DV_SKIP_TESTS=1 PR_DV_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a disconnected confirm-time scoped preflight"
  fi
  cp "$confirm_backup" "$CONFIRM"

  python3 - "$LIVE_SCOPE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "            HedgePreflightOperation::OrderFinality,\n"
if source.count(marker) != 1:
    raise SystemExit("PR-DV self-test setup failed: finality scope marker drifted")
path.write_text(source.replace(marker, "", 1), encoding="utf-8")
PY
  if PR_DV_SKIP_TESTS=1 PR_DV_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted order finality outside the ticket scope"
  fi
  cp "$scope_backup" "$LIVE_SCOPE"

  python3 - "$BROWSER" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = '.not.toContainText("双腿完成")'
if source.count(marker) != 2:
    raise SystemExit("PR-DV self-test setup failed: ACK non-final assertions drifted")
path.write_text(source.replace(marker, '.toContainText("双腿完成")', 1), encoding="utf-8")
PY
  if PR_DV_SKIP_TESTS=1 PR_DV_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted acknowledgement promotion to Hedged"
  fi

  printf 'PR-DV completion self-test passed\n'
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
title = "PR-DV HedgeTicket Scoped Preflight & Confirm Contract"
verify_anchor = "`bash scripts/check_pr_dv_completion.sh --self-test`"
evidence_contract = {
    "shared-preflight-operation-contract": "shared-types/src/hedge.rs",
    "venue-capability-registry": "shared-types/src/venue_capabilities.rs",
    "ticket-scoped-live-health": "crates/api/src/services/hedge_preflight/live_health/credentials/part_01.rs",
    "confirm-order-preflight-recheck": "crates/api/src/services/hedge_preflight/order_submission.rs",
    "confirm-scoped-validation": "crates/api/src/services/hedge_confirm/confirm_validate.rs",
    "confirm-validation-tests": "crates/api/src/services/hedge_confirm/confirm_validate/tests.rs",
    "confirm-router-wiring": "crates/api/src/services/hedge_confirm/confirm.rs",
    "operation-health-registry": "crates/api/src/services/venue_operation_health/snapshot.rs",
    "frontend-operation-evidence": "frontend/src/panels/modules/execution/components/risk_preview/evidence.rs",
    "frontend-ticket-evidence": "frontend/src/panels/modules/execution/components/risk_preview/preflight.rs",
    "frontend-ticket-tests": "frontend/src/panels/modules/execution/components/risk_preview/tests.rs",
    "frontend-action-state": "frontend/src/panels/modules/execution/data/actions.rs",
    "frontend-run-context-restore": "frontend/src/panels/modules/execution/data/run/context.rs",
    "execution-finality-state": "shared-types/src/execution_run.rs",
    "product-browser": "test/e2e/pr_dv_scoped_preflight.spec.ts",
    "confirm-context-upstream-gate": "scripts/check_pr_df_completion.sh",
    "finality-upstream-gate": "scripts/check_pr_ea_completion.sh",
    "instrument-upstream-gate": "scripts/check_pr_eb_completion.sh",
    "account-upstream-gate": "scripts/check_pr_ed_completion.sh",
    "runtime-upstream-gate": "scripts/check_pr_eg_completion.sh",
    "capability-upstream-gate": "scripts/check_pr_es_completion.sh",
    "product-suite-contract": "package.json",
    "completion-governance": "scripts/check_pr_dv_completion.sh",
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-DV completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be completed with no local remainder")
if row.count(verify_anchor) != 1:
    fail("roadmap verification must name the destructive completion gate")
queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-DV\b", queue):
    fail("completed PR-DV remains in the local queue")

history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(
    encoding="utf-8"
)
if "## 2026-07-15 PR-DV HedgeTicket Scoped Preflight and Confirm Closure" not in history:
    fail("history closure appendix is missing")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(
    encoding="utf-8", newline=""
) as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-DV"]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for kind, artifact in evidence_contract.items():
    if indexed[kind]["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"{kind} artifact drifted or is missing")

markers = {
    "shared-types/src/hedge.rs": (
        "HedgePreflightOperation",
        "PrivateRead,",
        "OrderFinality,",
        "Orderbook,",
    ),
    "crates/api/src/services/hedge_preflight/live_health/credentials/part_01.rs": (
        "HedgePreflightOperation::PrivateRead",
        "HedgePreflightOperation::OrderFinality",
        "HedgePreflightOperation::Orderbook",
        "live_operation_request_id(plans, rows)",
        "row_health: live_operation_row_health(plans, rows)",
    ),
    "crates/api/src/services/hedge_preflight/order_submission.rs": (
        "recheck_hedge_live_order_preflight_guards",
        "tokio::join!(",
        "collect_required_live_order_preflight",
        "hedge_live_order_preflight_guards(long, short)",
    ),
    "crates/api/src/services/hedge_confirm/confirm_validate.rs": (
        "validate_confirm_scoped_preflight",
        "recheck_hedge_live_order_preflight_guards",
        'with_details(serde_json::json!({ "guards": blocked }))',
        '#[path = "confirm_validate/tests.rs"]',
    ),
    "crates/api/src/services/hedge_confirm/confirm_validate/tests.rs": (
        "confirm_scoped_preflight_returns_every_blocked_guard",
        'blocked_guard("account_mode", "账户模式不可读")',
        'blocked_guard("live_operation_health", "订单终态回查缺证据")',
        '.and_then(|value| value.get("guards").cloned())',
    ),
    "crates/api/src/services/hedge_confirm/confirm.rs": (
        "validate_confirm_scoped_preflight(state, &preview).await",
        "with_confirm_context(error, &confirm_context)",
    ),
    "frontend/src/panels/modules/execution/components/risk_preview/evidence.rs": (
        'HedgePreflightOperation::PrivateRead => "私有读取"',
        'HedgePreflightOperation::OrderFinality => "订单终态"',
        'HedgePreflightOperation::Orderbook => "订单簿"',
    ),
    "frontend/src/panels/modules/execution/components/risk_preview/preflight.rs": (
        'parts.push(format!("checked {}", outcome.checked_at_ms))',
        'parts.push(format!("freshness {freshness_ms}ms"))',
        'parts.push(format!("request {request_id}"))',
        'account_data_health_summary(&outcome.row_health)',
    ),
    "frontend/src/panels/modules/execution/components/risk_preview/tests.rs": (
        'detail.contains("checked 1")',
        'detail.contains("freshness 42ms")',
        'detail.contains("request request-abcdef")',
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
browser_markers = (
    "PR-DV renders ticket-scoped capability, account, finality, and request evidence",
    "PR-DV keeps confirm-time scoped preflight failure and correlation context visible",
    "PR-DV restores submitted legs without promoting acknowledgements to Hedged",
    "HEDGE_PRE_TRADE_REJECTED",
    "req-pr-dv-confirm-blocked",
    "venue_operation_health:order_finality:pr_dv_fixture",
)
for marker in browser_markers:
    if marker not in browser:
        fail(f"browser fixture missing marker: {marker}")
if browser.count('test("PR-DV ') != 3:
    fail("browser fixture must keep all three non-skipping scenarios")
if browser.count('.not.toContainText("双腿完成")') != 2:
    fail("ACK must remain non-terminal before and after reload")

package = json.loads((root / "package.json").read_text(encoding="utf-8"))
scripts = package.get("scripts", {})
dedicated = "playwright test test/e2e/pr_dv_scoped_preflight.spec.ts"
if scripts.get("test:e2e:pr-dv") != dedicated:
    fail("dedicated browser command is missing")
if scripts.get("test:e2e:product", "").count(browser_path) != 1:
    fail("product suite must include the PR-DV fixture exactly once")
release_fixture = json.loads(
    (root / "scripts/fixtures/release_qa_contract/package.json").read_text(
        encoding="utf-8"
    )
)
if release_fixture.get("scripts", {}).get("test:e2e:product", "").count(browser_path) != 1:
    fail("release QA fixture must include the PR-DV fixture exactly once")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(
    encoding="utf-8", newline=""
) as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
for path in evidence_contract.values():
    if path == "package.json":
        continue
    if coverage.get(path, {}).get("coverage_status") != "exact":
        fail(f"exact coverage missing for {path}")

print(
    f"OK PR-DV static contract ({len(evidence_contract)} evidence types; "
    "preview/confirm scoped evidence, finality and durable UI restore closure)"
)
PY

if [[ "${PR_DV_SKIP_UPSTREAM:-0}" != "1" ]]; then
  PR_DF_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_df_completion.sh"
  PR_EA_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_ea_completion.sh"
  PR_EB_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_eb_completion.sh"
  PR_ED_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_ed_completion.sh"
  PR_EG_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_eg_completion.sh"
  PR_ES_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_es_completion.sh"
  bash "$ROOT/scripts/product_copy_gate.sh"
  bash "$ROOT/scripts/check_product_audit_evidence.sh"
  bash "$ROOT/scripts/check_product_audit_coverage.sh"
  bash "$ROOT/scripts/check_release_qa_contract.sh"
fi

if [[ "${PR_DV_SKIP_TESTS:-0}" != "1" ]]; then
  JOBS="${CARGO_BUILD_JOBS:-10}"
  CARGO_BUILD_JOBS="$JOBS" cargo test -p shared-types hedge --lib --no-fail-fast
  CARGO_BUILD_JOBS="$JOBS" cargo test -p api --bin crypto-arb-api services::hedge_preflight::tests --no-fail-fast
  CARGO_BUILD_JOBS="$JOBS" cargo test -p api --bin crypto-arb-api confirm_scoped_preflight_returns_every_blocked_guard --no-fail-fast
  CARGO_BUILD_JOBS="$JOBS" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib risk_preview --no-fail-fast
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-dv -- --workers=1
fi

printf 'OK PR-DV HedgeTicket scoped preflight and confirm completion contract\n'
