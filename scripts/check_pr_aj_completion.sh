#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
AUDIT="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
EVIDENCE="$ROOT/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
PROBLEM="$ROOT/shared-types/src/problem.rs"
PROBLEM_CODES="$ROOT/shared-types/src/problem/codes.rs"
CONFIRM="$ROOT/crates/api/src/services/hedge_confirm/confirm_validate.rs"
CONFIRM_TESTS="$ROOT/crates/api/src/services/hedge_confirm/confirm_validate/tests.rs"
ORDERS="$ROOT/crates/api/src/routers/trading/orders.rs"
REMEDY="$ROOT/frontend/src/panels/modules/execution/data/remedy.rs"
PREVIEW="$ROOT/frontend/src/panels/modules/execution/data/preview/model.rs"
REPO_GATE="$ROOT/scripts/verify_repo_gates.sh"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-AJ completion gate failed: %s\n' "$1" >&2
  exit 1
}

run_static_gate() {
  PR_AJ_SKIP_TESTS=1 PR_AJ_SKIP_UPSTREAM=1 bash "$0" >/dev/null
}

expect_rejected() {
  local message="$1"
  if run_static_gate 2>/dev/null; then
    fail "$message"
  fi
}

if [[ "$MODE" == "--self-test" ]]; then
  tmp="$(mktemp -d "${TMPDIR:-/tmp}/crossline-pr-aj.XXXXXX")"
  files=(
    "$AUDIT"
    "$EVIDENCE"
    "$PROBLEM"
    "$PROBLEM_CODES"
    "$CONFIRM"
    "$CONFIRM_TESTS"
    "$ORDERS"
    "$REMEDY"
    "$PREVIEW"
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

  run_static_gate

  python3 - "$AUDIT" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = "| `PR-AJ Trading HTTP Contract & Execution Problem Model` | ✅ 完成 |"
if text.count(old) != 1:
    raise SystemExit("PR-AJ self-test setup failed: roadmap row drifted")
path.write_text(text.replace(old, "| `PR-AJ Trading HTTP Contract & Execution Problem Model` | 🟡 部分完成 |", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted a downgraded roadmap row"
  restore_file "$AUDIT"

  python3 - "$EVIDENCE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
rows = path.read_text(encoding="utf-8").splitlines()
for index, row in enumerate(rows):
    if row.startswith("PR-AJ\ttyped-problem-taxonomy\t"):
        del rows[index]
        break
else:
    raise SystemExit("PR-AJ self-test setup failed: evidence row missing")
path.write_text("\n".join(rows) + "\n", encoding="utf-8")
PY
  expect_rejected "self-test accepted incomplete evidence"
  restore_file "$EVIDENCE"

  python3 - "$PROBLEM_CODES" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = 'pub const HEDGE_TICKET_REQUIRED: &str = "HEDGE_TICKET_REQUIRED";'
if text.count(old) != 1:
    raise SystemExit("PR-AJ self-test setup failed: problem code module drifted")
path.write_text(text.replace(old, "// required ticket problem removed", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted a missing ticket-required problem"
  restore_file "$PROBLEM_CODES"

  python3 - "$ORDERS" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = "explicit_idempotency_key(&headers).unwrap_or_else(|| cancel_idempotency_key(&id))"
if text.count(old) != 1:
    raise SystemExit("PR-AJ self-test setup failed: cancel key drifted")
path.write_text(text.replace(old, "cancel_idempotency_key(&id)", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted a permanently replayed failed cancel"
  restore_file "$ORDERS"

  python3 - "$REMEDY" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = "MutationRequestContext::new_idempotent_attempt(format!("
if text.count(old) != 1:
    raise SystemExit("PR-AJ self-test setup failed: frontend cancel attempt drifted")
path.write_text(text.replace(old, "MutationRequestContext::with_idempotency_key(format!(", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted a fixed frontend cancel retry key"
  restore_file "$REMEDY"

  python3 - "$PREVIEW" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = ".is_some_and(|ticket_id| !ticket_id.trim().is_empty())"
if text.count(old) != 1:
    raise SystemExit("PR-AJ self-test setup failed: ticket guard drifted")
path.write_text(text.replace(old, ".is_some_and(|_| true)", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted submission without ticket identity"
  restore_file "$PREVIEW"

  python3 - "$CONFIRM_TESTS" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = "#[test]\nfn confirm_requires_explicit_non_empty_ticket_identity()"
if text.count(old) != 1:
    raise SystemExit("PR-AJ self-test setup failed: confirm fixture drifted")
path.write_text(text.replace(old, "#[test]\n#[ignore]\nfn confirm_requires_explicit_non_empty_ticket_identity()", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted a skipped ticket identity fixture"
  restore_file "$CONFIRM_TESTS"

  python3 - "$AUDIT" <<'PY'
from pathlib import Path
import re
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
head = re.search(r"(?m)^1\. \*\*(PR-[A-Z]+)\b", text)
if head is None:
    raise SystemExit("PR-AJ self-test setup failed: queue head missing")
path.write_text(text[:head.start(1)] + "PR-AJ" + text[head.end(1):], encoding="utf-8")
PY
  expect_rejected "self-test accepted PR-AJ reinserted into the local queue"
  restore_file "$AUDIT"

  python3 - "$REPO_GATE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = 'PR_AJ_SKIP_TESTS=1 PR_AJ_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_aj_completion.sh"'
if text.count(old) != 2:
    raise SystemExit("PR-AJ self-test setup failed: repo wiring drifted")
path.write_text(text.replace(old, "true # PR-AJ gate removed", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted single-scope repository wiring"

  printf 'PR-AJ completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from __future__ import annotations

import csv
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])
pr_id = "PR-AJ"
title = "PR-AJ Trading HTTP Contract & Execution Problem Model"
verify_anchor = "`bash scripts/check_pr_aj_completion.sh --self-test`"
evidence_contract = {
    "typed-problem-taxonomy": "shared-types/src/problem.rs",
    "shared-trading-contract": "shared-types/src/live_trading.rs",
    "submit-idempotency-validator": "crates/api/src/routers/trading/types.rs",
    "ticket-bound-confirm": "crates/api/src/services/hedge_confirm/confirm_validate.rs",
    "frontend-ticket-boundary": "frontend/src/panels/modules/execution/data/preview/model.rs",
    "cancel-retry-action-run": "crates/api/src/routers/trading/orders.rs",
    "cancel-replay-finality": "crates/api/src/routers/trading/order_replay.rs",
    "cancel-domain-finality": "crates/trading/src/execution.rs",
    "frontend-mutation-attempt": "frontend/src/api/rest/transport.rs",
    "execution-cancel-retry": "frontend/src/panels/modules/execution/data/remedy.rs",
    "compensation-cancel-retry": "frontend/src/panels/modules/positions/data/actions/compensation.rs",
    "balance-partial-envelope": "crates/api/src/services/account_balances.rs",
    "position-partial-envelope": "crates/api/src/services/account_positions.rs",
    "open-order-partial-envelope": "crates/api/src/services/account_open_orders.rs",
    "account-state-snapshot": "crates/api/src/services/account_state.rs",
    "venue-capability-matrix": "shared-types/src/venue_capabilities.rs",
    "trading-venue-matrix": "crates/api/src/routers/trading/account.rs",
    "settings-venue-matrix": "frontend/src/panels/modules/settings/tabs/adapters/venue_capabilities.rs",
    "fee-provenance-audit": "crates/api/src/services/fees.rs",
    "action-run-audit-context": "crates/api/src/services/action_runs/audit_context.rs",
    "audit-correlation": "crates/api/src/middleware/audit/correlation.rs",
    "run-ledger-envelopes": "crates/api/src/routers/trading/listing.rs",
    "product-browser-correlation": "test/e2e/pr_dt_request_correlation.spec.ts",
    "product-browser-capability": "test/e2e/pr_es_venue_capability_matrix.spec.ts",
    "completion-governance": "scripts/check_pr_aj_completion.sh",
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-AJ completion gate failed: {message}")


def source(relative: str) -> str:
    return (root / relative).read_text(encoding="utf-8")


def require_markers(relative: str, markers: tuple[str, ...]) -> str:
    text = source(relative)
    for marker in markers:
        if marker not in text:
            fail(f"{relative} marker missing: {marker}")
    return text


def require_non_skipping_test(relative: str, name: str) -> None:
    text = source(relative)
    match = re.search(
        rf"(?P<attrs>(?:\s*#\[[^\n]+\]\s*)+)(?:async\s+)?fn\s+{re.escape(name)}\b",
        text,
    )
    if match is None or "test" not in match.group("attrs"):
        fail(f"runnable test missing: {relative}::{name}")
    attrs = match.group("attrs")
    if any(marker in attrs for marker in ("ignore", "should_panic", "cfg(")):
        fail(f"evidence test is skippable: {relative}::{name}")


doc = source("docs/PRODUCT_FULL_AUDIT_REFINEMENT.md")
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be complete with no local remainder")
if row.count(verify_anchor) != 1:
    fail("roadmap verification must name the destructive gate once")

queue_start = doc.find("### 🟡 6.5")
if queue_start < 0:
    fail("local execution queue is missing")
queue = doc[queue_start:]
if re.search(r"(?m)^\d+\.\s+\*\*PR-AJ\b", queue):
    fail("completed PR-AJ remains in the local queue")
successor = "PR-AK Arbitrage Hedge Preview & Ticket Contract"
successor_row = next(
    (line for line in doc.splitlines() if line.startswith(f"| `{successor}`")),
    None,
)
if successor_row is None:
    fail("PR-AK successor roadmap row is missing")
if "✅ 完成" in successor_row:
    if re.search(r"(?m)^\d+\.\s+\*\*PR-AK\b", queue):
        fail("completed PR-AK remains in the local queue")
elif re.search(r"(?m)^1\.\s+\*\*PR-AK\b", queue) is None:
    fail("unfinished PR-AK must be the local queue head")

for successor_id in (
    "PR-EA", "PR-BW", "PR-CA", "PR-DT", "PR-EI", "PR-ES",
    "PR-ED", "PR-DI", "PR-DB", "PR-CF", "PR-DV",
):
    successor_row = next(
        (line for line in doc.splitlines() if line.startswith(f"| `{successor_id} ")),
        None,
    )
    if successor_row is None or "✅ 完成" not in successor_row:
        fail(f"required successor authority is not complete: {successor_id}")

history = source("docs/audit_history/PRODUCT_AUDIT_HISTORY.md")
if "## 2026-07-17 PR-AJ Trading HTTP Contract and Execution Problem Model Closure" not in history:
    fail("history closure appendix is missing")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [item for item in csv.DictReader(handle, delimiter="\t") if item["pr_id"] == pr_id]
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
    item = coverage.get(artifact)
    if item is None or item["coverage_status"] != "exact" or not item["evidence"].startswith("line:"):
        fail(f"exact coverage missing for {artifact}")

require_markers(
    "shared-types/src/problem.rs",
    ("pub mod codes;", "pub struct ApiProblem", "pub request_id: Option<String>", "pub retry_after_ms: Option<u64>"),
)
require_markers(
    "shared-types/src/problem/codes.rs",
    ('pub const HEDGE_TICKET_REQUIRED: &str = "HEDGE_TICKET_REQUIRED";',),
)
require_markers(
    "crates/api/src/services/hedge_confirm/confirm_validate.rs",
    ("let requested = required_confirm_ticket_id(req)?;", "codes::HEDGE_TICKET_REQUIRED", '"field": "ticketId"', "codes::HEDGE_TICKET_MISMATCH"),
)
require_markers(
    "crates/api/src/services/hedge_confirm/confirm_replay.rs",
    ("codes::HEDGE_TICKET_REQUIRED => codes::HEDGE_TICKET_REQUIRED",),
)
require_markers(
    "crates/api/src/routers/trading/orders.rs",
    ("explicit_idempotency_key(&headers).unwrap_or_else(|| cancel_idempotency_key(&id))", "ActionRunKind::TradingOrderCancel"),
)
require_markers(
    "frontend/src/api/rest/transport.rs",
    ("pub fn new_idempotent_attempt", "format!(\"{scope}:{request_id}\")", "idempotency_key: Some(idempotency_key)"),
)
require_markers(
    "frontend/src/panels/modules/execution/data/remedy.rs",
    ("MutationRequestContext::new_idempotent_attempt", '"execution-cancel:{}:{order_id}"'),
)
require_markers(
    "frontend/src/panels/modules/positions/data/actions/compensation.rs",
    ("MutationRequestContext::new_idempotent_attempt", '"positions-compensation-cancel:{}:{key}"'),
)
require_markers(
    "frontend/src/panels/modules/execution/data/preview/model.rs",
    ("ticket_id", ".is_some_and(|ticket_id| !ticket_id.trim().is_empty())"),
)
require_markers(
    "crates/api/src/routers/trading/account.rs",
    ("TradingAdaptersResponse", "venues: live_venue_capabilities(&credentials)"),
)
require_markers(
    "frontend/src/panels/modules/settings/tabs/adapters/venue_capabilities.rs",
    ("venue_capabilities_table", 'data-settings-table="venue-capabilities"'),
)
require_markers(
    "crates/api/src/services/action_runs/audit_context.rs",
    ("ActionRunKind::TradingOrderCancel", "context.order_id", "context.run_id"),
)
require_markers(
    "crates/api/src/middleware/audit/correlation.rs",
    ("request_id", "action_run_id", "idempotency_key", "order_ids", "run_ids"),
)

for relative, test_name in (
    ("crates/api/src/services/hedge_confirm/confirm_validate/tests.rs", "confirm_requires_explicit_non_empty_ticket_identity"),
    ("crates/api/src/routers/trading/tests/cases_submit.rs", "cancel_order_accepts_new_explicit_key_after_failed_attempt"),
    ("frontend/src/api/rest/transport/tests.rs", "idempotent_attempt_is_stable_within_request_and_unique_across_retries"),
    ("frontend/src/panels/modules/execution/data/remedy/tests.rs", "cancel_retries_get_distinct_attempt_keys_for_the_same_run_order"),
    ("frontend/src/panels/modules/execution/data/preview_tests/ticket.rs", "ready_preview_without_ticket_identity_cannot_submit"),
    ("crates/api/src/services/account_balances/tests.rs", "partial_fanout_keeps_rows_and_surfaces_route_problem"),
    ("crates/api/src/services/account_positions/tests/error_paths.rs", "partial_position_envelope_keeps_rows_and_binds_each_venue"),
    ("crates/api/src/services/account_open_orders/tests.rs", "partial_open_order_fanout_keeps_rows_and_surfaces_route_health"),
    ("crates/api/src/trading_service/tests/reconcile/finality/part_01.rs", "cancel_refreshes_cancel_requested_to_cancelled_when_order_query_confirms"),
    ("crates/api/src/services/fees.rs", "standard_fee_snapshot_fails_closed_without_matching_evidence"),
):
    require_non_skipping_test(relative, test_name)

for relative, marker in (
    ("test/e2e/pr_dt_request_correlation.spec.ts", 'test.describe("PR-DT request and audit correlation"'),
    ("test/e2e/pr_es_venue_capability_matrix.spec.ts", 'test("PR-ES Settings renders all venue compiler contracts without credential filtering"'),
):
    browser = require_markers(relative, (marker,))
    if "test.skip" in browser or "test.fixme" in browser:
        fail(f"browser evidence is skippable: {relative}")

repo_gate = source("scripts/verify_repo_gates.sh")
if repo_gate.count("check_pr_aj_completion.sh") != 2:
    fail("repo gate must execute PR-AJ exactly once in docs and all scopes")

print(f"OK PR-AJ static contract ({len(evidence_contract)} evidence types)")
PY

if [[ "${PR_AJ_SKIP_UPSTREAM:-0}" != "1" ]]; then
  PR_EA_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_ea_completion.sh"
  PR_BW_SKIP_TESTS=1 PR_BW_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_bw_completion.sh"
  PR_CA_SKIP_TESTS=1 PR_CA_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_ca_completion.sh"
  PR_DT_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dt_completion.sh"
  PR_EI_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_ei_completion.sh"
  PR_ES_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_es_completion.sh"
  PR_ED_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_ed_completion.sh"
  PR_DI_SKIP_TESTS=1 PR_DI_SKIP_UPSTREAM=1 PR_DI_SKIP_BROWSER_LIST=1 bash "$ROOT/scripts/check_pr_di_completion.sh"
  PR_DB_SKIP_TESTS=1 PR_DB_SKIP_UPSTREAM=1 PR_DB_SKIP_BROWSER_LIST=1 bash "$ROOT/scripts/check_pr_db_completion.sh"
  PR_CF_SKIP_TESTS=1 PR_CF_SKIP_UPSTREAM=1 PR_CF_SKIP_BROWSER_LIST=1 bash "$ROOT/scripts/check_pr_cf_completion.sh"
  PR_DV_SKIP_TESTS=1 PR_DV_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_dv_completion.sh"
  bash "$ROOT/scripts/check_product_audit_progress.sh"
  bash "$ROOT/scripts/check_product_audit_evidence.sh"
  bash "$ROOT/scripts/check_product_audit_evidence_index.sh"
  bash "$ROOT/scripts/check_product_audit_coverage.sh"
fi

if [[ "${PR_AJ_SKIP_TESTS:-0}" != "1" ]]; then
  for test_name in \
    confirm_requires_explicit_non_empty_ticket_identity \
    cancel_order_accepts_new_explicit_key_after_failed_attempt \
    partial_fanout_keeps_rows_and_surfaces_route_problem \
    partial_position_envelope_keeps_rows_and_binds_each_venue \
    partial_open_order_fanout_keeps_rows_and_surfaces_route_health \
    cancel_refreshes_cancel_requested_to_cancelled_when_order_query_confirms \
    standard_fee_snapshot_fails_closed_without_matching_evidence
  do
    CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api "$test_name" --no-fail-fast
  done
  for test_name in \
    idempotent_attempt_is_stable_within_request_and_unique_across_retries \
    cancel_retries_get_distinct_attempt_keys_for_the_same_run_order \
    ready_preview_without_ticket_identity_cannot_submit
  do
    CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test \
      --manifest-path "$ROOT/frontend/Cargo.toml" "$test_name" --no-fail-fast
  done
  CI=1 npm run test:e2e:pr-dt -- --workers=1
  CI=1 npm run test:e2e:pr-es -- --workers=1
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo check -p api
fi

printf 'PR-AJ completion gate passed\n'
