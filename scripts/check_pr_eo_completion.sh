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


PR_TITLE = "PR-EO KuCoin Futures Order, Account & Private WS Semantics Contract"
EVIDENCE = {
    "scoped-position-mode-preflight": (
        "crates/api/src/services/hedge_preflight/tests/cases_account.rs",
        "account_mode_guard_records_kucoin_scope",
    ),
    "native-contract-order-compiler": (
        "crates/exchange/src/adapters/kucoin_instruments_tests.rs",
        "official_matrix_maps_usdt_usdc_and_verified_equity_contracts",
    ),
    "cancel-identity-finality-reconciliation": (
        "crates/exchange/src/adapters/kucoin_private_rest.rs",
        "cancel_target_prefers_exchange_order_id_then_client_oid_fallback",
    ),
    "private-rest-parser-registry": (
        "crates/exchange/src/venue_spec.rs",
        "kucoin_pr_eo_private_evidence_is_recorded",
    ),
    "private-fill-fee-ledger": (
        "crates/exchange/src/adapters/kucoin_private_data_tests.rs",
        "kucoin_fills_parse_official_fixture_without_defaulting_fee",
    ),
    "classic-ws-finality-ledger": (
        "crates/api/src/trading_service/private_ws_mapper/tests/cases_kucoin.rs",
        "kucoin_classic_match_projects_durable_fill_once",
    ),
    "private-ws-durable-health": (
        "crates/api/src/lifecycle/private_ws/tests/kucoin_finality.rs",
        "kucoin_terminal_fill_projects_execution_run_once_after_durable_ack",
    ),
    "pro-ws-beta-fail-closed": (
        "crates/exchange/tests/ws_trading_specs_test.rs",
        "kucoin_pro_ws_beta_stays_unavailable_without_runtime_evidence",
    ),
    "official-fixture-operation-registry": (
        "crates/exchange/src/venue_spec.rs",
        "kucoin_pr_eo_private_evidence_is_recorded",
    ),
    "product-browser-and-completion-governance": (
        "scripts/check_pr_eo_completion.sh",
        "bash scripts/check_pr_eo_completion.sh --self-test",
    ),
}

RUST_ANCHORS = (
    ("crates/api/src/services/hedge_preflight/tests/cases_account.rs", "account_mode_guard_records_kucoin_scope"),
    ("crates/exchange/src/adapters/kucoin_instruments_tests.rs", "official_matrix_maps_usdt_usdc_and_verified_equity_contracts"),
    ("crates/exchange/src/adapters/kucoin_contracts_tests.rs", "metadata_resolution_is_quote_aware_and_ambiguous_base_fails_closed"),
    ("crates/exchange/src/adapters/kucoin_private_rest.rs", "cancel_target_prefers_exchange_order_id_then_client_oid_fallback"),
    ("crates/exchange/src/adapters/kucoin_private_data_tests.rs", "kucoin_fills_parse_official_fixture_without_defaulting_fee"),
    ("crates/exchange/src/adapters/kucoin_private_data_tests.rs", "kucoin_fee_rate_parses_official_fixture_and_source_url"),
    ("crates/exchange/src/adapters/kucoin_ws_user_tests.rs", "duplicate_match_fixture_keeps_stable_fill_identity"),
    ("crates/exchange/src/venue_spec.rs", "kucoin_pr_eo_private_evidence_is_recorded"),
    ("crates/exchange/tests/ws_trading_specs_test.rs", "kucoin_classic_fill_stream_records_identity_evidence_without_invented_fee"),
    ("crates/exchange/tests/ws_trading_specs_test.rs", "kucoin_pro_ws_beta_stays_unavailable_without_runtime_evidence"),
    ("crates/api/src/trading_service/private_ws_mapper/tests/cases_kucoin.rs", "kucoin_classic_match_projects_durable_fill_once"),
    ("crates/api/src/lifecycle/private_ws/tests/kucoin_finality.rs", "kucoin_terminal_fill_projects_execution_run_once_after_durable_ack"),
)

BROWSER_ANCHORS = (
    "PR-EO renders verified KuCoin native multiplier identity and Classic finality",
    "PR-EO exposes missing KuCoin live credential evidence and blocks submit",
    "PR-EO KuCoin Pro WS production schema remains runtime-gated",
)

CLOSURE_PATHS = tuple("""
crates/api/src/lifecycle/private_ws/tests.rs
crates/api/src/lifecycle/private_ws/tests/kucoin_finality.rs
crates/api/src/routers/trading/tests/cases_registry.rs
crates/api/src/services/hedge_preflight/tests/cases_account.rs
crates/api/src/services/venue_credentials/validation/venues.rs
crates/api/src/services/venue_operation_health/snapshot/part_05.rs
crates/api/src/services/venue_operation_health/snapshot/part_06.rs
crates/api/src/services/venue_operation_health/snapshot/tests/part_05.rs
crates/api/src/trading_service/private_ws_mapper/gate_kucoin.rs
crates/api/src/trading_service/private_ws_mapper/tests/cases_kucoin.rs
crates/exchange/fixtures/kucoin/classic_ws_trade_orders_canceled.json
crates/exchange/fixtures/kucoin/classic_ws_trade_orders_filled.json
crates/exchange/fixtures/kucoin/classic_ws_trade_orders_match.json
crates/exchange/fixtures/kucoin/contracts_active_native_matrix.json
crates/exchange/fixtures/kucoin/fills_by_order_id.json
crates/exchange/fixtures/kucoin/futures_actual_fee_xbtusdtm.json
crates/exchange/src/adapters/kucoin.rs
crates/exchange/src/adapters/kucoin_contracts.rs
crates/exchange/src/adapters/kucoin_contracts_tests.rs
crates/exchange/src/adapters/kucoin_instruments.rs
crates/exchange/src/adapters/kucoin_instruments_tests.rs
crates/exchange/src/adapters/kucoin_market_data.rs
crates/exchange/src/adapters/kucoin_market_data_tests.rs
crates/exchange/src/adapters/kucoin_private_data.rs
crates/exchange/src/adapters/kucoin_private_data_tests.rs
crates/exchange/src/adapters/kucoin_private_rest.rs
crates/exchange/src/adapters/kucoin_tests.rs
crates/exchange/src/adapters/kucoin_trade_data.rs
crates/exchange/src/adapters/kucoin_trade_data_tests.rs
crates/exchange/src/adapters/kucoin_ws_user.rs
crates/exchange/src/adapters/kucoin_ws_user_data.rs
crates/exchange/src/adapters/kucoin_ws_user_tests.rs
crates/exchange/src/venue_spec.rs
crates/exchange/src/ws/trading.rs
crates/exchange/tests/kucoin_test.rs
crates/exchange/tests/ws_trading_specs_test.rs
frontend/src/panels/modules/settings/tabs/venue_credentials.rs
frontend/src/panels/modules/settings/tabs/venue_credentials/format.rs
frontend/src/panels/modules/settings/tabs/venue_credentials/tests_credentials.rs
package.json
scripts/check_exchange_evidence_debt.sh
scripts/check_exchange_operation_evidence_matrix.sh
scripts/check_pr_eo_completion.sh
scripts/check_product_audit_evidence_index.sh
scripts/exchange_evidence_debt_allowlist.tsv
shared-types/src/exchange_ws.rs
shared-types/src/lib.rs
shared-types/src/transport_registry.rs
test/e2e/mock_api.mjs
test/e2e/pr_eo_kucoin_pro_ws.spec.ts
docs/PRODUCT_FULL_AUDIT_REFINEMENT.md
docs/audit_history/PRODUCT_AUDIT_HISTORY.md
docs/PRODUCT_AUDIT_EVIDENCE.tsv
docs/PRODUCT_AUDIT_COVERAGE.tsv
""".split())

FIXTURES = {
    "crates/exchange/fixtures/kucoin/classic_ws_trade_orders_canceled.json": "69469e77cd9f7ed3d6b4ab24b13e8a2522b11018fa9cfa4aac89e47239d55810",
    "crates/exchange/fixtures/kucoin/classic_ws_trade_orders_filled.json": "958d5674e0b2ea9d2f87effbe1293b6d9ca3d6d41860d2f5c66877f2bc2cf3df",
    "crates/exchange/fixtures/kucoin/classic_ws_trade_orders_match.json": "25e4efb4849b0eb4ffbaab5b29391f6ec9a1982e5ba01abd270210210fc26980",
    "crates/exchange/fixtures/kucoin/contracts_active_native_matrix.json": "612f1509f278052c31888787baae9f3556086bff1280ee430f8e992aff641d7c",
    "crates/exchange/fixtures/kucoin/fills_by_order_id.json": "0d9814f361e3636aaa92f17c525dbc75081a65f952cf8e888b9e45d8dd7ce985",
    "crates/exchange/fixtures/kucoin/futures_actual_fee_xbtusdtm.json": "118317e838e199c08f627b38c4ba95a1870ef5807379d5c4b21afef2f856047d",
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
    matched = [line for line in section(text, "6.3").splitlines() if line.startswith("| `PR-EO ")]
    if len(matched) != 1:
        raise ValueError(f"expected one PR-EO roadmap row, found {len(matched)}")
    cells = [cell.strip() for cell in matched[0].strip().strip("|").split("|")]
    if len(cells) != 4 or cells[0].strip("`") != PR_TITLE:
        raise ValueError("PR-EO roadmap identity drifted")
    if cells[1] != "✅ 完成" or "剩余：无。" not in cells[2]:
        raise ValueError("PR-EO must be completed with no local remainder")
    if "bash scripts/check_pr_eo_completion.sh --self-test" not in cells[3]:
        raise ValueError("PR-EO roadmap row lacks completion self-test anchor")
    if re.search(r"(?m)^\d+\.\s+\*\*PR-EO\*\*", section(text, "6.5")):
        raise ValueError("completed PR-EO remains in the local queue")


def require_evidence(root: Path) -> None:
    selected = [row for row in rows(
        root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv",
        ("pr_id", "evidence_type", "artifact", "command", "notes"),
    ) if row["pr_id"].strip() == "PR-EO"]
    indexed = {row["evidence_type"].strip(): row for row in selected}
    if len(indexed) != len(selected) or set(indexed) != set(EVIDENCE):
        raise ValueError(f"PR-EO evidence type drift: expected={sorted(EVIDENCE)}, actual={sorted(indexed)}")
    for evidence_type, (artifact, anchor) in EVIDENCE.items():
        row = indexed[evidence_type]
        if row["artifact"].strip() != artifact or anchor not in row["command"]:
            raise ValueError(f"PR-EO {evidence_type} artifact/command drifted")
        if not (root / artifact).is_file():
            raise ValueError(f"missing PR-EO artifact {artifact}")
    if "live_credentials_claimed=false" not in indexed["product-browser-and-completion-governance"]["notes"]:
        raise ValueError("PR-EO browser evidence must disclaim live credentials")


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
    browser = (root / "test/e2e/pr_eo_kucoin_pro_ws.spec.ts").read_text(encoding="utf-8")
    if re.search(r"\b(?:test|describe)\.(?:skip|fixme)|FIXME", browser):
        raise ValueError("PR-EO browser gate contains skip/fixme")
    for title in BROWSER_ANCHORS:
        if f'test("{title}"' not in browser:
            raise ValueError(f"missing non-skipping browser anchor: {title}")


def require_registry_and_fixtures(root: Path) -> None:
    venue = (root / "crates/exchange/src/venue_spec.rs").read_text(encoding="utf-8")
    ws = (root / "crates/exchange/src/ws/trading.rs").read_text(encoding="utf-8")
    for marker in ("/api/v1/fills", "/api/v1/trade-fees", "/api/v1/orders/{orderId}"):
        if marker not in venue:
            raise ValueError(f"KuCoin registry missing {marker}")
    for marker in ("KUCOIN_ORDER_WS_EVIDENCE", "ws_evidence_requires_authenticated_runtime", "官方生产 schema 已发布"):
        if marker not in ws:
            raise ValueError(f"KuCoin Pro WS release gate missing {marker}")
    for relative, expected in FIXTURES.items():
        actual = hashlib.sha256((root / relative).read_bytes()).hexdigest()
        if actual != expected:
            raise ValueError(f"KuCoin fixture hash drifted: {relative}")


def require_closure_paths(root: Path) -> None:
    coverage = rows(
        root / "docs/PRODUCT_AUDIT_COVERAGE.tsv",
        ("file", "coverage_status", "owner_surface", "risk", "suggested_pr", "evidence", "notes"),
    )
    by_file = {row["file"].strip(): row for row in coverage}
    for relative in CLOSURE_PATHS:
        if not (root / relative).is_file():
            raise ValueError(f"missing exact PR-EO closure path: {relative}")
        if relative.startswith(("docs/", "crates/exchange/fixtures/")) or relative == "package.json":
            continue
        row = by_file.get(relative)
        if row is None or row["coverage_status"].strip() != "exact":
            raise ValueError(f"PR-EO closure path lacks exact coverage: {relative}")


def check(root: Path) -> None:
    require_roadmap(root)
    require_evidence(root)
    require_runnable_anchors(root)
    require_registry_and_fixtures(root)
    require_closure_paths(root)


def run_fixture(root: Path, expect_pass: bool) -> None:
    result = subprocess.run(
        ["bash", "scripts/check_pr_eo_completion.sh"],
        cwd=root,
        text=True,
        capture_output=True,
        check=False,
    )
    if expect_pass != (result.returncode == 0):
        detail = (result.stderr or result.stdout).strip()
        raise ValueError(f"PR-EO destructive self-test expectation failed: {detail}")


def self_test(root: Path) -> None:
    with tempfile.TemporaryDirectory(prefix="crossline-pr-eo-completion-") as temp:
        fixture = Path(temp) / "repo"
        for relative in CLOSURE_PATHS:
            source = root / relative
            target = fixture / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, target)
        run_fixture(fixture, True)

        ledger = fixture / "docs/PRODUCT_AUDIT_EVIDENCE.tsv"
        baseline = ledger.read_text(encoding="utf-8")
        ledger.write_text("\n".join(line for line in baseline.splitlines() if not line.startswith("PR-EO\tnative-contract-order-compiler\t")) + "\n", encoding="utf-8")
        run_fixture(fixture, False)
        ledger.write_text(baseline, encoding="utf-8")

        doc = fixture / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
        doc_baseline = doc.read_text(encoding="utf-8")
        doc.write_text(doc_baseline.replace("| `PR-EO KuCoin Futures Order, Account & Private WS Semantics Contract` | ✅ 完成 |", "| `PR-EO KuCoin Futures Order, Account & Private WS Semantics Contract` | 🟡 部分完成 |", 1), encoding="utf-8")
        run_fixture(fixture, False)
        doc.write_text(doc_baseline.replace("### 🟡 6.5 下一步执行队列", "### 🟡 6.5 下一步执行队列\n\n1. **PR-EO** — destructive fixture", 1), encoding="utf-8")
        run_fixture(fixture, False)
        doc.write_text(doc_baseline, encoding="utf-8")

        anchor = fixture / "crates/exchange/src/adapters/kucoin_instruments_tests.rs"
        anchor_baseline = anchor.read_text(encoding="utf-8")
        anchor.write_text(anchor_baseline.replace("#[test]\nfn official_matrix_maps_usdt_usdc_and_verified_equity_contracts", "#[test]\n#[ignore]\nfn official_matrix_maps_usdt_usdc_and_verified_equity_contracts", 1), encoding="utf-8")
        run_fixture(fixture, False)
        anchor.write_text(anchor_baseline, encoding="utf-8")

        browser = fixture / "test/e2e/pr_eo_kucoin_pro_ws.spec.ts"
        browser_baseline = browser.read_text(encoding="utf-8")
        browser.write_text(browser_baseline.replace('test("PR-EO KuCoin Pro WS production schema', 'test.skip("PR-EO KuCoin Pro WS production schema', 1), encoding="utf-8")
        run_fixture(fixture, False)
        browser.write_text(browser_baseline, encoding="utf-8")

        fixture_path = fixture / "crates/exchange/fixtures/kucoin/fills_by_order_id.json"
        fixture_path.write_text("{}\n", encoding="utf-8")
        run_fixture(fixture, False)

    print("OK PR-EO completion destructive self-test")


try:
    if sys.argv[2] == "--self-test":
        self_test(Path(sys.argv[1]))
    else:
        check(Path(sys.argv[1]))
        print(f"OK PR-EO completion gate ({len(EVIDENCE)} evidence types, {len(RUST_ANCHORS)} Rust anchors, {len(BROWSER_ANCHORS)} browser anchors, {len(CLOSURE_PATHS)} exact paths)")
except (OSError, ValueError) as exc:
    raise SystemExit(f"PR-EO completion gate failed: {exc}") from exc
PY
