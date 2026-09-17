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
import io
import re
import sys
import tempfile


PR_FZ_TITLE = "PR-FZ Runtime Artifact, Storage Path & Data Hygiene Contract"
REQUIRED_EVIDENCE = {
    "private-funding-payment-eight-venue-matrix-gate": (
        "crates/exchange/tests/funding_payment_matrix_test.rs",
        "funding_payment_evidence_matrix_covers_all_eight_venues_without_skips",
    ),
    "private-funding-payment-runtime-router-gate": (
        "crates/api/src/trading_service/live_adapters/route_tests/cases/funding_payments.rs",
        "live_router_funding_payments_are_partial_tolerant_and_skip_unsupported",
    ),
    "private-funding-payment-ingest-idempotency-gate": (
        "crates/api/src/trading_service/funding_payments/tests.rs",
        "private_funding_payment_ingest_is_idempotent_for_repeated_lookback",
    ),
    "sql-ledger-funding-slippage-replay-gate": (
        "crates/trading/src/journal/projection/tests/part_09.rs",
        "sql_ledger_replay_restores_funding_and_slippage_facts",
    ),
    "execution-run-cost-replay-gate": (
        "crates/api/src/services/execution_runs/tests/cases_d.rs",
        "execution_cost_facts_replay_after_restart",
    ),
    "close-run-cost-replay-gate": (
        "crates/api/src/services/close_runs/tests/cases_a.rs",
        "close_cost_facts_replay_after_restart",
    ),
    "sql-ledger-commit-ack-integrity-gate": (
        "crates/trading/tests/sql_ledger_postgres.rs",
        "sql_ledger_commit_ack_rejects_conflicting_duplicate",
    ),
    "run-cost-fact-rebuild-gate": (
        "crates/api/src/services/execution_runs/tests/cases_d/run_cost_rebuild.rs",
        "run_cost_facts_rebuild_execution_and_close_reconciliation",
    ),
    "postgres-roundtrip-restart-gate": (
        "crates/trading/tests/sql_ledger_postgres.rs",
        "sql_ledger_postgres_roundtrip_restarts_from_committed_facts",
    ),
}

TEST_ANCHORS = (
    (
        "crates/exchange/tests/funding_payment_matrix_test.rs",
        "funding_payment_evidence_matrix_covers_all_eight_venues_without_skips",
    ),
    (
        "crates/api/src/trading_service/funding_payments/tests.rs",
        "private_funding_payment_ingest_is_idempotent_for_repeated_lookback",
    ),
    (
        "crates/api/src/trading_service/live_adapters/route_tests/cases/funding_payments.rs",
        "live_router_funding_payments_are_partial_tolerant_and_skip_unsupported",
    ),
    (
        "crates/api/src/trading_service/private_ws_events/tests/funding_cross_venue.rs",
        "private_funding_delta_uses_cross_venue_fill_event_anchors",
    ),
    (
        "crates/api/src/lifecycle/private_ws/tests/projection.rs",
        "private_fill_chain_projects_execution_slippage_once",
    ),
    (
        "crates/api/src/lifecycle/private_ws/tests/projection.rs",
        "binance_fill_chain_projects_close_and_review_slippage_once",
    ),
    (
        "crates/api/src/trading_service/private_ws_events/tests/deltas/order_updates/part_02.rs",
        "duplicate_private_fill_returns_no_events_and_does_not_extend_ledger",
    ),
    (
        "crates/api/src/services/execution_runs/tests/cases_b.rs",
        "cross_venue_funding_events_project_each_linked_leg_once",
    ),
    (
        "crates/trading/src/journal/funding_replay_tests.rs",
        "cross_venue_funding_replay_preserves_run_leg_and_order_fact_links",
    ),
    (
        "crates/trading/tests/funding_journal_test.rs",
        "record_funding_by_venue_symbol_accepts_cross_venue_fill_event_anchors",
    ),
    (
        "crates/trading/src/sql_ledger.rs",
        "sql_replay_keeps_cross_venue_funding_fact_links",
    ),
    (
        "crates/trading/src/journal/projection/tests/part_08.rs",
        "fill_projection_returns_fill_then_slippage_and_suppresses_duplicate_event",
    ),
    (
        "crates/trading/src/journal/projection/tests/part_04.rs",
        "execution_ledger_jsonl_replays_fill_events_after_restart",
    ),
    (
        "crates/trading/src/journal/projection/tests/part_04.rs",
        "execution_ledger_jsonl_replays_funding_payments_after_restart",
    ),
    (
        "crates/trading/src/journal/projection/tests/part_09.rs",
        "sql_ledger_replay_restores_funding_and_slippage_facts",
    ),
    (
        "crates/api/src/services/execution_runs/tests/cases_d.rs",
        "execution_cost_facts_replay_after_restart",
    ),
    (
        "crates/api/src/services/close_runs/tests/cases_a.rs",
        "close_cost_facts_replay_after_restart",
    ),
    (
        "crates/trading/tests/sql_ledger_postgres.rs",
        "sql_ledger_commit_ack_rejects_conflicting_duplicate",
    ),
    (
        "crates/api/src/services/execution_runs/tests/cases_d/run_cost_rebuild.rs",
        "run_cost_facts_rebuild_execution_and_close_reconciliation",
    ),
    (
        "crates/trading/tests/sql_ledger_postgres.rs",
        "sql_ledger_postgres_roundtrip_restarts_from_committed_facts",
    ),
)

CLOSURE_PATHS = (
    "scripts/check_pr_fz_completion.sh",
    "scripts/check_product_audit_evidence_index.sh",
    "scripts/verify_repo_gates.sh",
    "crates/exchange/tests/funding_payment_matrix_test.rs",
    "crates/exchange/src/adapters/funding_payments.rs",
    "crates/exchange/src/venue_spec.rs",
    "crates/api/src/lifecycle/funding_payments.rs",
    "crates/api/src/trading_service/funding_payments.rs",
    "crates/api/src/trading_service/funding_payments/tests.rs",
    "crates/api/src/trading_service/live_adapters/route_tests/cases/funding_payments.rs",
    "crates/api/src/trading_service/private_ws_events/tests/funding_cross_venue.rs",
    "crates/api/src/lifecycle/private_ws/apply.rs",
    "crates/api/src/lifecycle/private_ws/tests.rs",
    "crates/api/src/lifecycle/private_ws/tests/projection.rs",
    "crates/api/src/lifecycle/private_ws/tests/projection_support.rs",
    "crates/api/src/trading_service/private_ws_events/apply.rs",
    "crates/api/src/trading_service/private_ws_events/types.rs",
    "crates/api/src/trading_service/private_ws_events/tests/deltas/order_updates/part_01.rs",
    "crates/api/src/trading_service/private_ws_events/tests/deltas/order_updates/part_02.rs",
    "crates/api/src/services/execution_runs/tests/cases_b.rs",
    "crates/trading/src/journal/funding_replay_tests.rs",
    "crates/trading/src/sql_ledger.rs",
    "crates/trading/src/journal/projection/part_04.rs",
    "crates/trading/src/journal/projection/tests/part_03.rs",
    "crates/trading/src/journal/projection/tests/part_04.rs",
    "crates/trading/src/journal/projection/tests/part_09.rs",
    "crates/trading/src/journal/projection/tests/part_08.rs",
    "crates/trading/tests/funding_journal_test.rs",
    "crates/api/src/services/execution_runs.rs",
    "crates/api/src/services/execution_runs/tests/cases_d.rs",
    "crates/api/src/services/execution_runs/tests/cases_d/run_cost_rebuild.rs",
    "crates/api/src/services/close_run_costs.rs",
    "crates/api/src/services/close_runs/tests/cases_a.rs",
    "crates/trading/migrations/20260710_run_cost_integrity.sql",
    "crates/trading/migrations/20260710_run_cost_sources.sql",
    "crates/trading/src/sql_ledger/run_cost.rs",
    "crates/trading/src/sql_ledger/run_cost_rebuild.rs",
    "crates/trading/src/sql_ledger/run_finality.rs",
    "crates/trading/src/sql_ledger/writer.rs",
    "crates/trading/src/sql_ledger/writer/protocol.rs",
    "crates/trading/src/sql_ledger/migrations.rs",
    "crates/trading/src/sql_ledger/migrations/tests.rs",
    "crates/trading/src/journal/sql_events.rs",
    "crates/trading/src/lib.rs",
    "crates/trading/tests/sql_ledger_postgres.rs",
    "crates/api/src/lifecycle/ledger_projection.rs",
    "crates/api/src/trading_service/orders.rs",
    "crates/api/src/services/close_runs/project.rs",
    "crates/api/src/services/close_runs/project/events.rs",
    "crates/api/src/services/close_runs/tests/cases_b/manual_terminal.rs",
    "crates/api/src/routers/portfolio.rs",
)

if len(CLOSURE_PATHS) != 51 or len(set(CLOSURE_PATHS)) != 51:
    raise SystemExit("PR-FZ completion gate internal error: closure path set must contain 51 unique paths")


def read_tsv(path: Path, expected_header: tuple[str, ...]) -> list[dict[str, str]]:
    if not path.is_file():
        raise ValueError(f"missing {path.relative_to(path.parents[1])}")
    with path.open(encoding="utf-8", newline="") as handle:
        reader = csv.DictReader(handle, delimiter="\t")
        if tuple(reader.fieldnames or ()) != expected_header:
            raise ValueError(
                f"{path.name} header drifted: expected {expected_header}, "
                f"got {tuple(reader.fieldnames or ())}"
            )
        return list(reader)


def completion_state(root: Path) -> bool:
    path = root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
    if not path.is_file():
        raise ValueError("missing docs/PRODUCT_FULL_AUDIT_REFINEMENT.md")
    text = path.read_text(encoding="utf-8")
    start = text.find("### 🟡 6.3")
    end = text.find("### 🟡 6.4", start)
    if start < 0 or end < 0:
        raise ValueError("missing bounded 6.3 roadmap section")
    rows = []
    for line in text[start:end].splitlines():
        if not line.startswith("| `PR-FZ "):
            continue
        rows.append([cell.strip() for cell in line.strip().strip("|").split("|")])
    if len(rows) != 1:
        raise ValueError(f"expected exactly one PR-FZ roadmap row, found {len(rows)}")
    cells = rows[0]
    if len(cells) != 4:
        raise ValueError(f"PR-FZ roadmap row must have 4 cells, found {len(cells)}")
    if cells[0].strip("`") != PR_FZ_TITLE:
        raise ValueError(f"PR-FZ roadmap title drifted: {cells[0]!r}")
    if cells[1] == "🟡 部分完成":
        return False
    if cells[1] != "✅ 完成":
        raise ValueError(f"PR-FZ roadmap row has unsupported status: {cells[1]!r}")
    if "剩余：无。" not in cells[2]:
        raise ValueError("PR-FZ complete row must declare 剩余：无。")
    if "scripts/check_pr_fz_completion.sh" not in cells[2]:
        raise ValueError("PR-FZ complete row must record the static completion gate")
    return True


def require_evidence(root: Path) -> None:
    rows = read_tsv(
        root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv",
        ("pr_id", "evidence_type", "artifact", "command", "notes"),
    )
    fz_rows = [row for row in rows if row["pr_id"].strip() == "PR-FZ"]
    seen: dict[str, dict[str, str]] = {}
    for row in fz_rows:
        evidence_type = row["evidence_type"].strip()
        if evidence_type in seen:
            raise ValueError(f"duplicate PR-FZ evidence type: {evidence_type}")
        seen[evidence_type] = row
    expected = set(REQUIRED_EVIDENCE)
    actual = set(seen)
    if actual != expected:
        missing = sorted(expected - actual)
        extra = sorted(actual - expected)
        raise ValueError(f"PR-FZ evidence type drift: missing={missing}, extra={extra}")
    for evidence_type, (artifact, command_anchor) in REQUIRED_EVIDENCE.items():
        row = seen[evidence_type]
        if row["artifact"].strip() != artifact:
            raise ValueError(
                f"PR-FZ {evidence_type} artifact must be {artifact}, "
                f"got {row['artifact'].strip()!r}"
            )
        if command_anchor not in row["command"]:
            raise ValueError(
                f"PR-FZ {evidence_type} command must name {command_anchor}"
            )


def strip_rust_comments(text: str) -> str:
    output: list[str] = []
    index = 0
    block_depth = 0
    in_string = False
    escaped = False
    while index < len(text):
        char = text[index]
        pair = text[index : index + 2]
        if block_depth:
            if pair == "/*":
                block_depth += 1
                output.extend("  ")
                index += 2
            elif pair == "*/":
                block_depth -= 1
                output.extend("  ")
                index += 2
            else:
                output.append("\n" if char == "\n" else " ")
                index += 1
            continue
        if in_string:
            output.append(char)
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == '"':
                in_string = False
            index += 1
            continue
        if pair == "//":
            newline = text.find("\n", index + 2)
            if newline < 0:
                output.extend(" " * (len(text) - index))
                break
            output.extend(" " * (newline - index))
            output.append("\n")
            index = newline + 1
        elif pair == "/*":
            block_depth = 1
            output.extend("  ")
            index += 2
        else:
            output.append(char)
            in_string = char == '"'
            index += 1
    return "".join(output)


def require_test_anchors(root: Path) -> None:
    for relative_path, function_name in TEST_ANCHORS:
        path = root / relative_path
        if not path.is_file():
            raise ValueError(f"missing PR-FZ anchor file: {relative_path}")
        text = strip_rust_comments(path.read_text(encoding="utf-8"))
        pattern = re.compile(
            r"(?m)^\s*#\[(?:tokio::)?test(?:\([^\]]*\))?\]\s*\n"
            r"(?:\s*#\[[^\n]+\]\s*\n)*"
            rf"\s*(?:async\s+)?fn\s+{re.escape(function_name)}\s*\("
        )
        if not pattern.search(text):
            raise ValueError(
                f"missing runnable PR-FZ test anchor {relative_path}::{function_name}"
            )


def require_exact_coverage(root: Path) -> None:
    rows = read_tsv(
        root / "docs/PRODUCT_AUDIT_COVERAGE.tsv",
        (
            "file",
            "coverage_status",
            "owner_surface",
            "risk",
            "suggested_pr",
            "evidence",
            "notes",
        ),
    )
    by_path: dict[str, dict[str, str]] = {}
    for row in rows:
        relative_path = row["file"].strip()
        if relative_path in by_path:
            raise ValueError(f"duplicate coverage path: {relative_path}")
        by_path[relative_path] = row
    for relative_path in CLOSURE_PATHS:
        if not (root / relative_path).is_file():
            raise ValueError(f"missing designated PR-FZ closure path: {relative_path}")
        row = by_path.get(relative_path)
        if row is None:
            raise ValueError(f"coverage ledger lacks PR-FZ closure path: {relative_path}")
        if row["coverage_status"].strip() != "exact":
            raise ValueError(
                f"PR-FZ closure path is not exact: {relative_path} "
                f"({row['coverage_status'].strip()!r})"
            )
        if not row["evidence"].strip().startswith("line:"):
            raise ValueError(f"PR-FZ closure path lacks line evidence: {relative_path}")


def validate(root: Path) -> bool:
    if not completion_state(root):
        return False
    require_evidence(root)
    require_test_anchors(root)
    require_exact_coverage(root)
    return True


def fixture_tsv(rows: list[list[str]]) -> str:
    output = io.StringIO()
    writer = csv.writer(output, delimiter="\t", lineterminator="\n")
    writer.writerows(rows)
    return output.getvalue()


def write_self_test_fixture(root: Path) -> None:
    doc = root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
    doc.parent.mkdir(parents=True, exist_ok=True)
    doc.write_text(
        "### 🟡 6.3 建议整改顺序\n\n"
        "| PR | 状态 | 范围 | 验收标准 |\n"
        "|---|---|---|---|\n"
        f"| `{PR_FZ_TITLE}` | ✅ 完成 | 剩余：无。验证："
        "`scripts/check_pr_fz_completion.sh`。 | static closure |\n\n"
        "### 🟡 6.4 其它\n",
        encoding="utf-8",
    )
    evidence_rows = [["pr_id", "evidence_type", "artifact", "command", "notes"]]
    for evidence_type, (artifact, command_anchor) in REQUIRED_EVIDENCE.items():
        evidence_rows.append(
            ["PR-FZ", evidence_type, artifact, f"cargo test {command_anchor}", "self-test"]
        )
    (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").write_text(
        fixture_tsv(evidence_rows), encoding="utf-8"
    )
    coverage_rows = [[
        "file",
        "coverage_status",
        "owner_surface",
        "risk",
        "suggested_pr",
        "evidence",
        "notes",
    ]]
    for index, relative_path in enumerate(CLOSURE_PATHS, start=1):
        path = root / relative_path
        path.parent.mkdir(parents=True, exist_ok=True)
        path.touch()
        coverage_rows.append(
            [relative_path, "exact", "self_test", "P0", "PR-FZ", f"line:main:{index}", "exact"]
        )
    (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").write_text(
        fixture_tsv(coverage_rows), encoding="utf-8"
    )
    for relative_path, function_name in TEST_ANCHORS:
        path = root / relative_path
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open("a", encoding="utf-8") as handle:
            handle.write(f"\n#[test]\nfn {function_name}() {{}}\n")


def expect_failure(root: Path, expected: str) -> None:
    try:
        if not validate(root):
            raise AssertionError("completed self-test fixture was treated as partial")
    except ValueError as error:
        if expected not in str(error):
            raise AssertionError(f"expected failure containing {expected!r}, got {error!r}") from error
        return
    raise AssertionError(f"expected validation failure containing {expected!r}")


def self_test() -> None:
    with tempfile.TemporaryDirectory(prefix="crossline-pr-fz-gate-") as temp:
        root = Path(temp)
        write_self_test_fixture(root)
        validate(root)

        doc = root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
        doc.write_text(doc.read_text(encoding="utf-8").replace("✅ 完成", "🟡 部分完成"), encoding="utf-8")
        if validate(root):
            raise AssertionError("partial PR-FZ row must skip completion enforcement")
        write_self_test_fixture(root)

        evidence = root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv"
        lines = evidence.read_text(encoding="utf-8").splitlines()
        evidence.write_text("\n".join(lines[:-1]) + "\n", encoding="utf-8")
        expect_failure(root, "evidence type drift")
        write_self_test_fixture(root)

        anchor_path = root / TEST_ANCHORS[0][0]
        anchor_path.write_text(
            f"/* #[test]\nfn {TEST_ANCHORS[0][1]}() {{}} */\n", encoding="utf-8"
        )
        expect_failure(root, "missing runnable PR-FZ test anchor")
        write_self_test_fixture(root)

        coverage = root / "docs/PRODUCT_AUDIT_COVERAGE.tsv"
        coverage.write_text(
            coverage.read_text(encoding="utf-8").replace("\texact\t", "\tbasename\t", 1),
            encoding="utf-8",
        )
        expect_failure(root, "closure path is not exact")


root = Path(sys.argv[1])
mode = sys.argv[2]
try:
    if mode == "--self-test":
        self_test()
        print("OK PR-FZ completion gate self-test")
    else:
        if validate(root):
            print(
                "OK PR-FZ completion gate "
                f"({len(REQUIRED_EVIDENCE)} evidence types; "
                f"{len(TEST_ANCHORS)} test anchors; {len(CLOSURE_PATHS)} exact paths)"
            )
        else:
            print("SKIP PR-FZ completion gate (roadmap row remains partial)")
except (AssertionError, ValueError) as error:
    raise SystemExit(f"PR-FZ completion gate failed: {error}") from error
PY
