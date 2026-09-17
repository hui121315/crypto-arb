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


BT = chr(96)
PR_FE_TITLE = "PR-FE CloseRun, Pairing Evidence & Emergency Action Contract"
REQUIRED_EVIDENCE = {
    "reciprocal-pair-evidence": (
        "crates/portfolio/src/pairing.rs",
        "applies_execution_run_pair_evidence",
    ),
    "auto-compensation-reconciliation": (
        "crates/api/src/services/close_runs/auto_compensation.rs",
        "auto_compensation_worker_submits_single_candidate_with_action_run",
    ),
    "compensation-retry-cost-ledger": (
        "crates/api/src/services/close_runs/tests/cases_c/compensation.rs",
        "auto_compensation_worker_retries_single_failed_candidate_once",
    ),
    "compensation-runtime-boundary": (
        "crates/api/src/services/close_runs/tests/cases_c/runtime.rs",
        "compensation_submit_runtime_requires_fresh_orderbook",
    ),
    "server-bound-compensation-idempotency": (
        "crates/api/src/routers/portfolio.rs",
        "close_run_compensation_submits_server_bound_order",
    ),
    "kill-switch-idempotency": (
        "crates/api/src/routers/trading/tests/cases_kill.rs",
        "set_kill_switch_replays_existing_idempotency_key_without_second_risk_event",
    ),
    "positions-recovery-browser": (
        "test/e2e/data_pipeline.spec.ts",
        "positions compensation cancel denial keeps CloseRun accepted and retryable",
    ),
}
RUST_TEST_ANCHORS = (
    ("crates/portfolio/src/pairing.rs", "applies_execution_run_pair_evidence"),
    ("crates/portfolio/src/pairing.rs", "does_not_apply_one_sided_evidence"),
    (
        "crates/api/src/services/close_runs/tests/cases_c/compensation.rs",
        "auto_compensation_worker_submits_single_candidate_with_action_run",
    ),
    (
        "crates/api/src/services/close_runs/tests/cases_c/compensation.rs",
        "auto_compensation_worker_retries_single_failed_candidate_once",
    ),
    (
        "crates/api/src/services/close_runs/tests/cases_c/recovery.rs",
        "auto_compensation_worker_skips_live_candidates",
    ),
    (
        "crates/api/src/services/close_runs/tests/cases_c/runtime.rs",
        "compensation_submit_runtime_requires_fresh_orderbook",
    ),
    (
        "crates/api/src/services/close_runs/tests/cases_c/runtime.rs",
        "generic_order_cannot_spoof_close_run_compensation_attempt",
    ),
    (
        "crates/api/src/routers/portfolio.rs",
        "close_run_compensation_submits_server_bound_order",
    ),
    (
        "crates/api/src/routers/portfolio.rs",
        "close_pair_replay_while_submitted_is_in_flight",
    ),
    (
        "crates/api/src/routers/trading/tests/cases_kill.rs",
        "set_kill_switch_replays_existing_idempotency_key_without_second_risk_event",
    ),
)
BROWSER_ANCHOR = (
    "test/e2e/data_pipeline.spec.ts",
    "positions compensation cancel denial keeps CloseRun accepted and retryable",
)
CLOSURE_PATHS = (
    "scripts/check_pr_fe_completion.sh",
    "scripts/verify_repo_gates.sh",
    "shared-types/src/portfolio/close.rs",
    "shared-types/src/portfolio/positions.rs",
    "crates/portfolio/src/pairing.rs",
    "crates/api/src/lifecycle/reconciliation.rs",
    "crates/api/src/services/close_runs/auto_compensation.rs",
    "crates/api/src/services/close_runs/auto_compensation/retry_policy.rs",
    "crates/api/src/services/close_runs/project.rs",
    "crates/api/src/services/close_runs/prepare.rs",
    "crates/api/src/services/close_runs/runtime.rs",
    "crates/api/src/services/close_runs/tests/cases_b/finality.rs",
    "crates/api/src/services/close_runs/tests/cases_c/compensation.rs",
    "crates/api/src/services/close_runs/tests/cases_c/recovery.rs",
    "crates/api/src/services/close_runs/tests/cases_c/runtime.rs",
    "crates/api/src/services/portfolio_actions.rs",
    "crates/api/src/routers/portfolio.rs",
    "crates/api/src/routers/trading/tests/cases_kill.rs",
    "crates/api/src/services/action_runs/tests/terminal_contract.rs",
    "frontend/src/panels/modules/positions/data/actions.rs",
    "frontend/src/panels/modules/positions/data/requests.rs",
    "frontend/src/panels/modules/positions/data/requests/close.rs",
    "frontend/src/panels/modules/positions/data/runs.rs",
    "frontend/src/panels/modules/positions/components/close_runs_panel.rs",
    "frontend/src/panels/modules/positions/components/close_runs_panel/derive.rs",
    "test/e2e/data_pipeline.spec.ts",
)


def read_tsv(path: Path, header: tuple[str, ...]) -> list[dict[str, str]]:
    if not path.is_file():
        raise ValueError(f"missing {path}")
    with path.open(encoding="utf-8", newline="") as handle:
        reader = csv.DictReader(handle, delimiter="\t")
        if tuple(reader.fieldnames or ()) != header:
            raise ValueError(f"{path.name} header drifted")
        return list(reader)


def roadmap_text(root: Path) -> tuple[str, list[str]]:
    text = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
    start = text.find("### 🟡 6.3")
    end = text.find("### 🟡 6.4", start)
    if start < 0 or end < 0:
        raise ValueError("missing bounded 6.3 roadmap section")
    rows = [
        [cell.strip() for cell in line.strip().strip("|").split("|")]
        for line in text[start:end].splitlines()
        if line.startswith("| " + BT + "PR-FE ")
    ]
    if len(rows) != 1 or len(rows[0]) != 4:
        raise ValueError("expected exactly one four-cell PR-FE roadmap row")
    return text, rows[0]


def completion_state(root: Path) -> bool:
    text, cells = roadmap_text(root)
    if cells[0].strip(BT) != PR_FE_TITLE:
        raise ValueError("PR-FE roadmap title drifted")
    if cells[1] == "🟡 部分完成":
        return False
    if cells[1] != "✅ 完成":
        raise ValueError(f"PR-FE status drifted: {cells[1]!r}")
    if "剩余：无。" not in cells[2] or "scripts/check_pr_fe_completion.sh" not in cells[2]:
        raise ValueError("PR-FE complete row must declare remaining-none and its gate")
    queue_start = text.find("### 🟡 6.5")
    queue_end = text.find("\n## ", queue_start + 1)
    queue = text[queue_start : queue_end if queue_end >= 0 else len(text)]
    if "PR-FE" in queue:
        raise ValueError("completed PR-FE must not remain in the active queue")
    return True


def require_evidence(root: Path) -> None:
    rows = read_tsv(
        root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv",
        ("pr_id", "evidence_type", "artifact", "command", "notes"),
    )
    seen = {row["evidence_type"].strip(): row for row in rows if row["pr_id"].strip() == "PR-FE"}
    if len(seen) != sum(row["pr_id"].strip() == "PR-FE" for row in rows):
        raise ValueError("duplicate PR-FE evidence type")
    expected = set(REQUIRED_EVIDENCE)
    if set(seen) != expected:
        raise ValueError(
            f"PR-FE evidence type drift: missing={sorted(expected - set(seen))}, "
            f"extra={sorted(set(seen) - expected)}"
        )
    for name, (artifact, command_anchor) in REQUIRED_EVIDENCE.items():
        row = seen[name]
        if row["artifact"].strip() != artifact or command_anchor not in row["command"]:
            raise ValueError(f"PR-FE evidence anchor drift: {name}")


def strip_rust_comments(text: str) -> str:
    text = re.sub(r"/\*.*?\*/", "", text, flags=re.S)
    return re.sub(r"//[^\n]*", "", text)


def require_rust_anchors(root: Path) -> None:
    for relative_path, function_name in RUST_TEST_ANCHORS:
        path = root / relative_path
        if not path.is_file():
            raise ValueError(f"missing PR-FE anchor file: {relative_path}")
        text = strip_rust_comments(path.read_text(encoding="utf-8"))
        pattern = re.compile(
            r"(?m)^\s*#\[(?:tokio::)?test(?:\([^\]]*\))?\]\s*\n"
            r"(?P<attrs>(?:\s*#\[[^\n]+\]\s*\n)*)"
            rf"\s*(?:async\s+)?fn\s+{re.escape(function_name)}\s*\("
        )
        match = pattern.search(text)
        if match is None:
            raise ValueError(f"missing runnable PR-FE anchor: {relative_path}::{function_name}")
        if "ignore" in match.group("attrs"):
            raise ValueError(f"PR-FE anchor is ignored: {relative_path}::{function_name}")


def require_browser_anchor(root: Path) -> None:
    relative_path, title = BROWSER_ANCHOR
    text = (root / relative_path).read_text(encoding="utf-8")
    escaped = re.escape(title)
    if re.search(rf'(?m)^\s*test\.skip\(\s*["\']{escaped}["\']', text):
        raise ValueError("PR-FE browser anchor must not be skipped")
    if not re.search(rf'(?m)^\s*test\(\s*["\']{escaped}["\']', text):
        raise ValueError("missing non-skipping PR-FE browser anchor")


def require_exact_coverage(root: Path) -> None:
    rows = read_tsv(
        root / "docs/PRODUCT_AUDIT_COVERAGE.tsv",
        ("file", "coverage_status", "owner_surface", "risk", "suggested_pr", "evidence", "notes"),
    )
    by_path = {row["file"].strip(): row for row in rows}
    for relative_path in CLOSURE_PATHS:
        if not (root / relative_path).is_file():
            raise ValueError(f"missing PR-FE closure path: {relative_path}")
        row = by_path.get(relative_path)
        if row is None or row["coverage_status"].strip() != "exact":
            raise ValueError(f"PR-FE closure path is not exact: {relative_path}")
        if not row["evidence"].strip().startswith("line:"):
            raise ValueError(f"PR-FE closure path lacks line evidence: {relative_path}")


def validate(root: Path) -> bool:
    if not completion_state(root):
        return False
    require_evidence(root)
    require_rust_anchors(root)
    require_browser_anchor(root)
    require_exact_coverage(root)
    return True


def tsv(rows: list[list[str]]) -> str:
    output = io.StringIO()
    csv.writer(output, delimiter="\t", lineterminator="\n").writerows(rows)
    return output.getvalue()


def write_fixture(root: Path) -> None:
    doc = root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
    doc.parent.mkdir(parents=True, exist_ok=True)
    doc.write_text(
        "### 🟡 6.3 建议整改顺序\n\n"
        "| PR | 状态 | 范围 | 验收标准 |\n|---|---|---|---|\n"
        f"| {BT}{PR_FE_TITLE}{BT} | ✅ 完成 | 剩余：无。验证：{BT}scripts/check_pr_fe_completion.sh{BT}。 | static |\n\n"
        "### 🟡 6.4 其它\n\n### 🟡 6.5 下一步执行队列\n1. PR-FF next local task\n",
        encoding="utf-8",
    )
    evidence = [["pr_id", "evidence_type", "artifact", "command", "notes"]]
    for name, (artifact, anchor) in REQUIRED_EVIDENCE.items():
        evidence.append(["PR-FE", name, artifact, f"cargo test {anchor}", "self-test"])
    (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").write_text(tsv(evidence), encoding="utf-8")
    coverage = [[
        "file", "coverage_status", "owner_surface", "risk", "suggested_pr", "evidence", "notes"
    ]]
    for index, relative_path in enumerate(CLOSURE_PATHS, start=1):
        path = root / relative_path
        path.parent.mkdir(parents=True, exist_ok=True)
        path.touch()
        coverage.append([relative_path, "exact", "self_test", "P0", "PR-FE", f"line:main:{index}", "exact"])
    (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").write_text(tsv(coverage), encoding="utf-8")
    by_path: dict[str, list[str]] = {}
    for relative_path, function_name in RUST_TEST_ANCHORS:
        by_path.setdefault(relative_path, []).append(function_name)
    for relative_path, function_names in by_path.items():
        path = root / relative_path
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(
            "".join(f"\n#[test]\nfn {name}() {{}}\n" for name in function_names),
            encoding="utf-8",
        )
    browser_path, title = BROWSER_ANCHOR
    path = root / browser_path
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(f'test("{title}", () => {{}});\n', encoding="utf-8")


def expect_failure(root: Path, expected: str) -> None:
    try:
        if not validate(root):
            raise AssertionError("completed fixture was treated as partial")
    except ValueError as error:
        if expected not in str(error):
            raise AssertionError(f"expected {expected!r}, got {error!r}") from error
        return
    raise AssertionError(f"expected validation failure containing {expected!r}")


def self_test() -> None:
    with tempfile.TemporaryDirectory(prefix="crossline-pr-fe-gate-") as temp:
        root = Path(temp)
        write_fixture(root)
        validate(root)
        doc = root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
        doc.write_text(doc.read_text(encoding="utf-8").replace("✅ 完成", "🟡 部分完成"), encoding="utf-8")
        if validate(root):
            raise AssertionError("partial PR-FE row must skip enforcement")
        write_fixture(root)
        evidence = root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv"
        evidence.write_text("\n".join(evidence.read_text(encoding="utf-8").splitlines()[:-1]) + "\n", encoding="utf-8")
        expect_failure(root, "evidence type drift")
        write_fixture(root)
        anchor_path = root / RUST_TEST_ANCHORS[0][0]
        anchor_path.write_text(f"/* #[test]\nfn {RUST_TEST_ANCHORS[0][1]}() {{}} */\n", encoding="utf-8")
        expect_failure(root, "missing runnable PR-FE anchor")
        write_fixture(root)
        anchor_path.write_text(f"#[test]\n#[ignore]\nfn {RUST_TEST_ANCHORS[0][1]}() {{}}\n", encoding="utf-8")
        expect_failure(root, "anchor is ignored")
        write_fixture(root)
        browser_path, title = BROWSER_ANCHOR
        (root / browser_path).write_text(f'test.skip("{title}", () => {{}});\n', encoding="utf-8")
        expect_failure(root, "browser anchor must not be skipped")
        write_fixture(root)
        coverage = root / "docs/PRODUCT_AUDIT_COVERAGE.tsv"
        coverage.write_text(coverage.read_text(encoding="utf-8").replace("\texact\t", "\tbasename\t", 1), encoding="utf-8")
        expect_failure(root, "closure path is not exact")


root = Path(sys.argv[1])
mode = sys.argv[2]
try:
    if mode == "--self-test":
        self_test()
        print("OK PR-FE completion gate self-test")
    elif validate(root):
        print(
            "OK PR-FE completion gate "
            f"({len(REQUIRED_EVIDENCE)} evidence types; {len(RUST_TEST_ANCHORS)} Rust anchors; "
            f"1 browser anchor; {len(CLOSURE_PATHS)} exact paths)"
        )
    else:
        print("SKIP PR-FE completion gate (roadmap row remains partial)")
except (AssertionError, ValueError) as error:
    raise SystemExit(f"PR-FE completion gate failed: {error}") from error
PY
