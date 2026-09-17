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

from pathlib import Path
import csv
import hashlib
import re
import shutil
import subprocess
import sys
import tempfile


PR_ID = "PR-EQ"
PR_TITLE = "PR-EQ Hyperliquid Builder Dex Account, Order & Finality Contract"
BROWSER_PATH = "test/e2e/pr_eq_hyperliquid_contract.spec.ts"
BROWSER_ANCHORS = (
    "PR-EQ renders ticket-owned Hyperliquid builder cloid and protected IOC evidence",
    "PR-EQ blocks submit when ticket-owned identity and protected IOC evidence is absent",
    "PR-EQ Settings keeps builder-scoped Hyperliquid endpoint and wallet/vault evidence non-live",
)
COMPLETED_STATUS = "\u2705 \u5b8c\u6210"
NO_LOCAL_REMAINDER = "\u5269\u4f59\uff1a\u65e0\u3002"
WAITING_POOL_EXTERNAL = (
    "\u771f\u5b9e Hyperliquid live credentials\u3001private streams\u3001live place/cancel "
    "\u4e0e order-finality \u8131\u654f capture \u4ecd\u9700\u771f\u7f51\u91c7\u96c6\uff0c"
    "\u4e0d\u56de\u9000\u672c\u5730\u5b8c\u6210\u72b6\u6001\u3002"
)

EVIDENCE = {
    "compiler-cache": (
        "crates/exchange/src/adapters/hyperliquid_compiler_tests.rs",
        "warmed_builder_compiler_uses_cached_official_metadata",
    ),
    "protected-ioc-cloid": (
        "crates/exchange/src/adapters/hyperliquid_trade_data_tests.rs",
        "cloid_policy_passes_official_128_bit_hex_and_derives_public_ids",
    ),
    "account-wallet-vault-parser": (
        "crates/api/src/services/venue_credentials/validation/tests.rs",
        "hyperliquid_relation_probe_accepts_approved_agent_for_led_vault",
    ),
    "private-ws-finality": (
        "crates/exchange/src/adapters/hyperliquid_ws_user_tests.rs",
        "ws_finality_fixture_maps_partial_and_terminal_states",
    ),
    "private-ws-dex-scope-fills-funding": (
        "crates/api/src/trading_service/private_ws_mapper/tests/cases_3.rs",
        "hyperliquid_dex_scoped_fill_and_funding_keep_their_venue",
    ),
    "endpoint-spec-operation-registry": (
        "crates/exchange/src/venue_spec.rs",
        "hyperliquid_operation_evidence_is_exact_and_transport_scoped",
    ),
    "ticket-plan-ownership": (
        "shared-types/src/hedge.rs",
        "ticket_order_plans_keep_hyperliquid_builder_compile_and_identity_evidence",
    ),
    "official-capture-fixture-registry": (
        "crates/exchange/src/venue_spec.rs",
        "hyperliquid_operation_fixtures_parse_and_keep_dex_scope",
    ),
    "product-browser": (BROWSER_PATH, "npm run test:e2e:pr-eq"),
    "completion-governance": (
        "scripts/check_pr_eq_completion.sh",
        "bash scripts/check_pr_eq_completion.sh --self-test",
    ),
}

RUST_ANCHORS = (
    (
        "crates/exchange/src/adapters/hyperliquid_compiler_tests.rs",
        "warmed_builder_compiler_uses_cached_official_metadata",
    ),
    (
        "crates/exchange/src/adapters/hyperliquid_compiler_tests.rs",
        "cold_compiler_fails_before_any_write_transport",
    ),
    (
        "crates/exchange/tests/hyperliquid_compiler_test.rs",
        "core_and_builder_metadata_requests_follow_official_info_shapes",
    ),
    (
        "crates/exchange/src/adapters/hyperliquid_instruments_tests.rs",
        "builder_metadata_uses_official_dex_scoped_asset_id_formula",
    ),
    (
        "crates/exchange/src/adapters/hyperliquid_instruments_tests.rs",
        "builder_metadata_fails_closed_when_dex_index_is_missing",
    ),
    (
        "crates/exchange/src/adapters/hyperliquid_trade_data_tests.rs",
        "market_order_is_protected_ioc_limit",
    ),
    (
        "crates/exchange/src/adapters/hyperliquid_trade_data_tests.rs",
        "protected_market_rejects_non_ioc_time_in_force",
    ),
    (
        "crates/exchange/src/adapters/hyperliquid_trade_data_tests.rs",
        "cloid_policy_passes_official_128_bit_hex_and_derives_public_ids",
    ),
    (
        "crates/exchange/src/adapters/hyperliquid_trade_data_tests.rs",
        "order_and_cancel_actions_share_derived_cloid",
    ),
    (
        "crates/api/src/services/venue_credentials/validation/tests.rs",
        "hyperliquid_relation_probe_accepts_direct_main_account_signer",
    ),
    (
        "crates/api/src/services/venue_credentials/validation/tests.rs",
        "hyperliquid_relation_probe_accepts_approved_agent_for_led_vault",
    ),
    (
        "crates/api/src/services/venue_credentials/validation/tests.rs",
        "hyperliquid_relation_probe_rejects_unrelated_signer_or_unproven_vault",
    ),
    (
        "crates/exchange/src/adapters/hyperliquid_private_data_tests.rs",
        "order_status_official_filled_envelope_requires_user_fills_evidence",
    ),
    (
        "crates/exchange/tests/hyperliquid_test.rs",
        "credential_relation_reads_main_agent_and_vault_facts",
    ),
    (
        "crates/exchange/src/adapters/hyperliquid_ws_trade_tests.rs",
        "filled_post_response_remains_a_non_final_transport_ack",
    ),
    (
        "crates/exchange/src/adapters/hyperliquid_ws_user_tests.rs",
        "ws_finality_fixture_maps_partial_and_terminal_states",
    ),
    (
        "crates/exchange/src/adapters/hyperliquid_ws_user_tests.rs",
        "dex_scoped_fill_and_funding_fixtures_preserve_reported_values",
    ),
    (
        "crates/api/src/trading_service/private_ws_mapper/tests/cases_3.rs",
        "hyperliquid_all_dexs_clearinghouse_maps_each_dex_snapshot",
    ),
    (
        "crates/api/src/trading_service/private_ws_mapper/tests/cases_3.rs",
        "hyperliquid_dex_scoped_fill_and_funding_keep_their_venue",
    ),
    (
        "crates/api/src/trading_service/private_ws_mapper/tests/cases_4.rs",
        "hyperliquid_non_user_cancel_maps_to_order_cancel_event",
    ),
    (
        "crates/exchange/src/venue_spec.rs",
        "hyperliquid_info_path_only_evidence_is_unavailable",
    ),
    (
        "crates/exchange/src/venue_spec.rs",
        "hyperliquid_operation_evidence_is_exact_and_transport_scoped",
    ),
    (
        "crates/exchange/src/venue_spec.rs",
        "hyperliquid_info_operation_request_contracts_are_distinct",
    ),
    (
        "crates/exchange/src/venue_spec.rs",
        "hyperliquid_ws_operation_request_contracts_remain_ws_only",
    ),
    (
        "crates/exchange/src/venue_spec.rs",
        "hyperliquid_operation_fixtures_parse_and_keep_dex_scope",
    ),
    (
        "crates/exchange/src/rest_registry.rs",
        "registry_covers_every_endpoint_spec_with_exact_evidence",
    ),
    (
        "shared-types/src/hedge.rs",
        "ticket_order_plans_keep_hyperliquid_builder_compile_and_identity_evidence",
    ),
    (
        "shared-types/src/hedge.rs",
        "ticket_order_plans_reject_swapped_compile_roles",
    ),
    (
        "crates/api/src/services/hedge_confirm/confirm_validate/tests.rs",
        "confirm_rejects_legacy_preview_without_ticket_order_plan_evidence",
    ),
    (
        "crates/api/src/services/hedge_confirm/confirm_validate/tests.rs",
        "confirm_uses_ticket_bound_hyperliquid_builder_plans",
    ),
    (
        "frontend/src/panels/modules/execution/data/preview_tests.rs",
        "from_api_preview_ignores_legacy_order_plans_without_ticket_evidence",
    ),
    (
        "frontend/src/panels/modules/execution/data/preview_tests.rs",
        "from_api_preview_uses_ticket_hyperliquid_builder_order_plan_evidence",
    ),
)

FIXTURES = {
    "crates/exchange/fixtures/hyperliquid/info_frontend_open_orders_dex.json": (
        "82f8ddebbfd3b17b9164e6709e487aefc1048341619f8d74a740fda82054db7d"
    ),
    "crates/exchange/fixtures/hyperliquid/info_open_orders_dex.json": (
        "0bb032ff7d509fcdb288c596bcb1f2a186ad116b736a8d2badd8a8023c829273"
    ),
    "crates/exchange/fixtures/hyperliquid/meta_compiler_xyz.json": (
        "116ac30eb6373619f3e4cfe31652d07341cd1d05782d988142dd2056afa9aca8"
    ),
    "crates/exchange/fixtures/hyperliquid/perp_dexs_compiler.json": (
        "e8e1f28bb6aab5a264a2f9c489f26b751c3b08ee550b00b21494114195a7aab1"
    ),
    "crates/exchange/fixtures/hyperliquid/ws_all_dexs_asset_ctxs_evidence.json": (
        "d6201da58fe964960fa8b123e99c44051b57cfffaac3cc00282ef2633b30cbbb"
    ),
    "crates/exchange/fixtures/hyperliquid/ws_all_dexs_clearinghouse_evidence.json": (
        "6a29c5cdfcb02a17f525add9d529c5e75e4fb1f1be2bb99af35bfa58ae43f6dc"
    ),
    "crates/exchange/fixtures/hyperliquid/ws_order_updates_finality.json": (
        "0e524ac81fa1b6389bb8b1e727d03214a4ccfa67c6ebeca54c22e186221b211e"
    ),
    "crates/exchange/fixtures/hyperliquid/ws_user_events_fill_xyz.json": (
        "0a27b72c8d1c6fb30c9635ee53141be0e5782b43568cd654ca1ac91bd2bcca14"
    ),
    "crates/exchange/fixtures/hyperliquid/ws_user_fundings_xyz.json": (
        "da44ad217ceb06820b5676b5262a3303039f6a25ccdadd491941b411cb7fa196"
    ),
}

CLOSURE_PATHS = tuple(
    """
scripts/check_pr_eq_completion.sh
docs/PRODUCT_FULL_AUDIT_REFINEMENT.md
docs/audit_history/PRODUCT_AUDIT_HISTORY.md
docs/PRODUCT_AUDIT_EVIDENCE.tsv
docs/PRODUCT_AUDIT_COVERAGE.tsv
package.json
test/e2e/pr_eq_hyperliquid_contract.spec.ts
shared-types/src/arbitrage.rs
shared-types/src/hedge.rs
frontend/src/panels/modules/execution/data/preview/response.rs
frontend/src/panels/modules/execution/data/preview_tests.rs
frontend/src/panels/modules/execution/data/preview_tests/fixtures.rs
crates/api/src/services/hedge_confirm/confirm_validate.rs
crates/api/src/services/hedge_confirm/confirm_validate/tests.rs
crates/api/src/routers/trading/tests/cases_registry.rs
crates/api/src/routers/venues.rs
crates/api/src/services/hedge_preview.rs
crates/api/src/services/instrument_registry.rs
crates/api/src/services/private_ws_health/counts.rs
crates/api/src/services/private_ws_health/tests.rs
crates/api/src/services/venue_credentials/validation/hyperliquid_probes.rs
crates/api/src/services/venue_credentials/validation/tests.rs
crates/api/src/services/venue_credentials/validation/venues.rs
crates/api/src/trading_service/private_ws_events/apply.rs
crates/api/src/trading_service/private_ws_events/tests/cache_patch.rs
crates/api/src/trading_service/private_ws_events/types.rs
crates/api/src/trading_service/private_ws_mapper/hyperliquid.rs
crates/api/src/trading_service/private_ws_mapper/tests/cases_binance_okx.rs
crates/api/src/trading_service/private_ws_mapper/tests/cases_gate_account.rs
crates/api/src/trading_service/private_ws_mapper/tests/cases_htx.rs
crates/api/src/trading_service/private_ws_mapper/tests/cases_hyperliquid_clearinghouse.rs
crates/api/src/trading_service/private_ws_mapper/tests/cases_kucoin.rs
crates/api/src/trading_service/private_ws_mapper/tests/cases_3.rs
crates/api/src/trading_service/private_ws_mapper/tests/cases_4.rs
crates/api/src/trading_service/private_ws_mapper/tests/fixtures_a.rs
crates/exchange/src/lib.rs
crates/exchange/src/adapters/mod.rs
crates/exchange/src/adapters/hyperliquid.rs
crates/exchange/src/adapters/hyperliquid_compiler_tests.rs
crates/exchange/src/adapters/hyperliquid_config.rs
crates/exchange/src/adapters/hyperliquid_instruments.rs
crates/exchange/src/adapters/hyperliquid_instruments_tests.rs
crates/exchange/src/adapters/hyperliquid_private_data.rs
crates/exchange/src/adapters/hyperliquid_private_data_tests.rs
crates/exchange/src/adapters/hyperliquid_public_rest.rs
crates/exchange/src/adapters/hyperliquid_support.rs
crates/exchange/src/adapters/hyperliquid_trade_data.rs
crates/exchange/src/adapters/hyperliquid_trade_data_tests.rs
crates/exchange/src/adapters/hyperliquid_ws_trade_tests.rs
crates/exchange/src/adapters/hyperliquid_ws_user.rs
crates/exchange/src/adapters/hyperliquid_ws_user_data.rs
crates/exchange/src/adapters/hyperliquid_ws_user_tests.rs
crates/exchange/src/rest_registry.rs
crates/exchange/src/venue_spec.rs
crates/exchange/tests/hyperliquid_compiler_test.rs
crates/exchange/fixtures/hyperliquid/info_frontend_open_orders_dex.json
crates/exchange/fixtures/hyperliquid/info_open_orders_dex.json
crates/exchange/fixtures/hyperliquid/meta_compiler_xyz.json
crates/exchange/fixtures/hyperliquid/perp_dexs_compiler.json
crates/exchange/fixtures/hyperliquid/ws_all_dexs_asset_ctxs_evidence.json
crates/exchange/fixtures/hyperliquid/ws_all_dexs_clearinghouse_evidence.json
crates/exchange/fixtures/hyperliquid/ws_order_updates_finality.json
crates/exchange/fixtures/hyperliquid/ws_user_events_fill_xyz.json
crates/exchange/fixtures/hyperliquid/ws_user_fundings_xyz.json
crates/exchange/tests/hyperliquid_test.rs
""".split()
)

if len(CLOSURE_PATHS) != 65 or len(set(CLOSURE_PATHS)) != 65:
    raise SystemExit("PR-EQ completion gate internal error: expected 65 unique closure paths")


def rows(path: Path, header: tuple[str, ...]) -> list[dict[str, str]]:
    with path.open(encoding="utf-8", newline="") as handle:
        reader = csv.DictReader(handle, delimiter="\t")
        if tuple(reader.fieldnames or ()) != header:
            raise ValueError(f"{path.name} header drifted")
        return list(reader)


def section(text: str, number: str) -> str:
    start = re.search(rf"(?m)^### .*\b{re.escape(number)}\b.*$", text)
    if not start:
        raise ValueError(f"missing section {number}")
    following = re.search(r"(?m)^### ", text[start.end() :])
    end = start.end() + following.start() if following else len(text)
    return text[start.start() : end]


def require_roadmap(root: Path) -> None:
    text = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
    roadmap = section(text, "6.3")
    matched = [line for line in roadmap.splitlines() if line.startswith("| `PR-EQ ")]
    if len(matched) != 1:
        raise ValueError(f"expected one PR-EQ roadmap row, found {len(matched)}")
    cells = [cell.strip() for cell in matched[0].strip().strip("|").split("|")]
    if len(cells) != 4 or cells[0].strip("`") != PR_TITLE:
        raise ValueError("PR-EQ roadmap identity drifted")
    if cells[1] != COMPLETED_STATUS or NO_LOCAL_REMAINDER not in cells[2]:
        raise ValueError("PR-EQ must be completed with no local remainder")
    self_test = "bash scripts/check_pr_eq_completion.sh --self-test"
    if self_test not in cells[3]:
        raise ValueError("PR-EQ roadmap row lacks completion self-test anchor")

    waiting = section(text, "6.5")
    if re.search(r"(?m)^\d+\.\s+\*\*PR-EQ\*\*", waiting):
        raise ValueError("completed PR-EQ remains in the local queue")
    matches = re.findall(
        rf"PR-EQ[^\u3002]*?[\uff1b;]\s*{re.escape(WAITING_POOL_EXTERNAL)}",
        waiting,
    )
    if len(matches) != 1:
        raise ValueError(
            "PR-EQ waiting-pool wording must contain only live credentials, private "
            "streams, live place/cancel, and order-finality"
        )


def require_evidence(root: Path) -> None:
    selected = [
        row
        for row in rows(
            root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv",
            ("pr_id", "evidence_type", "artifact", "command", "notes"),
        )
        if row["pr_id"].strip() == PR_ID
    ]
    indexed = {row["evidence_type"].strip(): row for row in selected}
    if len(indexed) != len(selected) or set(indexed) != set(EVIDENCE):
        raise ValueError(
            f"PR-EQ evidence type drift: expected={sorted(EVIDENCE)}, actual={sorted(indexed)}"
        )
    for evidence_type, (artifact, anchor) in EVIDENCE.items():
        row = indexed[evidence_type]
        if row["artifact"].strip() != artifact or anchor not in row["command"]:
            raise ValueError(f"PR-EQ {evidence_type} artifact/command drifted")
        if not (root / artifact).is_file():
            raise ValueError(f"missing PR-EQ artifact {artifact}")
    if "ack_not_final" not in indexed["private-ws-finality"]["notes"]:
        raise ValueError("PR-EQ private WS finality evidence lacks ack_not_final")
    if "live_credentials_claimed=false" not in indexed["product-browser"]["notes"]:
        raise ValueError("PR-EQ browser evidence must disclaim live credentials")


def test_attributes(text: str, function_start: int) -> str:
    attributes: list[str] = []
    for line in reversed(text[:function_start].splitlines()):
        stripped = line.strip()
        if not stripped:
            continue
        if stripped.startswith("#["):
            attributes.append(stripped)
            continue
        break
    return "\n".join(reversed(attributes))


def require_runnable_anchors(root: Path) -> None:
    for relative, name in RUST_ANCHORS:
        text = (root / relative).read_text(encoding="utf-8")
        match = re.search(
            rf"(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+{re.escape(name)}\s*(?:<[^>]*>)?\s*\(",
            text,
        )
        if not match:
            raise ValueError(f"missing runnable Rust anchor {relative}::{name}")
        attributes = test_attributes(text, match.start())
        if not re.search(r"#\[\s*(?:tokio::)?test(?:\]|\()", attributes):
            raise ValueError(f"non-test Rust anchor {relative}::{name}")
        if re.search(r"\b(?:ignore|should_panic)\b", attributes):
            raise ValueError(f"non-runnable Rust anchor {relative}::{name}")

    browser = (root / BROWSER_PATH).read_text(encoding="utf-8")
    if re.search(r"\b(?:test|describe)\.(?:skip|fixme)\b|\bFIXME\b", browser):
        raise ValueError("PR-EQ browser gate contains skip/fixme")
    titles = tuple(re.findall(r'(?m)^\s*test\("([^"]+)"', browser))
    if titles != BROWSER_ANCHORS:
        raise ValueError(
            f"PR-EQ must contain exactly three ordered browser anchors: {titles}"
        )


def require_registry_and_fixtures(root: Path) -> None:
    venue = (root / "crates/exchange/src/venue_spec.rs").read_text(encoding="utf-8")
    for marker in (
        "HyperliquidOperationEvidence",
        "HyperliquidOperationTransport",
        "HyperliquidDexScope",
        "endpoint_evidence_for_spec",
        "hyperliquid_info_path_only_evidence_is_unavailable",
        "hyperliquid_operation_evidence_is_exact_and_transport_scoped",
        "hyperliquid_info_operation_request_contracts_are_distinct",
        "hyperliquid_ws_operation_request_contracts_remain_ws_only",
        "hyperliquid_operation_fixtures_parse_and_keep_dex_scope",
        "metaAndAssetCtxs",
        "openOrders",
        "frontendOpenOrders",
        "orderStatus",
        "clearinghouseState",
        "spotClearinghouseState",
        "allDexsAssetCtxs",
        "allDexsClearinghouseState",
    ):
        if marker not in venue:
            raise ValueError(f"PR-EQ EndpointSpec registry missing {marker}")

    registry = (root / "crates/exchange/src/rest_registry.rs").read_text(encoding="utf-8")
    for marker in (
        "endpoint_evidence_for_spec",
        "registry_covers_every_endpoint_spec_with_exact_evidence",
        "operation_matrix_rest_buckets_match_exact_allowlist_endpoint_projection",
    ):
        if marker not in registry:
            raise ValueError(f"PR-EQ REST registry missing {marker}")

    for relative, expected in FIXTURES.items():
        actual = hashlib.sha256((root / relative).read_bytes()).hexdigest()
        if actual != expected:
            raise ValueError(f"PR-EQ Hyperliquid fixture hash drifted: {relative}")


def require_closure_coverage(root: Path) -> None:
    coverage = rows(
        root / "docs/PRODUCT_AUDIT_COVERAGE.tsv",
        ("file", "coverage_status", "owner_surface", "risk", "suggested_pr", "evidence", "notes"),
    )
    by_file: dict[str, list[dict[str, str]]] = {}
    for row in coverage:
        by_file.setdefault(row["file"].strip(), []).append(row)

    for relative in CLOSURE_PATHS:
        if not (root / relative).is_file():
            raise ValueError(f"missing exact PR-EQ closure path: {relative}")
        if relative == "package.json" or relative.startswith(("docs/", "crates/exchange/fixtures/")):
            continue
        entries = by_file.get(relative, [])
        if len(entries) != 1:
            raise ValueError(
                f"PR-EQ closure path must have one exact coverage row: {relative}"
            )
        row = entries[0]
        if row["coverage_status"].strip() != "exact":
            raise ValueError(f"PR-EQ closure coverage drifted: {relative}")


def require_browser_command(root: Path) -> None:
    package = (root / "package.json").read_text(encoding="utf-8")
    command = f'"test:e2e:pr-eq": "playwright test {BROWSER_PATH}"'
    if command not in package:
        raise ValueError("PR-EQ package browser command drifted")


def check(root: Path) -> None:
    require_roadmap(root)
    require_evidence(root)
    require_runnable_anchors(root)
    require_registry_and_fixtures(root)
    require_browser_command(root)
    require_closure_coverage(root)


def copy_fixture(root: Path, target: Path) -> None:
    for relative in CLOSURE_PATHS:
        source = root / relative
        destination = target / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, destination)


def run_gate(root: Path, expect_pass: bool) -> None:
    result = subprocess.run(
        ["bash", "scripts/check_pr_eq_completion.sh"],
        cwd=root,
        text=True,
        capture_output=True,
        check=False,
    )
    passed = result.returncode == 0
    if passed != expect_pass:
        detail = (result.stderr or result.stdout).strip()
        raise ValueError(f"PR-EQ destructive self-test expectation failed: {detail}")


def replace_once(path: Path, old: str, new: str, label: str) -> None:
    text = path.read_text(encoding="utf-8")
    updated, count = re.subn(re.escape(old), new, text, count=1)
    if count != 1:
        raise ValueError(f"self-test could not mutate {label}")
    path.write_text(updated, encoding="utf-8")


def self_test(root: Path) -> None:
    mutations = (
        (
            "missing evidence",
            "docs/PRODUCT_AUDIT_EVIDENCE.tsv",
            lambda path: path.write_text(
                "\n".join(
                    line
                    for line in path.read_text(encoding="utf-8").splitlines()
                    if not line.startswith("PR-EQ\tcompiler-cache\t")
                )
                + "\n",
                encoding="utf-8",
            ),
        ),
        (
            "completion status drift",
            "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md",
            lambda path: replace_once(
                path,
                f"| `{PR_TITLE}` | {COMPLETED_STATUS} |",
                f"| `{PR_TITLE}` | \U0001f7e1 \u90e8\u5206\u5b8c\u6210 |",
                "completion status",
            ),
        ),
        (
            "queued PR-EQ",
            "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md",
            lambda path: replace_once(
                path,
                "### \U0001f7e1 6.5 \u4e0b\u4e00\u6b65\u6267\u884c\u961f\u5217",
                "### \U0001f7e1 6.5 \u4e0b\u4e00\u6b65\u6267\u884c\u961f\u5217\n\n1. **PR-EQ** - destructive fixture",
                "PR-EQ queue",
            ),
        ),
        (
            "ignored Rust anchor",
            "crates/exchange/src/adapters/hyperliquid_compiler_tests.rs",
            lambda path: replace_once(
                path,
                "#[tokio::test]\nasync fn warmed_builder_compiler_uses_cached_official_metadata",
                "#[tokio::test]\n#[ignore]\nasync fn warmed_builder_compiler_uses_cached_official_metadata",
                "Rust ignore",
            ),
        ),
        (
            "corrupted fixture",
            "crates/exchange/fixtures/hyperliquid/ws_order_updates_finality.json",
            lambda path: path.write_bytes(path.read_bytes() + b"\n"),
        ),
        (
            "skipped browser",
            BROWSER_PATH,
            lambda path: replace_once(
                path,
                'test("PR-EQ renders ticket-owned Hyperliquid builder cloid and protected IOC evidence"',
                'test.skip("PR-EQ renders ticket-owned Hyperliquid builder cloid and protected IOC evidence"',
                "browser skip",
            ),
        ),
        (
            "coverage removal",
            "docs/PRODUCT_AUDIT_COVERAGE.tsv",
            lambda path: path.write_text(
                "\n".join(
                    line
                    for line in path.read_text(encoding="utf-8").splitlines()
                    if not line.startswith("scripts/check_pr_eq_completion.sh\t")
                )
                + "\n",
                encoding="utf-8",
            ),
        ),
    )
    with tempfile.TemporaryDirectory(prefix="crossline-pr-eq-completion-") as temp:
        baseline = Path(temp) / "baseline"
        copy_fixture(root, baseline)
        run_gate(baseline, True)
        for name, relative, mutate in mutations:
            case = Path(temp) / re.sub(r"\W+", "-", name)
            shutil.copytree(baseline, case)
            mutate(case / relative)
            run_gate(case, False)

    print("OK PR-EQ completion destructive self-test")


try:
    root = Path(sys.argv[1])
    if sys.argv[2] == "--self-test":
        self_test(root)
    else:
        check(root)
        print(
            "OK PR-EQ completion gate "
            f"({len(EVIDENCE)} evidence types, {len(RUST_ANCHORS)} Rust anchors, "
            f"{len(BROWSER_ANCHORS)} browser anchors, {len(CLOSURE_PATHS)} exact paths)"
        )
except (OSError, ValueError) as exc:
    raise SystemExit(f"PR-EQ completion gate failed: {exc}") from exc
PY
