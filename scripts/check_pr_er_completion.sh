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


PR_TITLE = "PR-ER Gate Futures Order, Account & Private WS Semantics Contract"
EVIDENCE = {
    "order-compiler-identity": (
        "crates/exchange/src/adapters/gate_trade_data_tests.rs",
        "place_order_converts_base_qty_to_contracts",
    ),
    "native-settle-instrument-spec": (
        "crates/exchange/src/adapters/gate_contracts_tests.rs",
        "official_fixture_closes_native_identity_and_contract_spec",
    ),
    "contract-fail-closed": (
        "crates/exchange/src/adapters/gate_contracts_tests.rs",
        "canonical_fallback_is_not_executable_until_officially_verified",
    ),
    "finality-reconciliation": (
        "crates/exchange/src/adapters/gate_tests.rs",
        "numeric_order_finality_uses_order_id_and_enriches_fill_fees",
    ),
    "account-maintenance-quality": (
        "crates/api/src/services/account_positions/tests/venue_quality.rs",
        "gate_maintenance_quality_tracks_actual_estimated_and_unknown_provenance",
    ),
    "private-fill-fee-ledger": (
        "crates/exchange/src/adapters/gate_fill_evidence_tests.rs",
        "parses_official_my_trades_fixture_without_combining_fee_units",
    ),
    "private-ws-operation-health": (
        "crates/api/src/services/private_ws_health/tests.rs",
        "gate_subscription_requires_every_server_ack",
    ),
    "official-fixture-registry": (
        "crates/exchange/src/venue_spec.rs",
        "gate_my_trades_evidence_uses_recorded_fixture_metadata",
    ),
    "product-browser": (
        "test/e2e/pr_er_gate_runtime.spec.ts",
        "npm run test:e2e:pr-er",
    ),
    "completion-governance": (
        "scripts/check_pr_er_completion.sh",
        "bash scripts/check_pr_er_completion.sh --self-test",
    ),
}
RUST_ANCHORS = (
    ("crates/exchange/src/adapters/gate_trade_data_tests.rs", "place_order_converts_base_qty_to_contracts"),
    ("crates/exchange/src/adapters/gate_trade_data_tests.rs", "ack_maps_numeric_exchange_order_id"),
    ("crates/exchange/src/adapters/gate_contracts_tests.rs", "official_fixture_closes_native_identity_and_contract_spec"),
    ("crates/exchange/src/adapters/gate_contracts_tests.rs", "endpoint_settle_must_match_native_contract_quote"),
    ("crates/exchange/src/adapters/gate_contracts_tests.rs", "canonical_fallback_is_not_executable_until_officially_verified"),
    ("crates/exchange/src/adapters/gate_contracts_tests.rs", "single_contract_response_identity_mismatch_fails_closed"),
    ("crates/exchange/src/adapters/gate_tests.rs", "numeric_order_finality_uses_order_id_and_enriches_fill_fees"),
    ("crates/api/src/trading_service/tests/reconcile/finality/part_01.rs", "refresh_order_state_prefers_numeric_exchange_order_id"),
    ("crates/exchange/src/adapters/gate_private_data_tests.rs", "gate_get_order_ioc_fixture_preserves_partial_fill_as_terminal_cancel"),
    ("crates/api/src/services/account_positions/tests/venue_quality.rs", "gate_maintenance_quality_tracks_actual_estimated_and_unknown_provenance"),
    ("crates/exchange/src/adapters/gate_private_rest.rs", "rest_position_prefers_average_maintenance_and_current_isolated_leverage"),
    ("crates/exchange/src/adapters/gate_fill_evidence_tests.rs", "parses_official_my_trades_fixture_without_combining_fee_units"),
    ("crates/exchange/src/adapters/gate_private_rest_fill_fee_tests.rs", "my_trades_uses_signed_order_query_and_official_fixture"),
    ("crates/api/src/services/private_ws_health/tests.rs", "ledger_apply_success_is_recorded_only_after_durable_ack"),
    ("crates/api/src/services/private_ws_health/tests.rs", "gate_subscription_requires_every_server_ack"),
    ("crates/api/src/lifecycle/private_ws/tests.rs", "gate_subscription_success_is_server_ack_evidence"),
    ("crates/api/src/lifecycle/private_ws/tests.rs", "gate_fill_health_waits_for_identity_apply_and_durable_ack"),
    ("crates/exchange/src/venue_spec.rs", "gate_my_trades_evidence_uses_recorded_fixture_metadata"),
    ("crates/exchange/src/venue_spec.rs", "gate_futures_fee_evidence_uses_recorded_fixture_metadata"),
    ("crates/exchange/src/adapters/gate_fill_evidence_tests.rs", "rejects_null_native_fee_instead_of_coercing_zero"),
    ("crates/exchange/src/adapters/gate_fee_evidence_tests.rs", "parses_official_fee_fixture_and_preserves_maker_rebate"),
    ("crates/exchange/src/adapters/gate_private_rest_fill_fee_tests.rs", "fee_read_uses_signed_endpoint_and_preserves_rebate"),
)
BROWSER_ANCHORS = (
    "PR-ER Gate diagnostics exposes native contract evidence without implying live credentials",
    "PR-ER Gate private runtime evidence stays unavailable when credentials and live samples are absent",
    "PR-ER Gate AccountState renders missing fee and maintenance evidence fail closed",
)
CLOSURE_PATHS = tuple(
    """
scripts/check_pr_er_completion.sh
scripts/check_product_audit_evidence_index.sh
scripts/check_exchange_evidence_debt.sh
scripts/check_exchange_operation_evidence_matrix.sh
scripts/exchange_evidence_debt_allowlist.tsv
scripts/exchange_operation_evidence_matrix.tsv
scripts/verify_repo_gates.sh
docs/PRODUCT_FULL_AUDIT_REFINEMENT.md
docs/audit_history/PRODUCT_AUDIT_HISTORY.md
docs/PRODUCT_AUDIT_EVIDENCE.tsv
docs/PRODUCT_AUDIT_COVERAGE.tsv
crates/exchange/src/venue_spec.rs
crates/exchange/fixtures/gate/futures_usdt_contracts_btc_usdt.json
crates/exchange/fixtures/gate/futures_usdt_get_order_ioc.json
crates/exchange/fixtures/gate/futures_usdt_my_trades_order.json
crates/exchange/fixtures/gate/futures_usdt_fee.json
crates/exchange/fixtures/gate/futures_usdt_positions.json
crates/exchange/fixtures/gate/ws_futures_order_place_success.json
crates/exchange/fixtures/gate/ws_futures_order_cancel_success.json
crates/exchange/src/adapters/gate.rs
crates/exchange/src/adapters/gate_tests.rs
crates/exchange/src/adapters/gate_contracts.rs
crates/exchange/src/adapters/gate_contracts_tests.rs
crates/exchange/src/adapters/gate_trade_data.rs
crates/exchange/src/adapters/gate_trade_data_tests.rs
crates/exchange/src/adapters/gate_private_rest.rs
crates/exchange/src/adapters/gate_private_rest_fill_fee_tests.rs
crates/exchange/src/adapters/gate_fill_evidence.rs
crates/exchange/src/adapters/gate_fill_evidence_tests.rs
crates/exchange/src/adapters/gate_fee_evidence.rs
crates/exchange/src/adapters/gate_fee_evidence_tests.rs
crates/exchange/src/adapters/gate_private_data.rs
crates/exchange/src/adapters/gate_private_data_tests.rs
crates/exchange/src/adapters/gate_ws_user.rs
crates/exchange/src/adapters/gate_ws_user_data.rs
crates/exchange/src/adapters/gate_ws_user_tests.rs
crates/exchange/src/adapters/gate_ws_trade.rs
crates/exchange/src/adapters/gate_ws_trade_tests.rs
crates/api/src/lifecycle/private_ws/gate.rs
crates/api/src/lifecycle/private_ws/tests.rs
crates/api/src/lifecycle/private_ws/transport.rs
crates/api/src/services/account_positions/projection.rs
crates/api/src/services/account_positions/tests/venue_quality.rs
crates/api/src/services/private_ws_health.rs
crates/api/src/services/private_ws_health/tests.rs
crates/api/src/services/private_ws_health/update.rs
crates/api/src/trading_service/live_adapters/router.rs
crates/api/src/trading_service/live_adapters/route_tests/cases.rs
crates/api/src/trading_service/private_ws_mapper/gate_kucoin.rs
crates/api/src/trading_service/private_ws_mapper/tests/cases_gate_account.rs
crates/api/src/trading_service/submit.rs
crates/api/src/trading_service/tests/adapters.rs
crates/api/src/trading_service/tests/reconcile/finality/part_01.rs
test/e2e/mock_api.mjs
test/e2e/pr_er_gate_runtime.spec.ts
""".split()
)

if len(CLOSURE_PATHS) != 55 or len(set(CLOSURE_PATHS)) != 55:
    raise SystemExit("PR-ER completion gate internal error: expected 55 unique closure paths")


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
    matched = [
        line for line in section(text, "6.3").splitlines()
        if line.startswith("| `PR-ER ")
    ]
    if len(matched) != 1:
        raise ValueError(f"expected one PR-ER roadmap row, found {len(matched)}")
    cells = [cell.strip() for cell in matched[0].strip().strip("|").split("|")]
    if len(cells) != 4 or cells[0].strip("`") != PR_TITLE:
        raise ValueError("PR-ER roadmap identity drifted")
    if cells[1] != "✅ 完成" or "剩余：无。" not in cells[2]:
        raise ValueError("PR-ER must be completed with no local remainder")
    if "bash scripts/check_pr_er_completion.sh --self-test" not in cells[3]:
        raise ValueError("PR-ER roadmap row lacks completion self-test anchor")
    if re.search(r"(?m)^\d+\.\s+\*\*PR-ER\*\*", section(text, "6.5")):
        raise ValueError("completed PR-ER remains in the local queue")


def require_evidence(root: Path) -> None:
    selected = [
        row for row in rows(
            root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv",
            ("pr_id", "evidence_type", "artifact", "command", "notes"),
        )
        if row["pr_id"].strip() == "PR-ER"
    ]
    indexed = {row["evidence_type"].strip(): row for row in selected}
    if len(indexed) != len(selected) or set(indexed) != set(EVIDENCE):
        raise ValueError(
            f"PR-ER evidence type drift: expected={sorted(EVIDENCE)}, actual={sorted(indexed)}"
        )
    for evidence_type, (artifact, anchor) in EVIDENCE.items():
        row = indexed[evidence_type]
        if row["artifact"].strip() != artifact or anchor not in row["command"]:
            raise ValueError(f"PR-ER {evidence_type} artifact/command drifted")
        if not (root / artifact).is_file():
            raise ValueError(f"missing PR-ER artifact {artifact}")
        combined = f'{row["command"]} {row["notes"]}'.lower()
        if re.search(r"fixed[-_ ]?usdt executable fallback|ack[-_ ]?as[-_ ]?final|fees?[-_ ]?zero pseudo", combined):
            raise ValueError(f"PR-ER {evidence_type} contains pseudo evidence")
    if "fallback_not_executable" not in indexed["contract-fail-closed"]["notes"]:
        raise ValueError("contract fail-closed evidence lacks fallback_not_executable")
    if "ack_not_final" not in indexed["finality-reconciliation"]["notes"]:
        raise ValueError("finality evidence lacks ack_not_final")
    if "zero_fee_assumption=false" not in indexed["private-fill-fee-ledger"]["notes"]:
        raise ValueError("fill/fee evidence lacks zero_fee_assumption=false")


def require_runnable_anchors(root: Path) -> None:
    invalid = re.compile(r"#\[\s*(?:ignore|should_panic|cfg(?:_attr)?\s*\()")
    for relative, name in RUST_ANCHORS:
        text = (root / relative).read_text(encoding="utf-8")
        match = re.search(rf"(?m)^\s*(?:async\s+)?fn\s+{re.escape(name)}\s*\(", text)
        if not match:
            raise ValueError(f"missing runnable Rust anchor {relative}::{name}")
        prefix = text[max(0, match.start() - 500):match.start()]
        attrs = "\n".join(line for line in prefix.splitlines()[-12:] if line.strip().startswith("#["))
        if not re.search(r"#\[\s*(?:tokio::)?test(?:\]|\()", attrs) or invalid.search(attrs):
            raise ValueError(f"non-runnable Rust anchor {relative}::{name}")
    browser = (root / "test/e2e/pr_er_gate_runtime.spec.ts").read_text(encoding="utf-8")
    if re.search(r"\b(?:test|describe)\.(?:skip|fixme)|FIXME", browser):
        raise ValueError("PR-ER browser gate contains skip/fixme")
    for title in BROWSER_ANCHORS:
        if f'test("{title}"' not in browser:
            raise ValueError(f"missing non-skipping browser anchor: {title}")


def require_official_registry(root: Path) -> None:
    venue = (root / "crates/exchange/src/venue_spec.rs").read_text(encoding="utf-8")
    required = (
        "/api/v4/futures/usdt/my_trades",
        "/api/v4/futures/usdt/fee",
        "gate-apiv4-v4.105.32-query-personal-trading-records-2026-07-11",
        "gate-apiv4-v4.105.32-query-futures-market-trading-fee-rates-2026-07-11",
        "query-personal-trading-records",
        "query-futures-market-trading-fee-rates",
        "GATE_MY_TRADES_SCHEMA_HASH",
        "GATE_FUTURES_FEE_SCHEMA_HASH",
        "auth_kind: SIGNED_AUTH_KIND",
    )
    for marker in required:
        if marker not in venue:
            raise ValueError(f"Gate official registry missing {marker}")
    fixtures = {
        "crates/exchange/fixtures/gate/futures_usdt_my_trades_order.json": "5fc511e1c98816438ef07b39f8c1e9bdb1f3d911d8244605aa07a49e8b1412b9",
        "crates/exchange/fixtures/gate/futures_usdt_fee.json": "8ad7680d9bbf1abf952dfb6158ab84a169a04c3fd580da1197613c504bdc5bcb",
    }
    for relative, expected in fixtures.items():
        actual = hashlib.sha256((root / relative).read_bytes()).hexdigest()
        if actual != expected:
            raise ValueError(f"Gate official fixture hash drifted: {relative}")


def require_closure_paths(root: Path) -> None:
    coverage = rows(
        root / "docs/PRODUCT_AUDIT_COVERAGE.tsv",
        ("file", "coverage_status", "owner_surface", "risk", "suggested_pr", "evidence", "notes"),
    )
    by_file = {row["file"].strip(): row for row in coverage}
    for relative in CLOSURE_PATHS:
        if not (root / relative).is_file():
            raise ValueError(f"missing exact PR-ER closure path: {relative}")
        if relative.startswith(("docs/", "crates/exchange/fixtures/")):
            continue
        row = by_file.get(relative)
        if row is None or row["coverage_status"].strip() != "exact":
            raise ValueError(f"PR-ER closure path lacks exact coverage: {relative}")


def check(root: Path) -> None:
    require_roadmap(root)
    require_evidence(root)
    require_runnable_anchors(root)
    require_official_registry(root)
    require_closure_paths(root)


def run_fixture(root: Path, expect_pass: bool) -> None:
    result = subprocess.run(
        ["bash", "scripts/check_pr_er_completion.sh"],
        cwd=root,
        text=True,
        capture_output=True,
        check=False,
    )
    if expect_pass != (result.returncode == 0):
        detail = (result.stderr or result.stdout).strip()
        raise ValueError(f"PR-ER destructive self-test expectation failed: {detail}")


def self_test(root: Path) -> None:
    with tempfile.TemporaryDirectory(prefix="crossline-pr-er-completion-") as temp:
        fixture = Path(temp) / "repo"
        for relative in CLOSURE_PATHS:
            source = root / relative
            target = fixture / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, target)
        run_fixture(fixture, True)

        ledger = fixture / "docs/PRODUCT_AUDIT_EVIDENCE.tsv"
        baseline = ledger.read_text(encoding="utf-8")
        for evidence_type in EVIDENCE:
            ledger.write_text(
                "\n".join(
                    line for line in baseline.splitlines()
                    if not line.startswith(f"PR-ER\t{evidence_type}\t")
                ) + "\n",
                encoding="utf-8",
            )
            run_fixture(fixture, False)
        ledger.write_text(baseline, encoding="utf-8")

        doc = fixture / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
        doc_baseline = doc.read_text(encoding="utf-8")
        doc.write_text(doc_baseline.replace("| `PR-ER Gate Futures Order, Account & Private WS Semantics Contract` | ✅ 完成 |", "| `PR-ER Gate Futures Order, Account & Private WS Semantics Contract` | 🟡 部分完成 |", 1), encoding="utf-8")
        run_fixture(fixture, False)
        doc.write_text(doc_baseline.replace("### 🟡 6.5 下一步执行队列", "### 🟡 6.5 下一步执行队列\n\n1. **PR-ER** — destructive fixture", 1), encoding="utf-8")
        run_fixture(fixture, False)
        doc.write_text(doc_baseline, encoding="utf-8")

        anchor = fixture / "crates/exchange/src/adapters/gate_trade_data_tests.rs"
        anchor_baseline = anchor.read_text(encoding="utf-8")
        anchor.write_text(anchor_baseline.replace("#[test]\nfn place_order_converts_base_qty_to_contracts", "#[test]\n#[ignore]\nfn place_order_converts_base_qty_to_contracts", 1), encoding="utf-8")
        run_fixture(fixture, False)
        anchor.write_text(anchor_baseline, encoding="utf-8")

        fixture_path = fixture / "crates/exchange/fixtures/gate/futures_usdt_fee.json"
        fixture_path.write_text("{}\n", encoding="utf-8")
        run_fixture(fixture, False)

    print("OK PR-ER completion destructive self-test")


try:
    if sys.argv[2] == "--self-test":
        self_test(Path(sys.argv[1]))
    else:
        check(Path(sys.argv[1]))
        print(f"OK PR-ER completion gate ({len(EVIDENCE)} evidence types, {len(RUST_ANCHORS)} Rust anchors, {len(BROWSER_ANCHORS)} browser anchors, {len(CLOSURE_PATHS)} exact paths)")
except (OSError, ValueError) as exc:
    raise SystemExit(f"PR-ER completion gate failed: {exc}") from exc
PY
