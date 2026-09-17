#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
SOURCE="$ROOT/crates/api/src/lifecycle/private_ws/plain_venues.rs"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

if [[ "$MODE" == "--self-test" ]]; then
  backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-ek.XXXXXX")"
  cp "$SOURCE" "$backup"
  restore() {
    cp "$backup" "$SOURCE"
    rm -f "$backup"
  }
  trap restore EXIT
  python3 - "$SOURCE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
start = source.index("pub(super) fn spawn_okx_private_ws")
end = source.index("pub(super) fn spawn_bybit_private_ws", start)
block = source[start:end]
marker = "tokio::spawn(run_confirmed_private_ws("
if marker not in block:
    raise SystemExit("PR-EK self-test setup failed: confirmed OKX runtime marker missing")
block = block.replace(marker, "tokio::spawn(run_plain_private_ws(", 1)
path.write_text(source[:start] + block + source[end:], encoding="utf-8")
PY
  if PR_EK_SKIP_TESTS=1 bash "$0" >/dev/null 2>&1; then
    printf 'PR-EK completion self-test failed: send-only OKX private WS passed\n' >&2
    exit 1
  fi
  printf 'PR-EK completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from __future__ import annotations

import csv
import hashlib
import re
import sys
from pathlib import Path


root = Path(sys.argv[1])
title = "PR-EK OKX V5 Order, Account & Private WS Semantics Contract"
evidence_contract = {
    "ack-fail-closed": (
        "crates/exchange/src/adapters/okx_trade_data_ack_tests.rs",
        "ack_confirmed_success_maps_to_accepted",
    ),
    "order-compiler-account-mode": (
        "crates/exchange/src/adapters/okx_trade_data_tests.rs",
        "long_short_mode_maps_open_and_close_pos_side",
    ),
    "instrument-sizing-native-contract": (
        "crates/exchange/tests/okx_test.rs",
        "live_place_order_uses_official_instrument_contract_sizing",
    ),
    "private-rest-official-registry": (
        "crates/exchange/src/venue_spec.rs",
        "okx_account_position_evidence_uses_recorded_fixture_metadata",
    ),
    "private-ws-server-ack-health": (
        "crates/api/src/lifecycle/private_ws/plain_venues_tests.rs",
        "okx_private_ws_waits_for_all_server_acknowledgements",
    ),
    "private-ws-fill-fee-finality": (
        "crates/api/src/trading_service/private_ws_events/tests/deltas/order_updates/part_03.rs",
        "okx_cancel_fixture_is_final_only_after_private_order_event",
    ),
    "official-fixture-operation-registry": (
        "crates/exchange/src/ws/trading.rs",
        "bash scripts/check_pr_ek_completion.sh",
    ),
    "product-browser-and-completion-governance": (
        "scripts/check_pr_ek_completion.sh",
        "bash scripts/check_pr_ek_completion.sh --self-test",
    ),
}

anchors = (
    ("crates/exchange/src/adapters/okx_trade_data_tests.rs", "limit_ioc_and_fok_use_official_ord_type_with_px"),
    ("crates/exchange/src/adapters/okx_trade_data_tests.rs", "limit_gtx_uses_official_post_only_ord_type"),
    ("crates/exchange/src/adapters/okx_trade_data_tests.rs", "net_mode_sends_net_pos_side_and_reduce_only"),
    ("crates/exchange/src/adapters/okx_trade_data_tests.rs", "long_short_mode_maps_open_and_close_pos_side"),
    ("crates/exchange/src/adapters/okx_live_tests.rs", "okx_account_config_parses_official_fixture_position_mode"),
    ("crates/exchange/src/adapters/okx_instruments_tests.rs", "okx_instrument_rule_parses_official_swap_fixture"),
    ("crates/exchange/tests/okx_test.rs", "live_place_order_uses_official_instrument_contract_sizing"),
    ("crates/exchange/tests/okx_test.rs", "live_cancel_order_uses_official_cancel_order_path_and_client_order_id"),
    ("crates/exchange/src/adapters/okx_ws_user_tests.rs", "private_ws_control_fixtures_require_login_and_three_subscription_acks"),
    ("crates/api/src/lifecycle/private_ws/plain_venues_tests.rs", "okx_login_and_subscription_rejections_fail_closed"),
    ("crates/api/src/trading_service/private_ws_events/tests/deltas/order_updates/part_03.rs", "okx_partial_fill_fixture_preserves_identity_fee_and_deduplicates"),
    ("crates/api/src/trading_service/private_ws_events/tests/deltas/order_updates/part_03.rs", "okx_cancel_fixture_is_final_only_after_private_order_event"),
    ("crates/exchange/src/venue_spec.rs", "okx_place_order_evidence_uses_recorded_fixture_metadata"),
    ("crates/exchange/src/venue_spec.rs", "okx_cancel_order_evidence_uses_recorded_fixture_metadata"),
    ("crates/exchange/src/venue_spec.rs", "okx_get_order_evidence_uses_recorded_fixture_metadata"),
    ("crates/exchange/src/venue_spec.rs", "okx_open_orders_evidence_uses_recorded_fixture_metadata"),
    ("crates/exchange/src/venue_spec.rs", "okx_account_config_evidence_uses_recorded_fixture_metadata"),
    ("crates/exchange/src/venue_spec.rs", "okx_account_balance_evidence_uses_recorded_fixture_metadata"),
    ("crates/exchange/src/venue_spec.rs", "okx_account_position_evidence_uses_recorded_fixture_metadata"),
)

fixtures = {
    "crates/exchange/fixtures/okx/public_instruments_swap.json": "12f958835984aafc2c8280c93e33fbefcadba9351f113275351faf51c9ac76eb",
    "crates/exchange/fixtures/okx/trade_place_order_ack.json": "60311d35248b9fc2385a2181f695c6d0d82d6739a1587ea6ad0e02406e0be4ef",
    "crates/exchange/fixtures/okx/trade_cancel_order_ack.json": "89dfd90d94b74305ab38077f311d50c7adf51f07ffd64a740398829b4f839dad",
    "crates/exchange/fixtures/okx/trade_get_order_filled.json": "ab130bf25d597425e16841477e61fac0b5d4a345eb305d340bcec40bd82392b1",
    "crates/exchange/fixtures/okx/trade_orders_pending.json": "b3c992124a74c65a35d662bc64713dc412ed1b4c08592cdb4f5255424a6f9cbf",
    "crates/exchange/fixtures/okx/account_config_long_short.json": "d8967eefd2b0a5a88960be4c41768d392b866c572e027021797eb16fd3a01aa5",
    "crates/exchange/fixtures/okx/account_balance_usdt.json": "5b845155b110daa670b43b945d0302f0c9fea4a54e373187d15f4c2559a8bb7f",
    "crates/exchange/fixtures/okx/account_positions_swap.json": "1e41e1e84e43736189c5614466e955494acae539e11dcc0fbafe33199b3ef03e",
    "crates/exchange/fixtures/okx/ws_trade_place_order_ack.json": "de0e9f857f687f60bbfab4a50af4042420371c6f1a46b357e63402f0cf7e9882",
    "crates/exchange/fixtures/okx/ws_trade_cancel_order_ack.json": "a439243e8c548d995079fe726503d98073dc7366fc5d395727e96f4210d62314",
    "crates/exchange/fixtures/okx/ws_user_account_snapshot.json": "837fffd52faf84446073be08d72cb23c6b3c458ed17855dafccf489d0257d9a8",
    "crates/exchange/fixtures/okx/ws_user_login_failure.json": "4d9937dcad99c83f2e82a2282cfe42dbedb77724e8145c8151763be428873dee",
    "crates/exchange/fixtures/okx/ws_user_login_success.json": "289c83d7658f998d3582b4588394360667b86310d9e7c67ba8fbd7b09fc48eb7",
    "crates/exchange/fixtures/okx/ws_user_orders_canceled.json": "7499ed246ddc78222a8c86007a174748f11bcf762fd8411177db372a23c94be5",
    "crates/exchange/fixtures/okx/ws_user_orders_partial_fill.json": "f1d2431b6abbb8639c83784ca70859b6cf5839b5cb8093b1b030df9d0837306d",
    "crates/exchange/fixtures/okx/ws_user_positions_snapshot.json": "0d85b05993c200d4371245547a751d4580bdc94defbc7064e3b13ea4771609eb",
    "crates/exchange/fixtures/okx/ws_user_subscribe_account_success.json": "a61a481ac0efdb07119fed50c88b6e38bcc28c03b10a189e54b3bcf01734a09b",
    "crates/exchange/fixtures/okx/ws_user_subscribe_error.json": "03c2e3a8d5b9ed0e39a172f359c21f5af39e5be91b70064b38e4f207e13b2471",
    "crates/exchange/fixtures/okx/ws_user_subscribe_orders_success.json": "85fd02403e090681f4dacc1c003c79b73ca856496aad71f20a2e59a542c3196b",
    "crates/exchange/fixtures/okx/ws_user_subscribe_positions_success.json": "6ff18c5efcd48b78e49ce590714943799f5fc823dba4aef4c731b8434b5d30bc",
}

def fail(message: str) -> None:
    raise SystemExit(f"PR-EK completion gate failed: {message}")

doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be completed with no local remainder")
queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-EK\b", queue):
    fail("completed PR-EK remains in the local queue")
successor_row = next(
    (line for line in doc.splitlines() if line.startswith("| `PR-EM Bybit V5 Order")),
    None,
)
successor_queued = re.search(r"(?m)^\d+\.\s+\*\*PR-EM\b", queue) is not None
successor_completed = successor_row is not None and "✅ 完成" in successor_row
if not successor_queued and not successor_completed:
    fail("PR-EM must remain queued or have a completed roadmap row")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-EK"]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for kind, (artifact, command_anchor) in evidence_contract.items():
    row = indexed[kind]
    if row["artifact"] != artifact or command_anchor not in row["command"]:
        fail(f"{kind} artifact/command drifted")
    if not (root / artifact).is_file():
        fail(f"missing evidence artifact {artifact}")

for path, anchor in anchors:
    source = (root / path).read_text(encoding="utf-8")
    if f"fn {anchor}(" not in source and anchor not in source:
        fail(f"missing non-skipping anchor {path}:{anchor}")

for path, expected in fixtures.items():
    artifact = root / path
    if not artifact.is_file():
        fail(f"missing fixture {path}")
    actual = hashlib.sha256(artifact.read_bytes()).hexdigest()
    if actual != expected:
        fail(f"fixture hash drift {path}: {actual}")

plain = (root / "crates/api/src/lifecycle/private_ws/plain_venues.rs").read_text(encoding="utf-8")
start = plain.index("pub(super) fn spawn_okx_private_ws")
end = plain.index("pub(super) fn spawn_bybit_private_ws", start)
okx_runtime = plain[start:end]
for marker, scope in (
    ("tokio::spawn(run_confirmed_private_ws(", okx_runtime),
    ("OKX_PRIVATE_HANDSHAKE_COUNT: usize = 4", plain),
    ("okx_subscription_control(text)", plain),
):
    if marker not in scope:
        fail(f"OKX server-ack runtime marker missing: {marker}")

registry = (root / "crates/exchange/src/ws/trading.rs").read_text(encoding="utf-8")
for path in (
    "ws_user_account_snapshot.json",
    "ws_user_positions_snapshot.json",
    "ws_user_orders_partial_fill.json",
):
    if path not in registry:
        fail(f"private WS fixture is not registered: {path}")

browser = (root / "test/e2e/data_pipeline.spec.ts").read_text(encoding="utf-8")
if "settings credentials surface local capture-shaped trading runtime 4/4 ok gate" not in browser:
    fail("missing OKX product runtime browser anchor")

print(f"OK PR-EK static contract ({len(evidence_contract)} evidence types; {len(anchors)} anchors; {len(fixtures)} fixture hashes)")
PY

bash "$ROOT/scripts/check_exchange_evidence_debt.sh"
bash "$ROOT/scripts/check_exchange_operation_evidence_matrix.sh"
bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_evidence_index.sh"

if [[ "${PR_EK_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p exchange --lib okx_ --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p exchange --test okx_account_mode_test --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p exchange --test okx_test --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p exchange --test ws_trading_specs_test --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api --bin crypto-arb-api okx_ --no-fail-fast
  CI=1 npx playwright test test/e2e/data_pipeline.spec.ts \
    --grep "settings credentials surface local capture-shaped trading runtime 4/4 ok gate" \
    --reporter=list
fi

printf 'OK PR-EK OKX V5 order, account, private WS, finality and fixture contract\n'
