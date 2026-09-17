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
import sys
import tempfile

PR_TITLE = "PR-FC Private Account Parser Strictness, Balance Evidence & Risk Field Contract"
EVIDENCE = {
    "eight-venue-private-parser-fixture-matrix": (
        "crates/exchange/src/adapters/bybit_private_data_tests.rs",
        "bybit_private_strict_fixture_sweep_fails_closed",
    ),
    "official-private-read-registry": (
        "scripts/exchange_evidence_debt_allowlist.tsv",
        "bash scripts/check_exchange_evidence_debt.sh",
    ),
    "operation-evidence-matrix": (
        "scripts/check_exchange_operation_evidence_matrix.sh",
        "bash scripts/check_exchange_operation_evidence_matrix.sh",
    ),
    "scoped-margin-freshness": (
        "crates/api/src/services/hedge_margin/tests/outcome.rs",
        "margin_outcome_blocks_stale_scoped_balance_evidence",
    ),
    "exact-venue-scope": (
        "crates/trading/src/execution.rs",
        "hedge_margin_check_rejects_sibling_hyperliquid_dex_balance",
    ),
    "position-row-account-evidence": (
        "crates/api/src/services/hedge_preview/positions.rs",
        "preview_and_ticket_evidence_keep_position_row_health_and_account_bindings",
    ),
    "settings-account-evidence": (
        "frontend/src/panels/modules/settings/tabs/venue_credentials/panels/account_state.rs",
        "selected_account_evidence_keeps_row_health_binding_and_scoped_problem",
    ),
    "product-browser": (
        "test/e2e/pr_fc_account_evidence.spec.ts",
        "npm run test:e2e:pr-fc",
    ),
    "completion-governance": (
        "scripts/check_pr_fc_completion.sh",
        "bash scripts/check_pr_fc_completion.sh --self-test",
    ),
}

RUST_ANCHORS = (
    ("crates/exchange/src/adapters/binance_private_data_tests.rs", "binance_private_strict_fixture_sweep_fails_closed"),
    ("crates/exchange/src/adapters/okx_private_data_tests.rs", "okx_private_strict_fixture_sweep_fails_closed"),
    ("crates/exchange/src/adapters/bybit_private_data_tests.rs", "bybit_private_strict_fixture_sweep_fails_closed"),
    ("crates/exchange/src/adapters/bitget_uta_private_data_tests.rs", "strict_account_fixture_rejects_defaulting"),
    ("crates/exchange/src/adapters/kucoin_private_data_tests.rs", "strict_account_fixture_rejects_defaulting"),
    ("crates/exchange/src/adapters/gate_private_data_tests.rs", "strict_account_fixture_rejects_defaulting"),
    ("crates/exchange/src/adapters/htx_private_data_tests.rs", "strict_account_fixture_rejects_defaulting"),
    ("crates/exchange/src/adapters/hyperliquid_private_data_tests.rs", "strict_account_fixture_rejects_defaulting"),
    ("crates/exchange/src/adapters/kucoin_ws_user_tests.rs", "strict_account_fixture_rejects_defaulting"),
    ("crates/api/src/services/hedge_margin/tests/outcome.rs", "margin_outcome_blocks_stale_scoped_balance_evidence"),
    ("crates/api/src/services/hedge_margin/tests/outcome.rs", "hyperliquid_sibling_dex_balance_cannot_cover_builder_margin_check"),
    ("crates/trading/src/execution.rs", "hedge_margin_check_rejects_sibling_hyperliquid_dex_balance"),
    ("shared-types/src/arbitrage.rs", "hedge_preview_positions_evidence_serializes_row_health_and_account_bindings"),
    ("crates/api/src/services/hedge_preview/positions.rs", "preview_and_ticket_evidence_keep_position_row_health_and_account_bindings"),
    ("frontend/src/panels/modules/settings/tabs/venue_credentials/panels/account_state/tests.rs", "selected_account_evidence_keeps_parent_venue_children_but_not_sibling_scopes"),
    ("frontend/src/panels/modules/settings/tabs/venue_credentials/panels/account_state/tests.rs", "selected_account_evidence_keeps_row_health_binding_and_scoped_problem"),
)
BROWSER_PATH = "test/e2e/pr_fc_account_evidence.spec.ts"
BROWSER_ANCHOR = "PR-FC Settings exposes selected-venue account field quality and scope evidence"

CLOSURE_PATHS = tuple(
    """
scripts/check_pr_fc_completion.sh
scripts/check_exchange_evidence_debt.sh
scripts/check_exchange_operation_evidence_matrix.sh
scripts/exchange_evidence_debt_allowlist.tsv
scripts/verify_repo_gates.sh
docs/PRODUCT_FULL_AUDIT_REFINEMENT.md
docs/audit_history/PRODUCT_AUDIT_HISTORY.md
docs/PRODUCT_AUDIT_EVIDENCE.tsv
docs/PRODUCT_AUDIT_COVERAGE.tsv
crates/exchange/src/venue_spec.rs
crates/exchange/fixtures/binance/private_parser_strict_rejections.json
crates/exchange/fixtures/bybit/private_parser_strict_rejections.json
crates/exchange/fixtures/okx/private_parser_strict_rejections.json
crates/exchange/fixtures/bitget/uta_current_position_btcusdt.json
crates/exchange/fixtures/hyperliquid/info_order_status_filled.json
crates/exchange/src/adapters/binance_private_data.rs
crates/exchange/src/adapters/binance_private_data_tests.rs
crates/exchange/src/adapters/okx_private_data.rs
crates/exchange/src/adapters/okx_private_data_tests.rs
crates/exchange/src/adapters/bybit_private_data.rs
crates/exchange/src/adapters/bybit_private_data_tests.rs
crates/exchange/src/adapters/bitget_uta_private_data.rs
crates/exchange/src/adapters/bitget_uta_private_data_tests.rs
crates/exchange/src/adapters/kucoin_private_data.rs
crates/exchange/src/adapters/kucoin_private_data_tests.rs
crates/exchange/src/adapters/kucoin_ws_user_data.rs
crates/exchange/src/adapters/kucoin_ws_user_tests.rs
crates/exchange/src/adapters/gate_private_data.rs
crates/exchange/src/adapters/gate_private_data_tests.rs
crates/exchange/src/adapters/htx_private_data.rs
crates/exchange/src/adapters/htx_private_data_tests.rs
crates/exchange/src/adapters/hyperliquid_private_data.rs
crates/exchange/src/adapters/hyperliquid_private_data_tests.rs
crates/api/src/services/hedge_margin.rs
crates/api/src/services/hedge_margin/health.rs
crates/api/src/services/hedge_margin/problems.rs
crates/api/src/services/hedge_margin/tests.rs
crates/api/src/services/hedge_margin/tests/outcome.rs
crates/api/src/services/hedge_margin/venues.rs
crates/api/src/services/hedge_preview/positions.rs
crates/api/src/services/hedge_preview/positions/evidence.rs
crates/trading/src/execution.rs
shared-types/src/arbitrage.rs
frontend/src/api/rest/trading.rs
frontend/src/panels/modules/settings/data/actions.rs
frontend/src/panels/modules/settings/data/resources.rs
frontend/src/panels/modules/settings/tabs/venue_credentials.rs
frontend/src/panels/modules/settings/tabs/venue_credentials/panels.rs
frontend/src/panels/modules/settings/tabs/venue_credentials/panels/account_state.rs
frontend/src/panels/modules/settings/tabs/venue_credentials/panels/account_state/rows.rs
frontend/src/panels/modules/settings/tabs/venue_credentials/panels/account_state/selection.rs
frontend/src/panels/modules/settings/tabs/venue_credentials/panels/account_state/tests.rs
frontend/src/panels/modules/execution/components/risk_preview/tests/cases.rs
package.json
test/e2e/pr_fc_account_evidence.spec.ts
""".split()
)
if len(CLOSURE_PATHS) != 55 or len(set(CLOSURE_PATHS)) != 55:
    raise SystemExit("PR-FC completion gate internal error: expected 55 unique closure paths")
COVERAGE_PATHS = {
    path for path in CLOSURE_PATHS
    if path.startswith(("scripts/", "crates/", "shared-types/", "frontend/", "test/"))
    and not path.endswith(".json")
}
FIXTURE_HASHES = {
    "crates/exchange/fixtures/binance/private_parser_strict_rejections.json": "fed38b3f9cfc9ad2b3cad4e93a07ce7d7a9f7950f74636b45ba1219c23bbfe8b",
    "crates/exchange/fixtures/bybit/private_parser_strict_rejections.json": "f8d9ac035270459782a4146cf033edb4d8069852f50d3de753486993fe3d2464",
    "crates/exchange/fixtures/okx/private_parser_strict_rejections.json": "472218b1aaac1f332df8bee1a6436fe6fa63ecc2e5e610f854ea6367485480d8",
    "crates/exchange/fixtures/bitget/uta_current_position_btcusdt.json": "22ead2a4c3fe6a979be913b9db5e75bedb8a9e6c8170b9c437d30b0d5731f0fb",
    "crates/exchange/fixtures/hyperliquid/info_order_status_filled.json": "875f9a4b3f868c1936d4ee274e3183001a9200521c39dc9067034b1d07a6a19a",
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
    matched = [line for line in section(text, "6.3").splitlines() if line.startswith("| `PR-FC ")]
    if len(matched) != 1:
        raise ValueError(f"expected one PR-FC roadmap row, found {len(matched)}")
    cells = [cell.strip() for cell in matched[0].strip().strip("|").split("|")]
    if len(cells) != 4 or cells[0].strip("`") != PR_TITLE:
        raise ValueError("PR-FC roadmap identity drifted")
    if cells[1] != "✅ 完成" or "剩余：无。" not in cells[2]:
        raise ValueError("PR-FC must be completed with no local remainder")
    if "bash scripts/check_pr_fc_completion.sh --self-test" not in cells[3]:
        raise ValueError("PR-FC roadmap row lacks completion self-test anchor")
    if re.search(r"(?m)^\d+\.\s+\*\*PR-FC\*\*", section(text, "6.5")):
        raise ValueError("completed PR-FC remains in the local queue")


def require_evidence(root: Path) -> None:
    selected = [
        row for row in rows(
            root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv",
            ("pr_id", "evidence_type", "artifact", "command", "notes"),
        )
        if row["pr_id"].strip() == "PR-FC"
    ]
    indexed = {row["evidence_type"].strip(): row for row in selected}
    if len(indexed) != len(selected) or set(indexed) != set(EVIDENCE):
        raise ValueError(f"PR-FC evidence type drift: expected={sorted(EVIDENCE)}, actual={sorted(indexed)}")
    for evidence_type, (artifact, anchor) in EVIDENCE.items():
        row = indexed[evidence_type]
        if row["artifact"].strip() != artifact or anchor not in row["command"]:
            raise ValueError(f"PR-FC {evidence_type} artifact/command drifted")
        if not (root / artifact).is_file():
            raise ValueError(f"missing PR-FC artifact {artifact}")
        combined = f'{row["command"]} {row["notes"]}'.lower()
        if re.search(r"live credentials? (?:passed|verified)|zero_fee_assumption=true|assumed_live", combined):
            raise ValueError(f"PR-FC {evidence_type} contains pseudo evidence")
    if "live_credentials_claimed=false" not in indexed["product-browser"]["notes"]:
        raise ValueError("PR-FC browser evidence must disclaim live credentials")
    for marker in (
        "bash -n scripts/check_pr_fc_completion.sh",
        "bash scripts/check_pr_fc_completion.sh --self-test",
        "bash scripts/check_pr_fc_completion.sh",
    ):
        if marker not in indexed["completion-governance"]["command"]:
            raise ValueError(f"PR-FC completion evidence lacks {marker}")


def require_runnable_anchors(root: Path) -> None:
    invalid = re.compile(r"#\[\s*(?:ignore|should_panic)|#\[\s*cfg_attr[^\]]*ignore")
    for relative, name in RUST_ANCHORS:
        text = (root / relative).read_text(encoding="utf-8")
        match = re.search(rf"(?m)^\s*(?:async\s+)?fn\s+{re.escape(name)}\s*\(", text)
        if not match:
            raise ValueError(f"missing runnable Rust anchor {relative}::{name}")
        attrs = "\n".join(
            line for line in text[max(0, match.start() - 600):match.start()].splitlines()[-14:]
            if line.strip().startswith("#[")
        )
        if not re.search(r"#\[\s*(?:tokio::)?test(?:\]|\()", attrs) or invalid.search(attrs):
            raise ValueError(f"non-runnable Rust anchor {relative}::{name}")
    browser = (root / BROWSER_PATH).read_text(encoding="utf-8")
    if re.search(r"\b(?:test|describe)\.(?:skip|fixme)|FIXME", browser):
        raise ValueError("PR-FC browser gate contains skip/fixme")
    if f'test("{BROWSER_ANCHOR}"' not in browser:
        raise ValueError("missing non-skipping PR-FC browser anchor")


def require_private_registry(root: Path) -> None:
    venue = (root / "crates/exchange/src/venue_spec.rs").read_text(encoding="utf-8")
    allowlist = (root / "scripts/exchange_evidence_debt_allowlist.tsv").read_text(encoding="utf-8")
    for marker in (
        "HYPERLIQUID_GET_ORDER_SCHEMA_HASH",
        "sha256:875f9a4b3f868c1936d4ee274e3183001a9200521c39dc9067034b1d07a6a19a",
        "BITGET_CURRENT_POSITION_SCHEMA_HASH",
        "sha256:22ead2a4c3fe6a979be913b9db5e75bedb8a9e6c8170b9c437d30b0d5731f0fb",
    ):
        if marker not in venue:
            raise ValueError(f"missing private registry marker: {marker}")
    for marker in (
        "hyperliquid-info-order-status-2026-06-30",
        "bitget-uta-trade-get-current-position-2026-07-02",
        "bybit_account_summary_parses_equity_margin_rates_and_source",
    ):
        if marker not in allowlist:
            raise ValueError(f"missing private allowlist marker: {marker}")
    strict_markers = {
        "crates/exchange/src/adapters/hyperliquid_private_data.rs": ("is_trigger: bool", "reduce_only: bool", "requires isTrigger=true"),
        "crates/exchange/src/adapters/bitget_uta_private_data.rs": ("maintenance_margin_rate", '"mmr"', "parse_required_non_negative_decimal"),
        "crates/exchange/src/adapters/bybit_private_data.rs": (
            'parse_positive_number(&scope, "avgPrice", &row.avg_price)?',
            'parse_positive_number(&scope, "markPrice", &row.mark_price)?',
            "UnsupportedCapability",
        ),
    }
    for relative, markers in strict_markers.items():
        text = (root / relative).read_text(encoding="utf-8")
        if any(marker not in text for marker in markers):
            raise ValueError(f"strict parser markers drifted: {relative}")


def require_fixture_hashes(root: Path) -> None:
    for relative, digest in FIXTURE_HASHES.items():
        if hashlib.sha256((root / relative).read_bytes()).hexdigest() != digest:
            raise ValueError(f"PR-FC fixture hash drift: {relative}")


def require_paths_and_coverage(root: Path) -> None:
    for relative in CLOSURE_PATHS:
        if not (root / relative).is_file():
            raise ValueError(f"missing exact PR-FC closure path: {relative}")
    coverage = rows(
        root / "docs/PRODUCT_AUDIT_COVERAGE.tsv",
        ("file", "coverage_status", "owner_surface", "risk", "suggested_pr", "evidence", "notes"),
    )
    indexed = {row["file"].strip(): row for row in coverage}
    missing = sorted(COVERAGE_PATHS - indexed.keys())
    if missing:
        raise ValueError(f"PR-FC closure coverage missing: {missing}")
    if any(indexed[path]["coverage_status"].strip() != "exact" for path in COVERAGE_PATHS):
        raise ValueError("PR-FC closure coverage is not exact")


def check(root: Path) -> None:
    require_roadmap(root)
    require_evidence(root)
    require_runnable_anchors(root)
    require_private_registry(root)
    require_fixture_hashes(root)
    require_paths_and_coverage(root)


def copy_fixture(root: Path, target: Path) -> None:
    for relative in CLOSURE_PATHS:
        destination = target / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(root / relative, destination)


def self_test(root: Path) -> None:
    mutations = {
        "missing evidence": (
            "docs/PRODUCT_AUDIT_EVIDENCE.tsv",
            lambda p: p.write_text("\n".join(
                line for line in p.read_text(encoding="utf-8").splitlines()
                if not line.startswith("PR-FC\teight-venue-private-parser-fixture-matrix\t")
            ) + "\n", encoding="utf-8"),
        ),
        "partial roadmap": (
            "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md",
            lambda p: p.write_text(p.read_text(encoding="utf-8").replace(
                "| `PR-FC Private Account Parser Strictness, Balance Evidence & Risk Field Contract` | ✅ 完成 |",
                "| `PR-FC Private Account Parser Strictness, Balance Evidence & Risk Field Contract` | 🟡 部分完成 |",
                1,
            ), encoding="utf-8"),
        ),
        "queue reinsertion": (
            "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md",
            lambda p: p.write_text(p.read_text(encoding="utf-8") + "\n1. **PR-FC** regression\n", encoding="utf-8"),
        ),
        "ignored Rust anchor": (
            "crates/exchange/src/adapters/binance_private_data_tests.rs",
            lambda p: p.write_text(p.read_text(encoding="utf-8").replace(
                "fn binance_private_strict_fixture_sweep_fails_closed",
                "#[ignore]\nfn binance_private_strict_fixture_sweep_fails_closed",
                1,
            ), encoding="utf-8"),
        ),
        "skipped browser": (
            BROWSER_PATH,
            lambda p: p.write_text(p.read_text(encoding="utf-8").replace(
                'test("PR-FC Settings',
                'test.skip("PR-FC Settings',
                1,
            ), encoding="utf-8"),
        ),
        "fixture drift": (
            "crates/exchange/fixtures/binance/private_parser_strict_rejections.json",
            lambda p: p.write_bytes(p.read_bytes() + b"\n"),
        ),
        "coverage removal": (
            "docs/PRODUCT_AUDIT_COVERAGE.tsv",
            lambda p: p.write_text("\n".join(
                line for line in p.read_text(encoding="utf-8").splitlines()
                if not line.startswith("test/e2e/pr_fc_account_evidence.spec.ts\t")
            ) + "\n", encoding="utf-8"),
        ),
        "registry removal": (
            "crates/exchange/src/venue_spec.rs",
            lambda p: p.write_text(p.read_text(encoding="utf-8").replace(
                "sha256:875f9a4b3f868c1936d4ee274e3183001a9200521c39dc9067034b1d07a6a19a",
                "sha256:0000000000000000000000000000000000000000000000000000000000000000",
                1,
            ), encoding="utf-8"),
        ),
    }
    with tempfile.TemporaryDirectory(prefix="pr-fc-completion-") as tmp:
        baseline = Path(tmp) / "baseline"
        copy_fixture(root, baseline)
        check(baseline)
        for name, (relative, mutate) in mutations.items():
            case = Path(tmp) / re.sub(r"\W+", "-", name)
            shutil.copytree(baseline, case)
            mutate(case / relative)
            try:
                check(case)
            except (ValueError, OSError):
                continue
            raise ValueError(f"destructive self-test unexpectedly passed: {name}")


root = Path(sys.argv[1])
try:
    check(root)
    if sys.argv[2] == "--self-test":
        self_test(root)
except (ValueError, OSError) as error:
    raise SystemExit(f"PR-FC completion gate failed: {error}") from error

print(
    f"PR-FC completion gate passed ({len(EVIDENCE)} evidence types, "
    f"{len(RUST_ANCHORS)} Rust anchors, 1 browser anchor, {len(CLOSURE_PATHS)} exact paths)"
)
PY
