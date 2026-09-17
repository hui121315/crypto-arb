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

import csv
import io
import re
import sys
import tempfile
from pathlib import Path


PR_FD_TITLE = "PR-FD ExecutionRun Finality, Fill Ledger & Review Fact Source Contract"
REQUIRED_EVIDENCE = {
    "close-run-evidence": (
        "shared-types/src/portfolio/close.rs",
        "portfolio::tests::close",
    ),
    "run-finality-venue-outcome": (
        "crates/api/src/services/run_finality.rs",
        "run_finality",
    ),
    "cross-venue-funding-projection": (
        "crates/api/src/services/execution_runs/tests/cases_b.rs",
        "cross_venue_funding_events_project_each_linked_leg_once",
    ),
    "durable-funding-replay": (
        "crates/trading/src/journal/funding_replay_tests.rs",
        "cross_venue_funding_replay_preserves_run_leg_and_order_fact_links",
    ),
    "execution-close-cost-replay": (
        "crates/api/src/services/execution_runs/tests/cases_d.rs",
        "execution_cost_facts_replay_after_restart",
    ),
    "normalized-run-cost-fact-source": (
        "crates/trading/src/sql_ledger/run_cost.rs",
        "non_usd_funding_keeps_native_amount_without_fabricated_usd",
    ),
    "manual-terminal-cost-ack": (
        "crates/api/src/services/close_runs/tests/cases_b/manual_terminal.rs",
        "manual_terminal_ack_success_commits_once_with_manual_source",
    ),
}

TEST_ANCHORS = (
    (
        "crates/api/src/services/execution_runs/tests/cases_b.rs",
        "cross_venue_funding_events_project_each_linked_leg_once",
    ),
    (
        "crates/api/src/trading_service/funding_payments/tests.rs",
        "funding_payment_ingest_batch_preserves_events_for_runtime_projection",
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
        "crates/api/src/services/execution_runs/tests/cases_d.rs",
        "execution_cost_facts_replay_after_restart",
    ),
    (
        "crates/trading/src/sql_ledger/run_cost.rs",
        "non_usd_funding_keeps_native_amount_without_fabricated_usd",
    ),
    (
        "crates/trading/src/sql_ledger/run_cost.rs",
        "repeated_finality_snapshot_keeps_first_identical_fact_source",
    ),
    (
        "crates/api/src/services/close_runs/tests/cases_a.rs",
        "close_cost_facts_replay_after_restart",
    ),
    (
        "crates/api/src/services/close_runs/tests/cases_b/manual_terminal.rs",
        "manual_terminal_ack_success_commits_once_with_manual_source",
    ),
)

CLOSURE_PATHS = (
    "scripts/check_pr_fd_completion.sh",
    "scripts/verify_repo_gates.sh",
    "shared-types/src/portfolio/close.rs",
    "crates/api/src/services/run_finality.rs",
    "crates/api/src/lifecycle/funding_payments.rs",
    "crates/api/src/lifecycle/ledger_projection.rs",
    "crates/api/src/trading_service/funding_payments.rs",
    "crates/api/src/trading_service/private_ws_events/funding.rs",
    "crates/api/src/services/execution_runs/project.rs",
    "crates/api/src/services/execution_runs/tests/cases_b.rs",
    "crates/api/src/services/execution_runs/tests/cases_d.rs",
    "crates/api/src/services/execution_runs/tests/cases_d/run_cost_rebuild.rs",
    "crates/api/src/services/close_runs/project.rs",
    "crates/api/src/services/close_runs/project/events.rs",
    "crates/api/src/services/close_runs/tests/cases_a.rs",
    "crates/api/src/services/close_runs/tests/cases_b/manual_terminal.rs",
    "crates/trading/src/ledger.rs",
    "crates/trading/src/journal/projection/part_04.rs",
    "crates/trading/src/journal/funding_replay_tests.rs",
    "crates/trading/src/sql_ledger/run_cost.rs",
    "crates/trading/src/sql_ledger/run_cost_rebuild.rs",
    "crates/trading/tests/funding_journal_test.rs",
)


def read_tsv(path: Path, expected_header: tuple[str, ...]) -> list[dict[str, str]]:
    if not path.is_file():
        raise ValueError(f"missing {path}")
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
        if line.startswith("| `PR-FD "):
            rows.append([cell.strip() for cell in line.strip().strip("|").split("|")])
    if len(rows) != 1:
        raise ValueError(f"expected exactly one PR-FD roadmap row, found {len(rows)}")
    cells = rows[0]
    if len(cells) != 4:
        raise ValueError(f"PR-FD roadmap row must have 4 cells, found {len(cells)}")
    if cells[0].strip("`") != PR_FD_TITLE:
        raise ValueError(f"PR-FD roadmap title drifted: {cells[0]!r}")
    if cells[1] == "🟡 部分完成":
        return False
    if cells[1] != "✅ 完成":
        raise ValueError(f"PR-FD roadmap row has unsupported status: {cells[1]!r}")
    if "剩余：无。" not in cells[2]:
        raise ValueError("PR-FD complete row must declare 剩余：无。")
    if "scripts/check_pr_fd_completion.sh" not in cells[2]:
        raise ValueError("PR-FD complete row must record the static completion gate")
    return True


def require_evidence(root: Path) -> None:
    rows = read_tsv(
        root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv",
        ("pr_id", "evidence_type", "artifact", "command", "notes"),
    )
    fd_rows = [row for row in rows if row["pr_id"].strip() == "PR-FD"]
    seen: dict[str, dict[str, str]] = {}
    for row in fd_rows:
        evidence_type = row["evidence_type"].strip()
        if evidence_type in seen:
            raise ValueError(f"duplicate PR-FD evidence type: {evidence_type}")
        seen[evidence_type] = row
    expected = set(REQUIRED_EVIDENCE)
    actual = set(seen)
    if actual != expected:
        missing = sorted(expected - actual)
        extra = sorted(actual - expected)
        raise ValueError(f"PR-FD evidence type drift: missing={missing}, extra={extra}")
    for evidence_type, (artifact, command_anchor) in REQUIRED_EVIDENCE.items():
        row = seen[evidence_type]
        if row["artifact"].strip() != artifact:
            raise ValueError(
                f"PR-FD {evidence_type} artifact must be {artifact}, "
                f"got {row['artifact'].strip()!r}"
            )
        if command_anchor not in row["command"]:
            raise ValueError(
                f"PR-FD {evidence_type} command must name {command_anchor}"
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
            raise ValueError(f"missing PR-FD anchor file: {relative_path}")
        text = strip_rust_comments(path.read_text(encoding="utf-8"))
        pattern = re.compile(
            r"(?m)^\s*#\[(?:tokio::)?test(?:\([^\]]*\))?\]\s*\n"
            r"(?P<attrs>(?:\s*#\[[^\n]+\]\s*\n)*)"
            rf"\s*(?:async\s+)?fn\s+{re.escape(function_name)}\s*\("
        )
        match = pattern.search(text)
        if match is None:
            raise ValueError(
                f"missing runnable PR-FD test anchor {relative_path}::{function_name}"
            )
        if "ignore" in match.group("attrs"):
            raise ValueError(
                f"PR-FD test anchor must not be ignored: {relative_path}::{function_name}"
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
    by_path = {row["file"].strip(): row for row in rows}
    for relative_path in CLOSURE_PATHS:
        if not (root / relative_path).is_file():
            raise ValueError(f"missing designated PR-FD closure path: {relative_path}")
        row = by_path.get(relative_path)
        if row is None:
            raise ValueError(f"coverage ledger lacks PR-FD closure path: {relative_path}")
        if row["coverage_status"].strip() != "exact":
            raise ValueError(
                f"PR-FD closure path is not exact: {relative_path} "
                f"({row['coverage_status'].strip()!r})"
            )
        if not row["evidence"].strip().startswith("line:"):
            raise ValueError(f"PR-FD closure path lacks line evidence: {relative_path}")


def validate(root: Path) -> bool:
    if not completion_state(root):
        return False
    require_evidence(root)
    require_test_anchors(root)
    require_exact_coverage(root)
    return True


def fixture_tsv(rows: list[list[str]]) -> str:
    output = io.StringIO()
    csv.writer(output, delimiter="\t", lineterminator="\n").writerows(rows)
    return output.getvalue()


def write_self_test_fixture(root: Path) -> None:
    doc = root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
    doc.parent.mkdir(parents=True, exist_ok=True)
    doc.write_text(
        "### 🟡 6.3 建议整改顺序\n\n"
        "| PR | 状态 | 范围 | 验收标准 |\n"
        "|---|---|---|---|\n"
        f"| `{PR_FD_TITLE}` | ✅ 完成 | 剩余：无。验证："
        "`scripts/check_pr_fd_completion.sh`。 | static closure |\n\n"
        "### 🟡 6.4 其它\n",
        encoding="utf-8",
    )
    evidence_rows = [["pr_id", "evidence_type", "artifact", "command", "notes"]]
    for evidence_type, (artifact, command_anchor) in REQUIRED_EVIDENCE.items():
        evidence_rows.append(
            ["PR-FD", evidence_type, artifact, f"cargo test {command_anchor}", "self-test"]
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
            [relative_path, "exact", "self_test", "P0", "PR-FD", f"line:main:{index}", "exact"]
        )
    (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").write_text(
        fixture_tsv(coverage_rows), encoding="utf-8"
    )
    anchors_by_path: dict[str, list[str]] = {}
    for relative_path, function_name in TEST_ANCHORS:
        anchors_by_path.setdefault(relative_path, []).append(function_name)
    for relative_path, function_names in anchors_by_path.items():
        path = root / relative_path
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(
            "".join(f"\n#[test]\nfn {function_name}() {{}}\n" for function_name in function_names),
            encoding="utf-8",
        )


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
    with tempfile.TemporaryDirectory(prefix="crossline-pr-fd-gate-") as temp:
        root = Path(temp)
        write_self_test_fixture(root)
        validate(root)

        doc = root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
        doc.write_text(doc.read_text(encoding="utf-8").replace("✅ 完成", "🟡 部分完成"), encoding="utf-8")
        if validate(root):
            raise AssertionError("partial PR-FD row must skip completion enforcement")
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
        expect_failure(root, "missing runnable PR-FD test anchor")
        write_self_test_fixture(root)

        anchor_path = root / TEST_ANCHORS[0][0]
        anchor_path.write_text(
            f"#[test]\n#[ignore]\nfn {TEST_ANCHORS[0][1]}() {{}}\n", encoding="utf-8"
        )
        expect_failure(root, "test anchor must not be ignored")
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
        print("OK PR-FD completion gate self-test")
    elif validate(root):
        print(
            "OK PR-FD completion gate "
            f"({len(REQUIRED_EVIDENCE)} evidence types; "
            f"{len(TEST_ANCHORS)} test anchors; {len(CLOSURE_PATHS)} exact paths)"
        )
    else:
        print("SKIP PR-FD completion gate (roadmap row remains partial)")
except (AssertionError, ValueError) as error:
    raise SystemExit(f"PR-FD completion gate failed: {error}") from error
PY
