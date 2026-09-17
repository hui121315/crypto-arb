#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
DOC="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
EVIDENCE="$ROOT/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
COVERAGE="$ROOT/docs/PRODUCT_AUDIT_COVERAGE.tsv"
IDENTITY="$ROOT/crates/api/src/services/hedge_preview/guards/identity/bitget.rs"
PRIVATE_REST="$ROOT/crates/exchange/src/adapters/bitget_uta_private_rest.rs"
REST_TEST="$ROOT/crates/exchange/tests/bitget_test.rs"
ORDER_FIXTURE="$ROOT/crates/exchange/fixtures/bitget/uta_order_info_filled.json"
BROWSER="$ROOT/test/e2e/pr_es_venue_capability_matrix.spec.ts"
REPO_GATE="$ROOT/scripts/verify_repo_gates.sh"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-CX completion gate failed: %s\n' "$1" >&2
  exit 1
}

if [[ "$MODE" == "--self-test" ]]; then
  temp="$(mktemp -d "${TMPDIR:-/tmp}/crossline-pr-cx.XXXXXX")"
  cp "$DOC" "$temp/doc"
  cp "$EVIDENCE" "$temp/evidence"
  cp "$COVERAGE" "$temp/coverage"
  cp "$IDENTITY" "$temp/identity"
  cp "$PRIVATE_REST" "$temp/private_rest"
  cp "$REST_TEST" "$temp/rest_test"
  cp "$ORDER_FIXTURE" "$temp/order_fixture"
  cp "$BROWSER" "$temp/browser"
  cp "$REPO_GATE" "$temp/repo_gate"
  restore() {
    cp "$temp/doc" "$DOC"
    cp "$temp/evidence" "$EVIDENCE"
    cp "$temp/coverage" "$COVERAGE"
    cp "$temp/identity" "$IDENTITY"
    cp "$temp/private_rest" "$PRIVATE_REST"
    cp "$temp/rest_test" "$REST_TEST"
    cp "$temp/order_fixture" "$ORDER_FIXTURE"
    cp "$temp/browser" "$BROWSER"
    cp "$temp/repo_gate" "$REPO_GATE"
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
    PR_CX_SKIP_TESTS=1 PR_CX_SKIP_UPSTREAM=1 PR_CX_SKIP_BROWSER_LIST=1 \
      bash "$0" >/dev/null 2>&1
    status=$?
    set -e
    if [[ "$status" -eq 0 ]]; then
      fail "self-test accepted $label"
    fi
    restore
  }

  PR_CX_SKIP_TESTS=1 PR_CX_SKIP_UPSTREAM=1 PR_CX_SKIP_BROWSER_LIST=1 \
    bash "$0" >/dev/null

  python3 - "$DOC" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "| `PR-CX Bitget UTA V3 Official Order Semantics` | ✅ 完成 |"
if source.count(marker) != 1:
    raise SystemExit("PR-CX self-test setup failed: roadmap row drifted")
path.write_text(source.replace(marker, marker.replace("✅ 完成", "🟡 部分完成"), 1), encoding="utf-8")
PY
  assert_rejected "a downgraded roadmap row"

  python3 - "$EVIDENCE" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
rows = path.read_text(encoding="utf-8").splitlines()
path.write_text("\n".join(row for row in rows if not row.startswith("PR-CX\tunknown-result-query-recovery\t")) + "\n", encoding="utf-8")
PY
  assert_rejected "an incomplete evidence matrix"

  python3 - "$IDENTITY" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = '"bitget-uta-v3-private-order-finality"'
if source.count(marker) != 1:
    raise SystemExit("PR-CX self-test setup failed: finality marker drifted")
path.write_text(source.replace(marker, '"removed-bitget-finality"', 1), encoding="utf-8")
PY
  assert_rejected "detached shared finality evidence"

  python3 - "$PRIVATE_REST" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = '"40010" | "40725" | "45001"'
if source.count(marker) != 1:
    raise SystemExit("PR-CX self-test setup failed: unknown-result codes drifted")
path.write_text(source.replace(marker, '"40010"', 1), encoding="utf-8")
PY
  assert_rejected "an incomplete unknown-result code set"

  python3 - "$REST_TEST" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "async fn live_place_order_recovers_unknown_result_by_client_oid_query"
if source.count(marker) != 1:
    raise SystemExit("PR-CX self-test setup failed: recovery test drifted")
path.write_text(source.replace(marker, "#[ignore]\n" + marker, 1), encoding="utf-8")
PY
  assert_rejected "a skipped unknown-result recovery contract"

  python3 - "$ORDER_FIXTURE" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = '"fee": "0.00000744"'
if source.count(marker) != 1:
    raise SystemExit("PR-CX self-test setup failed: fee fixture drifted")
path.write_text(source.replace(marker, '"fee": "0"', 1), encoding="utf-8")
PY
  assert_rejected "a zeroed official fee fixture"

  python3 - "$BROWSER" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = 'test("PR-ES Settings renders all venue compiler contracts without credential filtering"'
if source.count(marker) != 1:
    raise SystemExit("PR-CX self-test setup failed: browser marker drifted")
path.write_text(source.replace(marker, marker.replace("test(", "test.skip("), 1), encoding="utf-8")
PY
  assert_rejected "a skipped product fixture"

  python3 - "$DOC" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
heading = "### 🟡 6.5 下一步执行队列"
path.write_text(source.replace(heading, heading + "\n\n1. **PR-CX Bitget UTA V3 Official Order Semantics** — stale", 1), encoding="utf-8")
PY
  assert_rejected "completed PR-CX returned to the queue"

  python3 - "$COVERAGE" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
rows = path.read_text(encoding="utf-8").splitlines()
prefix = "crates/exchange/tests/bitget_test.rs\t"
path.write_text("\n".join(row for row in rows if not row.startswith(prefix)) + "\n", encoding="utf-8")
PY
  assert_rejected "missing Bitget HTTP contract coverage"

  python3 - "$REPO_GATE" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "check_pr_cx_completion.sh"
if source.count(marker) != 2:
    raise SystemExit("PR-CX self-test setup failed: repo wiring drifted")
path.write_text(source.replace(marker, "removed_pr_cx_completion.sh", 1), encoding="utf-8")
PY
  assert_rejected "single-scope repo wiring"

  printf 'PR-CX completion self-test passed\n'
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
title = "PR-CX Bitget UTA V3 Official Order Semantics"
verify_anchor = "`bash scripts/check_pr_cx_completion.sh --self-test`"
evidence_contract = {
    "successor-pr-en": ("scripts/check_pr_en_completion.sh", "check_pr_en_completion.sh"),
    "successor-pr-eb": ("scripts/check_pr_eb_completion.sh", "check_pr_eb_completion.sh"),
    "successor-pr-bw": ("scripts/check_pr_bw_completion.sh", "check_pr_bw_completion.sh"),
    "successor-pr-bz": ("scripts/check_pr_bz_completion.sh", "check_pr_bz_completion.sh"),
    "client-order-id-policy": ("crates/exchange/src/client_order_id_policy.rs", "bitget_policy_rejects_outside_official_regex"),
    "shared-order-identity-plan": ("crates/api/src/services/hedge_preview/guards/identity/bitget.rs", "bitget_identity_constraints_bind_native_finality_and_fee_fixtures"),
    "identity-finality-fail-closed": ("crates/api/src/services/hedge_preview/guards/identity/bitget.rs", "bitget_identity_plan_fails_closed_without_private_finality_evidence"),
    "native-instrument-identity": ("crates/exchange/src/adapters/bitget_instruments_tests.rs", "official_identity_matrix_covers_usdt_usdc_coin_and_reality_boundaries"),
    "account-hold-mode-compiler": ("crates/exchange/src/adapters/bitget_order_compiler_tests.rs", "hedge_mode_derives_all_open_close_pos_side_combinations"),
    "rest-payload-position-side": ("crates/exchange/src/adapters/bitget_uta_trade_data_tests.rs", "compiled_usdc_hedge_order_preserves_native_category_symbol_and_pos_side"),
    "ws-payload-time-in-force": ("crates/exchange/src/adapters/bitget_uta_ws_trade_tests.rs", "place_request_matches_v3_envelope_shape"),
    "one-way-reduce-only": ("crates/exchange/src/adapters/bitget_order_compiler_tests.rs", "hedge_mode_derives_all_open_close_pos_side_combinations"),
    "rest-place-http-contract": ("crates/exchange/tests/bitget_test.rs", "live_place_order_sends_uta_v3_limit_order"),
    "unknown-result-query-recovery": ("crates/exchange/tests/bitget_test.rs", "live_place_order_recovers_unknown_result_by_client_oid_query"),
    "unknown-result-fail-closed": ("crates/exchange/tests/bitget_test.rs", "live_place_order_unknown_result_fails_closed_when_query_has_no_order"),
    "rest-cancel-ack-not-final": ("crates/exchange/tests/bitget_test.rs", "live_cancel_order_returns_cancel_requested"),
    "rest-order-query-client-oid": ("crates/exchange/tests/bitget_test.rs", "live_get_order_queries_detail_by_client_oid"),
    "rest-order-fee-context": ("crates/exchange/src/adapters/bitget_uta_private_data_tests.rs", "bitget_get_order_preserves_exec_type_and_cancel_reason_context"),
    "rest-cancelled-finality": ("crates/exchange/src/adapters/bitget_uta_private_data_tests.rs", "parse_open_order_accepts_official_cancelled_status"),
    "private-rest-strict-parser": ("crates/exchange/src/adapters/bitget_uta_private_data_tests.rs", "parse_open_order_rejects_unknown_side_type_status_or_tif"),
    "ws-place-cancel-ack": ("crates/exchange/src/adapters/bitget_uta_ws_trade_tests.rs", "bitget_uta_ws_place_order_ack_parses_official_fixture"),
    "private-order-stream-finality": ("crates/exchange/src/adapters/bitget_uta_ws_user_tests.rs", "order_and_fill_fixtures_preserve_terminal_finality_and_fees"),
    "private-fill-stream-fee": ("crates/exchange/src/adapters/bitget_uta_ws_user_tests.rs", "order_and_fill_fixtures_preserve_terminal_finality_and_fees"),
    "private-order-ledger-finality": ("crates/api/src/trading_service/private_ws_events/tests/deltas/order_updates/part_05.rs", "bitget_private_fill_and_order_finality_project_once"),
    "private-ws-server-ack-health": ("crates/api/src/lifecycle/private_ws/plain_venues_tests.rs", "bitget_private_ws_waits_for_login_and_all_topic_acknowledgements"),
    "official-rest-operation-registry": ("crates/exchange/src/venue_spec.rs", "bitget_get_order_evidence_uses_recorded_fixture_metadata"),
    "official-ws-operation-registry": ("crates/exchange/tests/ws_trading_specs_test.rs", "bitget_live_ws_write_ops_carry_official_evidence"),
    "capability-product-browser": ("test/e2e/pr_es_venue_capability_matrix.spec.ts", "test:e2e:pr-es"),
    "runtime-health-product-browser": ("test/e2e/pr_eg_runtime_health.spec.ts", "test:e2e:pr-eg"),
    "external-live-boundary": ("docs/PRODUCT_FULL_AUDIT_REFINEMENT.md", "check_product_audit_progress.sh"),
    "completion-governance": ("scripts/check_pr_cx_completion.sh", "check_pr_cx_completion.sh --self-test"),
}
runnable_anchors = (
    ("crates/exchange/src/client_order_id_policy.rs", "bitget_policy_rejects_outside_official_regex"),
    ("crates/api/src/services/hedge_preview/guards/identity/bitget.rs", "bitget_identity_constraints_bind_native_finality_and_fee_fixtures"),
    ("crates/api/src/services/hedge_preview/guards/identity/bitget.rs", "bitget_identity_plan_fails_closed_without_private_finality_evidence"),
    ("crates/exchange/src/adapters/bitget_instruments_tests.rs", "official_identity_matrix_covers_usdt_usdc_coin_and_reality_boundaries"),
    ("crates/exchange/src/adapters/bitget_order_compiler_tests.rs", "hedge_mode_derives_all_open_close_pos_side_combinations"),
    ("crates/exchange/src/adapters/bitget_order_compiler_tests.rs", "inverse_reality_and_misaligned_orders_fail_closed"),
    ("crates/exchange/src/adapters/bitget_uta_trade_data_tests.rs", "place_order_body_uses_time_in_force_and_v3_category"),
    ("crates/exchange/src/adapters/bitget_uta_trade_data_tests.rs", "compiled_usdc_hedge_order_preserves_native_category_symbol_and_pos_side"),
    ("crates/exchange/src/adapters/bitget_uta_trade_data_tests.rs", "rejects_client_oid_outside_official_policy"),
    ("crates/exchange/src/adapters/bitget_uta_ws_trade_tests.rs", "place_request_matches_v3_envelope_shape"),
    ("crates/exchange/src/adapters/bitget_uta_ws_trade_tests.rs", "place_request_translates_reduce_only_for_ws_schema"),
    ("crates/exchange/src/adapters/bitget_uta_ws_trade_tests.rs", "bitget_uta_ws_place_order_ack_parses_official_fixture"),
    ("crates/exchange/src/adapters/bitget_uta_ws_trade_tests.rs", "bitget_uta_ws_cancel_order_ack_parses_official_fixture"),
    ("crates/exchange/tests/bitget_test.rs", "live_place_order_sends_uta_v3_limit_order"),
    ("crates/exchange/tests/bitget_test.rs", "live_place_order_recovers_unknown_result_by_client_oid_query"),
    ("crates/exchange/tests/bitget_test.rs", "live_place_order_unknown_result_fails_closed_when_query_has_no_order"),
    ("crates/exchange/tests/bitget_test.rs", "live_cancel_order_returns_cancel_requested"),
    ("crates/exchange/tests/bitget_test.rs", "live_get_order_queries_detail_by_client_oid"),
    ("crates/exchange/src/adapters/bitget_uta_private_data_tests.rs", "bitget_get_order_parses_official_fixture"),
    ("crates/exchange/src/adapters/bitget_uta_private_data_tests.rs", "bitget_get_order_preserves_exec_type_and_cancel_reason_context"),
    ("crates/exchange/src/adapters/bitget_uta_private_data_tests.rs", "parse_open_order_accepts_official_cancelled_status"),
    ("crates/exchange/src/adapters/bitget_uta_private_data_tests.rs", "parse_open_order_rejects_unknown_side_type_status_or_tif"),
    ("crates/exchange/src/adapters/bitget_uta_ws_user_tests.rs", "order_and_fill_fixtures_preserve_terminal_finality_and_fees"),
    ("crates/api/src/trading_service/private_ws_events/tests/deltas/order_updates/part_05.rs", "bitget_private_fill_and_order_finality_project_once"),
    ("crates/api/src/lifecycle/private_ws/plain_venues_tests.rs", "bitget_private_ws_waits_for_login_and_all_topic_acknowledgements"),
    ("crates/exchange/src/venue_spec.rs", "bitget_get_order_evidence_uses_recorded_fixture_metadata"),
    ("crates/exchange/tests/ws_trading_specs_test.rs", "bitget_live_ws_write_ops_carry_official_evidence"),
)
browser_contract = {
    "test/e2e/pr_es_venue_capability_matrix.spec.ts": (
        "PR-ES Settings renders all venue compiler contracts without credential filtering",
    ),
    "test/e2e/pr_eg_runtime_health.spec.ts": (
        "PR-EG Settings consumes typed venue runtime health and fails closed without evidence",
    ),
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-CX completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row or row.count(verify_anchor) != 1:
    fail("roadmap row must be complete, remaining-none and bound to the destructive gate")
queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-CX\b", queue):
    fail("completed PR-CX remains in the local queue")
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
if "PR-CX 本地" not in queue or "真实 Bitget credential" not in queue:
    fail("external live-capture boundary is missing")

history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(encoding="utf-8")
if "## 2026-07-16 PR-CX Bitget UTA V3 Order Semantics Closure" not in history:
    fail("history closure appendix is missing")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [entry for entry in csv.DictReader(handle, delimiter="\t") if entry["pr_id"] == "PR-CX"]
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
if "single_post=true" not in indexed["unknown-result-query-recovery"]["notes"]:
    fail("unknown-result recovery must prove a single write")
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

identity = (root / "crates/api/src/services/hedge_preview/guards/identity/bitget.rs").read_text(encoding="utf-8")
for marker in (
    '"bitget-uta-v3-instrument-identity"',
    '"bitget-uta-v3-private-order-finality"',
    '"bitget-uta-v3-private-fill-fee"',
    '"bitget/uta_ws_order_filled.json"',
    '"bitget/uta_ws_fill.json"',
    '"bitget_private_fill_and_order_finality_project_once"',
):
    if marker not in identity:
        fail(f"shared identity marker missing: {marker}")

fixture_hashes = {
    "crates/exchange/fixtures/bitget/uta_instruments_identity_matrix.json": "1d638cbce67260f8fd1509a3d2fdf0cbda9a767821f877bcfb4295c832e09783",
    "crates/exchange/fixtures/bitget/uta_ws_order_filled.json": "6f5da324c84e6c7fd9a4d7b4a69a089631ebc0f749fd607887c71a9601066e1b",
    "crates/exchange/fixtures/bitget/uta_ws_order_cancelled.json": "9fdcd4a1ddf4d5e4bf0749b073f638754219f8ad7b76a4af62b58198faf8a52b",
    "crates/exchange/fixtures/bitget/uta_ws_fill.json": "5e700644d762d9f24dc1d5b9ab9f9c8f9dcac9d03d7349e73d55db77b56e9279",
    "crates/exchange/fixtures/bitget/uta_place_order_ack.json": "22dc071f603168741b8d5e8f393f5838a5286a992286bc0369c0b7e4a1b33932",
    "crates/exchange/fixtures/bitget/uta_cancel_order_ack.json": "bd083965f8d4e03c3fa9cf64d2aacdc8d6337ed763d2d7f050c9cb7a087db348",
    "crates/exchange/fixtures/bitget/uta_order_info_filled.json": "79653caccf87a5bab7cd986acd4046a6fd2dccee61d346a636e5249de51f6ddd",
    "crates/exchange/fixtures/bitget/uta_ws_place_order_ack.json": "edb7ed0bc16594f7538548d67d9f8a53e831a093432c76bac0bf41d85016c9e6",
    "crates/exchange/fixtures/bitget/uta_ws_cancel_order_ack.json": "4599399cd0b6fe7ba69dbca6128b226792ddb7ae35517ef3f132cfad6b7d879d",
}
for relative, expected in fixture_hashes.items():
    if hashlib.sha256((root / relative).read_bytes()).hexdigest() != expected:
        fail(f"official fixture hash drifted: {relative}")

order_payload = json.loads((root / "crates/exchange/fixtures/bitget/uta_order_info_filled.json").read_text(encoding="utf-8"))
order = order_payload.get("data") or {}
if order.get("orderStatus") != "filled" or Decimal(order.get("cumExecQty", "0")) <= 0:
    fail("official order detail fixture must retain terminal fill truth")
fees = order.get("feeDetail") or []
if not fees or Decimal(fees[0].get("fee", "0")) <= 0 or not fees[0].get("feeCoin"):
    fail("official order detail fixture must retain positive fee truth")
for field in ("clientOid", "orderId", "execType", "cancelReason"):
    if field not in order:
        fail(f"official order detail fixture is missing rich field: {field}")

private_rest = (root / "crates/exchange/src/adapters/bitget_uta_private_rest.rs").read_text(encoding="utf-8")
for code in ('"40010"', '"40725"', '"45001"'):
    if code not in private_rest:
        fail(f"unknown-result recovery code missing: {code}")
if private_rest.count("LiveOrderState::CancelRequested") != 1 or "final state requires order query" not in private_rest:
    fail("REST cancel ACK must remain non-terminal")
adapter = (root / "crates/exchange/src/adapters/bitget.rs").read_text(encoding="utf-8")
for marker in ("is_unknown_place_result", "LiveTradingAdapter::get_order", "ack_from_order_query"):
    if marker not in adapter:
        fail(f"unknown-result query recovery marker missing: {marker}")
order_parser = (root / "crates/exchange/src/adapters/bitget_uta_private_data/order.rs").read_text(encoding="utf-8")
for marker in ("fee_detail", "exec_type", "cancel_reason", "parse_fees"):
    if marker not in order_parser:
        fail(f"read-side rich order marker missing: {marker}")
private_data = (root / "crates/exchange/src/adapters/bitget_uta_private_data.rs").read_text(encoding="utf-8")
if '"canceled" | "cancelled"' not in private_data:
    fail("official cancelled spelling is not accepted")

for relative, titles in browser_contract.items():
    browser = (root / relative).read_text(encoding="utf-8")
    for title_text in titles:
        escaped = re.escape(title_text)
        if re.search(rf'test\.(?:skip|fixme)\(\s*["\']{escaped}["\']', browser):
            fail(f"browser anchor is skipped: {title_text}")
        if not re.search(rf'test\(\s*["\']{escaped}["\']', browser):
            fail(f"browser anchor is missing: {title_text}")

repo_gate = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
if repo_gate.count("check_pr_cx_completion.sh") != 2:
    fail("repo gate must execute PR-CX once in docs and all scopes")

print(
    f"OK PR-CX static contract ({len(evidence_contract)} evidence types; "
    f"{len(runnable_anchors)} runnable anchors; "
    f"{sum(len(titles) for titles in browser_contract.values())} browser anchors)"
)
PY

if [[ "${PR_CX_SKIP_BROWSER_LIST:-0}" != "1" ]]; then
  npx playwright test "$BROWSER" "$ROOT/test/e2e/pr_eg_runtime_health.spec.ts" --list >/dev/null
fi

if [[ "${PR_CX_SKIP_UPSTREAM:-0}" != "1" ]]; then
  PR_EN_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_en_completion.sh"
  PR_EB_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_eb_completion.sh"
  PR_BW_SKIP_TESTS=1 PR_BW_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_bw_completion.sh"
  PR_BZ_SKIP_TESTS=1 PR_BZ_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_bz_completion.sh"
  bash "$ROOT/scripts/check_product_audit_progress.sh"
  bash "$ROOT/scripts/check_product_audit_evidence.sh"
  bash "$ROOT/scripts/check_product_audit_evidence_index.sh"
  bash "$ROOT/scripts/check_product_audit_coverage.sh"
fi

if [[ "${PR_CX_SKIP_TESTS:-0}" != "1" ]]; then
  jobs="${CARGO_BUILD_JOBS:-10}"
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --lib bitget_ --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --test bitget_test --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --test bitget_pr_en_contract_test --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --test ws_trading_specs_test bitget_ --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api bitget_ --no-fail-fast
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-es -- --workers=1
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-eg -- --workers=1
fi

printf 'PR-CX Bitget UTA V3 order semantics completion passed\n'
