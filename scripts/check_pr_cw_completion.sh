#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
DOC="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
EVIDENCE="$ROOT/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
COVERAGE="$ROOT/docs/PRODUCT_AUDIT_COVERAGE.tsv"
HISTORY="$ROOT/docs/audit_history/PRODUCT_AUDIT_HISTORY.md"
IDENTITY="$ROOT/crates/api/src/services/hedge_preview/guards/identity.rs"
REST_COMPILER="$ROOT/crates/exchange/src/adapters/bybit_trade_data_tests.rs"
WS_COMPILER="$ROOT/crates/exchange/src/adapters/bybit_ws_trade_tests.rs"
ORDER_FIXTURE="$ROOT/crates/exchange/fixtures/bybit/ws_user_order_filled.json"
EXECUTION_FIXTURE="$ROOT/crates/exchange/fixtures/bybit/ws_user_execution_fill.json"
ACCOUNT_BROWSER="$ROOT/test/e2e/pr_em_bybit_account.spec.ts"
REPO_GATE="$ROOT/scripts/verify_repo_gates.sh"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-CW completion gate failed: %s\n' "$1" >&2
  exit 1
}

if [[ "$MODE" == "--self-test" ]]; then
  temp="$(mktemp -d "${TMPDIR:-/tmp}/crossline-pr-cw.XXXXXX")"
  for path in "$DOC" "$EVIDENCE" "$COVERAGE" "$IDENTITY" "$REST_COMPILER" \
    "$WS_COMPILER" "$ORDER_FIXTURE" "$ACCOUNT_BROWSER" "$REPO_GATE"; do
    cp "$path" "$temp/$(basename "$path")"
  done
  restore() {
    for path in "$DOC" "$EVIDENCE" "$COVERAGE" "$IDENTITY" "$REST_COMPILER" \
      "$WS_COMPILER" "$ORDER_FIXTURE" "$ACCOUNT_BROWSER" "$REPO_GATE"; do
      cp "$temp/$(basename "$path")" "$path"
    done
  }
  cleanup() {
    restore
    rm -rf "$temp"
  }
  trap cleanup EXIT

  assert_rejected() {
    local label="$1"
    local status
    set +e
    PR_CW_SKIP_TESTS=1 PR_CW_SKIP_UPSTREAM=1 PR_CW_SKIP_BROWSER_LIST=1 \
      bash "$0" >/dev/null 2>&1
    status=$?
    set -e
    if [[ "$status" -eq 0 ]]; then
      fail "self-test accepted $label"
    fi
    restore
  }

  PR_CW_SKIP_TESTS=1 PR_CW_SKIP_UPSTREAM=1 PR_CW_SKIP_BROWSER_LIST=1 \
    bash "$0" >/dev/null

  python3 - "$DOC" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "| `PR-CW Bybit Official V5 Order Semantics` | ✅ 完成 |"
if source.count(marker) != 1:
    raise SystemExit("PR-CW self-test setup failed: roadmap row drifted")
path.write_text(source.replace(marker, marker.replace("✅ 完成", "🟡 部分完成"), 1), encoding="utf-8")
PY
  assert_rejected "a downgraded roadmap row"

  python3 - "$EVIDENCE" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
rows = path.read_text(encoding="utf-8").splitlines()
path.write_text("\n".join(row for row in rows if not row.startswith("PR-CW\tprivate-execution-stream-fee\t")) + "\n", encoding="utf-8")
PY
  assert_rejected "an incomplete evidence matrix"

  python3 - "$IDENTITY" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = '"bybit-v5-private-order-finality"'
if source.count(marker) != 1:
    raise SystemExit("PR-CW self-test setup failed: finality marker drifted")
path.write_text(source.replace(marker, '"removed-bybit-finality"', 1), encoding="utf-8")
PY
  assert_rejected "detached shared finality evidence"

  python3 - "$ORDER_FIXTURE" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = '"orderStatus":"Filled"'
if source.count(marker) != 1:
    raise SystemExit("PR-CW self-test setup failed: terminal fixture drifted")
path.write_text(source.replace(marker, '"orderStatus":"PartiallyFilled"', 1), encoding="utf-8")
PY
  assert_rejected "a non-terminal private order fixture"

  python3 - "$REST_COMPILER" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "fn market_order_sends_official_percent_slippage"
if source.count(marker) != 1:
    raise SystemExit("PR-CW self-test setup failed: market slippage test drifted")
path.write_text(source.replace(marker, "#[ignore]\n" + marker, 1), encoding="utf-8")
PY
  assert_rejected "an ignored market slippage contract"

  python3 - "$WS_COMPILER" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "LiveOrderState::CancelRequested"
if source.count(marker) < 2:
    raise SystemExit("PR-CW self-test setup failed: cancel ACK marker drifted")
path.write_text(source.replace(marker, "LiveOrderState::Cancelled", 1), encoding="utf-8")
PY
  assert_rejected "a cancel ACK promoted to terminal"

  python3 - "$ACCOUNT_BROWSER" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = 'test("PR-EM renders Bybit UNIFIED equity available and margin evidence"'
if source.count(marker) != 1:
    raise SystemExit("PR-CW self-test setup failed: browser marker drifted")
path.write_text(source.replace(marker, marker.replace("test(", "test.skip("), 1), encoding="utf-8")
PY
  assert_rejected "a skipped product fixture"

  python3 - "$DOC" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
heading = "### 🟡 6.5 下一步执行队列"
path.write_text(source.replace(heading, heading + "\n\n1. **PR-CW Bybit Official V5 Order Semantics** — stale", 1), encoding="utf-8")
PY
  assert_rejected "completed PR-CW returned to the queue"

  python3 - "$COVERAGE" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
rows = path.read_text(encoding="utf-8").splitlines()
prefix = "crates/exchange/tests/bybit_test.rs\t"
path.write_text("\n".join(row for row in rows if not row.startswith(prefix)) + "\n", encoding="utf-8")
PY
  assert_rejected "missing Bybit HTTP contract coverage"

  python3 - "$REPO_GATE" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "check_pr_cw_completion.sh"
if source.count(marker) != 2:
    raise SystemExit("PR-CW self-test setup failed: repo wiring drifted")
path.write_text(source.replace(marker, "removed_pr_cw_completion.sh", 1), encoding="utf-8")
PY
  assert_rejected "single-scope repo wiring"

  printf 'PR-CW completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from __future__ import annotations

import csv
import hashlib
import json
import re
import sys
from decimal import Decimal
from pathlib import Path

root = Path(sys.argv[1])
title = "PR-CW Bybit Official V5 Order Semantics"
verify_anchor = "`bash scripts/check_pr_cw_completion.sh --self-test`"
evidence_contract = {
    "successor-pr-em": ("scripts/check_pr_em_completion.sh", "check_pr_em_completion.sh"),
    "successor-pr-eb": ("scripts/check_pr_eb_completion.sh", "check_pr_eb_completion.sh"),
    "successor-pr-bw": ("scripts/check_pr_bw_completion.sh", "check_pr_bw_completion.sh"),
    "successor-pr-bz": ("scripts/check_pr_bz_completion.sh", "check_pr_bz_completion.sh"),
    "client-order-id-policy": ("crates/exchange/src/client_order_id_policy.rs", "bybit_policy_derives_unsupported_public_id"),
    "shared-order-identity-plan": ("crates/api/src/services/hedge_preview/guards.rs", "bybit_identity_constraints_produce_execution_ready_usdc_plan"),
    "identity-finality-fail-closed": ("crates/api/src/services/hedge_preview/guards.rs", "bybit_identity_plan_fails_closed_without_private_finality_evidence"),
    "rest-tif-position-payload": ("crates/exchange/src/adapters/bybit_trade_data_tests.rs", "limit_ioc_and_fok_preserve_time_in_force"),
    "ws-tif-position-payload": ("crates/exchange/src/adapters/bybit_ws_trade_tests.rs", "order_create_preserves_ioc_and_fok_time_in_force"),
    "market-slippage-rest": ("crates/exchange/src/adapters/bybit_trade_data_tests.rs", "market_order_sends_official_percent_slippage"),
    "market-slippage-ws": ("crates/exchange/src/adapters/bybit_ws_trade_tests.rs", "order_create_market_sends_slippage_percent"),
    "account-position-mode": ("crates/exchange/src/adapters/bybit_private_data_tests.rs", "bybit_position_mode_parses_official_fixture"),
    "native-instrument-sizing": ("crates/exchange/src/adapters/bybit_instruments_tests.rs", "bybit_identity_matrix_preserves_usdt_usdc_and_rwa_contracts"),
    "market-depth-cost-guard": ("crates/api/src/services/hedge_ticket/tests/cost_evidence.rs", "ticket_cost_rejects_missing_or_stale_depth_evidence"),
    "rest-place-http-contract": ("crates/exchange/tests/bybit_test.rs", "live_place_order_sends_v5_limit_order"),
    "rest-market-http-contract": ("crates/exchange/tests/bybit_test.rs", "live_place_market_order_sends_official_slippage"),
    "rest-cancel-ack-not-final": ("crates/exchange/tests/bybit_test.rs", "live_cancel_order_returns_cancel_requested"),
    "rest-realtime-order-contract": ("crates/exchange/tests/bybit_test.rs", "live_get_order_queries_realtime_by_order_link_id"),
    "ws-place-cancel-ack": ("crates/exchange/src/adapters/bybit_ws_trade_tests.rs", "bybit_ws_place_order_ack_parses_official_fixture"),
    "private-rest-strict-parser": ("crates/exchange/src/adapters/bybit_private_data_tests.rs", "bybit_private_strict_fixture_sweep_fails_closed"),
    "private-order-stream-finality": ("crates/exchange/src/adapters/bybit_ws_user_tests.rs", "parses_official_order_fixture_to_terminal_delta"),
    "private-execution-stream-fee": ("crates/exchange/src/adapters/bybit_ws_user_tests.rs", "parses_official_execution_fixture_with_usdc_fee_identity"),
    "private-order-ledger-finality": ("crates/api/src/trading_service/private_ws_events/tests/deltas/order_updates/part_04.rs", "bybit_private_fill_and_order_finality_project_once"),
    "private-ws-server-ack-health": ("crates/api/src/lifecycle/private_ws/plain_venues_tests.rs", "bybit_private_ws_waits_for_auth_and_subscription_acknowledgements"),
    "official-rest-operation-registry": ("crates/exchange/src/venue_spec.rs", "bybit_place_order_evidence_uses_recorded_fixture_metadata"),
    "official-ws-operation-registry": ("crates/exchange/tests/ws_trading_specs_test.rs", "bybit_live_ws_write_ops_carry_official_evidence"),
    "account-product-browser": ("test/e2e/pr_em_bybit_account.spec.ts", "test:e2e:pr-em"),
    "instrument-product-browser": ("test/e2e/pr_eb_instrument_sizing.spec.ts", "test:e2e:pr-eb"),
    "external-live-boundary": ("docs/PRODUCT_FULL_AUDIT_REFINEMENT.md", "check_product_audit_progress.sh"),
    "completion-governance": ("scripts/check_pr_cw_completion.sh", "check_pr_cw_completion.sh --self-test"),
}
runnable_anchors = (
    ("crates/exchange/src/client_order_id_policy.rs", "bybit_policy_derives_unsupported_public_id"),
    ("crates/api/src/services/hedge_preview/guards.rs", "bybit_identity_constraints_produce_execution_ready_usdc_plan"),
    ("crates/api/src/services/hedge_preview/guards.rs", "bybit_identity_plan_fails_closed_without_private_finality_evidence"),
    ("crates/exchange/src/adapters/bybit_trade_data_tests.rs", "market_order_sends_official_percent_slippage"),
    ("crates/exchange/src/adapters/bybit_trade_data_tests.rs", "limit_ioc_and_fok_preserve_time_in_force"),
    ("crates/exchange/src/adapters/bybit_trade_data_tests.rs", "post_only_order_uses_limit_post_only"),
    ("crates/exchange/src/adapters/bybit_trade_data_tests.rs", "hedge_position_idx_is_serialized"),
    ("crates/exchange/src/adapters/bybit_trade_data_tests.rs", "order_link_id_accepts_official_36_char_charset"),
    ("crates/exchange/src/adapters/bybit_ws_trade_tests.rs", "order_create_preserves_ioc_and_fok_time_in_force"),
    ("crates/exchange/src/adapters/bybit_ws_trade_tests.rs", "order_create_maps_gtx_to_post_only_time_in_force"),
    ("crates/exchange/src/adapters/bybit_ws_trade_tests.rs", "order_create_market_sends_slippage_percent"),
    ("crates/exchange/src/adapters/bybit_private_data_tests.rs", "bybit_position_mode_parses_official_fixture"),
    ("crates/exchange/src/adapters/bybit_private_data_tests.rs", "bybit_get_order_parses_official_fixture"),
    ("crates/exchange/src/adapters/bybit_private_data_tests.rs", "bybit_private_strict_fixture_sweep_fails_closed"),
    ("crates/exchange/src/adapters/bybit_instruments_tests.rs", "bybit_identity_matrix_preserves_usdt_usdc_and_rwa_contracts"),
    ("crates/exchange/src/adapters/bybit_ws_user_tests.rs", "parses_official_order_fixture_to_terminal_delta"),
    ("crates/exchange/src/adapters/bybit_ws_user_tests.rs", "parses_official_execution_fixture_with_usdc_fee_identity"),
    ("crates/exchange/tests/bybit_test.rs", "live_place_order_sends_v5_limit_order"),
    ("crates/exchange/tests/bybit_test.rs", "live_place_market_order_sends_official_slippage"),
    ("crates/exchange/tests/bybit_test.rs", "live_cancel_order_returns_cancel_requested"),
    ("crates/exchange/tests/bybit_test.rs", "live_get_order_queries_realtime_by_order_link_id"),
    ("crates/api/src/trading_service/private_ws_events/tests/deltas/order_updates/part_04.rs", "bybit_private_fill_and_order_finality_project_once"),
    ("crates/api/src/lifecycle/private_ws/plain_venues_tests.rs", "bybit_private_ws_waits_for_auth_and_subscription_acknowledgements"),
    ("crates/exchange/src/venue_spec.rs", "bybit_place_order_evidence_uses_recorded_fixture_metadata"),
    ("crates/exchange/tests/ws_trading_specs_test.rs", "bybit_live_ws_write_ops_carry_official_evidence"),
    ("crates/api/src/services/hedge_ticket/tests/cost_evidence.rs", "ticket_cost_rejects_missing_or_stale_depth_evidence"),
)
browser_contract = {
    "test/e2e/pr_em_bybit_account.spec.ts": (
        "PR-EM renders Bybit UNIFIED equity available and margin evidence",
    ),
    "test/e2e/pr_eb_instrument_sizing.spec.ts": (
        "PR-EB renders ticket-bound native symbol, sizing and rounding evidence",
        "PR-EB live preview disables submit when instrument sizing evidence is absent",
    ),
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-CW completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row or row.count(verify_anchor) != 1:
    fail("roadmap row must be complete, remaining-none and bound to the destructive gate")
queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-CW\b", queue):
    fail("completed PR-CW remains in the local queue")
cx_row = next((line for line in doc.splitlines() if line.startswith("| `PR-CX ")), "")
if "✅ 完成" in cx_row:
    cy_row = next((line for line in doc.splitlines() if line.startswith("| `PR-CY ")), "")
    if "✅ 完成" in cy_row:
        cz_row = next((line for line in doc.splitlines() if line.startswith("| `PR-CZ ")), "")
        if "✅ 完成" in cz_row:
            cg_row = next((line for line in doc.splitlines() if line.startswith("| `PR-CG ")), "")
            if "✅ 完成" in cg_row:
                da_row = next((line for line in doc.splitlines() if line.startswith("| `PR-DA ")), "")
                if "✅ 完成" in da_row:
                    next_pr = None
                elif "🟡 部分完成" in da_row:
                    next_pr = "PR-DA"
                else:
                    fail("PR-DA successor must be complete or partial")
            else:
                next_pr = "PR-CG"
        else:
            next_pr = "PR-CZ"
    else:
        next_pr = "PR-CY"
else:
    next_pr = "PR-CX"
if next_pr is None:
    head = re.search(r"(?m)^1\.\s+\*\*(PR-[A-Z]+)\b", queue)
    head_row = next((line for line in doc.splitlines() if head and line.startswith(f"| `{head.group(1)} ")), "")
    if not head_row or "✅ 完成" in head_row:
        fail("completed successor chain must hand off to an incomplete roadmap row")
elif not re.search(rf"(?m)^1\.\s+\*\*{next_pr}\b", queue):
    fail(f"{next_pr} must be the next incomplete queue head")
queue_items = re.findall(r"(?m)^\d+\.\s+\*\*", queue)
if not queue_items:
    fail("local queue plus external pool must not be empty")
if "PR-CW 本地" not in queue or "真实 Bybit credential" not in queue:
    fail("external live-capture boundary is missing")

history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(encoding="utf-8")
if "## 2026-07-16 PR-CW Bybit V5 Order Semantics Closure" not in history:
    fail("history closure appendix is missing")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [entry for entry in csv.DictReader(handle, delimiter="\t") if entry["pr_id"] == "PR-CW"]
indexed = {entry["evidence_type"]: entry for entry in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for evidence_type, (artifact, command_anchor) in evidence_contract.items():
    evidence = indexed[evidence_type]
    if evidence["artifact"] != artifact or command_anchor not in evidence["command"]:
        fail(f"evidence anchor drifted: {evidence_type}")
    if not (root / artifact).is_file() or not evidence["notes"].strip():
        fail(f"evidence artifact or note is missing: {artifact}")
if "ack_not_final=true" not in indexed["rest-cancel-ack-not-final"]["notes"]:
    fail("cancel ACK evidence must retain the ACK-not-final marker")
if "identity_fail_closed=true" not in indexed["identity-finality-fail-closed"]["notes"]:
    fail("shared identity fail-closed marker is missing")
if "live_capture_external=true" not in indexed["external-live-boundary"]["notes"]:
    fail("external live-capture marker is missing")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {entry["file"]: entry for entry in csv.DictReader(handle, delimiter="\t")}
for artifact, _ in evidence_contract.values():
    if Path(artifact).suffix in {".json", ".md"}:
        continue
    item = coverage.get(artifact)
    if item is None or item["coverage_status"] != "exact" or not item["evidence"].startswith("line:"):
        fail(f"exact coverage missing for {artifact}")

for relative, test_name in runnable_anchors:
    source = (root / relative).read_text(encoding="utf-8")
    match = re.search(rf"#\[(?:tokio::)?test\][\s\S]{{0,120}}?(?:async\s+)?fn\s+{re.escape(test_name)}\b", source)
    if match is None:
        fail(f"runnable anchor missing: {relative}:{test_name}")
    prefix = source[max(0, match.start() - 120):match.end()]
    if "#[ignore" in prefix or "should_panic" in prefix:
        fail(f"runnable anchor is skipped or panic-expected: {relative}:{test_name}")

identity = (root / "crates/api/src/services/hedge_preview/guards/identity.rs").read_text(encoding="utf-8")
for marker in (
    '"bybit-v5-private-order-finality"',
    '"bybit-v5-private-execution-fee"',
    '"bybit/ws_user_order_filled.json"',
    '"bybit/ws_user_execution_fill.json"',
    '"bybit_private_fill_and_order_finality_project_once"',
):
    if marker not in identity:
        fail(f"shared identity marker missing: {marker}")

fixture_hashes = {
    "crates/exchange/fixtures/bybit/ws_user_order_filled.json": "92185c0e848f978cfe0e301a86439ea47d95984b583304c25c5897e5b9104536",
    "crates/exchange/fixtures/bybit/ws_user_execution_fill.json": "0a74cdbfda5e62ad0fdd105592fa5493fda63bdf396a515adcd489213b1bdb39",
    "crates/exchange/fixtures/bybit/order_create_ack.json": "d9d397e431457b054abd260a76b5a2261c4bc21abb673b70826a20165ffff65e",
    "crates/exchange/fixtures/bybit/order_cancel_ack.json": "9e66839ea77befc4089a173c25a707a41c95d636e671d02eb134876be9e3b32e",
    "crates/exchange/fixtures/bybit/order_realtime_linear_open.json": "62fb1814eb2c0fd419439dc5d123cca08da410f6c2151215ed10ab22f83b650a",
    "crates/exchange/fixtures/bybit/ws_order_create_ack.json": "b7a1a38959a03f88011ebc6b28e8861bec7eafdfe70468522f4fbd171c42a794",
    "crates/exchange/fixtures/bybit/ws_order_cancel_ack.json": "5450bef4c50c8eb370be02788198bd4eb43887f2e23652aa3f918a7e2d624940",
}
for relative, expected in fixture_hashes.items():
    if hashlib.sha256((root / relative).read_bytes()).hexdigest() != expected:
        fail(f"official fixture hash drifted: {relative}")

order_payload = json.loads((root / "crates/exchange/fixtures/bybit/ws_user_order_filled.json").read_text(encoding="utf-8"))
order_rows = order_payload.get("data", [])
if order_payload.get("topic") != "order.linear" or len(order_rows) != 1:
    fail("private order fixture must contain one linear order row")
order = order_rows[0]
if order.get("orderStatus") != "Filled" or Decimal(order.get("leavesQty", "-1")) != 0:
    fail("private order fixture must be terminal with zero leaves quantity")
if not order.get("orderId") or not order.get("orderLinkId"):
    fail("private order fixture must retain exchange and client identity")
for field in ("cancelType", "rejectReason", "cumExecFee"):
    if field not in order:
        fail(f"private order fixture is missing richer field: {field}")

execution_payload = json.loads((root / "crates/exchange/fixtures/bybit/ws_user_execution_fill.json").read_text(encoding="utf-8"))
execution_rows = execution_payload.get("data", [])
if execution_payload.get("topic") != "execution.linear" or len(execution_rows) != 1:
    fail("private execution fixture must contain one linear execution row")
execution = execution_rows[0]
if Decimal(execution.get("execFee", "0")) <= 0 or execution.get("feeCurrency") != "USDC":
    fail("private execution fixture must preserve positive USDC fee truth")
for field in ("execId", "orderId", "orderLinkId"):
    if not execution.get(field):
        fail(f"private execution fixture is missing identity: {field}")

rest_adapter = (root / "crates/exchange/src/adapters/bybit.rs").read_text(encoding="utf-8")
ws_compiler = (root / "crates/exchange/src/adapters/bybit_ws_trade_tests.rs").read_text(encoding="utf-8")
if rest_adapter.count("LiveOrderState::CancelRequested") != 1 or "final state requires order query" not in rest_adapter:
    fail("REST adapter no longer preserves cancel ACK as non-terminal")
if ws_compiler.count("LiveOrderState::CancelRequested") < 2 or "final state requires order query" not in ws_compiler:
    fail("WS compiler no longer proves cancel ACK is non-terminal")

for relative, titles in browser_contract.items():
    browser = (root / relative).read_text(encoding="utf-8")
    for title_text in titles:
        escaped = re.escape(title_text)
        if re.search(rf'test\.(?:skip|fixme)\(\s*["\']{escaped}["\']', browser):
            fail(f"browser anchor is skipped: {title_text}")
        if not re.search(rf'test\(\s*["\']{escaped}["\']', browser):
            fail(f"browser anchor is missing: {title_text}")

repo_gate = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
if repo_gate.count("check_pr_cw_completion.sh") != 2:
    fail("repo gate must execute PR-CW once in docs and all scopes")

print(
    f"OK PR-CW static contract ({len(evidence_contract)} evidence types; "
    f"{len(runnable_anchors)} runnable anchors; "
    f"{sum(len(titles) for titles in browser_contract.values())} browser anchors)"
)
PY

if [[ "${PR_CW_SKIP_BROWSER_LIST:-0}" != "1" ]]; then
  npx playwright test "$ACCOUNT_BROWSER" "$ROOT/test/e2e/pr_eb_instrument_sizing.spec.ts" --list >/dev/null
fi

if [[ "${PR_CW_SKIP_UPSTREAM:-0}" != "1" ]]; then
  PR_EM_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_em_completion.sh"
  PR_EB_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_eb_completion.sh"
  PR_BW_SKIP_TESTS=1 PR_BW_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_bw_completion.sh"
  PR_BZ_SKIP_TESTS=1 PR_BZ_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_bz_completion.sh"
  bash "$ROOT/scripts/check_product_audit_progress.sh"
  bash "$ROOT/scripts/check_product_audit_evidence.sh"
  bash "$ROOT/scripts/check_product_audit_evidence_index.sh"
  bash "$ROOT/scripts/check_product_audit_coverage.sh"
fi

if [[ "${PR_CW_SKIP_TESTS:-0}" != "1" ]]; then
  jobs="${CARGO_BUILD_JOBS:-10}"
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --lib bybit_ --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --test bybit_test --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --test ws_trading_specs_test bybit_ --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api bybit_ --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api ticket_cost_rejects_missing_or_stale_depth_evidence --no-fail-fast
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-em -- --workers=1
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-eb -- --workers=1
fi

printf 'PR-CW Bybit V5 order semantics completion passed\n'
