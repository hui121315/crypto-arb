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
import re
import shutil
import sys
import tempfile
from pathlib import Path


PR_TITLE = "PR-DA Hyperliquid Official Exchange Runtime Contract"
VERIFY_ANCHOR = "`bash scripts/check_pr_da_completion.sh --self-test`"
FIXTURE = "crates/exchange/fixtures/hyperliquid/ws_user_events_liquidation_fill.json"
FIXTURE_HASH = "c3f85fe3ca6a5338bb568a77d47b1b750bd535046c123a4cda7ceeb11ef437aa"

EVIDENCE = {
    "successor-pr-eq": ("scripts/check_pr_eq_completion.sh", "check_pr_eq_completion.sh"),
    "successor-pr-eb": ("scripts/check_pr_eb_completion.sh", "check_pr_eb_completion.sh"),
    "successor-pr-ea": ("scripts/check_pr_ea_completion.sh", "check_pr_ea_completion.sh"),
    "successor-pr-ed": ("scripts/check_pr_ed_completion.sh", "check_pr_ed_completion.sh"),
    "successor-pr-eg": ("scripts/check_pr_eg_completion.sh", "check_pr_eg_completion.sh"),
    "successor-pr-dg": ("scripts/check_pr_dg_completion.sh", "check_pr_dg_completion.sh"),
    "official-action-msgpack": ("crates/exchange/src/signing/hyperliquid_action_msgpack.rs", "matches_official_python_sdk"),
    "official-sdk-signing-vectors": ("crates/exchange/src/signing/hyperliquid.rs", "matches_official_python_sdk_l1_action_signing_vectors"),
    "official-sdk-order-cloid-vectors": ("crates/exchange/src/signing/hyperliquid.rs", "matches_official_python_sdk_order_with_cloid_signing_vectors"),
    "unsupported-action-fail-closed": ("crates/exchange/src/signing/hyperliquid.rs", "rejects_unsupported_l1_action_before_signing"),
    "per-signer-nonce": ("crates/exchange/src/adapters/hyperliquid_ws_trade_tests.rs", "next_nonce_never_collides_within_same_millisecond"),
    "signer-ownership-boundary": ("crates/exchange/src/adapters/hyperliquid_ws_trade.rs", "signer_nonce_contract_declares_single_process_api_wallet_ownership"),
    "account-wallet-vault-role": ("crates/api/src/services/venue_credentials/validation/tests.rs", "hyperliquid_relation_probe_accepts_approved_agent_for_led_vault"),
    "builder-instrument-scope": ("crates/exchange/src/adapters/hyperliquid_compiler_tests.rs", "warmed_builder_compiler_uses_cached_official_metadata"),
    "tif-fok-boundary": ("crates/exchange/src/adapters/hyperliquid_trade_data_tests.rs", "limit_order_rejects_unsupported_time_in_force"),
    "spot-write-boundary": ("crates/api/src/trading_service/live_adapters/route_tests/cases.rs", "live_router_routes_exact_builder_venue_and_family_fallback"),
    "per-dex-account-runtime": ("crates/api/src/trading_service/private_ws_mapper/tests/cases_3.rs", "hyperliquid_all_dexs_clearinghouse_maps_each_dex_snapshot"),
    "private-order-finality": ("crates/exchange/src/adapters/hyperliquid_ws_user_tests.rs", "ws_finality_fixture_maps_partial_and_terminal_states"),
    "fill-liquidation-parser": ("crates/exchange/tests/hyperliquid_runtime_contract_test.rs", "hyperliquid_runtime_contract_test"),
    "fill-ledger-evidence": ("crates/api/src/trading_service/private_ws_events/tests/pnl_hyperliquid.rs", "hyperliquid_fill_evidence_is_order_linked_and_json_round_trippable"),
    "funding-ledger": ("crates/api/src/trading_service/private_ws_events/tests/pnl.rs", "private_funding_delta_flows_to_review_and_portfolio_pnl"),
    "account-liquidation-boundary": ("crates/api/src/trading_service/private_ws_events/tests/pnl.rs", "funding_and_liquidation_mark_scoped_account_cache_dirty_without_matched_order_ledger"),
    "non-user-cancel-ledger": ("crates/api/src/trading_service/private_ws_events/tests/pnl.rs", "non_user_cancel_updates_order_by_exchange_order_id"),
    "operation-registry": ("crates/exchange/src/venue_spec.rs", "hyperliquid_operation_evidence_is_exact_and_transport_scoped"),
    "product-browser": ("test/e2e/pr_eq_hyperliquid_contract.spec.ts", "test:e2e:pr-eq"),
    "external-live-boundary": ("docs/PRODUCT_FULL_AUDIT_REFINEMENT.md", "check_product_audit_progress.sh"),
    "completion-governance": ("scripts/check_pr_da_completion.sh", "check_pr_da_completion.sh --self-test"),
}

RUNNABLE_ANCHORS = (
    ("crates/exchange/src/signing/hyperliquid.rs", "matches_official_python_sdk_l1_action_signing_vectors"),
    ("crates/exchange/src/signing/hyperliquid.rs", "matches_official_python_sdk_order_with_cloid_signing_vectors"),
    ("crates/exchange/src/signing/hyperliquid.rs", "rejects_unsupported_l1_action_before_signing"),
    ("crates/exchange/src/adapters/hyperliquid_ws_trade_tests.rs", "next_nonce_never_collides_within_same_millisecond"),
    ("crates/exchange/src/adapters/hyperliquid_ws_trade_tests.rs", "signer_nonce_contract_declares_single_process_api_wallet_ownership"),
    ("crates/api/src/services/venue_credentials/validation/tests.rs", "hyperliquid_relation_probe_accepts_approved_agent_for_led_vault"),
    ("crates/exchange/src/adapters/hyperliquid_compiler_tests.rs", "warmed_builder_compiler_uses_cached_official_metadata"),
    ("crates/exchange/src/adapters/hyperliquid_trade_data_tests.rs", "limit_order_rejects_unsupported_time_in_force"),
    ("crates/api/src/trading_service/live_adapters/route_tests/cases.rs", "live_router_routes_exact_builder_venue_and_family_fallback"),
    ("crates/api/src/trading_service/private_ws_mapper/tests/cases_3.rs", "hyperliquid_all_dexs_clearinghouse_maps_each_dex_snapshot"),
    ("crates/exchange/src/adapters/hyperliquid_ws_user_tests.rs", "ws_finality_fixture_maps_partial_and_terminal_states"),
    ("crates/exchange/tests/hyperliquid_runtime_contract_test.rs", "official_user_fill_fixture_preserves_closed_pnl_and_liquidation_evidence"),
    ("crates/exchange/tests/hyperliquid_runtime_contract_test.rs", "user_fill_liquidation_rejects_unknown_official_method"),
    ("crates/api/src/trading_service/private_ws_events/tests/pnl_hyperliquid.rs", "hyperliquid_fill_evidence_is_order_linked_and_json_round_trippable"),
    ("crates/api/src/trading_service/private_ws_events/tests/pnl.rs", "private_funding_delta_flows_to_review_and_portfolio_pnl"),
    ("crates/api/src/trading_service/private_ws_events/tests/pnl.rs", "funding_and_liquidation_mark_scoped_account_cache_dirty_without_matched_order_ledger"),
    ("crates/api/src/trading_service/private_ws_events/tests/pnl.rs", "non_user_cancel_updates_order_by_exchange_order_id"),
    ("crates/exchange/src/venue_spec.rs", "hyperliquid_operation_evidence_is_exact_and_transport_scoped"),
)

BROWSER_TITLES = (
    "PR-EQ renders ticket-owned Hyperliquid builder cloid and protected IOC evidence",
    "PR-EQ blocks submit when ticket-owned identity and protected IOC evidence is absent",
    "PR-EQ Settings keeps builder-scoped Hyperliquid endpoint and wallet/vault evidence non-live",
)


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
    if re.search(r"(?m)^\d+\.\s+\*\*PR-DA\b", queue):
        fail("completed PR-DA remains in the local queue")
    pr_db = next(
        (
            line
            for line in doc.splitlines()
            if line.startswith("| `PR-DB PaperLive Runtime Mode & Legacy Readiness Removal Contract`")
        ),
        None,
    )
    if pr_db is None or "✅ 完成" not in pr_db or "剩余：无。" not in pr_db:
        fail("successor PR-DB must remain complete with no local remainder")
    if re.search(r"(?m)^\d+\.\s+\*\*PR-DB\b", queue):
        fail("completed successor PR-DB remains in the local queue")
    pr_di_row = next(
        (line for line in doc.splitlines() if line.startswith("| `PR-DI Portfolio AccountState & CloseRun Contract`")),
        None,
    )
    if pr_di_row is None or "✅ 完成" not in pr_di_row or "剩余：无。" not in pr_di_row:
        fail("successor PR-DI completion drifted")
    if re.search(r"(?m)^\d+\.\s+\*\*PR-DI\b", queue):
        fail("completed successor PR-DI remains in the local queue")
    pr_cf_row = next(
        (line for line in doc.splitlines() if line.startswith("| `PR-CF CEX Credential Validation & Account Mode Evidence Matrix`")),
        None,
    )
    if pr_cf_row is None or "✅ 完成" not in pr_cf_row or "剩余：无。" not in pr_cf_row:
        fail("successor PR-CF completion drifted")
    if re.search(r"(?m)^\d+\.\s+\*\*PR-CF\b", queue):
        fail("completed successor PR-CF remains in the local queue")
    if "one API wallet per trading process" not in queue or "真实 Hyperliquid" not in queue:
        fail("cross-process signer and real-live capture boundaries must remain explicit")

    history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(encoding="utf-8")
    if "## 2026-07-16 PR-DA Hyperliquid Official Exchange Runtime Contract Closure" not in history:
        fail("history closure appendix is missing")

    selected = [row for row in table_rows(root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv") if row["pr_id"] == "PR-DA"]
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
        "official-action-msgpack": "protocol_ordered=true",
        "signer-ownership-boundary": "cross_process_shared_signer_unsupported=true",
        "spot-write-boundary": "spot_write_observation_only=true",
        "fill-ledger-evidence": "order_linked=true",
        "account-liquidation-boundary": "no_fabricated_order=true",
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

    signing = (root / "crates/exchange/src/signing/hyperliquid.rs").read_text(encoding="utf-8")
    msgpack = (root / "crates/exchange/src/signing/hyperliquid_action_msgpack.rs").read_text(encoding="utf-8")
    if "action_msgpack::to_vec(action)" not in signing or "OfficialAction" not in msgpack:
        fail("official protocol-ordered msgpack signing path is detached")
    for marker in ("ORDER_ACTION_KEYS", "ORDER_KEYS", '"c"', '"cancelByCloid"'):
        if marker not in msgpack:
            fail(f"official action schema marker missing: {marker}")

    ws_trade = (root / "crates/exchange/src/adapters/hyperliquid_ws_trade.rs").read_text(encoding="utf-8")
    if 'SIGNER_OWNERSHIP_BOUNDARY: &str = "one_api_wallet_per_trading_process"' not in ws_trade:
        fail("single-process API-wallet nonce ownership boundary drifted")
    route_tests = (root / "crates/api/src/trading_service/live_adapters/route_tests/cases.rs").read_text(encoding="utf-8")
    if 'route_name(&router, "hyperliquid:spot")' not in route_tests:
        fail("Hyperliquid spot write boundary is no longer locked")

    shared = (root / "shared-types/src/live_trading.rs").read_text(encoding="utf-8")
    mapper = (root / "crates/api/src/trading_service/private_ws_mapper/hyperliquid.rs").read_text(encoding="utf-8")
    apply = (root / "crates/api/src/trading_service/private_ws_events/apply.rs").read_text(encoding="utf-8")
    journal = (root / "crates/trading/src/journal/projection/part_08.rs").read_text(encoding="utf-8")
    ledger = (root / "crates/trading/src/ledger.rs").read_text(encoding="utf-8")
    for source, marker in (
        (shared, "VenueFillTransportEvidence"),
        (mapper, "FillWithEvidence"),
        (apply, "record_fill_by_order_identity_with_metadata_deferred_sql"),
        (journal, "record_fill_by_order_identity_with_metadata_deferred_sql"),
        (ledger, "record_fill_event_with_context_and_metadata"),
    ):
        if marker not in source:
            fail(f"order-linked fill evidence chain is detached: {marker}")

    if hashlib.sha256((root / FIXTURE).read_bytes()).hexdigest() != FIXTURE_HASH:
        fail("official liquidation fill fixture hash drifted")
    browser = (root / "test/e2e/pr_eq_hyperliquid_contract.spec.ts").read_text(encoding="utf-8")
    for title in BROWSER_TITLES:
        escaped = re.escape(title)
        if re.search(rf'test\.(?:skip|fixme)\(\s*["\']{escaped}["\']', browser):
            fail(f"browser anchor is skipped: {title}")
        if not re.search(rf'test\(\s*["\']{escaped}["\']', browser):
            fail(f"browser anchor is missing: {title}")

    repo_gate = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
    if repo_gate.count("check_pr_da_completion.sh") != 2:
        fail("repo gate must execute PR-DA once in docs and all scopes")


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
        "shared-types/src/live_trading.rs",
        "crates/exchange/src/signing/hyperliquid.rs",
        "crates/exchange/src/signing/hyperliquid_action_msgpack.rs",
        "crates/exchange/src/adapters/hyperliquid_ws_trade.rs",
        "crates/exchange/fixtures/hyperliquid/ws_user_events_liquidation_fill.json",
        "crates/api/src/trading_service/live_adapters/route_tests/cases.rs",
        "crates/api/src/trading_service/private_ws_mapper/hyperliquid.rs",
        "crates/api/src/trading_service/private_ws_events/apply.rs",
        "crates/trading/src/journal/projection/part_08.rs",
        "crates/trading/src/ledger.rs",
        "test/e2e/pr_eq_hyperliquid_contract.spec.ts",
        "scripts/verify_repo_gates.sh",
    }
    paths.update(artifact for artifact, _ in EVIDENCE.values())
    paths.update(relative for relative, _ in RUNNABLE_ANCHORS)
    with tempfile.TemporaryDirectory(prefix="crossline-pr-da-completion-") as temp:
        root = Path(temp) / "repo"
        for relative in paths:
            target = root / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source_root / relative, target)
        check(root)
        assert_rejected(root, "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md", lambda text: text.replace(f"| `{PR_TITLE}` | ✅ 完成 |", f"| `{PR_TITLE}` | 🟡 部分完成 |", 1), "a downgraded roadmap row")
        assert_rejected(root, "docs/PRODUCT_AUDIT_EVIDENCE.tsv", lambda text: "\n".join(line for line in text.splitlines() if not line.startswith("PR-DA\tofficial-action-msgpack\t")) + "\n", "an incomplete evidence matrix")
        assert_rejected(root, "crates/exchange/src/signing/hyperliquid.rs", lambda text: text.replace("action_msgpack::to_vec(action)", "rmp_serde::to_vec_named(action)", 1), "unordered JSON msgpack signing")
        assert_rejected(root, "crates/exchange/tests/hyperliquid_runtime_contract_test.rs", lambda text: text.replace("#[test]\nfn official_user_fill_fixture", "#[test]\n#[ignore]\nfn official_user_fill_fixture", 1), "a skipped liquidation fixture")
        assert_rejected(root, FIXTURE, lambda text: text.replace('"closedPnl": "-12.375"', '"closedPnl": "0"', 1), "a zeroed closed-PnL fixture")
        assert_rejected(root, "shared-types/src/live_trading.rs", lambda text: text.replace("VenueFillTransportEvidence", "RemovedFillTransportEvidence"), "detached shared fill evidence")
        assert_rejected(root, "crates/api/src/trading_service/live_adapters/route_tests/cases.rs", lambda text: text.replace('route_name(&router, "hyperliquid:spot")', 'route_name(&router, "hyperliquid:spot-enabled")', 1), "an unlocked spot write boundary")
        assert_rejected(root, "test/e2e/pr_eq_hyperliquid_contract.spec.ts", lambda text: text.replace('test("PR-EQ renders ticket-owned', 'test.skip("PR-EQ renders ticket-owned', 1), "a skipped product fixture")
        assert_rejected(root, "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md", lambda text: text.replace("### 🟡 6.5 下一步执行队列", "### 🟡 6.5 下一步执行队列\n\n1. **PR-DA stale**", 1), "completed PR-DA returned to the queue")
        assert_rejected(root, "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md", lambda text: text.replace("1. **PR-CE Hyperliquid", "1. **PR-DB stale successor\n2. **PR-DI stale successor\n3. **PR-CF stale successor\n4. **PR-CE Hyperliquid", 1), "completed PR-DB, PR-DI or PR-CF returned to the queue head")
        assert_rejected(root, "docs/PRODUCT_AUDIT_COVERAGE.tsv", lambda text: "\n".join(line for line in text.splitlines() if not line.startswith("crates/exchange/src/signing/hyperliquid_action_msgpack.rs\t")) + "\n", "missing signing coverage")
        assert_rejected(root, "scripts/verify_repo_gates.sh", lambda text: text.replace("check_pr_da_completion.sh", "removed_pr_da_completion.sh", 1), "single-scope repo wiring")
    print("PR-DA completion destructive self-test passed")


try:
    root = Path(sys.argv[1])
    if sys.argv[2] == "--self-test":
        self_test(root)
    else:
        check(root)
        print(f"OK PR-DA static contract ({len(EVIDENCE)} evidence types; {len(RUNNABLE_ANCHORS)} runnable anchors; {len(BROWSER_TITLES)} browser anchors)")
except (OSError, KeyError, ValueError) as exc:
    raise SystemExit(f"PR-DA completion gate failed: {exc}") from exc
PY

if [[ "$MODE" == "--self-test" ]]; then
  exit 0
fi

if [[ "${PR_DA_SKIP_BROWSER_LIST:-0}" != "1" ]]; then
  npx playwright test "$ROOT/test/e2e/pr_eq_hyperliquid_contract.spec.ts" --list >/dev/null
fi

if [[ "${PR_DA_SKIP_UPSTREAM:-0}" != "1" ]]; then
  bash "$ROOT/scripts/check_pr_eq_completion.sh"
  PR_EB_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_eb_completion.sh"
  PR_EA_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_ea_completion.sh"
  PR_ED_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_ed_completion.sh"
  PR_EG_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_eg_completion.sh"
  PR_DG_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dg_completion.sh"
  bash "$ROOT/scripts/check_product_audit_progress.sh"
  bash "$ROOT/scripts/check_product_audit_evidence.sh"
  bash "$ROOT/scripts/check_product_audit_evidence_index.sh"
  bash "$ROOT/scripts/check_product_audit_coverage.sh"
fi

if [[ "${PR_DA_SKIP_TESTS:-0}" != "1" ]]; then
  jobs="${CARGO_BUILD_JOBS:-10}"
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --lib signing::hyperliquid::tests:: --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --test hyperliquid_runtime_contract_test --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --lib hyperliquid_ --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api hyperliquid_ --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api live_router_routes_exact_builder_venue_and_family_fallback --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api private_funding_delta_flows_to_review_and_portfolio_pnl --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api funding_and_liquidation_mark_scoped_account_cache_dirty_without_matched_order_ledger --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api non_user_cancel_updates_order_by_exchange_order_id --no-fail-fast
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-eq -- --workers=1
fi

printf 'PR-DA Hyperliquid official runtime contract completion passed\n'
