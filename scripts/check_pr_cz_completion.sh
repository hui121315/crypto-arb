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


PR_TITLE = "PR-CZ HTX Official USDT-M Swap Order Semantics"
VERIFY_ANCHOR = "`bash scripts/check_pr_cz_completion.sh --self-test`"
EVIDENCE = {
    "successor-pr-ep": ("scripts/check_pr_ep_completion.sh", "check_pr_ep_completion.sh"),
    "successor-pr-eb": ("scripts/check_pr_eb_completion.sh", "check_pr_eb_completion.sh"),
    "successor-pr-bw": ("scripts/check_pr_bw_completion.sh", "check_pr_bw_completion.sh"),
    "successor-pr-bz": ("scripts/check_pr_bz_completion.sh", "check_pr_bz_completion.sh"),
    "successor-pr-eg": ("scripts/check_pr_eg_completion.sh", "check_pr_eg_completion.sh"),
    "successor-pr-ff": ("scripts/check_pr_ff_completion.sh", "check_pr_ff_completion.sh"),
    "client-order-id-policy": ("crates/exchange/src/client_order_id_policy.rs", "htx_policy_derives_positive_numeric_client_order_id"),
    "shared-order-identity-plan": ("crates/api/src/services/hedge_preview/guards/identity/htx.rs", "htx_identity_constraints_bind_native_finality_and_reported_fee_fixtures"),
    "identity-finality-fail-closed": ("crates/api/src/services/hedge_preview/guards/identity/htx.rs", "htx_identity_plan_fails_closed_without_private_finality_evidence"),
    "native-contract-identity": ("crates/exchange/src/adapters/htx_contracts_tests.rs", "htx_swap_contract_info_parses_official_fixture_metadata"),
    "account-type-preflight": ("crates/exchange/tests/htx_test.rs", "live_place_order_blocks_unified_account_after_contract_resolution"),
    "api-order-status-preflight": ("crates/exchange/tests/htx_test.rs", "live_place_order_blocks_disabled_order_price_type_after_contract_and_account_checks"),
    "position-leverage-preflight": ("crates/exchange/tests/htx_compiler_test.rs", "place_uses_verified_native_contract_size_and_position_mode"),
    "market-like-order-styles": ("crates/exchange/src/adapters/htx_trade_data_tests.rs", "market_like_styles_cover_official_bbo_and_optimal_families"),
    "single-dual-offset-semantics": ("crates/exchange/src/adapters/htx_trade_data_tests.rs", "single_side_compiles_both_offset_and_reduce_only"),
    "generic-market-fail-closed": ("crates/exchange/tests/htx_compiler_test.rs", "generic_market_is_blocked_before_any_network_request"),
    "rest-place-ack": ("crates/exchange/src/adapters/htx_trade_data_tests.rs", "htx_place_order_ack_parses_official_fixture"),
    "non-idempotent-http-once": ("crates/exchange/src/http.rs", "execute_once_does_not_replay_server_error"),
    "signed-write-single-attempt": ("crates/exchange/src/adapters/htx_private_rest.rs", "signed_write_post_object"),
    "ambiguous-result-query-recovery": ("crates/exchange/tests/htx_compiler_test.rs", "ambiguous_place_result_recovers_by_client_order_id_without_replaying_post"),
    "ambiguous-result-identity-fail-closed": ("crates/exchange/tests/htx_compiler_test.rs", "ambiguous_place_result_fails_closed_on_mismatched_query_identity"),
    "query-ack-fill-fee": ("crates/exchange/src/adapters/htx_trade_data.rs", "ack_from_order_query"),
    "deterministic-api-rejection": ("crates/exchange/src/adapters/htx_private_rest.rs", "ambiguous_place_result_excludes_deterministic_api_rejection"),
    "cancel-single-attempt": ("crates/exchange/tests/htx_compiler_test.rs", "cancel_write_server_error_is_not_replayed"),
    "client-id-query-strict": ("crates/exchange/tests/htx_compiler_test.rs", "client_order_lookup_cold_cache_uses_resolved_native_symbol"),
    "strict-order-parser": ("crates/exchange/src/adapters/htx_private_data_tests.rs", "htx_cross_order_info_parses_official_fixture"),
    "cancel-ack-not-final": ("crates/exchange/src/adapters/htx_trade_data_tests.rs", "htx_cancel_order_ack_parses_official_fixture"),
    "notification-ws-fill-fee": ("crates/api/src/trading_service/private_ws_mapper/tests/cases_htx.rs", "htx_settled_order_maps_stable_fill_identity_and_reported_fee"),
    "private-ws-durable-finality": ("crates/api/src/lifecycle/private_ws/tests/htx_finality.rs", "htx_terminal_fill_projects_execution_run_once_after_durable_ack"),
    "trade-ws-runtime-fail-closed": ("crates/exchange/tests/ws_trading_specs_test.rs", "htx_schema_or_announcement_cannot_enable_live_submit_without_authenticated_runtime"),
    "official-operation-registry": ("crates/exchange/src/venue_spec.rs", "htx_pr_ep_private_trade_rest_registry_contains_recorded_core"),
    "settings-operation-health": ("crates/api/src/services/venue_operation_health/snapshot/tests/part_01.rs", "htx_order_permission_evidence_still_keeps_unknown_status"),
    "product-browser": ("test/e2e/pr_ep_htx_ws_gate.spec.ts", "test:e2e:pr-ep"),
    "external-live-boundary": ("docs/PRODUCT_FULL_AUDIT_REFINEMENT.md", "check_product_audit_progress.sh"),
    "completion-governance": ("scripts/check_pr_cz_completion.sh", "check_pr_cz_completion.sh --self-test"),
}

RUNNABLE_ANCHORS = (
    ("crates/exchange/src/client_order_id_policy.rs", "htx_policy_derives_positive_numeric_client_order_id"),
    ("crates/api/src/services/hedge_preview/guards/identity/htx.rs", "htx_identity_constraints_bind_native_finality_and_reported_fee_fixtures"),
    ("crates/api/src/services/hedge_preview/guards/identity/htx.rs", "htx_identity_plan_fails_closed_without_private_finality_evidence"),
    ("crates/exchange/src/adapters/htx_contracts_tests.rs", "htx_swap_contract_info_parses_official_fixture_metadata"),
    ("crates/exchange/tests/htx_test.rs", "live_place_order_blocks_unified_account_after_contract_resolution"),
    ("crates/exchange/tests/htx_test.rs", "live_place_order_blocks_disabled_order_price_type_after_contract_and_account_checks"),
    ("crates/exchange/tests/htx_compiler_test.rs", "place_uses_verified_native_contract_size_and_position_mode"),
    ("crates/exchange/src/adapters/htx_trade_data_tests.rs", "market_like_styles_cover_official_bbo_and_optimal_families"),
    ("crates/exchange/src/adapters/htx_trade_data_tests.rs", "single_side_compiles_both_offset_and_reduce_only"),
    ("crates/exchange/tests/htx_compiler_test.rs", "generic_market_is_blocked_before_any_network_request"),
    ("crates/exchange/src/adapters/htx_trade_data_tests.rs", "htx_place_order_ack_parses_official_fixture"),
    ("crates/exchange/src/http.rs", "execute_once_does_not_replay_server_error"),
    ("crates/exchange/tests/htx_compiler_test.rs", "ambiguous_place_result_recovers_by_client_order_id_without_replaying_post"),
    ("crates/exchange/tests/htx_compiler_test.rs", "ambiguous_place_result_fails_closed_on_mismatched_query_identity"),
    ("crates/exchange/src/adapters/htx_private_rest.rs", "ambiguous_place_result_excludes_deterministic_api_rejection"),
    ("crates/exchange/tests/htx_compiler_test.rs", "cancel_write_server_error_is_not_replayed"),
    ("crates/exchange/tests/htx_compiler_test.rs", "client_order_lookup_cold_cache_uses_resolved_native_symbol"),
    ("crates/exchange/src/adapters/htx_private_data_tests.rs", "htx_cross_order_info_parses_official_fixture"),
    ("crates/exchange/src/adapters/htx_trade_data_tests.rs", "htx_cancel_order_ack_parses_official_fixture"),
    ("crates/api/src/trading_service/private_ws_mapper/tests/cases_htx.rs", "htx_settled_order_maps_stable_fill_identity_and_reported_fee"),
    ("crates/api/src/lifecycle/private_ws/tests/htx_finality.rs", "htx_terminal_fill_projects_execution_run_once_after_durable_ack"),
    ("crates/exchange/tests/ws_trading_specs_test.rs", "htx_schema_or_announcement_cannot_enable_live_submit_without_authenticated_runtime"),
    ("crates/exchange/src/venue_spec.rs", "htx_pr_ep_private_trade_rest_registry_contains_recorded_core"),
    ("crates/api/src/services/venue_operation_health/snapshot/tests/part_01.rs", "htx_order_permission_evidence_still_keeps_unknown_status"),
)

BROWSER_TITLES = (
    "PR-EP keeps HTX notification WS distinct from the absent trade WS",
    "PR-EP keeps HTX schema and announcement evidence visible but blocks submit",
    "PR-EP notification runtime health cannot satisfy HTX trade writer readiness",
)

FIXTURE_HASHES = {
    "crates/exchange/fixtures/htx/swap_contract_info_execution_matrix.json": "98cb96e77663435b3274c25ebcd8311024b257b563d963ead5be9f806c5c6f47",
    "crates/exchange/fixtures/htx/swap_cross_order_ack.json": "43a7aaaf25cbd963330d465138dc5dfcbf768ace19db0da9c005ee9d3ef876fd",
    "crates/exchange/fixtures/htx/swap_cross_cancel_ack.json": "f270177dcbe17e7bc67402d56ee16bce0051d559be519c34907b9e1bedc76b25",
    "crates/exchange/fixtures/htx/swap_cross_order_info_filled.json": "c639d3b390988fc6682e92217dcddd3a81795c65f30405969bba2984d4f3f58a",
    "crates/exchange/fixtures/htx/ws_orders_cross_filled.json": "565c32b583056349283bce2bccef4bf27d89f37ca363c3de8bcc9ff70eb4a40c",
    "crates/exchange/fixtures/htx/ws_match_orders_cross_filled.json": "a9ba6b769e04b3752508afb5dc265150c7158ddaf1aaaef871da09bac2c51dae",
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
    if re.search(r"(?m)^\d+\.\s+\*\*PR-CZ HTX", queue):
        fail("completed PR-CZ remains in the local queue")
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
    if "PR-CZ 本地" not in queue or "真实 HTX" not in queue:
        fail("HTX external live-capture boundary is missing")

    history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(encoding="utf-8")
    if "## 2026-07-16 PR-CZ HTX USDT-M Swap Order Semantics Closure" not in history:
        fail("history closure appendix is missing")

    selected = [item for item in table_rows(root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv") if item["pr_id"] == "PR-CZ"]
    indexed = {item["evidence_type"]: item for item in selected}
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
        "signed-write-single-attempt": "single_write=true",
        "ambiguous-result-query-recovery": "single_post=true",
        "cancel-ack-not-final": "ack_not_final=true",
        "external-live-boundary": "live_capture_external=true",
    }
    for evidence_type, marker in required_notes.items():
        if marker not in indexed[evidence_type]["notes"]:
            fail(f"evidence note marker is missing: {evidence_type}:{marker}")

    coverage = {item["file"]: item for item in table_rows(root / "docs/PRODUCT_AUDIT_COVERAGE.tsv")}
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

    identity = (root / "crates/api/src/services/hedge_preview/guards/identity/htx.rs").read_text(encoding="utf-8")
    for marker in (
        '"htx-usdt-swap-native-contract-identity"',
        '"htx-usdt-swap-orders-cross-stream"',
        '"htx-usdt-swap-durable-order-finality"',
        '"htx-usdt-swap-settled-fill-fee"',
        '"htx/ws_orders_cross_filled.json"',
    ):
        if marker not in identity:
            fail(f"shared identity marker missing: {marker}")

    private_rest = (root / "crates/exchange/src/adapters/htx_private_rest.rs").read_text(encoding="utf-8")
    write_match = re.search(r"async fn signed_write_post_object[\s\S]*?^}", private_rest, re.MULTILINE)
    once_match = re.search(r"async fn post_json_once[\s\S]*?^}", private_rest, re.MULTILINE)
    if (
        write_match is None
        or "post_json_once(" not in write_match.group(0)
        or "post_json(" in write_match.group(0)
        or once_match is None
        or ".execute_once(" not in once_match.group(0)
        or ".execute_with_retry(" in once_match.group(0)
    ):
        fail("HTX signed writes must use the single-attempt HTTP path")
    if private_rest.count("signed_write_post_object::<") != 2:
        fail("HTX place and cancel must both use the single-attempt write helper")

    adapter = (root / "crates/exchange/src/adapters/htx.rs").read_text(encoding="utf-8")
    for marker in ("is_ambiguous_place_result", "LiveTradingAdapter::get_order", "ack_from_order_query", "ensure_order_client_id"):
        if marker not in adapter:
            fail(f"ambiguous-result recovery marker missing: {marker}")
    contract = (root / "crates/exchange/tests/htx_compiler_test.rs").read_text(encoding="utf-8")
    if contract.count("assert_place_query_counts(&server, 1, 1)") != 2:
        fail("wire contract must prove one POST and one recovery query on both branches")

    for relative, expected in FIXTURE_HASHES.items():
        if hashlib.sha256((root / relative).read_bytes()).hexdigest() != expected:
            fail(f"official fixture hash drifted: {relative}")
    terminal = json.loads((root / "crates/exchange/fixtures/htx/ws_orders_cross_filled.json").read_text(encoding="utf-8"))
    if terminal.get("status") != 6 or Decimal(str(terminal.get("fee", "0"))) == 0 or not terminal.get("trade"):
        fail("HTX private order fixture must preserve terminal fill and reported fee truth")

    browser = (root / "test/e2e/pr_ep_htx_ws_gate.spec.ts").read_text(encoding="utf-8")
    for title in BROWSER_TITLES:
        escaped = re.escape(title)
        if re.search(rf'test\.(?:skip|fixme)\(\s*["\']{escaped}["\']', browser):
            fail(f"browser anchor is skipped: {title}")
        if not re.search(rf'test\(\s*["\']{escaped}["\']', browser):
            fail(f"browser anchor is missing: {title}")

    repo_gate = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
    if repo_gate.count("check_pr_cz_completion.sh") != 2:
        fail("repo gate must execute PR-CZ once in docs and all scopes")


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
        "crates/api/src/services/hedge_preview/guards/identity/htx.rs",
        "crates/exchange/src/adapters/htx.rs",
        "crates/exchange/src/adapters/htx_private_rest.rs",
        "crates/exchange/src/adapters/htx_trade_data.rs",
        "crates/exchange/tests/htx_compiler_test.rs",
        "crates/exchange/fixtures/htx/ws_orders_cross_filled.json",
        "test/e2e/pr_ep_htx_ws_gate.spec.ts",
        "scripts/verify_repo_gates.sh",
    }
    paths.update(artifact for artifact, _ in EVIDENCE.values())
    paths.update(relative for relative, _ in RUNNABLE_ANCHORS)
    paths.update(FIXTURE_HASHES)
    with tempfile.TemporaryDirectory(prefix="crossline-pr-cz-completion-") as temp:
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
            lambda text: "\n".join(line for line in text.splitlines() if not line.startswith("PR-CZ\tambiguous-result-query-recovery\t")) + "\n",
            "an incomplete evidence matrix",
        )
        assert_rejected(
            root,
            "crates/api/src/services/hedge_preview/guards/identity/htx.rs",
            lambda text: text.replace('"htx-usdt-swap-durable-order-finality"', '"removed-htx-finality"', 1),
            "detached shared finality evidence",
        )
        assert_rejected(
            root,
            "crates/exchange/src/adapters/htx_private_rest.rs",
            lambda text: text.replace(".execute_once(||", ".execute_with_retry(||", 1),
            "a replay-capable signed write path",
        )
        assert_rejected(
            root,
            "crates/exchange/tests/htx_compiler_test.rs",
            lambda text: text.replace("#[tokio::test]\nasync fn ambiguous_place_result_recovers", "#[tokio::test]\n#[ignore]\nasync fn ambiguous_place_result_recovers", 1),
            "a skipped ambiguous-result recovery contract",
        )
        assert_rejected(
            root,
            "crates/exchange/fixtures/htx/ws_orders_cross_filled.json",
            lambda text: text.replace('"fee": -0.01931396', '"fee": 0', 1),
            "a zeroed reported-fee fixture",
        )
        assert_rejected(
            root,
            "test/e2e/pr_ep_htx_ws_gate.spec.ts",
            lambda text: text.replace('test("PR-EP keeps HTX schema', 'test.skip("PR-EP keeps HTX schema', 1),
            "a skipped product fixture",
        )
        assert_rejected(
            root,
            "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md",
            lambda text: text.replace("### 🟡 6.5 下一步执行队列", "### 🟡 6.5 下一步执行队列\n\n1. **PR-CZ HTX Official USDT-M Swap Order Semantics** — stale", 1),
            "completed PR-CZ returned to the queue",
        )
        assert_rejected(
            root,
            "docs/PRODUCT_AUDIT_COVERAGE.tsv",
            lambda text: "\n".join(line for line in text.splitlines() if not line.startswith("crates/exchange/tests/htx_compiler_test.rs\t")) + "\n",
            "missing HTX recovery coverage",
        )
        assert_rejected(
            root,
            "scripts/verify_repo_gates.sh",
            lambda text: text.replace("check_pr_cz_completion.sh", "removed_pr_cz_completion.sh", 1),
            "single-scope repo wiring",
        )
    print("PR-CZ completion destructive self-test passed")


try:
    root = Path(sys.argv[1])
    if sys.argv[2] == "--self-test":
        self_test(root)
    else:
        check(root)
        print(
            f"OK PR-CZ static contract ({len(EVIDENCE)} evidence types; "
            f"{len(RUNNABLE_ANCHORS)} runnable anchors; {len(BROWSER_TITLES)} browser anchors)"
        )
except (OSError, KeyError, ValueError, json.JSONDecodeError) as exc:
    raise SystemExit(f"PR-CZ completion gate failed: {exc}") from exc
PY

if [[ "$MODE" == "--self-test" ]]; then
  exit 0
fi

if [[ "${PR_CZ_SKIP_BROWSER_LIST:-0}" != "1" ]]; then
  npx playwright test "$ROOT/test/e2e/pr_ep_htx_ws_gate.spec.ts" --list >/dev/null
fi

if [[ "${PR_CZ_SKIP_UPSTREAM:-0}" != "1" ]]; then
  bash "$ROOT/scripts/check_pr_ep_completion.sh"
  PR_EB_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_eb_completion.sh"
  PR_BW_SKIP_TESTS=1 PR_BW_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_bw_completion.sh"
  PR_BZ_SKIP_TESTS=1 PR_BZ_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_bz_completion.sh"
  PR_EG_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_eg_completion.sh"
  PR_FF_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_ff_completion.sh"
  bash "$ROOT/scripts/check_product_audit_progress.sh"
  bash "$ROOT/scripts/check_product_audit_evidence.sh"
  bash "$ROOT/scripts/check_product_audit_evidence_index.sh"
  bash "$ROOT/scripts/check_product_audit_coverage.sh"
fi

if [[ "${PR_CZ_SKIP_TESTS:-0}" != "1" ]]; then
  jobs="${CARGO_BUILD_JOBS:-10}"
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --lib htx_ --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --lib market_like_styles_cover_official_bbo_and_optimal_families --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --lib single_side_compiles_both_offset_and_reduce_only --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --test htx_compiler_test --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --test htx_test --no-fail-fast -- --test-threads=1
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --test ws_trading_specs_test htx_ --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api htx_ --no-fail-fast
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-ep -- --workers=1
fi

printf 'PR-CZ HTX USDT-M Swap order semantics completion passed\n'
