#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
if [ "$#" -gt 0 ]; then
  MODE="$1"
else
  MODE="check"
fi

if [ "$MODE" != "check" ] && [ "$MODE" != "--self-test" ]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

SKIP_UPSTREAM="$(printenv CROSSLINE_PR_L_SKIP_UPSTREAM 2>/dev/null || printf '0')"
if [ "$MODE" = "check" ] && [ "$SKIP_UPSTREAM" != "1" ]; then
  bash "$ROOT/scripts/check_pr_c_completion.sh"
fi

python3 - "$ROOT" "$MODE" <<'PY'
from __future__ import annotations

import csv
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile


PR_TITLE = "PR-L Profitability Scoring"
EVIDENCE = {
    "fee-source-registry": (
        "crates/arbitrage/src/algorithms/fee_evidence.rs",
        "algorithms::fee_evidence",
    ),
    "yield-basis-traceability": (
        "shared-types/src/fees.rs",
        "cargo test -p shared-types fees --lib",
    ),
    "ticket-depth-cost-guard": (
        "crates/api/src/services/hedge_ticket/tests/cost_evidence.rs",
        "services::hedge_ticket::tests::cost_evidence",
    ),
    "score-ranking-evidence": (
        "crates/arbitrage/src/calculator/tests.rs",
        "score_breakdown_carries_verified_fee_evidence_ids",
    ),
    "verified-list-cost-option-contract": (
        "crates/api/src/services/opportunity/row.rs",
        "services::opportunity::tests::list",
    ),
    "snapshot-none-zero-delta": (
        "crates/api/src/lifecycle/snapshot/top_window.rs",
        "lifecycle::snapshot",
    ),
    "frontend-unverified-cost-diagnostic": (
        "frontend/src/panels/modules/opportunity_view_model/model.rs",
        "opportunity_view_model",
    ),
    "fq-evidence-registry": (
        "docs/PRODUCT_AUDIT_EVIDENCE.tsv",
        "bash scripts/check_pr_l_completion.sh",
    ),
    "durable-cost-handoff": (
        "scripts/check_pr_c_completion.sh",
        "bash scripts/check_pr_c_completion.sh",
    ),
    "completion-governance": (
        "scripts/check_pr_l_completion.sh",
        "bash scripts/check_pr_l_completion.sh --self-test",
    ),
}

FQ_EVIDENCE = {
    "frontend-evidence": (
        "frontend/src/panels/modules/execution/data/preview/model.rs",
        "score_evidence_surfaces_penalty_and_fee_ids",
    ),
    "hedge-ticket-depth": (
        "crates/api/src/services/hedge_ticket/cost.rs",
        "services::hedge_ticket::tests::cost_evidence",
    ),
    "yield-basis-contract": (
        "shared-types/src/fees.rs",
        "cargo test -p shared-types fees --lib",
    ),
    "fee-registry-schema": (
        "crates/arbitrage/src/algorithms/fee_evidence.rs",
        "algorithms::fee_evidence",
    ),
    "aggregate-local-gate": (
        "docs/PRODUCT_AUDIT_EVIDENCE.tsv",
        "bash scripts/check_product_audit_evidence_index.sh",
    ),
}

TEST_ANCHORS = (
    (
        "shared-types/src/arbitrage.rs",
        "execution_ready_requires_market_depth_and_fee_evidence",
    ),
    ("shared-types/src/fees.rs", "official_schedule_requires_valid_evidence"),
    (
        "shared-types/src/fees.rs",
        "funding_window_mismatch_requires_explicit_basis_and_settlement_trace",
    ),
    (
        "crates/arbitrage/src/calculator/tests.rs",
        "score_breakdown_carries_verified_fee_evidence_ids",
    ),
    (
        "crates/api/src/services/hedge_ticket/tests/cost_evidence.rs",
        "ticket_cost_requires_both_fee_snapshots",
    ),
    (
        "crates/api/src/services/opportunity/tests/list.rs",
        "list_row_does_not_verify_cost_from_score_evidence_only",
    ),
    (
        "crates/api/src/lifecycle/snapshot/top_window.rs",
        "optional_number_fingerprint_distinguishes_missing_from_zero",
    ),
    (
        "frontend/src/panels/modules/opportunity_view_model/testing.rs",
        "view_model_marks_partial_cost_evidence",
    ),
    (
        "frontend/src/panels/modules/execution/components/risk_preview/tests/cases.rs",
        "missing_one_cycle_cost_hides_zero_cost_values",
    ),
)

UPSTREAM_GATES = ("scripts/check_pr_c_completion.sh",)
CLOSURE_PATHS = (
    "scripts/check_pr_l_completion.sh",
    *UPSTREAM_GATES,
    *(path for path, _ in EVIDENCE.values()),
    *(path for path, _ in FQ_EVIDENCE.values()),
    *(path for path, _ in TEST_ANCHORS),
)
TSV_HEADER = ("pr_id", "evidence_type", "artifact", "command", "notes")
COVERAGE_HEADER = (
    "file",
    "coverage_status",
    "owner_surface",
    "risk",
    "suggested_pr",
    "evidence",
    "notes",
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


def require_roadmap(root: Path) -> None:
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
        if line.startswith("| `PR-L "):
            rows.append([cell.strip() for cell in line.strip().strip("|").split("|")])
    if len(rows) != 1:
        raise ValueError(f"expected exactly one PR-L roadmap row, found {len(rows)}")
    cells = rows[0]
    if len(cells) != 4:
        raise ValueError(f"PR-L roadmap row must have 4 cells, found {len(cells)}")
    if cells[0].strip("`") != PR_TITLE:
        raise ValueError(f"PR-L roadmap title drifted: {cells[0]!r}")
    if cells[1] != "✅ 完成":
        raise ValueError(f"PR-L roadmap status must be complete, got {cells[1]!r}")
    if "剩余：无。" not in cells[2]:
        raise ValueError("PR-L completion must declare no remaining local work")
    if "bash scripts/check_pr_l_completion.sh --self-test" not in cells[3]:
        raise ValueError("PR-L completion must record its destructive self-test")

    queue_start = text.find("### 🟡 6.5")
    if queue_start < 0:
        raise ValueError("missing 6.5 execution queue")
    queue = text[queue_start:]
    if re.search(r"(?m)^\d+\.\s+\*\*PR-L\b", queue):
        raise ValueError("completed PR-L must not remain in the 6.5 queue")
    if not re.search(r"(?m)^1\.\s+\*\*PR-[A-Z0-9-]+\b", queue):
        raise ValueError("completed PR-L requires a concrete next local execution queue head")


def require_evidence_set(
    root: Path,
    pr_id: str,
    expected: dict[str, tuple[str, str]],
) -> None:
    rows = read_tsv(root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv", TSV_HEADER)
    pr_rows = [row for row in rows if row["pr_id"].strip() == pr_id]
    by_type: dict[str, dict[str, str]] = {}
    for row in pr_rows:
        evidence_type = row["evidence_type"].strip()
        if evidence_type in by_type:
            raise ValueError(f"duplicate {pr_id} evidence type: {evidence_type}")
        by_type[evidence_type] = row
    if set(by_type) != set(expected):
        raise ValueError(
            f"{pr_id} evidence type drift: "
            f"missing={sorted(set(expected) - set(by_type))}, "
            f"extra={sorted(set(by_type) - set(expected))}"
        )
    for evidence_type, (artifact, command_anchor) in expected.items():
        row = by_type[evidence_type]
        if row["artifact"].strip() != artifact:
            raise ValueError(
                f"{pr_id} {evidence_type} artifact must be {artifact}, "
                f"got {row['artifact'].strip()!r}"
            )
        if command_anchor not in row["command"]:
            raise ValueError(
                f"{pr_id} {evidence_type} command must contain {command_anchor!r}"
            )
        if not (root / artifact).is_file():
            raise ValueError(f"{pr_id} {evidence_type} artifact is missing: {artifact}")


def require_runnable_tests(root: Path) -> None:
    for relative_path, function_name in TEST_ANCHORS:
        path = root / relative_path
        if not path.is_file():
            raise ValueError(f"missing PR-L test anchor file: {relative_path}")
        text = path.read_text(encoding="utf-8")
        pattern = re.compile(
            r"(?m)^\s*#\[(?:tokio::)?test(?:\([^\]]*\))?\]\s*\n"
            r"(?P<attrs>(?:\s*#\[[^\n]+\]\s*\n)*)"
            rf"\s*(?:pub\s+)?(?:async\s+)?fn\s+{re.escape(function_name)}\s*\("
        )
        match = pattern.search(text)
        if match is None:
            raise ValueError(
                f"missing runnable PR-L test anchor {relative_path}::{function_name}"
            )
        if "ignore" in match.group("attrs").lower():
            raise ValueError(
                f"PR-L test anchor must not be ignored: {relative_path}::{function_name}"
            )


def require_coverage(root: Path) -> None:
    rows = read_tsv(root / "docs/PRODUCT_AUDIT_COVERAGE.tsv", COVERAGE_HEADER)
    matches = [
        row
        for row in rows
        if row["file"].strip() == "scripts/check_pr_l_completion.sh"
    ]
    if len(matches) != 1 or matches[0]["coverage_status"].strip() != "exact":
        raise ValueError("PR-L completion gate requires exact coverage ledger ownership")


def require_upstream_gates(root: Path) -> None:
    for relative_path in UPSTREAM_GATES:
        if not (root / relative_path).is_file():
            raise ValueError(f"missing PR-L upstream completion gate: {relative_path}")


def check(root: Path) -> None:
    if os.environ.get("CROSSLINE_PR_L_SKIP_UPSTREAM") != "1":
        require_upstream_gates(root)
    require_roadmap(root)
    require_evidence_set(root, "PR-FQ", FQ_EVIDENCE)
    require_evidence_set(root, "PR-L", EVIDENCE)
    require_runnable_tests(root)
    require_coverage(root)


def stage_fixture_docs(root: Path) -> None:
    docs = root / "docs"
    docs.mkdir(parents=True, exist_ok=True)
    (docs / "PRODUCT_FULL_AUDIT_REFINEMENT.md").write_text(
        "### 🟡 6.3 建议整改顺序\n\n"
        "| PR | 状态 | 范围 | 验收标准 |\n"
        "|---|---|---|---|\n"
        "| `PR-L Profitability Scoring` | ✅ 完成 | verified profitability closure; 剩余：无。 | "
        "验证：`bash scripts/check_pr_l_completion.sh --self-test` |\n\n"
        "### 🟡 6.4 下一批继续审计重点\n\n"
        "### 🟡 6.5 下一步执行队列\n\n"
        "1. **PR-Q Config & Credential Safety** — next local queue item\n",
        encoding="utf-8",
    )

    evidence_lines = ["\t".join(TSV_HEADER)]
    for pr_id, expected in (("PR-FQ", FQ_EVIDENCE), ("PR-L", EVIDENCE)):
        evidence_lines.extend(
            "\t".join((pr_id, evidence_type, artifact, command, "fixture"))
            for evidence_type, (artifact, command) in expected.items()
        )
    (docs / "PRODUCT_AUDIT_EVIDENCE.tsv").write_text(
        "\n".join(evidence_lines) + "\n",
        encoding="utf-8",
    )
    (docs / "PRODUCT_AUDIT_COVERAGE.tsv").write_text(
        "\t".join(COVERAGE_HEADER)
        + "\n"
        + "scripts/check_pr_l_completion.sh\texact\tverification_script\tP0\tPR-L\t"
        "line:main:1\texact path referenced in product audit\n",
        encoding="utf-8",
    )


def run_fixture(root: Path, expect_pass: bool) -> None:
    environment = os.environ.copy()
    environment["CROSSLINE_PR_L_SKIP_UPSTREAM"] = "1"
    result = subprocess.run(
        ["bash", "scripts/check_pr_l_completion.sh"],
        cwd=root,
        text=True,
        capture_output=True,
        check=False,
        env=environment,
    )
    if expect_pass != (result.returncode == 0):
        detail = (result.stderr or result.stdout).strip()
        raise ValueError(f"PR-L destructive self-test expectation failed: {detail}")


def self_test(root: Path) -> None:
    with tempfile.TemporaryDirectory(prefix="crossline-pr-l-completion-") as temp:
        fixture = Path(temp) / "repo"
        for relative_path in dict.fromkeys(CLOSURE_PATHS):
            source = root / relative_path
            if not source.is_file():
                raise ValueError(f"self-test source missing: {relative_path}")
            target = fixture / relative_path
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, target)
        stage_fixture_docs(fixture)
        run_fixture(fixture, True)

        evidence = fixture / "docs/PRODUCT_AUDIT_EVIDENCE.tsv"
        evidence_baseline = evidence.read_text(encoding="utf-8")
        evidence.write_text(
            "\n".join(
                line
                for line in evidence_baseline.splitlines()
                if not line.startswith("PR-L\tverified-list-cost-option-contract\t")
            )
            + "\n",
            encoding="utf-8",
        )
        run_fixture(fixture, False)
        evidence.write_text(evidence_baseline, encoding="utf-8")

        evidence.write_text(
            "\n".join(
                line
                for line in evidence_baseline.splitlines()
                if not line.startswith("PR-FQ\taggregate-local-gate\t")
            )
            + "\n",
            encoding="utf-8",
        )
        run_fixture(fixture, False)
        evidence.write_text(evidence_baseline, encoding="utf-8")

        doc = fixture / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
        doc_baseline = doc.read_text(encoding="utf-8")
        doc.write_text(
            doc_baseline.replace(
                "| `PR-L Profitability Scoring` | ✅ 完成 |",
                "| `PR-L Profitability Scoring` | 🟡 部分完成 |",
                1,
            ),
            encoding="utf-8",
        )
        run_fixture(fixture, False)
        doc.write_text(
            doc_baseline.replace(
                "### 🟡 6.5 下一步执行队列",
                "### 🟡 6.5 下一步执行队列\n\n1. **PR-L** — stale queue entry",
                1,
            ),
            encoding="utf-8",
        )
        run_fixture(fixture, False)
        doc.write_text(doc_baseline, encoding="utf-8")

        anchor = fixture / "crates/api/src/services/opportunity/tests/list.rs"
        anchor_baseline = anchor.read_text(encoding="utf-8")
        anchor.write_text(
            anchor_baseline.replace(
                "#[test]\nfn list_row_does_not_verify_cost_from_score_evidence_only",
                "#[test]\n#[ignore]\nfn list_row_does_not_verify_cost_from_score_evidence_only",
                1,
            ),
            encoding="utf-8",
        )
        run_fixture(fixture, False)
        anchor.write_text(anchor_baseline, encoding="utf-8")

        coverage = fixture / "docs/PRODUCT_AUDIT_COVERAGE.tsv"
        coverage_baseline = coverage.read_text(encoding="utf-8")
        coverage.write_text(
            coverage_baseline.replace(
                "scripts/check_pr_l_completion.sh\texact\t",
                "scripts/check_pr_l_completion.sh\tmissing\t",
                1,
            ),
            encoding="utf-8",
        )
        run_fixture(fixture, False)

    print("OK PR-L completion destructive self-test")


root = Path(sys.argv[1])
mode = sys.argv[2]
try:
    if mode == "--self-test":
        self_test(root)
    else:
        check(root)
        print(
            "OK PR-L completion gate "
            f"({len(EVIDENCE)} evidence types, {len(FQ_EVIDENCE)} PR-FQ evidence types, "
            f"{len(TEST_ANCHORS)} Rust anchors, {len(UPSTREAM_GATES)} upstream completion gate)"
        )
except (OSError, ValueError) as exc:
    raise SystemExit(f"PR-L completion gate failed: {exc}") from exc
PY
