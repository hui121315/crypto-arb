#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
DOC="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
EVIDENCE="$ROOT/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
COVERAGE="$ROOT/docs/PRODUCT_AUDIT_COVERAGE.tsv"
HISTORY="$ROOT/docs/audit_history/PRODUCT_AUDIT_HISTORY.md"
IDENTITY="$ROOT/crates/api/src/services/hedge_preview/guards/identity.rs"
PARSER="$ROOT/crates/exchange/src/adapters/binance_ws_user_tests.rs"
FIXTURE="$ROOT/crates/exchange/fixtures/binance/usdm_order_trade_update_filled.json"
BROWSER="$ROOT/test/e2e/pr_el_binance_identity.spec.ts"
REPO_GATE="$ROOT/scripts/verify_repo_gates.sh"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-CV completion gate failed: %s\n' "$1" >&2
  exit 1
}

if [[ "$MODE" == "--self-test" ]]; then
  temp="$(mktemp -d "${TMPDIR:-/tmp}/crossline-pr-cv.XXXXXX")"
  for path in "$DOC" "$EVIDENCE" "$COVERAGE" "$IDENTITY" "$PARSER" "$FIXTURE" "$BROWSER" "$REPO_GATE"; do
    cp "$path" "$temp/$(basename "$path")"
  done
  restore() {
    for path in "$DOC" "$EVIDENCE" "$COVERAGE" "$IDENTITY" "$PARSER" "$FIXTURE" "$BROWSER" "$REPO_GATE"; do
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
    PR_CV_SKIP_TESTS=1 PR_CV_SKIP_UPSTREAM=1 PR_CV_SKIP_BROWSER_LIST=1 \
      bash "$0" >/dev/null 2>&1
    status=$?
    set -e
    if [[ "$status" -eq 0 ]]; then
      fail "self-test accepted $label"
    fi
    restore
  }

  PR_CV_SKIP_TESTS=1 PR_CV_SKIP_UPSTREAM=1 PR_CV_SKIP_BROWSER_LIST=1 \
    bash "$0" >/dev/null

  python3 - "$DOC" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "| `PR-CV Binance Official USD-M Order Semantics` | ✅ 完成 |"
if source.count(marker) != 1:
    raise SystemExit("PR-CV self-test setup failed: roadmap row drifted")
path.write_text(source.replace(marker, marker.replace("✅ 完成", "🟡 部分完成"), 1), encoding="utf-8")
PY
  assert_rejected "a downgraded roadmap row"

  python3 - "$EVIDENCE" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
rows = path.read_text(encoding="utf-8").splitlines()
path.write_text("\n".join(row for row in rows if not row.startswith("PR-CV\tterminal-private-order-fixture\t")) + "\n", encoding="utf-8")
PY
  assert_rejected "an incomplete evidence matrix"

  python3 - "$IDENTITY" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = '"binance-usdm-durable-finality"'
if source.count(marker) != 1:
    raise SystemExit("PR-CV self-test setup failed: finality marker drifted")
path.write_text(source.replace(marker, '"removed-binance-finality"', 1), encoding="utf-8")
PY
  assert_rejected "detached shared finality evidence"

  python3 - "$FIXTURE" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = '"X": "FILLED"'
if source.count(marker) != 1:
    raise SystemExit("PR-CV self-test setup failed: terminal fixture drifted")
path.write_text(source.replace(marker, '"X": "PARTIALLY_FILLED"', 1), encoding="utf-8")
PY
  assert_rejected "a non-terminal finality fixture"

  python3 - "$PARSER" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "fn parses_order_trade_update_filled_fixture_to_terminal_delta"
if source.count(marker) != 1:
    raise SystemExit("PR-CV self-test setup failed: parser marker drifted")
path.write_text(source.replace(marker, "#[ignore]\n" + marker, 1), encoding="utf-8")
PY
  assert_rejected "an ignored terminal parser"

  python3 - "$BROWSER" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = 'test("PR-EL renders Binance USDC canonical and native identity with client-id policy"'
if source.count(marker) != 1:
    raise SystemExit("PR-CV self-test setup failed: browser marker drifted")
path.write_text(source.replace(marker, marker.replace("test(", "test.skip("), 1), encoding="utf-8")
PY
  assert_rejected "a skipped product fixture"

  python3 - "$DOC" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
heading = "### 🟡 6.5 下一步执行队列"
path.write_text(source.replace(heading, heading + "\n\n1. **PR-CV Binance Official USD-M Order Semantics** — stale", 1), encoding="utf-8")
PY
  assert_rejected "completed PR-CV returned to the queue"

  python3 - "$DOC" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "| `PR-CW Bybit Official V5 Order Semantics` | ✅ 完成 |"
if source.count(marker) != 1:
    raise SystemExit("PR-CV self-test setup failed: PR-CW successor row drifted")
path.write_text(source.replace(marker, marker.replace("✅ 完成", "🟡 部分完成"), 1), encoding="utf-8")
PY
  assert_rejected "a downgraded successor with an advanced queue"

  python3 - "$COVERAGE" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
rows = path.read_text(encoding="utf-8").splitlines()
prefix = "crates/exchange/src/adapters/binance_ws_user_tests.rs\t"
path.write_text("\n".join(row for row in rows if not row.startswith(prefix)) + "\n", encoding="utf-8")
PY
  assert_rejected "missing terminal parser coverage"

  python3 - "$REPO_GATE" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "check_pr_cv_completion.sh"
if source.count(marker) != 2:
    raise SystemExit("PR-CV self-test setup failed: repo wiring drifted")
path.write_text(source.replace(marker, "removed_pr_cv_completion.sh", 1), encoding="utf-8")
PY
  assert_rejected "single-scope repo wiring"

  printf 'PR-CV completion self-test passed\n'
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
title = "PR-CV Binance Official USD-M Order Semantics"
verify_anchor = "`bash scripts/check_pr_cv_completion.sh --self-test`"
evidence_contract = {
    "successor-pr-el": ("scripts/check_pr_el_completion.sh", "check_pr_el_completion.sh"),
    "successor-pr-eb": ("scripts/check_pr_eb_completion.sh", "check_pr_eb_completion.sh"),
    "successor-pr-bv": ("scripts/check_pr_bv_completion.sh", "check_pr_bv_completion.sh"),
    "successor-pr-bw": ("scripts/check_pr_bw_completion.sh", "check_pr_bw_completion.sh"),
    "successor-pr-bz": ("scripts/check_pr_bz_completion.sh", "check_pr_bz_completion.sh"),
    "client-order-id-policy": ("crates/exchange/src/client_order_id_policy.rs", "binance_policy_accepts_official_regex"),
    "shared-order-identity-plan": ("crates/api/src/services/hedge_preview/guards/identity.rs", "binance_identity_constraints_bind_terminal_fill_fee_fixture"),
    "identity-finality-fail-closed": ("crates/api/src/services/hedge_preview/guards/identity.rs", "binance_identity_plan_fails_closed_without_private_finality_evidence"),
    "rest-order-payload": ("crates/exchange/src/adapters/binance_trade_data_tests.rs", "rest_market_order_omits_price_and_tif"),
    "ws-order-payload": ("crates/exchange/src/adapters/binance_ws_trade_tests.rs", "order_place_request_validates_official_client_order_id_rule"),
    "account-position-mode": ("crates/exchange/src/adapters/binance_private_data_tests.rs", "binance_position_mode_parses_official_fixture"),
    "native-instrument-sizing": ("crates/exchange/src/adapters/binance_exchange_info_tests.rs", "registry_projection_uses_compiled_usdt_and_usdc_specs"),
    "market-depth-cost-guard": ("crates/api/src/services/hedge_ticket/tests/cost_evidence.rs", "ticket_cost_rejects_missing_or_stale_depth_evidence"),
    "rest-place-http-fixture": ("crates/exchange/src/adapters/binance_private_rest.rs", "place_order_official_envelope_builds_accepted_ack"),
    "rest-cancel-http-fixture": ("crates/exchange/src/adapters/binance_private_rest.rs", "cancel_order_official_envelope_builds_cancelled_ack"),
    "rest-query-http-fixture": ("crates/exchange/src/adapters/binance_private_rest.rs", "get_order_parses_official_filled_order"),
    "ws-place-cancel-fixture": ("crates/exchange/src/adapters/binance_ws_trade_tests.rs", "binance_ws_place_order_ack_parses_official_fixture"),
    "private-order-stream-partial-fill": ("crates/exchange/fixtures/binance/usdm_order_trade_update_partial_fill.json", "parses_order_trade_update_to_order_delta"),
    "terminal-private-order-fixture": ("crates/exchange/fixtures/binance/usdm_order_trade_update_filled.json", "parses_order_trade_update_filled_fixture_to_terminal_delta"),
    "terminal-private-order-parser": ("crates/exchange/src/adapters/binance_ws_user_tests.rs", "parses_order_trade_update_filled_fixture_to_terminal_delta"),
    "private-order-ledger-finality": ("crates/api/src/trading_service/private_ws_events/tests/deltas/order_updates/part_02.rs", "binance_order_trade_fill_is_one_identity_preserving_ledger_outcome"),
    "private-ws-durable-health": ("crates/api/src/lifecycle/private_ws/tests/projection.rs", "binance_terminal_fill_projects_execution_and_health_once_after_ack"),
    "commission-rate-fixture": ("crates/exchange/src/adapters/binance_fee_evidence_tests.rs", "parses_official_commission_fixture_without_zeroing_rates"),
    "official-operation-registry": ("crates/exchange/src/venue_spec.rs", "binance_pr_el_private_rest_registry_is_recorded"),
    "product-browser": ("test/e2e/pr_el_binance_identity.spec.ts", "test:e2e:pr-el"),
    "external-live-boundary": ("docs/PRODUCT_FULL_AUDIT_REFINEMENT.md", "check_product_audit_progress.sh"),
    "completion-governance": ("scripts/check_pr_cv_completion.sh", "check_pr_cv_completion.sh --self-test"),
}
runnable_anchors = (
    ("crates/exchange/src/client_order_id_policy.rs", "binance_policy_accepts_official_regex"),
    ("crates/exchange/src/adapters/binance_trade_data_tests.rs", "rest_place_order_validates_official_client_order_id_rule"),
    ("crates/exchange/src/adapters/binance_trade_data_tests.rs", "rest_market_order_omits_price_and_tif"),
    ("crates/exchange/src/adapters/binance_trade_data_tests.rs", "position_side_is_serialized_from_verified_mode"),
    ("crates/exchange/src/adapters/binance_trade_data_tests.rs", "hedge_mode_reduce_only_is_rejected"),
    ("crates/exchange/src/adapters/binance_trade_data_tests.rs", "market_order_uses_market_lot_size_quantity_filter"),
    ("crates/exchange/src/adapters/binance_ws_trade_tests.rs", "order_place_request_validates_official_client_order_id_rule"),
    ("crates/exchange/src/adapters/binance_ws_trade_tests.rs", "order_place_request_serializes_verified_position_side"),
    ("crates/exchange/src/adapters/binance_ws_trade_tests.rs", "order_place_request_rejects_hedge_reduce_only"),
    ("crates/exchange/src/adapters/binance_private_data_tests.rs", "binance_position_mode_parses_official_fixture"),
    ("crates/exchange/src/adapters/binance_exchange_info_tests.rs", "registry_projection_uses_compiled_usdt_and_usdc_specs"),
    ("crates/exchange/src/adapters/binance_ws_user_tests.rs", "parses_order_trade_update_filled_fixture_to_terminal_delta"),
    ("crates/api/src/services/hedge_preview/guards/identity.rs", "binance_identity_constraints_bind_terminal_fill_fee_fixture"),
    ("crates/api/src/services/hedge_preview/guards/identity.rs", "binance_identity_plan_fails_closed_without_private_finality_evidence"),
    ("crates/api/src/services/hedge_ticket/tests/cost_evidence.rs", "ticket_cost_rejects_missing_or_stale_depth_evidence"),
    ("crates/api/src/trading_service/private_ws_events/tests/deltas/order_updates/part_02.rs", "binance_order_trade_fill_is_one_identity_preserving_ledger_outcome"),
    ("crates/api/src/lifecycle/private_ws/tests/projection.rs", "binance_terminal_fill_projects_execution_and_health_once_after_ack"),
)
browser_titles = (
    "PR-EL renders Binance USDC canonical and native identity with client-id policy",
    "PR-EL blocks submit when the shared identity evidence contract is missing",
    "PR-EL exposes canonical mismatch and unavailable live runtime evidence fail closed",
)


def fail(message: str) -> None:
    raise SystemExit(f"PR-CV completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row or row.count(verify_anchor) != 1:
    fail("roadmap row must be complete, remaining-none and bound to the destructive gate")
queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-CV\b", queue):
    fail("completed PR-CV remains in the local queue")
cw_row = next((line for line in doc.splitlines() if line.startswith("| `PR-CW Bybit Official V5 Order Semantics`")), None)
if cw_row is None:
    fail("PR-CW successor roadmap row is missing")
if "✅ 完成" in cw_row:
    if re.search(r"(?m)^\d+\.\s+\*\*PR-CW\b", queue):
        fail("completed PR-CW successor remains in the local queue")
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
        if head is None:
            fail("completed successor chain must hand off to an incomplete roadmap row")
        head_row = next((line for line in doc.splitlines() if line.startswith(f"| `{head.group(1)} ")), "")
        if not head_row or "✅ 完成" in head_row:
            fail("completed successor chain handed off to a completed or missing roadmap row")
    elif not re.search(rf"(?m)^1\.\s+\*\*{next_pr}\b", queue):
        fail(f"{next_pr} must follow the completed PR-CW successor")
elif "🟡 部分完成" in cw_row:
    if not re.search(r"(?m)^1\.\s+\*\*PR-CW\b", queue):
        fail("incomplete PR-CW must remain the next queue head")
else:
    fail("PR-CW successor must be complete or partial")
queue_items = re.findall(r"(?m)^\d+\.\s+\*\*", queue)
if not queue_items:
    fail("local queue plus external pool must not be empty")
if "PR-CV 本地" not in queue or "真实 Binance credential" not in queue:
    fail("external live-capture boundary is missing")

history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(encoding="utf-8")
if "## 2026-07-16 PR-CV Binance USD-M Order Semantics Closure" not in history:
    fail("history closure appendix is missing")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [entry for entry in csv.DictReader(handle, delimiter="\t") if entry["pr_id"] == "PR-CV"]
indexed = {entry["evidence_type"]: entry for entry in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for evidence_type, (artifact, command_anchor) in evidence_contract.items():
    evidence = indexed[evidence_type]
    if evidence["artifact"] != artifact or command_anchor not in evidence["command"]:
        fail(f"evidence anchor drifted: {evidence_type}")
    if not (root / artifact).is_file() or not evidence["notes"].strip():
        fail(f"evidence artifact or note is missing: {artifact}")
if "ack_not_final=true" not in indexed["terminal-private-order-parser"]["notes"]:
    fail("terminal parser must preserve the ACK-not-final boundary")
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
    '"binance-usdm-durable-finality"',
    '"binance-usdm-order-trade-fill-fee"',
    '"binance/usdm_order_trade_update_filled.json"',
    '"parses_order_trade_update_filled_fixture_to_terminal_delta"',
):
    if marker not in identity:
        fail(f"shared identity marker missing: {marker}")

fixture_path = root / "crates/exchange/fixtures/binance/usdm_order_trade_update_filled.json"
if hashlib.sha256(fixture_path.read_bytes()).hexdigest() != "f2172c9694e90bfe310e7145db8600c42ca3e3bc3893df1a7ff8dc5ec3b71758":
    fail("terminal private-order fixture hash drifted")
payload = json.loads(fixture_path.read_text(encoding="utf-8"))
order = payload.get("o", {})
if payload.get("e") != "ORDER_TRADE_UPDATE" or order.get("x") != "TRADE" or order.get("X") != "FILLED":
    fail("terminal fixture must be a filled ORDER_TRADE_UPDATE trade")
if not order.get("c") or not order.get("i") or not order.get("t"):
    fail("terminal fixture must preserve client, exchange and trade identity")
if Decimal(order.get("z", "0")) != Decimal(order.get("q", "-1")):
    fail("terminal fixture cumulative fill must equal order quantity")
if not order.get("N") or Decimal(order.get("n", "0")) <= 0:
    fail("terminal fixture must preserve positive fill fee and currency")

browser = (root / "test/e2e/pr_el_binance_identity.spec.ts").read_text(encoding="utf-8")
for title_text in browser_titles:
    escaped = re.escape(title_text)
    if re.search(rf'test\.(?:skip|fixme)\(\s*["\']{escaped}["\']', browser):
        fail(f"browser anchor is skipped: {title_text}")
    if not re.search(rf'test\(\s*["\']{escaped}["\']', browser):
        fail(f"browser anchor is missing: {title_text}")

repo_gate = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
if repo_gate.count("check_pr_cv_completion.sh") != 2:
    fail("repo gate must execute PR-CV once in docs and all scopes")

print(
    f"OK PR-CV static contract ({len(evidence_contract)} evidence types; "
    f"{len(runnable_anchors)} runnable anchors; {len(browser_titles)} browser anchors)"
)
PY

if [[ "${PR_CV_SKIP_BROWSER_LIST:-0}" != "1" ]]; then
  npx playwright test "$BROWSER" --list >/dev/null
fi

if [[ "${PR_CV_SKIP_UPSTREAM:-0}" != "1" ]]; then
  bash "$ROOT/scripts/check_pr_el_completion.sh"
  PR_EB_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_eb_completion.sh"
  PR_BV_SKIP_TESTS=1 PR_BV_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_bv_completion.sh"
  PR_BW_SKIP_TESTS=1 PR_BW_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_bw_completion.sh"
  PR_BZ_SKIP_TESTS=1 PR_BZ_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_bz_completion.sh"
  bash "$ROOT/scripts/check_product_audit_progress.sh"
  bash "$ROOT/scripts/check_product_audit_evidence.sh"
  bash "$ROOT/scripts/check_product_audit_evidence_index.sh"
  bash "$ROOT/scripts/check_product_audit_coverage.sh"
fi

if [[ "${PR_CV_SKIP_TESTS:-0}" != "1" ]]; then
  jobs="${CARGO_BUILD_JOBS:-10}"
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --lib binance_ --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --test binance_test --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --test ws_trading_specs_test binance_ --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api binance_identity_ --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api binance_order_trade_ --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api binance_terminal_ --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api ticket_cost_rejects_missing_or_stale_depth_evidence --no-fail-fast
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-el -- --workers=1
fi

printf 'PR-CV Binance USD-M order semantics completion passed\n'
