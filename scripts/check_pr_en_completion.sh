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
  if ! PR_EN_SKIP_TESTS=1 bash "$0" >/dev/null 2>&1; then
    printf 'PR-EN completion self-test setup failed: baseline gate does not pass\n' >&2
    exit 1
  fi
  backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-en.XXXXXX")"
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
start = source.index("pub(super) fn spawn_bitget_private_ws")
end = source.index("fn map_okx_text", start)
block = source[start:end]
marker = "tokio::spawn(run_confirmed_private_ws("
if marker not in block:
    raise SystemExit("PR-EN self-test setup failed: confirmed Bitget runtime marker missing")
block = block.replace(marker, "spawn_plain_private_ws(", 1)
path.write_text(source[:start] + block + source[end:], encoding="utf-8")
PY
  if PR_EN_SKIP_TESTS=1 bash "$0" >/dev/null 2>&1; then
    printf 'PR-EN completion self-test failed: send-only Bitget private WS passed\n' >&2
    exit 1
  fi
  printf 'PR-EN completion self-test passed\n'
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
title = "PR-EN Bitget UTA V3 Order, Account & Private WS Semantics Contract"
evidence_contract = {
    "native-instrument-identity-contract": (
        "crates/exchange/src/adapters/bitget_instruments_tests.rs",
        "official_identity_matrix_covers_usdt_usdc_coin_and_reality_boundaries",
    ),
    "private-read-category-fanout": (
        "crates/exchange/tests/bitget_test.rs",
        "refresh_instrument_specs_fans_out_all_uta_futures_categories",
    ),
    "order-compiler-hold-mode": (
        "crates/exchange/src/adapters/bitget_order_compiler_tests.rs",
        "hedge_mode_derives_all_open_close_pos_side_combinations",
    ),
    "order-payload-category-pos-side": (
        "crates/exchange/src/adapters/bitget_uta_trade_data_tests.rs",
        "compiled_usdc_hedge_order_preserves_native_category_symbol_and_pos_side",
    ),
    "account-summary-rest-ws-contract": (
        "crates/exchange/src/adapters/bitget_uta_private_data_pr_en_tests.rs",
        "official_account_fixture_projects_complete_uta_summary",
    ),
    "private-rest-ws-fail-closed": (
        "crates/exchange/src/adapters/bitget_uta_ws_user_tests.rs",
        "private_parser_fails_closed_on_missing_or_unknown_contract_fields",
    ),
    "private-ws-server-ack-health": (
        "crates/api/src/lifecycle/private_ws/plain_venues_tests.rs",
        "bitget_private_ws_waits_for_login_and_all_topic_acknowledgements",
    ),
    "private-ws-fill-fee-finality": (
        "crates/api/src/trading_service/private_ws_events/tests/deltas/order_updates/part_05.rs",
        "bitget_private_fill_and_order_finality_project_once",
    ),
    "official-fixture-operation-registry": (
        "crates/exchange/tests/bitget_pr_en_contract_test.rs",
        "bitget_private_operations_are_fixture_backed_and_not_ack_only",
    ),
    "completion-governance": (
        "scripts/check_pr_en_completion.sh",
        "bash scripts/check_pr_en_completion.sh --self-test",
    ),
}

rust_anchors = (
    ("crates/exchange/src/adapters/bitget_instruments_tests.rs", "official_identity_matrix_covers_usdt_usdc_coin_and_reality_boundaries"),
    ("crates/exchange/src/adapters/bitget_instruments_tests.rs", "native_identity_routes_exact_usdt_usdc_and_coin_symbols"),
    ("crates/exchange/tests/bitget_test.rs", "refresh_instrument_specs_fans_out_all_uta_futures_categories"),
    ("crates/exchange/src/adapters/bitget_order_compiler_tests.rs", "hedge_mode_derives_all_open_close_pos_side_combinations"),
    ("crates/exchange/src/adapters/bitget_order_compiler_tests.rs", "inverse_reality_and_misaligned_orders_fail_closed"),
    ("crates/exchange/src/adapters/bitget_uta_trade_data_tests.rs", "compiled_usdc_hedge_order_preserves_native_category_symbol_and_pos_side"),
    ("crates/exchange/src/adapters/bitget_uta_private_data_pr_en_tests.rs", "official_account_fixture_projects_complete_uta_summary"),
    ("crates/exchange/src/adapters/bitget_uta_private_data_pr_en_tests.rs", "account_summary_rejects_missing_or_non_finite_risk_fields"),
    ("crates/exchange/src/adapters/bitget_uta_ws_user_tests.rs", "control_fixtures_confirm_login_and_subscription_and_reject_failures"),
    ("crates/exchange/src/adapters/bitget_uta_ws_user_tests.rs", "order_and_fill_fixtures_preserve_terminal_finality_and_fees"),
    ("crates/exchange/src/adapters/bitget_uta_ws_user_tests.rs", "private_parser_fails_closed_on_missing_or_unknown_contract_fields"),
    ("crates/api/src/lifecycle/private_ws/plain_venues_tests.rs", "bitget_private_ws_waits_for_login_and_all_topic_acknowledgements"),
    ("crates/api/src/lifecycle/private_ws/plain_venues_tests.rs", "bitget_login_and_subscription_rejections_fail_closed"),
    ("crates/api/src/trading_service/private_ws_events/tests/deltas/order_updates/part_05.rs", "bitget_private_fill_and_order_finality_project_once"),
    ("crates/exchange/tests/bitget_pr_en_contract_test.rs", "bitget_private_operations_are_fixture_backed_and_not_ack_only"),
)

fixtures = {
    "crates/exchange/fixtures/bitget/uta_account_assets.json": "60f1410d2d8d70b4a1e8a603250d6f60b515bfda4bff218bc1274b25291e86c2",
    "crates/exchange/fixtures/bitget/uta_instruments_identity_matrix.json": "1d638cbce67260f8fd1509a3d2fdf0cbda9a767821f877bcfb4295c832e09783",
    "crates/exchange/fixtures/bitget/uta_ws_login_success.json": "26f44390143cc1a6b2911f4fd9524e6b9f40c150340a42efb1ce7f8915d920ea",
    "crates/exchange/fixtures/bitget/uta_ws_login_failure.json": "3c138975acb3e445482352cc2ffda17f9565e741186538aeeb888f1ad099299a",
    "crates/exchange/fixtures/bitget/uta_ws_subscribe_account_success.json": "2a7e83e9e961027802ad480ce52cfe7e857cfb3ecadc2979e302d53dd8a02eab",
    "crates/exchange/fixtures/bitget/uta_ws_subscribe_failure.json": "0942cbc3ace782fdddfe8bf329f574a948ddef22a54afc4420631cc6b658cd0e",
    "crates/exchange/fixtures/bitget/uta_ws_account_snapshot.json": "27210ec8419cefb14a8d0b69defe3a24440ee3800ab428bc223f4ad4029d6438",
    "crates/exchange/fixtures/bitget/uta_ws_position_snapshot.json": "c96446c79901534aa0e62d5d54829f6498e102de18763f0710fb93aba97b0a7e",
    "crates/exchange/fixtures/bitget/uta_ws_order_filled.json": "6f5da324c84e6c7fd9a4d7b4a69a089631ebc0f749fd607887c71a9601066e1b",
    "crates/exchange/fixtures/bitget/uta_ws_order_cancelled.json": "9fdcd4a1ddf4d5e4bf0749b073f638754219f8ad7b76a4af62b58198faf8a52b",
    "crates/exchange/fixtures/bitget/uta_ws_fill.json": "5e700644d762d9f24dc1d5b9ab9f9c8f9dcac9d03d7349e73d55db77b56e9279",
}

coverage_paths = (
    "scripts/check_pr_en_completion.sh",
    "scripts/verify_repo_gates.sh",
    "shared-types/src/instrument_registry.rs",
    "crates/exchange/src/adapters/bitget.rs",
    "crates/exchange/src/adapters/bitget_support.rs",
    "crates/exchange/src/adapters/bitget_instruments.rs",
    "crates/exchange/src/adapters/bitget_order_compiler.rs",
    "crates/exchange/src/adapters/bitget_order_compiler_tests.rs",
    "crates/exchange/src/adapters/bitget_uta_trade_data.rs",
    "crates/exchange/src/adapters/bitget_uta_private_data.rs",
    "crates/exchange/src/adapters/bitget_uta_private_data_pr_en_tests.rs",
    "crates/exchange/src/adapters/bitget_uta_ws_user.rs",
    "crates/exchange/src/adapters/bitget_uta_ws_user_data.rs",
    "crates/exchange/src/ws/trading.rs",
    "crates/exchange/tests/bitget_test.rs",
    "crates/exchange/tests/bitget_pr_en_contract_test.rs",
    "crates/api/src/lifecycle/private_ws/plain_venues.rs",
    "crates/api/src/trading_service/private_ws_mapper/bybit_bitget/bitget.rs",
    "crates/api/src/trading_service/private_ws_events/tests/deltas/order_updates/part_05.rs",
)


def fail(message: str) -> None:
    raise SystemExit(f"PR-EN completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be completed with no local remainder")
queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-EN\b", queue):
    fail("completed PR-EN remains in the local queue")
if re.search(r"(?m)^\d+\.\s+\*\*PR-(?!EN\b)[A-Z0-9-]+", queue) is None:
    fail("queue must advance to another local executable PR")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-EN"]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for kind, (artifact, command_anchor) in evidence_contract.items():
    item = indexed[kind]
    if item["artifact"] != artifact or command_anchor not in item["command"]:
        fail(f"{kind} artifact/command drifted")
    if not (root / artifact).is_file():
        fail(f"missing evidence artifact {artifact}")

for path, anchor in rust_anchors:
    source = (root / path).read_text(encoding="utf-8")
    pattern = re.compile(r"#\[(?:tokio::)?test(?:\([^]]*\))?\][\s\S]{0,500}?fn\s+" + re.escape(anchor) + r"\s*\(")
    if pattern.search(source) is None:
        fail(f"missing non-skipping Rust anchor {path}:{anchor}")

for path, expected in fixtures.items():
    artifact = root / path
    if not artifact.is_file():
        fail(f"missing fixture {path}")
    actual = hashlib.sha256(artifact.read_bytes()).hexdigest()
    if actual != expected:
        fail(f"fixture hash drift {path}: {actual}")

plain = (root / "crates/api/src/lifecycle/private_ws/plain_venues.rs").read_text(encoding="utf-8")
start = plain.index("pub(super) fn spawn_bitget_private_ws")
end = plain.index("fn map_okx_text", start)
bitget_runtime = plain[start:end]
for marker, scope in (
    ("tokio::spawn(run_confirmed_private_ws(", bitget_runtime),
    ("BITGET_PRIVATE_HANDSHAKE_COUNT: usize = 5", plain),
    ("bitget_subscription_control(text)", plain),
):
    if marker not in scope:
        fail(f"Bitget server-ack runtime marker missing: {marker}")

registry = (root / "crates/exchange/src/ws/trading.rs").read_text(encoding="utf-8")
for fixture in (
    "uta_ws_account_snapshot.json",
    "uta_ws_position_snapshot.json",
    "uta_ws_order_filled.json",
    "uta_ws_fill.json",
):
    if fixture not in registry:
        fail(f"private WS fixture is not registered: {fixture}")

instrument = (root / "crates/exchange/src/adapters/bitget_instruments.rs").read_text(encoding="utf-8")
for marker in ("BitgetInstrumentCache", "execution_supported", "UsdcFutures", "CoinFutures"):
    if marker not in instrument:
        fail(f"native instrument contract marker missing: {marker}")

compiler = (root / "crates/exchange/src/adapters/bitget_order_compiler.rs").read_text(encoding="utf-8")
for marker in ("BitgetPositionMode::Hedge", "pos_side", "reduce_only", "execution_supported"):
    if marker not in compiler:
        fail(f"order compiler marker missing: {marker}")

account = (root / "crates/exchange/src/adapters/bitget_uta_private_data.rs").read_text(encoding="utf-8")
for marker in ("account_equity", "effective_equity", "initial_margin", "maintenance_margin"):
    if marker not in account:
        fail(f"account summary marker missing: {marker}")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
for path in coverage_paths:
    item = coverage.get(path)
    if item is None or item["coverage_status"] != "exact":
        fail(f"exact coverage missing for {path}")

print(
    f"OK PR-EN static contract ({len(evidence_contract)} evidence types; "
    f"{len(rust_anchors)} Rust anchors; {len(fixtures)} fixture hashes)"
)
PY

bash "$ROOT/scripts/check_exchange_evidence_debt.sh"
bash "$ROOT/scripts/check_exchange_operation_evidence_matrix.sh"
bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_evidence_index.sh"

if [[ "${PR_EN_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p exchange --lib bitget_ --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p exchange --test bitget_test --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p exchange --test bitget_pr_en_contract_test --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p exchange --test ws_trading_specs_test --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api --bin crypto-arb-api bitget_ --no-fail-fast
fi

printf 'OK PR-EN Bitget UTA V3 order, account, private WS, finality and fixture contract\n'
