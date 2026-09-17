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
import re
import shutil
import sys
import tempfile


PR_TITLE = "PR-EL Binance USD-M Order, Account & User Stream Semantics Contract"
EVIDENCE = {
    "native-symbol-usdc-registry": (
        "crates/exchange/src/adapters/binance_exchange_info_tests.rs",
        "registry_projection_uses_compiled_usdt_and_usdc_specs",
    ),
    "order-identity-contract": (
        "shared-types/src/order_identity.rs",
        "verified_usdc_native_identity_is_execution_ready",
    ),
    "client-order-id-fail-closed": (
        "crates/exchange/src/adapters/binance_trade_data_tests.rs",
        "rest_place_order_validates_official_client_order_id_rule",
    ),
    "private-rest-parser-registry": (
        "crates/exchange/src/venue_spec.rs",
        "binance_pr_el_private_rest_registry_is_recorded",
    ),
    "commission-rate-fixture": (
        "crates/exchange/src/adapters/binance_fee_evidence_tests.rs",
        "parses_official_commission_fixture_without_zeroing_rates",
    ),
    "order-trade-update-finality-ledger": (
        "crates/api/src/trading_service/private_ws_events/tests/deltas/order_updates/part_02.rs",
        "binance_order_trade_fill_is_one_identity_preserving_ledger_outcome",
    ),
    "private-ws-durable-health": (
        "crates/api/src/lifecycle/private_ws/tests/projection.rs",
        "binance_terminal_fill_projects_execution_and_health_once_after_ack",
    ),
    "official-fixture-operation-registry": (
        "crates/exchange/src/venue_spec.rs",
        "binance_commission_rate_evidence_uses_recorded_fixture_metadata",
    ),
    "product-browser": (
        "test/e2e/pr_el_binance_identity.spec.ts",
        "npm run test:e2e:pr-el",
    ),
    "completion-governance": (
        "scripts/check_pr_el_completion.sh",
        "bash scripts/check_pr_el_completion.sh --self-test",
    ),
}

RUST_ANCHORS = (
    ("crates/exchange/src/venue_spec.rs", "binance_exchange_info_evidence_uses_recorded_fixture_metadata"),
    ("crates/exchange/src/venue_spec.rs", "binance_commission_rate_evidence_uses_recorded_fixture_metadata"),
    ("crates/exchange/src/venue_spec.rs", "binance_pr_el_private_rest_registry_is_recorded"),
    ("crates/exchange/tests/ws_trading_specs_test.rs", "binance_private_order_stream_fixture_is_recorded"),
    ("crates/exchange/src/adapters/binance_exchange_info_tests.rs", "resolver_preserves_explicit_usdc_and_never_crosses_quotes"),
    ("crates/exchange/src/adapters/binance_exchange_info_tests.rs", "registry_projection_uses_compiled_usdt_and_usdc_specs"),
    ("crates/exchange/src/adapters/binance_fee_evidence_tests.rs", "parses_official_commission_fixture_without_zeroing_rates"),
    ("crates/exchange/src/adapters/binance_private_rest.rs", "signed_private_reads_use_official_paths_and_preserve_commission_rates"),
    ("crates/exchange/src/adapters/binance_trade_data_tests.rs", "rest_place_order_validates_official_client_order_id_rule"),
    ("crates/exchange/tests/binance_test.rs", "live_place_order_preserves_verified_usdc_native_symbol"),
    ("shared-types/src/order_identity.rs", "verified_usdc_native_identity_is_execution_ready"),
    ("shared-types/src/order_identity.rs", "missing_and_mismatched_evidence_fail_closed"),
    ("crates/api/src/services/hedge_preview/guards.rs", "binance_identity_constraints_produce_execution_ready_usdc_plan"),
    ("crates/api/src/services/instrument_registry/tests.rs", "canonical_resolution_requires_one_verified_native_contract"),
    ("crates/api/src/trading_service/private_ws_mapper/tests/cases_binance_okx.rs", "binance_trade_update_maps_to_private_fill_delta"),
    ("crates/api/src/trading_service/private_ws_events/tests/deltas/order_updates/part_02.rs", "binance_order_trade_fill_is_one_identity_preserving_ledger_outcome"),
    ("crates/api/src/trading_service/private_ws_events/tests/deltas/order_updates/part_02.rs", "binance_terminal_cancel_preserves_reason_in_order_state_ledger_once"),
    ("crates/api/src/lifecycle/private_ws/tests/projection.rs", "binance_terminal_fill_projects_execution_and_health_once_after_ack"),
    ("frontend/src/panels/modules/execution/components/action_bar/tests.rs", "can_submit_fails_closed_when_identity_evidence_is_missing"),
)

BROWSER_ANCHORS = (
    "PR-EL renders Binance USDC canonical and native identity with client-id policy",
    "PR-EL blocks submit when the shared identity evidence contract is missing",
    "PR-EL exposes canonical mismatch and unavailable live runtime evidence fail closed",
)

CLOSURE_PATHS = tuple(
    """
scripts/check_pr_el_completion.sh
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
crates/exchange/src/ws/trading.rs
crates/exchange/tests/ws_trading_specs_test.rs
crates/exchange/fixtures/binance/usdm_account_commission_rate_btcusdt.json
crates/exchange/fixtures/binance/usdm_exchange_info_usdt_usdc.json
crates/exchange/fixtures/binance/usdm_order_trade_update_partial_fill.json
crates/exchange/src/adapters/binance.rs
crates/exchange/src/adapters/binance_exchange_info.rs
crates/exchange/src/adapters/binance_exchange_info_tests.rs
crates/exchange/src/adapters/binance_fee_evidence.rs
crates/exchange/src/adapters/binance_fee_evidence_tests.rs
crates/exchange/src/adapters/binance_format.rs
crates/exchange/src/adapters/binance_format_tests.rs
crates/exchange/src/adapters/binance_metadata.rs
crates/exchange/src/adapters/binance_private_data.rs
crates/exchange/src/adapters/binance_private_data_tests.rs
crates/exchange/src/adapters/binance_private_rest.rs
crates/exchange/src/adapters/binance_support.rs
crates/exchange/src/adapters/binance_trade_data.rs
crates/exchange/src/adapters/binance_trade_data_tests.rs
crates/exchange/src/adapters/binance_ws_user.rs
crates/exchange/src/adapters/binance_ws_user_data.rs
crates/exchange/src/adapters/binance_ws_user_tests.rs
crates/exchange/src/adapters/binance_ws_trade_tests.rs
crates/exchange/src/adapters/mod.rs
crates/exchange/tests/binance_test.rs
shared-types/src/hedge.rs
shared-types/src/lib.rs
shared-types/src/order_identity.rs
crates/api/src/lifecycle/private_ws/apply.rs
crates/api/src/lifecycle/private_ws/binance.rs
crates/api/src/lifecycle/private_ws/tests.rs
crates/api/src/lifecycle/private_ws/tests/projection.rs
crates/api/src/lifecycle/private_ws/tests/projection_support.rs
crates/api/src/routers/trading/tests/cases_registry.rs
crates/api/src/services/hedge_preview/guards.rs
crates/api/src/services/instrument_registry.rs
crates/api/src/services/instrument_registry/tests.rs
crates/api/src/services/private_ws_health/counts.rs
crates/api/src/services/ws_publish.rs
crates/api/src/trading_service/private_ws_events.rs
crates/api/src/trading_service/private_ws_events/apply.rs
crates/api/src/trading_service/private_ws_events/tests/deltas/order_updates/part_02.rs
crates/api/src/trading_service/private_ws_events/types.rs
crates/api/src/trading_service/private_ws_mapper.rs
crates/api/src/trading_service/private_ws_mapper/binance_okx.rs
crates/api/src/trading_service/private_ws_mapper/tests/cases_binance_okx.rs
frontend/src/panels/modules/execution/components/action_bar/tests.rs
frontend/src/panels/modules/execution/components/params_panel/capability/tests.rs
frontend/src/panels/modules/execution/components/risk_preview/evidence.rs
frontend/src/panels/modules/execution/components/risk_preview/tests.rs
frontend/src/panels/modules/execution/data/preview/build.rs
frontend/src/panels/modules/execution/data/preview/model.rs
frontend/src/panels/modules/execution/data/preview/response.rs
package.json
test/e2e/mock_api.mjs
test/e2e/pr_el_binance_identity.spec.ts
""".split()
)

COVERAGE_PATHS = {
    "scripts/check_pr_el_completion.sh",
    "crates/exchange/src/adapters/binance_fee_evidence.rs",
    "crates/exchange/src/adapters/binance_fee_evidence_tests.rs",
    "test/e2e/pr_el_binance_identity.spec.ts",
}
# Official JSON fixtures remain closure paths and are protected by
# require_fixture_hashes; the audit coverage ledger intentionally tracks
# source-like file types only.

if len(CLOSURE_PATHS) != 68 or len(set(CLOSURE_PATHS)) != 68:
    raise SystemExit("PR-EL completion gate internal error: expected 68 unique closure paths")


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
    matched = [line for line in section(text, "6.3").splitlines() if line.startswith("| `PR-EL ")]
    if len(matched) != 1:
        raise ValueError(f"expected one PR-EL roadmap row, found {len(matched)}")
    cells = [cell.strip() for cell in matched[0].strip().strip("|").split("|")]
    if len(cells) != 4 or cells[0].strip("`") != PR_TITLE:
        raise ValueError("PR-EL roadmap identity drifted")
    if cells[1] != "✅ 完成" or "剩余：无。" not in cells[2]:
        raise ValueError("PR-EL must be completed with no local remainder")
    if "bash scripts/check_pr_el_completion.sh --self-test" not in cells[3]:
        raise ValueError("PR-EL roadmap row lacks completion self-test anchor")
    if re.search(r"(?m)^\d+\.\s+\*\*PR-EL\*\*", section(text, "6.5")):
        raise ValueError("completed PR-EL remains in the local queue")


def require_evidence(root: Path) -> None:
    selected = [
        row for row in rows(
            root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv",
            ("pr_id", "evidence_type", "artifact", "command", "notes"),
        )
        if row["pr_id"].strip() == "PR-EL"
    ]
    indexed = {row["evidence_type"].strip(): row for row in selected}
    if len(indexed) != len(selected) or set(indexed) != set(EVIDENCE):
        raise ValueError(f"PR-EL evidence type drift: expected={sorted(EVIDENCE)}, actual={sorted(indexed)}")
    for evidence_type, (artifact, anchor) in EVIDENCE.items():
        row = indexed[evidence_type]
        if row["artifact"].strip() != artifact or anchor not in row["command"]:
            raise ValueError(f"PR-EL {evidence_type} artifact/command drifted")
        if not (root / artifact).is_file():
            raise ValueError(f"missing PR-EL artifact {artifact}")
        combined = f'{row["command"]} {row["notes"]}'.lower()
        if re.search(r"live credentials? (?:passed|verified)|ack_as_final|zero_fee_assumption=true", combined):
            raise ValueError(f"PR-EL {evidence_type} contains pseudo evidence")
    if "ack_not_final" not in indexed["order-trade-update-finality-ledger"]["notes"]:
        raise ValueError("ORDER_TRADE_UPDATE evidence must preserve ack_not_final")
    if "live_credentials_claimed=false" not in indexed["product-browser"]["notes"]:
        raise ValueError("browser evidence must disclaim live credentials")


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
    browser = (root / "test/e2e/pr_el_binance_identity.spec.ts").read_text(encoding="utf-8")
    if re.search(r"\b(?:test|describe)\.(?:skip|fixme)|FIXME", browser):
        raise ValueError("PR-EL browser gate contains skip/fixme")
    for title in BROWSER_ANCHORS:
        if f'test("{title}"' not in browser:
            raise ValueError(f"missing non-skipping browser anchor: {title}")


def require_official_registry(root: Path) -> None:
    venue = (root / "crates/exchange/src/venue_spec.rs").read_text(encoding="utf-8")
    required = (
        "/fapi/v1/exchangeInfo",
        "/fapi/v1/commissionRate",
        "/fapi/v3/balance",
        "/fapi/v3/positionRisk",
        "/fapi/v1/openOrders",
        "/fapi/v1/order",
        "usdm_exchange_info_usdt_usdc.json",
        "usdm_account_commission_rate_btcusdt.json",
        "account#user-commission-rate",
        "BINANCE_USDM_COMMISSION_RATE_SCHEMA_HASH",
        "auth_kind: SIGNED_AUTH_KIND",
    )
    for marker in required:
        if marker not in venue:
            raise ValueError(f"missing official Binance registry marker: {marker}")
    ws_registry = (root / "crates/exchange/src/ws/trading.rs").read_text(encoding="utf-8")
    for marker in ("usdm_order_trade_update_partial_fill.json", "sha256:590ce8eeaa980adeed148aeaac453da49bd130571bfb6fe0aeed47178ea26127", "parses_order_trade_update_to_order_delta"):
        if marker not in ws_registry:
            raise ValueError(f"missing Binance private-stream evidence marker: {marker}")
    matrix = (root / "scripts/exchange_operation_evidence_matrix.tsv").read_text(encoding="utf-8")
    if "ack_not_final" not in matrix:
        raise ValueError("missing Binance ACK finality boundary")


def require_fixture_hashes(root: Path) -> None:
    import hashlib
    expected = {
        "crates/exchange/fixtures/binance/usdm_exchange_info_usdt_usdc.json": "10c40d2f5bce94b4f4dcf57226e90fd559f5f63dc79fc7867b6b9866f59a9525",
        "crates/exchange/fixtures/binance/usdm_account_commission_rate_btcusdt.json": "28c1d83fde43698bca438bdb828eda321af35765923a6ea049863f0bc3cb518f",
        "crates/exchange/fixtures/binance/usdm_order_trade_update_partial_fill.json": "590ce8eeaa980adeed148aeaac453da49bd130571bfb6fe0aeed47178ea26127",
    }
    for relative, digest in expected.items():
        actual = hashlib.sha256((root / relative).read_bytes()).hexdigest()
        if actual != digest:
            raise ValueError(f"PR-EL fixture hash drift: {relative}")


def require_paths_and_coverage(root: Path) -> None:
    for relative in CLOSURE_PATHS:
        if not (root / relative).is_file():
            raise ValueError(f"missing exact PR-EL closure path: {relative}")
    coverage = rows(
        root / "docs/PRODUCT_AUDIT_COVERAGE.tsv",
        ("file", "coverage_status", "owner_surface", "risk", "suggested_pr", "evidence", "notes"),
    )
    indexed = {row["file"].strip(): row for row in coverage}
    missing = sorted(COVERAGE_PATHS - indexed.keys())
    if missing:
        raise ValueError(f"PR-EL closure coverage missing: {missing}")
    for relative in COVERAGE_PATHS:
        row = indexed[relative]
        if row["coverage_status"].strip() != "exact":
            raise ValueError(f"PR-EL closure coverage drifted: {relative}")


def check(root: Path) -> None:
    require_roadmap(root)
    require_evidence(root)
    require_runnable_anchors(root)
    require_official_registry(root)
    require_fixture_hashes(root)
    require_paths_and_coverage(root)


def copy_fixture(root: Path, target: Path) -> None:
    for relative in CLOSURE_PATHS:
        source = root / relative
        destination = target / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, destination)


def self_test(root: Path) -> None:
    mutations = {
        "missing evidence": lambda p: p.write_text("\n".join(line for line in p.read_text(encoding="utf-8").splitlines() if not line.startswith("PR-EL\tcommission-rate-fixture\t")) + "\n", encoding="utf-8"),
        "partial roadmap": lambda p: p.write_text(p.read_text(encoding="utf-8").replace("| `PR-EL Binance USD-M Order, Account & User Stream Semantics Contract` | ✅ 完成 |", "| `PR-EL Binance USD-M Order, Account & User Stream Semantics Contract` | 🟡 部分完成 |", 1), encoding="utf-8"),
        "queue reinsertion": lambda p: p.write_text(p.read_text(encoding="utf-8") + "\n1. **PR-EL** — regression\n", encoding="utf-8"),
        "ignored Rust anchor": lambda p: p.write_text(p.read_text(encoding="utf-8").replace("    #[test]\n    fn verified_usdc_native_identity_is_execution_ready", "    #[test]\n    #[ignore]\n    fn verified_usdc_native_identity_is_execution_ready", 1), encoding="utf-8"),
        "skipped browser": lambda p: p.write_text(p.read_text(encoding="utf-8").replace("test(\"PR-EL renders", "test.skip(\"PR-EL renders", 1), encoding="utf-8"),
        "fixture drift": lambda p: p.write_bytes(p.read_bytes() + b"\n"),
        "coverage removal": lambda p: p.write_text("\n".join(line for line in p.read_text(encoding="utf-8").splitlines() if not line.startswith("test/e2e/pr_el_binance_identity.spec.ts\t")) + "\n", encoding="utf-8"),
        "registry removal": lambda p: p.write_text(p.read_text(encoding="utf-8").replace("/fapi/v1/commissionRate", "/removed/commissionRate"), encoding="utf-8"),
        "operation evidence removal": lambda p: p.write_text(p.read_text(encoding="utf-8").replace("usdm_order_trade_update_partial_fill.json", "removed_order_trade_update.json", 1), encoding="utf-8"),
    }
    mutation_paths = {
        "missing evidence": "docs/PRODUCT_AUDIT_EVIDENCE.tsv",
        "partial roadmap": "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md",
        "queue reinsertion": "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md",
        "ignored Rust anchor": "shared-types/src/order_identity.rs",
        "skipped browser": "test/e2e/pr_el_binance_identity.spec.ts",
        "fixture drift": "crates/exchange/fixtures/binance/usdm_order_trade_update_partial_fill.json",
        "coverage removal": "docs/PRODUCT_AUDIT_COVERAGE.tsv",
        "registry removal": "crates/exchange/src/venue_spec.rs",
        "operation evidence removal": "crates/exchange/src/ws/trading.rs",
    }
    with tempfile.TemporaryDirectory(prefix="pr-el-completion-") as tmp:
        baseline = Path(tmp) / "baseline"
        copy_fixture(root, baseline)
        check(baseline)
        for name, mutate in mutations.items():
            case = Path(tmp) / re.sub(r"\W+", "-", name)
            shutil.copytree(baseline, case)
            mutate(case / mutation_paths[name])
            try:
                check(case)
            except (ValueError, OSError):
                continue
            raise ValueError(f"destructive self-test unexpectedly passed: {name}")


root = Path(sys.argv[1])
mode = sys.argv[2]
try:
    check(root)
    if mode == "--self-test":
        self_test(root)
except (ValueError, OSError) as error:
    raise SystemExit(f"PR-EL completion gate failed: {error}") from error

print(f"PR-EL completion gate passed ({len(EVIDENCE)} evidence types, {len(RUST_ANCHORS)} Rust anchors, {len(BROWSER_ANCHORS)} browser anchors, {len(CLOSURE_PATHS)} exact paths)")
PY
