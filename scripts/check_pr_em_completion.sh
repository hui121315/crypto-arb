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
  backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-em.XXXXXX")"
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
start = source.index("pub(super) fn spawn_bybit_private_ws")
end = source.index("pub(super) fn spawn_bitget_private_ws", start)
block = source[start:end]
marker = "tokio::spawn(run_confirmed_private_ws("
if marker not in block:
    raise SystemExit("PR-EM self-test setup failed: confirmed Bybit runtime marker missing")
block = block.replace(marker, "spawn_plain_private_ws(", 1)
path.write_text(source[:start] + block + source[end:], encoding="utf-8")
PY
  if PR_EM_SKIP_TESTS=1 bash "$0" >/dev/null 2>&1; then
    printf 'PR-EM completion self-test failed: send-only Bybit private WS passed\n' >&2
    exit 1
  fi
  printf 'PR-EM completion self-test passed\n'
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
title = "PR-EM Bybit V5 Order, Account & Private WS Semantics Contract"
evidence_contract = {
    "account-summary-rest-ws-contract": (
        "crates/exchange/src/adapters/bybit_private_data_tests.rs",
        "bybit_account_summary_parses_equity_margin_rates_and_source",
    ),
    "account-state-source-freshness-problem-ui": (
        "crates/api/src/services/account_state/tests.rs",
        "bybit_unified_summary_problem_degrades_all_account_facts",
    ),
    "native-symbol-settle-rwa-contract": (
        "crates/exchange/src/adapters/bybit_instruments_tests.rs",
        "bybit_identity_matrix_preserves_usdt_usdc_and_rwa_contracts",
    ),
    "private-read-settle-fanout": (
        "crates/exchange/src/adapters/bybit_tests.rs",
        "private_account_reads_fan_out_usdt_and_usdc_settles",
    ),
    "order-compiler-position-mode": (
        "crates/exchange/src/adapters/bybit_trade_data_tests.rs",
        "hedge_position_idx_is_serialized",
    ),
    "order-identity-execution-readiness": (
        "crates/api/src/services/hedge_preview/guards.rs",
        "bybit_identity_constraints_produce_execution_ready_usdc_plan",
    ),
    "private-rest-ws-fail-closed": (
        "crates/exchange/src/adapters/bybit_private_data_tests.rs",
        "bybit_private_strict_fixture_sweep_fails_closed",
    ),
    "private-ws-server-ack-health": (
        "crates/api/src/lifecycle/private_ws/plain_venues_tests.rs",
        "bybit_private_ws_waits_for_auth_and_subscription_acknowledgements",
    ),
    "private-ws-fill-fee-finality": (
        "crates/api/src/trading_service/private_ws_events/tests/deltas/order_updates/part_04.rs",
        "bybit_private_fill_and_order_finality_project_once",
    ),
    "official-fixture-operation-registry": (
        "crates/exchange/tests/ws_trading_specs_test.rs",
        "bybit_private_stream_fixtures_are_recorded",
    ),
    "product-browser": (
        "test/e2e/pr_em_bybit_account.spec.ts",
        "npm run test:e2e:pr-em",
    ),
    "completion-governance": (
        "scripts/check_pr_em_completion.sh",
        "bash scripts/check_pr_em_completion.sh --self-test",
    ),
}

rust_anchors = (
    ("crates/exchange/src/adapters/bybit_private_data_tests.rs", "bybit_account_summary_parses_equity_margin_rates_and_source"),
    ("crates/exchange/src/adapters/bybit_ws_user_tests.rs", "parses_position_and_wallet_events_without_available_balance_guessing"),
    ("crates/api/src/services/account_state/tests.rs", "bybit_unified_summary_drives_actual_account_field_quality"),
    ("crates/api/src/services/account_state/tests.rs", "bybit_unified_summary_problem_degrades_all_account_facts"),
    ("crates/exchange/src/adapters/bybit_instruments_tests.rs", "bybit_identity_matrix_preserves_usdt_usdc_and_rwa_contracts"),
    ("crates/exchange/src/adapters/bybit_tests.rs", "private_account_reads_fan_out_usdt_and_usdc_settles"),
    ("crates/exchange/src/adapters/bybit_trade_data_tests.rs", "hedge_position_idx_is_serialized"),
    ("crates/exchange/src/adapters/bybit_trade_data_tests.rs", "market_order_sends_official_percent_slippage"),
    ("crates/api/src/services/hedge_preview/guards.rs", "bybit_identity_constraints_produce_execution_ready_usdc_plan"),
    ("crates/exchange/src/adapters/bybit_private_data_tests.rs", "bybit_private_strict_fixture_sweep_fails_closed"),
    ("crates/exchange/src/adapters/bybit_ws_user_tests.rs", "private_ws_control_fixtures_require_auth_and_subscription_ack"),
    ("crates/exchange/src/adapters/bybit_ws_user_tests.rs", "private_ws_control_rejections_preserve_auth_scope"),
    ("crates/api/src/lifecycle/private_ws/plain_venues_tests.rs", "bybit_private_ws_waits_for_auth_and_subscription_acknowledgements"),
    ("crates/api/src/lifecycle/private_ws/plain_venues_tests.rs", "bybit_auth_and_subscription_rejections_fail_closed"),
    ("crates/api/src/trading_service/private_ws_events/tests/deltas/order_updates/part_04.rs", "bybit_private_fill_and_order_finality_project_once"),
    ("crates/exchange/tests/ws_trading_specs_test.rs", "bybit_private_stream_fixtures_are_recorded"),
)

fixtures = {
    "crates/exchange/fixtures/bybit/instruments_info_linear_identity_matrix.json": "5bd6c38c7d43fd01780a8066901ff0ff548b02c25cd263f57b61713da4b6f0f3",
    "crates/exchange/fixtures/bybit/wallet_balance_unified_account_metrics.json": "39775d9331b6ba06d3ff435d78ecb62acbf544c13474e498545b19f414a9751f",
    "crates/exchange/fixtures/bybit/ws_user_auth_failure.json": "62071cd06397bb6d2df5074793c1fea69af39d1a5794be10f8e8baa773752fc0",
    "crates/exchange/fixtures/bybit/ws_user_auth_success.json": "18bf9a8215427eeecf5145aac2a4ffe8a14f898885e91034733f06515964b780",
    "crates/exchange/fixtures/bybit/ws_user_execution_fill.json": "0a74cdbfda5e62ad0fdd105592fa5493fda63bdf396a515adcd489213b1bdb39",
    "crates/exchange/fixtures/bybit/ws_user_order_canceled.json": "74c2adeec369890d4240e4ab11d6d3d61db36e10c2b8b9a7197934532a978cb5",
    "crates/exchange/fixtures/bybit/ws_user_order_filled.json": "92185c0e848f978cfe0e301a86439ea47d95984b583304c25c5897e5b9104536",
    "crates/exchange/fixtures/bybit/ws_user_position_snapshot.json": "85aa610f0572bc1de40bb72500927ea456b63d815aa0edc787e7a146a606fd56",
    "crates/exchange/fixtures/bybit/ws_user_subscribe_failure.json": "be769c65a431a0273884f6fa41bb16d9b0ec4359a750ac4212e3039d4120d148",
    "crates/exchange/fixtures/bybit/ws_user_subscribe_success.json": "ed44a05eeb558b62175837cd400bb1df3c5275f375548bd1422b7e8e24c9aeca",
    "crates/exchange/fixtures/bybit/ws_user_wallet_snapshot.json": "aa7b33dddabafac0c57e34644bf15d9a6363f8bed48fd8a2d9fec7d9af5f8799",
}

coverage_paths = (
    "scripts/check_pr_em_completion.sh",
    "scripts/check_product_audit_evidence_index.sh",
    "scripts/exchange_evidence_debt_allowlist.tsv",
    "scripts/verify_repo_gates.sh",
    "shared-types/src/orders.rs",
    "crates/exchange/src/live.rs",
    "crates/exchange/src/adapters/bybit.rs",
    "crates/exchange/src/adapters/bybit_private_data.rs",
    "crates/exchange/src/adapters/bybit_private_rest.rs",
    "crates/exchange/src/adapters/bybit_instruments.rs",
    "crates/exchange/src/adapters/bybit_ws_user.rs",
    "crates/exchange/src/ws/trading.rs",
    "crates/exchange/tests/ws_trading_specs_test.rs",
    "crates/api/src/lifecycle/private_ws/plain_venues.rs",
    "crates/api/src/services/account_state.rs",
    "crates/api/src/services/account_state/derive.rs",
    "crates/api/src/services/hedge_preview/guards.rs",
    "crates/api/src/trading_service/private_ws_events/tests/deltas/order_updates/part_04.rs",
    "frontend/src/panels/modules/settings/tabs/venue_credentials/panels/account_state.rs",
    "frontend/src/panels/modules/settings/tabs/venue_credentials/panels/account_state/rows.rs",
    "test/e2e/pr_em_bybit_account.spec.ts",
)


def fail(message: str) -> None:
    raise SystemExit(f"PR-EM completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be completed with no local remainder")
queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-EM\b", queue):
    fail("completed PR-EM remains in the local queue")
successor_row = next(
    (line for line in doc.splitlines() if line.startswith("| `PR-EN Bitget UTA V3 Order")),
    None,
)
successor_queued = re.search(r"(?m)^\d+\.\s+\*\*PR-EN\b", queue) is not None
successor_completed = successor_row is not None and "✅ 完成" in successor_row
if not successor_queued and not successor_completed:
    fail("PR-EN must remain queued or have a completed roadmap row")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-EM"]
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

browser = (root / "test/e2e/pr_em_bybit_account.spec.ts").read_text(encoding="utf-8")
browser_title = "PR-EM renders Bybit UNIFIED equity available and margin evidence"
if f'test("{browser_title}"' not in browser or f'test.skip("{browser_title}"' in browser:
    fail("missing non-skipping PR-EM browser anchor")

for path, expected in fixtures.items():
    artifact = root / path
    if not artifact.is_file():
        fail(f"missing fixture {path}")
    actual = hashlib.sha256(artifact.read_bytes()).hexdigest()
    if actual != expected:
        fail(f"fixture hash drift {path}: {actual}")

plain = (root / "crates/api/src/lifecycle/private_ws/plain_venues.rs").read_text(encoding="utf-8")
start = plain.index("pub(super) fn spawn_bybit_private_ws")
end = plain.index("pub(super) fn spawn_bitget_private_ws", start)
bybit_runtime = plain[start:end]
for marker, scope in (
    ("tokio::spawn(run_confirmed_private_ws(", bybit_runtime),
    ("BYBIT_PRIVATE_HANDSHAKE_COUNT: usize = 2", plain),
    ("bybit_subscription_control(text)", plain),
):
    if marker not in scope:
        fail(f"Bybit server-ack runtime marker missing: {marker}")

registry = (root / "crates/exchange/src/ws/trading.rs").read_text(encoding="utf-8")
for path in (
    "ws_user_wallet_snapshot.json",
    "ws_user_position_snapshot.json",
    "ws_user_execution_fill.json",
    "ws_user_order_filled.json",
):
    if path not in registry:
        fail(f"private WS fixture is not registered: {path}")

ui = (root / "frontend/src/panels/modules/settings/tabs/venue_credentials/panels/account_state/rows.rs").read_text(encoding="utf-8")
for marker in ("total_available_balance_usd", "total_initial_margin_usd", "total_maintenance_margin_usd", "account_im_rate", "account_mm_rate"):
    if marker not in ui:
        fail(f"Settings account evidence marker missing: {marker}")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
for path in coverage_paths:
    item = coverage.get(path)
    if item is None or item["coverage_status"] != "exact":
        fail(f"exact coverage missing for {path}")

print(
    f"OK PR-EM static contract ({len(evidence_contract)} evidence types; "
    f"{len(rust_anchors)} Rust anchors; {len(fixtures)} fixture hashes)"
)
PY

bash "$ROOT/scripts/check_exchange_evidence_debt.sh"
bash "$ROOT/scripts/check_exchange_operation_evidence_matrix.sh"
bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_evidence_index.sh"

if [[ "${PR_EM_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p exchange --lib bybit_ --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p exchange --lib private_account_reads_fan_out_usdt_and_usdc_settles --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p exchange --test ws_trading_specs_test bybit_private_stream_fixtures_are_recorded --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api --bin crypto-arb-api bybit_ --no-fail-fast
  CI=1 npm run test:e2e:pr-em
fi

printf 'OK PR-EM Bybit V5 order, account, private WS, finality and fixture contract\n'
