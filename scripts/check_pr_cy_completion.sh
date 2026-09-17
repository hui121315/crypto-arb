#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

python3 - "$ROOT" "$MODE" <<'PY'
from __future__ import annotations

import csv
import hashlib
import json
import re
import shutil
import sys
import tempfile
from decimal import Decimal
from pathlib import Path


PR_TITLE = "PR-CY KuCoin Official Futures Order Semantics"
VERIFY_ANCHOR = "`bash scripts/check_pr_cy_completion.sh --self-test`"
EVIDENCE = {
    "successor-pr-eo": ("scripts/check_pr_eo_completion.sh", "check_pr_eo_completion.sh"),
    "successor-pr-eb": ("scripts/check_pr_eb_completion.sh", "check_pr_eb_completion.sh"),
    "successor-pr-bw": ("scripts/check_pr_bw_completion.sh", "check_pr_bw_completion.sh"),
    "successor-pr-bz": ("scripts/check_pr_bz_completion.sh", "check_pr_bz_completion.sh"),
    "client-order-id-policy": ("crates/exchange/src/client_order_id_policy.rs", "kucoin_policy_keeps_trimmed_client_oid_without_unverified_charset"),
    "shared-order-identity-plan": ("crates/api/src/services/hedge_preview/guards/identity/kucoin.rs", "kucoin_identity_constraints_bind_native_finality_and_actual_fee_fixtures"),
    "identity-finality-fail-closed": ("crates/api/src/services/hedge_preview/guards/identity/kucoin.rs", "kucoin_identity_plan_fails_closed_without_private_finality_evidence"),
    "native-instrument-identity": ("crates/exchange/src/adapters/kucoin_instruments_tests.rs", "official_matrix_maps_usdt_usdc_and_verified_equity_contracts"),
    "position-mode-preflight": ("crates/api/src/services/hedge_preflight/tests/cases_account.rs", "account_mode_guard_records_kucoin_scope"),
    "position-side-fail-closed": ("crates/exchange/src/adapters/kucoin_private_data_tests.rs", "position_mode_side_mapping_is_fail_closed"),
    "position-drift-write-guard": ("crates/exchange/tests/kucoin_test.rs", "live_place_order_rechecks_position_change_and_submits_zero_orders"),
    "rest-payload-time-in-force": ("crates/exchange/src/adapters/kucoin_trade_data_tests.rs", "limit_ioc_and_fok_preserve_time_in_force"),
    "rest-place-ack": ("crates/exchange/src/adapters/kucoin_trade_data_tests.rs", "kucoin_place_order_ack_parses_official_fixture"),
    "non-idempotent-http-once": ("crates/exchange/src/http.rs", "execute_once_does_not_replay_server_error"),
    "ambiguous-result-query-recovery": ("crates/exchange/tests/kucoin_pr_cy_contract_test.rs", "ambiguous_place_result_recovers_by_client_oid_without_replaying_post"),
    "ambiguous-result-identity-fail-closed": ("crates/exchange/tests/kucoin_pr_cy_contract_test.rs", "ambiguous_place_result_fails_closed_on_mismatched_query_identity"),
    "deterministic-api-rejection": ("crates/exchange/src/adapters/kucoin_private_rest.rs", "ambiguous_place_result_excludes_deterministic_api_rejection"),
    "rest-cancel-target-identity": ("crates/exchange/src/adapters/kucoin_private_rest.rs", "cancel_target_prefers_exchange_order_id_then_client_oid_fallback"),
    "rest-cancel-ack-not-final": ("crates/exchange/src/adapters/kucoin_trade_data_tests.rs", "kucoin_cancel_order_ack_parses_official_fixture"),
    "rest-order-query-client-oid": ("crates/exchange/src/adapters/kucoin_trade_data_tests.rs", "get_order_path_uses_official_by_client_oid_query"),
    "rest-fill-actual-fee": ("crates/exchange/src/adapters/kucoin_private_data_tests.rs", "kucoin_fills_parse_official_fixture_without_defaulting_fee"),
    "private-rest-strict-parser": ("crates/exchange/src/adapters/kucoin_private_data_tests.rs", "parse_open_order_rejects_unknown_status_instead_of_pending"),
    "classic-order-stream-finality": ("crates/exchange/src/adapters/kucoin_ws_user_tests.rs", "official_terminal_fixtures_require_terminal_event_type"),
    "classic-match-fill-identity": ("crates/exchange/src/adapters/kucoin_ws_user_tests.rs", "duplicate_match_fixture_keeps_stable_fill_identity"),
    "private-order-ledger-finality": ("crates/api/src/lifecycle/private_ws/tests/kucoin_finality.rs", "kucoin_terminal_fill_projects_execution_run_once_after_durable_ack"),
    "private-ws-runtime-contract": ("crates/api/src/lifecycle/private_ws/tests.rs", "kucoin_private_subscribe_messages_include_positions"),
    "pro-ws-beta-gate": ("crates/exchange/tests/ws_trading_specs_test.rs", "kucoin_pro_ws_beta_stays_unavailable_without_runtime_evidence"),
    "official-rest-operation-registry": ("crates/exchange/src/venue_spec.rs", "kucoin_pr_eo_private_evidence_is_recorded"),
    "official-ws-operation-registry": ("crates/exchange/tests/ws_trading_specs_test.rs", "kucoin_classic_fill_stream_records_identity_evidence_without_invented_fee"),
    "product-browser": ("test/e2e/pr_eo_kucoin_pro_ws.spec.ts", "test:e2e:pr-eo"),
    "external-live-boundary": ("docs/PRODUCT_FULL_AUDIT_REFINEMENT.md", "check_product_audit_progress.sh"),
    "completion-governance": ("scripts/check_pr_cy_completion.sh", "check_pr_cy_completion.sh --self-test"),
}

RUNNABLE_ANCHORS = (
    ("crates/exchange/src/client_order_id_policy.rs", "kucoin_policy_keeps_trimmed_client_oid_without_unverified_charset"),
    ("crates/api/src/services/hedge_preview/guards/identity/kucoin.rs", "kucoin_identity_constraints_bind_native_finality_and_actual_fee_fixtures"),
    ("crates/api/src/services/hedge_preview/guards/identity/kucoin.rs", "kucoin_identity_plan_fails_closed_without_private_finality_evidence"),
    ("crates/exchange/src/adapters/kucoin_instruments_tests.rs", "official_matrix_maps_usdt_usdc_and_verified_equity_contracts"),
    ("crates/api/src/services/hedge_preflight/tests/cases_account.rs", "account_mode_guard_records_kucoin_scope"),
    ("crates/exchange/src/adapters/kucoin_private_data_tests.rs", "position_mode_side_mapping_is_fail_closed"),
    ("crates/exchange/tests/kucoin_test.rs", "live_place_order_rechecks_position_change_and_submits_zero_orders"),
    ("crates/exchange/src/adapters/kucoin_trade_data_tests.rs", "limit_ioc_and_fok_preserve_time_in_force"),
    ("crates/exchange/src/adapters/kucoin_trade_data_tests.rs", "kucoin_place_order_ack_parses_official_fixture"),
    ("crates/exchange/src/http.rs", "execute_once_does_not_replay_server_error"),
    ("crates/exchange/tests/kucoin_pr_cy_contract_test.rs", "ambiguous_place_result_recovers_by_client_oid_without_replaying_post"),
    ("crates/exchange/tests/kucoin_pr_cy_contract_test.rs", "ambiguous_place_result_fails_closed_on_mismatched_query_identity"),
    ("crates/exchange/src/adapters/kucoin_private_rest.rs", "ambiguous_place_result_excludes_deterministic_api_rejection"),
    ("crates/exchange/src/adapters/kucoin_private_rest.rs", "cancel_target_prefers_exchange_order_id_then_client_oid_fallback"),
    ("crates/exchange/src/adapters/kucoin_trade_data_tests.rs", "kucoin_cancel_order_ack_parses_official_fixture"),
    ("crates/exchange/src/adapters/kucoin_trade_data_tests.rs", "get_order_path_uses_official_by_client_oid_query"),
    ("crates/exchange/src/adapters/kucoin_private_data_tests.rs", "kucoin_fills_parse_official_fixture_without_defaulting_fee"),
    ("crates/exchange/src/adapters/kucoin_private_data_tests.rs", "parse_open_order_rejects_unknown_status_instead_of_pending"),
    ("crates/exchange/src/adapters/kucoin_ws_user_tests.rs", "official_terminal_fixtures_require_terminal_event_type"),
    ("crates/exchange/src/adapters/kucoin_ws_user_tests.rs", "duplicate_match_fixture_keeps_stable_fill_identity"),
    ("crates/api/src/lifecycle/private_ws/tests/kucoin_finality.rs", "kucoin_terminal_fill_projects_execution_run_once_after_durable_ack"),
    ("crates/api/src/lifecycle/private_ws/tests.rs", "kucoin_private_subscribe_messages_include_positions"),
    ("crates/exchange/tests/ws_trading_specs_test.rs", "kucoin_pro_ws_beta_stays_unavailable_without_runtime_evidence"),
    ("crates/exchange/src/venue_spec.rs", "kucoin_pr_eo_private_evidence_is_recorded"),
    ("crates/exchange/tests/ws_trading_specs_test.rs", "kucoin_classic_fill_stream_records_identity_evidence_without_invented_fee"),
)

BROWSER_TITLES = (
    "PR-EO renders verified KuCoin native multiplier identity and Classic finality",
    "PR-EO exposes missing KuCoin live credential evidence and blocks submit",
    "PR-EO KuCoin Pro WS production schema remains runtime-gated",
)

FIXTURE_HASHES = {
    "crates/exchange/fixtures/kucoin/contracts_active_native_matrix.json": "612f1509f278052c31888787baae9f3556086bff1280ee430f8e992aff641d7c",
    "crates/exchange/fixtures/kucoin/place_order_ack.json": "28326340f23fda8ba48e0ab15bbdd23f32bfe5e4467b525e923bdeb3b3cdf090",
    "crates/exchange/fixtures/kucoin/cancel_order_by_client_oid_ack.json": "b239aa651f0d267fe9ee6de16ec0f77200763a86fb8a157bfc31351a9de7db5e",
    "crates/exchange/fixtures/kucoin/get_order_by_client_oid_open.json": "43f0847e8fa858abc15901801e2a811376d18f74b61c9dd4794cf378bd3ecf1a",
    "crates/exchange/fixtures/kucoin/fills_by_order_id.json": "0d9814f361e3636aaa92f17c525dbc75081a65f952cf8e888b9e45d8dd7ce985",
    "crates/exchange/fixtures/kucoin/futures_actual_fee_xbtusdtm.json": "118317e838e199c08f627b38c4ba95a1870ef5807379d5c4b21afef2f856047d",
    "crates/exchange/fixtures/kucoin/classic_ws_trade_orders_match.json": "25e4efb4849b0eb4ffbaab5b29391f6ec9a1982e5ba01abd270210210fc26980",
    "crates/exchange/fixtures/kucoin/classic_ws_trade_orders_filled.json": "958d5674e0b2ea9d2f87effbe1293b6d9ca3d6d41860d2f5c66877f2bc2cf3df",
    "crates/exchange/fixtures/kucoin/classic_ws_trade_orders_canceled.json": "69469e77cd9f7ed3d6b4ab24b13e8a2522b11018fa9cfa4aac89e47239d55810",
    "crates/exchange/fixtures/kucoin/wsapi_pro_order_ack.json": "55fce65cecb72815dfb731f151d3adbf50ab576414ddbc231f6f125c0558ea9d",
    "crates/exchange/fixtures/kucoin/wsapi_pro_cancel_ack.json": "7088a84487eb2b905a93360bad0d7ddcbff869c443f2c73c0409224bc955d53b",
}


def fail(message: str) -> None:
    raise ValueError(message)


def table_rows(path: Path) -> list[dict[str, str]]:
    with path.open(encoding="utf-8", newline="") as handle:
        return list(csv.DictReader(handle, delimiter="\t"))


def check(root: Path) -> None:
    doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
    row = next((line for line in doc.splitlines() if line.startswith(f"| `{PR_TITLE}`")), None)
    if row is None or "✅ 完成" not in row or "剩余：无。" not in row or row.count(VERIFY_ANCHOR) != 1:
        fail("roadmap row must be complete, remaining-none and bound to the destructive gate")
    queue = doc[doc.index("### 🟡 6.5"):]
    if re.search(r"(?m)^\d+\.\s+\*\*PR-CY KuCoin Official Futures Order Semantics\*\*", queue):
        fail("completed PR-CY remains in the local queue")
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
    if "PR-CY 本地" not in queue or "真实 KuCoin credential" not in queue:
        fail("external live-capture boundary is missing")

    history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(encoding="utf-8")
    if "## 2026-07-16 PR-CY KuCoin Futures Order Semantics Closure" not in history:
        fail("history closure appendix is missing")

    selected = [row for row in table_rows(root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv") if row["pr_id"] == "PR-CY"]
    indexed = {row["evidence_type"]: row for row in selected}
    if len(indexed) != len(selected) or set(indexed) != set(EVIDENCE):
        fail(f"evidence type drift: expected={sorted(EVIDENCE)}, actual={sorted(indexed)}")
    for evidence_type, (artifact, command_anchor) in EVIDENCE.items():
        evidence = indexed[evidence_type]
        if evidence["artifact"] != artifact or command_anchor not in evidence["command"]:
            fail(f"evidence anchor drifted: {evidence_type}")
        if not (root / artifact).is_file() or not evidence["notes"].strip():
            fail(f"evidence artifact or note is missing: {artifact}")
    required_notes = {
        "identity-finality-fail-closed": "identity_fail_closed=true",
        "ambiguous-result-query-recovery": "single_post=true",
        "rest-cancel-ack-not-final": "ack_not_final=true",
        "pro-ws-beta-gate": "pro_ws_beta=true",
        "external-live-boundary": "live_capture_external=true",
    }
    for evidence_type, marker in required_notes.items():
        if marker not in indexed[evidence_type]["notes"]:
            fail(f"evidence note marker is missing: {evidence_type}:{marker}")

    coverage = {row["file"]: row for row in table_rows(root / "docs/PRODUCT_AUDIT_COVERAGE.tsv")}
    for artifact, _ in EVIDENCE.values():
        if Path(artifact).suffix in {".json", ".md"}:
            continue
        item = coverage.get(artifact)
        if item is None or item["coverage_status"] != "exact" or not item["evidence"].startswith("line:"):
            fail(f"exact coverage missing for {artifact}")

    invalid = re.compile(r"#\[\s*(?:ignore|should_panic)|#\[\s*cfg_attr[^\]]*ignore")
    for relative, test_name in RUNNABLE_ANCHORS:
        source = (root / relative).read_text(encoding="utf-8")
        match = re.search(rf"#\[(?:tokio::)?test\][\s\S]{{0,180}}?(?:async\s+)?fn\s+{re.escape(test_name)}\b", source)
        if match is None or invalid.search(source[max(0, match.start() - 160):match.end()]):
            fail(f"runnable anchor missing, skipped or panic-expected: {relative}:{test_name}")

    identity = (root / "crates/api/src/services/hedge_preview/guards/identity/kucoin.rs").read_text(encoding="utf-8")
    for marker in (
        '"kucoin-futures-native-contract-identity"',
        '"kucoin-classic-private-order-stream"',
        '"kucoin-classic-private-order-finality"',
        '"kucoin-signed-rest-actual-fill-fee"',
        '"kucoin/classic_ws_trade_orders_filled.json"',
        '"kucoin/fills_by_order_id.json"',
    ):
        if marker not in identity:
            fail(f"shared identity marker missing: {marker}")

    private_rest = (root / "crates/exchange/src/adapters/kucoin_private_rest.rs").read_text(encoding="utf-8")
    write_match = re.search(r"async fn signed_write_json[\s\S]*?^}", private_rest, re.MULTILINE)
    if write_match is None or ".execute_once(" not in write_match.group(0) or ".execute_with_retry(" in write_match.group(0):
        fail("KuCoin signed writes must use the single-attempt HTTP path")
    adapter = (root / "crates/exchange/src/adapters/kucoin.rs").read_text(encoding="utf-8")
    for marker in ("is_ambiguous_place_result", "LiveTradingAdapter::get_order", "ack_from_order_query", "ensure_order_client_oid"):
        if marker not in adapter:
            fail(f"ambiguous-result recovery marker missing: {marker}")
    contract = (root / "crates/exchange/tests/kucoin_pr_cy_contract_test.rs").read_text(encoding="utf-8")
    if contract.count("assert_request_counts(&server, 1, 1)") != 2:
        fail("wire contract must prove one POST and one recovery query on both branches")

    for relative, expected in FIXTURE_HASHES.items():
        if hashlib.sha256((root / relative).read_bytes()).hexdigest() != expected:
            fail(f"official fixture hash drifted: {relative}")
    fills = json.loads((root / "crates/exchange/fixtures/kucoin/fills_by_order_id.json").read_text(encoding="utf-8"))
    rows = (fills.get("data") or {}).get("items") or []
    if not rows or Decimal(str(rows[0].get("fee", "0"))) <= 0 or not rows[0].get("feeCurrency"):
        fail("official fill fixture must preserve positive actual fee truth")
    terminal = json.loads((root / "crates/exchange/fixtures/kucoin/classic_ws_trade_orders_filled.json").read_text(encoding="utf-8"))
    data = terminal.get("data") or {}
    if data.get("type") != "filled" or data.get("status") != "done" or Decimal(str(data.get("filledSize", "0"))) <= 0:
        fail("Classic private fixture must preserve terminal filled truth")

    browser = (root / "test/e2e/pr_eo_kucoin_pro_ws.spec.ts").read_text(encoding="utf-8")
    for title in BROWSER_TITLES:
        escaped = re.escape(title)
        if re.search(rf'test\.(?:skip|fixme)\(\s*["\']{escaped}["\']', browser):
            fail(f"browser anchor is skipped: {title}")
        if not re.search(rf'test\(\s*["\']{escaped}["\']', browser):
            fail(f"browser anchor is missing: {title}")

    repo_gate = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
    if repo_gate.count("check_pr_cy_completion.sh") != 2:
        fail("repo gate must execute PR-CY once in docs and all scopes")


def assert_rejected(root: Path, relative: str, transform, label: str) -> None:
    path = root / relative
    baseline = path.read_text(encoding="utf-8")
    changed = transform(baseline)
    if changed == baseline:
        fail(f"self-test setup drifted: {label}")
    path.write_text(changed, encoding="utf-8")
    try:
        check(root)
    except ValueError:
        pass
    else:
        fail(f"self-test accepted {label}")
    finally:
        path.write_text(baseline, encoding="utf-8")


def self_test(source_root: Path) -> None:
    paths = {
        "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md",
        "docs/PRODUCT_AUDIT_EVIDENCE.tsv",
        "docs/PRODUCT_AUDIT_COVERAGE.tsv",
        "docs/audit_history/PRODUCT_AUDIT_HISTORY.md",
        "crates/api/src/services/hedge_preview/guards/identity/kucoin.rs",
        "crates/exchange/src/adapters/kucoin.rs",
        "crates/exchange/src/adapters/kucoin_private_rest.rs",
        "crates/exchange/src/http.rs",
        "crates/exchange/tests/kucoin_pr_cy_contract_test.rs",
        "crates/exchange/fixtures/kucoin/fills_by_order_id.json",
        "crates/exchange/fixtures/kucoin/classic_ws_trade_orders_filled.json",
        "test/e2e/pr_eo_kucoin_pro_ws.spec.ts",
        "scripts/verify_repo_gates.sh",
    }
    paths.update(artifact for artifact, _ in EVIDENCE.values())
    paths.update(relative for relative, _ in RUNNABLE_ANCHORS)
    paths.update(FIXTURE_HASHES)
    with tempfile.TemporaryDirectory(prefix="crossline-pr-cy-completion-") as temp:
        root = Path(temp) / "repo"
        for relative in paths:
            target = root / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source_root / relative, target)
        check(root)
        assert_rejected(
            root,
            "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md",
            lambda text: text.replace(f"| `{PR_TITLE}` | ✅ 完成 |", f"| `{PR_TITLE}` | 🟡 部分完成 |", 1),
            "a downgraded roadmap row",
        )
        assert_rejected(
            root,
            "docs/PRODUCT_AUDIT_EVIDENCE.tsv",
            lambda text: "\n".join(line for line in text.splitlines() if not line.startswith("PR-CY\tambiguous-result-query-recovery\t")) + "\n",
            "an incomplete evidence matrix",
        )
        assert_rejected(
            root,
            "crates/api/src/services/hedge_preview/guards/identity/kucoin.rs",
            lambda text: text.replace('"kucoin-classic-private-order-finality"', '"removed-kucoin-finality"', 1),
            "detached shared finality evidence",
        )
        assert_rejected(
            root,
            "crates/exchange/src/adapters/kucoin_private_rest.rs",
            lambda text: text.replace(".execute_once(|| signed_builder", ".execute_with_retry(|| signed_builder", 1),
            "a replay-capable signed write path",
        )
        assert_rejected(
            root,
            "crates/exchange/tests/kucoin_pr_cy_contract_test.rs",
            lambda text: text.replace("#[tokio::test]\nasync fn ambiguous_place_result_recovers", "#[tokio::test]\n#[ignore]\nasync fn ambiguous_place_result_recovers", 1),
            "a skipped ambiguous-result recovery contract",
        )
        assert_rejected(
            root,
            "crates/exchange/fixtures/kucoin/fills_by_order_id.json",
            lambda text: text.replace('"fee": "0.05176506"', '"fee": "0"', 1),
            "a zeroed actual-fee fixture",
        )
        assert_rejected(
            root,
            "test/e2e/pr_eo_kucoin_pro_ws.spec.ts",
            lambda text: text.replace('test("PR-EO KuCoin Pro WS production schema', 'test.skip("PR-EO KuCoin Pro WS production schema', 1),
            "a skipped product fixture",
        )
        assert_rejected(
            root,
            "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md",
            lambda text: text.replace("### 🟡 6.5 下一步执行队列", "### 🟡 6.5 下一步执行队列\n\n1. **PR-CY KuCoin Official Futures Order Semantics** — stale", 1),
            "completed PR-CY returned to the queue",
        )
        assert_rejected(
            root,
            "docs/PRODUCT_AUDIT_COVERAGE.tsv",
            lambda text: "\n".join(line for line in text.splitlines() if not line.startswith("crates/exchange/tests/kucoin_pr_cy_contract_test.rs\t")) + "\n",
            "missing KuCoin recovery coverage",
        )
        assert_rejected(
            root,
            "scripts/verify_repo_gates.sh",
            lambda text: text.replace("check_pr_cy_completion.sh", "removed_pr_cy_completion.sh", 1),
            "single-scope repo wiring",
        )
    print("PR-CY completion destructive self-test passed")


try:
    root = Path(sys.argv[1])
    if sys.argv[2] == "--self-test":
        self_test(root)
    else:
        check(root)
        print(
            f"OK PR-CY static contract ({len(EVIDENCE)} evidence types; "
            f"{len(RUNNABLE_ANCHORS)} runnable anchors; {len(BROWSER_TITLES)} browser anchors)"
        )
except (OSError, KeyError, ValueError, json.JSONDecodeError) as exc:
    raise SystemExit(f"PR-CY completion gate failed: {exc}") from exc
PY

if [[ "$MODE" == "--self-test" ]]; then
  exit 0
fi

if [[ "${PR_CY_SKIP_BROWSER_LIST:-0}" != "1" ]]; then
  npx playwright test "$ROOT/test/e2e/pr_eo_kucoin_pro_ws.spec.ts" --list >/dev/null
fi

if [[ "${PR_CY_SKIP_UPSTREAM:-0}" != "1" ]]; then
  bash "$ROOT/scripts/check_pr_eo_completion.sh"
  PR_EB_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_eb_completion.sh"
  PR_BW_SKIP_TESTS=1 PR_BW_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_bw_completion.sh"
  PR_BZ_SKIP_TESTS=1 PR_BZ_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_bz_completion.sh"
  bash "$ROOT/scripts/check_product_audit_progress.sh"
  bash "$ROOT/scripts/check_product_audit_evidence.sh"
  bash "$ROOT/scripts/check_product_audit_evidence_index.sh"
  bash "$ROOT/scripts/check_product_audit_coverage.sh"
fi

if [[ "${PR_CY_SKIP_TESTS:-0}" != "1" ]]; then
  jobs="${CARGO_BUILD_JOBS:-10}"
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --lib kucoin_ --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --lib execute_once_does_not_replay_server_error --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --test kucoin_test --no-fail-fast -- --test-threads=1
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --test kucoin_pr_cy_contract_test --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --test ws_trading_specs_test kucoin_ --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api kucoin_ --no-fail-fast
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-eo -- --workers=1
fi

printf 'PR-CY KuCoin Futures order semantics completion passed\n'
