#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"

if [ "$MODE" != "check" ] && [ "$MODE" != "--self-test" ]; then
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


PR_TITLE = "PR-EP HTX USDT-M Swap Order, Account & Private WS Semantics Contract"
EVIDENCE = {
    "account-position-order-preflight": (
        "crates/exchange/tests/htx_compiler_test.rs",
        "place_uses_verified_native_contract_size_and_position_mode",
    ),
    "native-contract-order-compiler": (
        "crates/exchange/tests/htx_compiler_test.rs",
        "preflight_cold_cache_uses_native_alias_and_cross_flat_context",
    ),
    "market-like-offset-order-compiler": (
        "crates/exchange/src/adapters/htx_trade_data_tests.rs",
        "market_like_styles_cover_official_bbo_and_optimal_families",
    ),
    "private-rest-parser-registry": (
        "crates/exchange/src/venue_spec.rs",
        "htx_pr_ep_private_trade_rest_registry_contains_recorded_core",
    ),
    "notification-ws-fill-fee-ledger": (
        "crates/api/src/trading_service/private_ws_mapper/tests/cases_htx.rs",
        "htx_settled_order_maps_stable_fill_identity_and_reported_fee",
    ),
    "private-ws-durable-finality-health": (
        "crates/api/src/lifecycle/private_ws/tests/htx_finality.rs",
        "htx_terminal_fill_projects_execution_run_once_after_durable_ack",
    ),
    "trade-ws-authenticated-runtime-fail-closed": (
        "crates/exchange/tests/ws_trading_specs_test.rs",
        "htx_schema_or_announcement_cannot_enable_live_submit_without_authenticated_runtime",
    ),
    "official-fixture-operation-registry": (
        "crates/exchange/src/venue_spec.rs",
        "htx_pr_ep_private_trade_rest_registry_contains_recorded_core",
    ),
    "product-browser-and-completion-governance": (
        "scripts/check_pr_ep_completion.sh",
        "bash scripts/check_pr_ep_completion.sh --self-test",
    ),
}

RUST_ANCHORS = (
    ("crates/exchange/tests/htx_compiler_test.rs", "preflight_cold_cache_uses_native_alias_and_cross_flat_context"),
    ("crates/exchange/tests/htx_compiler_test.rs", "isolated_preflight_uses_contract_code_context_body"),
    ("crates/exchange/tests/htx_compiler_test.rs", "place_uses_verified_native_contract_size_and_position_mode"),
    ("crates/exchange/tests/htx_compiler_test.rs", "cancel_cold_cache_uses_resolved_native_symbol"),
    ("crates/exchange/tests/htx_compiler_test.rs", "client_order_lookup_cold_cache_uses_resolved_native_symbol"),
    ("crates/exchange/tests/htx_compiler_test.rs", "exchange_order_lookup_cold_cache_uses_native_order_id"),
    ("crates/exchange/tests/htx_compiler_test.rs", "generic_market_is_blocked_before_any_network_request"),
    ("crates/exchange/tests/htx_test.rs", "live_place_order_sends_signed_swap_order"),
    ("crates/exchange/tests/htx_test.rs", "live_preflight_order_accepts_contract_status_and_position_context"),
    ("crates/exchange/src/adapters/htx_contracts_tests.rs", "htx_swap_contract_info_parses_official_fixture_metadata"),
    ("crates/exchange/src/adapters/htx_instruments_tests.rs", "htx_instrument_rule_parses_official_contract_fixture"),
    ("crates/exchange/src/adapters/htx_trade_data_tests.rs", "single_side_compiles_both_offset_and_reduce_only"),
    ("crates/exchange/src/adapters/htx_trade_data_tests.rs", "market_like_styles_cover_official_bbo_and_optimal_families"),
    ("crates/exchange/src/adapters/htx_private_data_tests.rs", "parse_positions_maps_position_and_margin_mode"),
    ("crates/exchange/src/adapters/htx_ws_user_tests.rs", "settled_order_rejects_missing_trade_identity_or_fee"),
    ("crates/exchange/src/adapters/htx_ws_user_tests.rs", "match_order_with_unknown_fee_is_ignored_until_settled_order_arrives"),
    ("crates/exchange/src/adapters/htx_ws_user_tests.rs", "ws_order_update_rejects_unknown_status"),
    ("crates/exchange/src/adapters/htx_ws_user_tests.rs", "ws_order_update_accepts_pending_status"),
    ("crates/exchange/src/adapters/htx_ws_user_tests.rs", "ws_position_update_rejects_unknown_direction"),
    ("crates/api/src/trading_service/private_ws_mapper/tests/cases_htx.rs", "htx_settled_order_maps_stable_fill_identity_and_reported_fee"),
    ("crates/api/src/trading_service/private_ws_mapper/tests/cases_htx.rs", "htx_match_notification_does_not_emit_fee_less_duplicate_fill"),
    ("crates/api/src/lifecycle/private_ws/tests/htx_finality.rs", "htx_terminal_fill_projects_execution_run_once_after_durable_ack"),
    ("crates/exchange/src/venue_spec.rs", "htx_pr_ep_private_trade_rest_registry_contains_recorded_core"),
    ("crates/exchange/tests/ws_trading_specs_test.rs", "htx_schema_or_announcement_cannot_enable_live_submit_without_authenticated_runtime"),
    ("crates/exchange/tests/kucoin_test.rs", "live_place_order_rechecks_position_change_and_submits_zero_orders"),
    ("crates/exchange/tests/kucoin_test.rs", "live_place_order_blocks_missing_margin_mode_evidence"),
    ("crates/api/src/routers/arbitrage/hedge_tests/position_evidence.rs", "kucoin_current_position_matching_ticket_passes"),
    ("crates/api/src/routers/arbitrage/hedge_tests/position_evidence.rs", "kucoin_current_position_conflict_blocks_ticket"),
    ("crates/api/src/routers/arbitrage/hedge_tests/position_evidence.rs", "kucoin_current_position_missing_margin_mode_fails_visibly"),
    ("crates/api/src/routers/arbitrage/hedge_tests/position_evidence.rs", "kucoin_opening_without_current_position_stays_allowed"),
)

BROWSER_PATH = "test/e2e/pr_ep_htx_ws_gate.spec.ts"
BROWSER_ANCHORS = (
    "PR-EP keeps HTX notification WS distinct from the absent trade WS",
    "PR-EP keeps HTX schema and announcement evidence visible but blocks submit",
    "PR-EP notification runtime health cannot satisfy HTX trade writer readiness",
)

CLOSURE_PATHS = tuple("""
crates/api/src/lifecycle/private_ws/htx.rs
crates/api/src/lifecycle/private_ws/tests.rs
crates/api/src/lifecycle/private_ws/tests/htx_finality.rs
crates/api/src/routers/arbitrage/hedge_tests/position_evidence.rs
crates/api/src/routers/arbitrage/hedge_tests/preview.rs
crates/api/src/services/hedge_preview.rs
crates/api/src/services/hedge_preview/positions.rs
crates/api/src/trading_service/private_ws_mapper/gate_kucoin.rs
crates/api/src/trading_service/private_ws_mapper/hyperliquid.rs
crates/api/src/trading_service/private_ws_mapper/tests/cases_htx.rs
crates/exchange/fixtures/htx/swap_contract_info_execution_matrix.json
crates/exchange/fixtures/htx/swap_contract_info_native_alias.json
crates/exchange/fixtures/htx/swap_order_context_single_side_flat.json
crates/exchange/fixtures/htx/ws_match_orders_cross_filled.json
crates/exchange/fixtures/htx/ws_orders_cross_filled.json
crates/exchange/src/adapters/htx.rs
crates/exchange/src/adapters/htx_contracts.rs
crates/exchange/src/adapters/htx_contracts_tests.rs
crates/exchange/src/adapters/htx_instruments.rs
crates/exchange/src/adapters/htx_instruments_tests.rs
crates/exchange/src/adapters/htx_market_data.rs
crates/exchange/src/adapters/htx_order_context.rs
crates/exchange/src/adapters/htx_private_data.rs
crates/exchange/src/adapters/htx_private_data_tests.rs
crates/exchange/src/adapters/htx_support.rs
crates/exchange/src/adapters/htx_trade_data.rs
crates/exchange/src/adapters/htx_trade_data_tests.rs
crates/exchange/src/adapters/htx_ws_user.rs
crates/exchange/src/adapters/htx_ws_user_data.rs
crates/exchange/src/adapters/htx_ws_user_tests.rs
crates/exchange/src/adapters/kucoin.rs
crates/exchange/src/venue_spec.rs
crates/exchange/src/ws/trading.rs
crates/exchange/tests/htx_compiler_test.rs
crates/exchange/tests/htx_test.rs
crates/exchange/tests/kucoin_test.rs
crates/exchange/tests/ws_trading_specs_test.rs
frontend/src/panels/modules/settings/tabs/venue_credentials/format.rs
frontend/src/panels/modules/settings/tabs/venue_credentials/tests_credentials.rs
frontend/src/panels/modules/settings/tabs/venue_credentials/ws.rs
package.json
scripts/check_exchange_evidence_debt.sh
scripts/check_exchange_operation_evidence_matrix.sh
scripts/check_pr_ep_completion.sh
scripts/check_product_audit_evidence_index.sh
scripts/exchange_evidence_debt_allowlist.tsv
shared-types/src/exchange_ws.rs
shared-types/src/transport_registry.rs
test/e2e/mock_api.mjs
test/e2e/pr_ep_htx_ws_gate.spec.ts
docs/PRODUCT_FULL_AUDIT_REFINEMENT.md
docs/audit_history/PRODUCT_AUDIT_HISTORY.md
docs/PRODUCT_AUDIT_EVIDENCE.tsv
docs/PRODUCT_AUDIT_COVERAGE.tsv
""".split())

FIXTURES = {
    "crates/exchange/fixtures/htx/swap_contract_info_execution_matrix.json": "98cb96e77663435b3274c25ebcd8311024b257b563d963ead5be9f806c5c6f47",
    "crates/exchange/fixtures/htx/swap_contract_info_native_alias.json": "f90a8be75079df454d05d9d9f6be477edf342ed1dbed43f50b6c6bddc3c807d3",
    "crates/exchange/fixtures/htx/swap_order_context_single_side_flat.json": "adc50bc759f6350770610ce453208dae3a963e61bd3a8375e57efff20b7c770a",
    "crates/exchange/fixtures/htx/ws_match_orders_cross_filled.json": "a9ba6b769e04b3752508afb5dc265150c7158ddaf1aaaef871da09bac2c51dae",
    "crates/exchange/fixtures/htx/ws_orders_cross_filled.json": "565c32b583056349283bce2bccef4bf27d89f37ca363c3de8bcc9ff70eb4a40c",
}


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
    following = re.search(r"(?m)^### ", text[start.end():])
    end = start.end() + following.start() if following else len(text)
    return text[start.start():end]


def require_roadmap(root: Path) -> None:
    text = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
    matched = [line for line in section(text, "6.3").splitlines() if line.startswith("| `PR-EP ")]
    if len(matched) != 1:
        raise ValueError(f"expected one PR-EP roadmap row, found {len(matched)}")
    cells = [cell.strip() for cell in matched[0].strip().strip("|").split("|")]
    if len(cells) != 4 or cells[0].strip("`") != PR_TITLE:
        raise ValueError("PR-EP roadmap identity drifted")
    if cells[1] != "✅ 完成" or "剩余：无。" not in cells[2]:
        raise ValueError("PR-EP must be completed with no local remainder")
    if "bash scripts/check_pr_ep_completion.sh --self-test" not in cells[3]:
        raise ValueError("PR-EP roadmap row lacks completion self-test anchor")
    if re.search(r"(?m)^\d+\.\s+\*\*PR-EP\*\*", section(text, "6.5")):
        raise ValueError("completed PR-EP remains in the local queue")
    waiting = section(text, "6.5")
    for marker in ("真实 HTX auth", "live trade", "finality"):
        if marker not in waiting:
            raise ValueError(f"PR-EP external waiting pool lacks {marker}")


def require_evidence(root: Path) -> None:
    selected = [row for row in rows(
        root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv",
        ("pr_id", "evidence_type", "artifact", "command", "notes"),
    ) if row["pr_id"].strip() == "PR-EP"]
    indexed = {row["evidence_type"].strip(): row for row in selected}
    if len(indexed) != len(selected) or set(indexed) != set(EVIDENCE):
        raise ValueError(f"PR-EP evidence type drift: expected={sorted(EVIDENCE)}, actual={sorted(indexed)}")
    for evidence_type, (artifact, anchor) in EVIDENCE.items():
        row = indexed[evidence_type]
        if row["artifact"].strip() != artifact or anchor not in row["command"]:
            raise ValueError(f"PR-EP {evidence_type} artifact/command drifted")
        if not (root / artifact).is_file():
            raise ValueError(f"missing PR-EP artifact {artifact}")
    notes = indexed["product-browser-and-completion-governance"]["notes"]
    if "live_credentials_claimed=false" not in notes:
        raise ValueError("PR-EP browser evidence must disclaim live credentials")


def require_runnable_anchors(root: Path) -> None:
    invalid = re.compile(r"#\[\s*(?:ignore|should_panic)|#\[\s*cfg_attr[^\]]*ignore")
    for relative, name in RUST_ANCHORS:
        text = (root / relative).read_text(encoding="utf-8")
        match = re.search(rf"(?m)^\s*(?:async\s+)?fn\s+{re.escape(name)}\s*\(", text)
        if not match:
            raise ValueError(f"missing runnable Rust anchor {relative}::{name}")
        prefix = text[max(0, match.start() - 600):match.start()]
        attrs = "\n".join(line for line in prefix.splitlines()[-14:] if line.strip().startswith("#["))
        if not re.search(r"#\[\s*(?:tokio::)?test(?:\]|\()", attrs) or invalid.search(attrs):
            raise ValueError(f"non-runnable Rust anchor {relative}::{name}")
    browser = (root / BROWSER_PATH).read_text(encoding="utf-8")
    if re.search(r"\b(?:test|describe)\.(?:skip|fixme)|FIXME", browser):
        raise ValueError("PR-EP browser gate contains skip/fixme")
    titles = re.findall(r'(?m)^test\("([^"]+)"', browser)
    if tuple(titles) != BROWSER_ANCHORS:
        raise ValueError(f"PR-EP must contain exactly three ordered browser anchors: {titles}")


def require_registry_and_fixtures(root: Path) -> None:
    venue = (root / "crates/exchange/src/venue_spec.rs").read_text(encoding="utf-8")
    ws = (root / "crates/exchange/src/ws/trading.rs").read_text(encoding="utf-8")
    for marker in (
        "/linear-swap-api/v1/swap_contract_info",
        "/linear-swap-api/v1/swap_cross_account_position_info",
        "/linear-swap-api/v1/swap_cross_order",
        "/linear-swap-api/v1/swap_cross_cancel",
        "/linear-swap-api/v1/swap_cross_order_info",
    ):
        if marker not in venue:
            raise ValueError(f"HTX registry missing {marker}")
    for marker in (
        "requires_authenticated_runtime_evidence",
        "authenticated_runtime_evidence",
        "HTX_ORDER_PLACE_WS_EVIDENCE",
        "HTX_ORDER_CANCEL_WS_EVIDENCE",
    ):
        if marker not in ws:
            raise ValueError(f"HTX trade WS runtime gate missing {marker}")
    for relative, expected in FIXTURES.items():
        actual = hashlib.sha256((root / relative).read_bytes()).hexdigest()
        if actual != expected:
            raise ValueError(f"HTX fixture hash drifted: {relative}")


def require_closure_paths(root: Path) -> None:
    coverage = rows(
        root / "docs/PRODUCT_AUDIT_COVERAGE.tsv",
        ("file", "coverage_status", "owner_surface", "risk", "suggested_pr", "evidence", "notes"),
    )
    by_file = {row["file"].strip(): row for row in coverage}
    for relative in CLOSURE_PATHS:
        if not (root / relative).is_file():
            raise ValueError(f"missing exact PR-EP closure path: {relative}")
        if relative.startswith(("docs/", "crates/exchange/fixtures/")) or relative == "package.json":
            continue
        row = by_file.get(relative)
        if row is None or row["coverage_status"].strip() != "exact":
            raise ValueError(f"PR-EP closure path lacks exact coverage: {relative}")


def check(root: Path) -> None:
    require_roadmap(root)
    require_evidence(root)
    require_runnable_anchors(root)
    require_registry_and_fixtures(root)
    require_closure_paths(root)


def run_fixture(root: Path, expect_pass: bool) -> None:
    result = subprocess.run(
        ["bash", "scripts/check_pr_ep_completion.sh"],
        cwd=root,
        text=True,
        capture_output=True,
        check=False,
    )
    if expect_pass != (result.returncode == 0):
        detail = (result.stderr or result.stdout).strip()
        raise ValueError(f"PR-EP destructive self-test expectation failed: {detail}")


def stage_completed_roadmap_fixture(root: Path) -> None:
    doc = root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
    text = doc.read_text(encoding="utf-8")
    completed_row = (
        "| `PR-EP HTX USDT-M Swap Order, Account & Private WS Semantics Contract` "
        "| ✅ 完成 | destructive self-test completion fixture；剩余：无。 "
        "| 验证：`bash scripts/check_pr_ep_completion.sh --self-test`。 |"
    )
    text, count = re.subn(
        r"(?m)^\| `PR-EP HTX USDT-M Swap Order, Account & Private WS Semantics Contract` \|.*$",
        completed_row,
        text,
        count=1,
    )
    if count != 1:
        raise ValueError("self-test could not stage completed PR-EP roadmap row")
    if not re.search(r"(?m)^\d+\. \*\*PR-EP\*\*", text):
        text = text.replace(
            "### 🟡 6.5 下一步执行队列",
            "### 🟡 6.5 下一步执行队列\n\n1. **PR-EP** — destructive fixture",
            1,
        )
    text, count = re.subn(
        r"(?m)^\d+\. \*\*PR-EP\*\*.*\n",
        "",
        text,
        count=1,
    )
    if count != 1:
        raise ValueError("self-test could not remove PR-EP from staged queue")
    text = text.replace(
        "### 🟡 6.5 下一步执行队列",
        "### 🟡 6.5 下一步执行队列\n\n"
        "> destructive fixture external waiting: 真实 HTX auth / live trade / finality",
        1,
    )
    doc.write_text(text, encoding="utf-8")


def self_test(root: Path) -> None:
    with tempfile.TemporaryDirectory(prefix="crossline-pr-ep-completion-") as temp:
        fixture = Path(temp) / "repo"
        for relative in CLOSURE_PATHS:
            source = root / relative
            target = fixture / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, target)
        stage_completed_roadmap_fixture(fixture)
        run_fixture(fixture, True)

        ledger = fixture / "docs/PRODUCT_AUDIT_EVIDENCE.tsv"
        baseline = ledger.read_text(encoding="utf-8")
        ledger.write_text("\n".join(line for line in baseline.splitlines() if not line.startswith("PR-EP\tnative-contract-order-compiler\t")) + "\n", encoding="utf-8")
        run_fixture(fixture, False)
        ledger.write_text(baseline, encoding="utf-8")

        doc = fixture / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
        doc_baseline = doc.read_text(encoding="utf-8")
        doc.write_text(doc_baseline.replace("| `PR-EP HTX USDT-M Swap Order, Account & Private WS Semantics Contract` | ✅ 完成 |", "| `PR-EP HTX USDT-M Swap Order, Account & Private WS Semantics Contract` | 🟡 部分完成 |", 1), encoding="utf-8")
        run_fixture(fixture, False)
        doc.write_text(doc_baseline.replace("### 🟡 6.5 下一步执行队列", "### 🟡 6.5 下一步执行队列\n\n1. **PR-EP** — destructive fixture", 1), encoding="utf-8")
        run_fixture(fixture, False)
        doc.write_text(doc_baseline, encoding="utf-8")

        anchor = fixture / "crates/exchange/src/adapters/htx_trade_data_tests.rs"
        anchor_baseline = anchor.read_text(encoding="utf-8")
        anchor.write_text(anchor_baseline.replace("#[test]\nfn market_like_styles_cover_official_bbo_and_optimal_families", "#[test]\n#[ignore]\nfn market_like_styles_cover_official_bbo_and_optimal_families", 1), encoding="utf-8")
        run_fixture(fixture, False)
        anchor.write_text(anchor_baseline, encoding="utf-8")

        browser = fixture / BROWSER_PATH
        browser_baseline = browser.read_text(encoding="utf-8")
        browser.write_text(browser_baseline.replace('test("PR-EP keeps HTX schema', 'test.skip("PR-EP keeps HTX schema', 1), encoding="utf-8")
        run_fixture(fixture, False)
        browser.write_text(browser_baseline + '\ntest("PR-EP unexpected fourth anchor", async () => {});\n', encoding="utf-8")
        run_fixture(fixture, False)
        browser.write_text(browser_baseline, encoding="utf-8")

        fixture_path = fixture / "crates/exchange/fixtures/htx/ws_orders_cross_filled.json"
        fixture_path.write_text("{}\n", encoding="utf-8")
        run_fixture(fixture, False)

        coverage = fixture / "docs/PRODUCT_AUDIT_COVERAGE.tsv"
        coverage_baseline = coverage.read_text(encoding="utf-8")
        coverage.write_text(coverage_baseline.replace("scripts/check_pr_ep_completion.sh\texact\t", "scripts/check_pr_ep_completion.sh\tmissing\t", 1), encoding="utf-8")
        run_fixture(fixture, False)

    print("OK PR-EP completion destructive self-test")


try:
    if sys.argv[2] == "--self-test":
        self_test(Path(sys.argv[1]))
    else:
        check(Path(sys.argv[1]))
        print(f"OK PR-EP completion gate ({len(EVIDENCE)} evidence types, {len(RUST_ANCHORS)} Rust anchors, {len(BROWSER_ANCHORS)} browser anchors, {len(CLOSURE_PATHS)} exact paths)")
except (OSError, ValueError) as exc:
    raise SystemExit(f"PR-EP completion gate failed: {exc}") from exc
PY
