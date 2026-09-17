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

SKIP_UPSTREAM="$(printenv CROSSLINE_PR_I_SKIP_UPSTREAM 2>/dev/null || printf '0')"
if [ "$MODE" = "check" ] && [ "$SKIP_UPSTREAM" != "1" ]; then
  for gate in \
    check_pr_bx_completion.sh \
    check_pr_el_completion.sh \
    check_pr_c_completion.sh; do
    bash "$ROOT/scripts/$gate"
  done
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


PR_TITLE = "PR-I Shared Trading DTO"
EVIDENCE = {
    "shared-trading-dto-contract": (
        "shared-types/src/live_trading.rs",
        "submit_order_request_keeps_flat_http_shape",
    ),
    "frontend-dto-mirror": (
        "scripts/check_frontend_dto_mirror.sh",
        "bash scripts/check_frontend_dto_mirror.sh",
    ),
    "venue-runtime-health-contract": (
        "scripts/check_pr_bx_completion.sh",
        "bash scripts/check_pr_bx_completion.sh",
    ),
    "venue-client-order-id-policy": (
        "crates/exchange/src/client_order_id_policy.rs",
        "gate_policy_compacts_to_official_text_shape",
    ),
    "instrument-spec-order-compiler": (
        "crates/api/src/services/instrument_registry.rs",
        "canonical_resolution_requires_one_verified_native_contract",
    ),
    "ticket-order-plan-identity": (
        "shared-types/src/hedge.rs",
        "ticket_order_plans_keep_hyperliquid_builder_compile_and_identity_evidence",
    ),
    "live-ticket-idempotency": (
        "crates/api/src/routers/arbitrage/hedge_tests/pricing_confirm_partial_outcome.rs",
        "confirm_replay_with_hot_run_preserves_stored_partial_outcome",
    ),
    "finality-cost-handoff": (
        "scripts/check_pr_c_completion.sh",
        "bash scripts/check_pr_c_completion.sh",
    ),
    "completion-governance": (
        "scripts/check_pr_i_completion.sh",
        "bash scripts/check_pr_i_completion.sh --self-test",
    ),
}

TEST_ANCHORS = (
    (
        "shared-types/src/live_trading.rs",
        "submit_order_request_keeps_flat_http_shape",
    ),
    (
        "shared-types/src/live_trading.rs",
        "order_intent_keeps_client_order_id_policy_backward_compatible",
    ),
    (
        "crates/exchange/src/client_order_id_policy.rs",
        "gate_policy_compacts_to_official_text_shape",
    ),
    (
        "crates/exchange/src/client_order_id_policy.rs",
        "htx_policy_derives_positive_numeric_client_order_id",
    ),
    (
        "crates/exchange/src/client_order_id_policy.rs",
        "hyperliquid_policy_derives_128_bit_cloid_for_builder_venue",
    ),
    (
        "shared-types/src/hedge.rs",
        "ticket_order_plans_keep_hyperliquid_builder_compile_and_identity_evidence",
    ),
    (
        "crates/api/src/routers/arbitrage/hedge_tests/compile_basic.rs",
        "order_compile_plan_exposes_client_order_id_policy",
    ),
    (
        "crates/api/src/services/instrument_registry/tests.rs",
        "canonical_resolution_requires_one_verified_native_contract",
    ),
    (
        "crates/api/src/services/hedge_preview/guards.rs",
        "binance_identity_constraints_produce_execution_ready_usdc_plan",
    ),
    (
        "crates/api/src/routers/arbitrage/hedge_tests/pricing_confirm_partial_outcome.rs",
        "confirm_replay_with_hot_run_preserves_stored_partial_outcome",
    ),
    (
        "shared-types/src/venues/tests_runtime_health.rs",
        "runtime_health_exposes_every_pr_bx_operation_slot",
    ),
)

UPSTREAM_GATES = (
    "scripts/check_pr_bx_completion.sh",
    "scripts/check_pr_el_completion.sh",
    "scripts/check_pr_c_completion.sh",
)

CLOSURE_PATHS = (
    "scripts/check_pr_i_completion.sh",
    "scripts/check_frontend_dto_mirror.sh",
    "crates/api/src/services/instrument_registry.rs",
    *UPSTREAM_GATES,
    *(path for path, _ in TEST_ANCHORS),
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
        if line.startswith("| `PR-I "):
            rows.append([cell.strip() for cell in line.strip().strip("|").split("|")])
    if len(rows) != 1:
        raise ValueError(f"expected exactly one PR-I roadmap row, found {len(rows)}")
    cells = rows[0]
    if len(cells) != 4:
        raise ValueError(f"PR-I roadmap row must have 4 cells, found {len(cells)}")
    if cells[0].strip("`") != PR_TITLE:
        raise ValueError(f"PR-I roadmap title drifted: {cells[0]!r}")
    if cells[1] != "✅ 完成":
        raise ValueError(f"PR-I roadmap status must be complete, got {cells[1]!r}")
    if "剩余：无。" not in cells[2]:
        raise ValueError("PR-I completion must declare no remaining local work")
    if "bash scripts/check_pr_i_completion.sh --self-test" not in cells[3]:
        raise ValueError("PR-I completion must record its destructive self-test")

    queue_start = text.find("### 🟡 6.5")
    if queue_start < 0:
        raise ValueError("missing 6.5 execution queue")
    if re.search(r"(?m)^\d+\.\s+\*\*PR-I\b", text[queue_start:]):
        raise ValueError("completed PR-I must not remain in the 6.5 queue")


def require_evidence(root: Path) -> None:
    rows = read_tsv(
        root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv",
        ("pr_id", "evidence_type", "artifact", "command", "notes"),
    )
    i_rows = [row for row in rows if row["pr_id"].strip() == "PR-I"]
    by_type: dict[str, dict[str, str]] = {}
    for row in i_rows:
        evidence_type = row["evidence_type"].strip()
        if evidence_type in by_type:
            raise ValueError(f"duplicate PR-I evidence type: {evidence_type}")
        by_type[evidence_type] = row

    if set(by_type) != set(EVIDENCE):
        raise ValueError(
            "PR-I evidence type drift: "
            f"missing={sorted(set(EVIDENCE) - set(by_type))}, "
            f"extra={sorted(set(by_type) - set(EVIDENCE))}"
        )
    for evidence_type, (artifact, command_anchor) in EVIDENCE.items():
        row = by_type[evidence_type]
        if row["artifact"].strip() != artifact:
            raise ValueError(
                f"PR-I {evidence_type} artifact must be {artifact}, "
                f"got {row['artifact'].strip()!r}"
            )
        if command_anchor not in row["command"]:
            raise ValueError(
                f"PR-I {evidence_type} command must contain {command_anchor!r}"
            )
        if not (root / artifact).is_file():
            raise ValueError(f"PR-I {evidence_type} artifact is missing: {artifact}")


def require_runnable_tests(root: Path) -> None:
    for relative_path, function_name in TEST_ANCHORS:
        path = root / relative_path
        if not path.is_file():
            raise ValueError(f"missing PR-I test anchor file: {relative_path}")
        text = path.read_text(encoding="utf-8")
        pattern = re.compile(
            r"(?m)^\s*#\[(?:tokio::)?test(?:\([^\]]*\))?\]\s*\n"
            r"(?P<attrs>(?:\s*#\[[^\n]+\]\s*\n)*)"
            rf"\s*(?:pub\s+)?(?:async\s+)?fn\s+{re.escape(function_name)}\s*\("
        )
        match = pattern.search(text)
        if match is None:
            raise ValueError(
                f"missing runnable PR-I test anchor {relative_path}::{function_name}"
            )
        if "ignore" in match.group("attrs").lower():
            raise ValueError(
                f"PR-I test anchor must not be ignored: {relative_path}::{function_name}"
            )


def require_coverage(root: Path) -> None:
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
    matches = [
        row
        for row in rows
        if row["file"].strip() == "scripts/check_pr_i_completion.sh"
    ]
    if len(matches) != 1 or matches[0]["coverage_status"].strip() != "exact":
        raise ValueError("PR-I completion gate requires exact coverage ledger ownership")


def require_upstream_gates(root: Path) -> None:
    for relative_path in UPSTREAM_GATES:
        if not (root / relative_path).is_file():
            raise ValueError(f"missing PR-I upstream completion gate: {relative_path}")


def check(root: Path) -> None:
    if os.environ.get("CROSSLINE_PR_I_SKIP_UPSTREAM") != "1":
        require_upstream_gates(root)
    require_roadmap(root)
    require_evidence(root)
    require_runnable_tests(root)
    require_coverage(root)


def stage_fixture_docs(root: Path) -> None:
    docs = root / "docs"
    docs.mkdir(parents=True, exist_ok=True)
    (docs / "PRODUCT_FULL_AUDIT_REFINEMENT.md").write_text(
        "### 🟡 6.3 建议整改顺序\n\n"
        "| PR | 状态 | 范围 | 验收标准 |\n"
        "|---|---|---|---|\n"
        "| `PR-I Shared Trading DTO` | ✅ 完成 | shared contract closure; 剩余：无。 | "
        "验证：`bash scripts/check_pr_i_completion.sh --self-test` |\n\n"
        "### 🟡 6.4 下一批继续审计重点\n\n"
        "### 🟡 6.5 下一步执行队列\n",
        encoding="utf-8",
    )

    evidence = docs / "PRODUCT_AUDIT_EVIDENCE.tsv"
    evidence.write_text(
        "pr_id\tevidence_type\tartifact\tcommand\tnotes\n"
        + "".join(
            f"PR-I\t{evidence_type}\t{artifact}\t{command}\tfixture\n"
            for evidence_type, (artifact, command) in EVIDENCE.items()
        ),
        encoding="utf-8",
    )
    (docs / "PRODUCT_AUDIT_COVERAGE.tsv").write_text(
        "file\tcoverage_status\towner_surface\trisk\tsuggested_pr\tevidence\tnotes\n"
        "scripts/check_pr_i_completion.sh\texact\tverification_script\tP0\tPR-GI/PR-AW\t"
        "line:main:1\texact path referenced in product audit\n",
        encoding="utf-8",
    )


def run_fixture(root: Path, expect_pass: bool) -> None:
    environment = os.environ.copy()
    environment["CROSSLINE_PR_I_SKIP_UPSTREAM"] = "1"
    result = subprocess.run(
        ["bash", "scripts/check_pr_i_completion.sh"],
        cwd=root,
        text=True,
        capture_output=True,
        check=False,
        env=environment,
    )
    if expect_pass != (result.returncode == 0):
        detail = (result.stderr or result.stdout).strip()
        raise ValueError(f"PR-I destructive self-test expectation failed: {detail}")


def self_test(root: Path) -> None:
    with tempfile.TemporaryDirectory(prefix="crossline-pr-i-completion-") as temp:
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
                if not line.startswith("PR-I\tvenue-client-order-id-policy\t")
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
                "| `PR-I Shared Trading DTO` | ✅ 完成 |",
                "| `PR-I Shared Trading DTO` | 🟡 部分完成 |",
                1,
            ),
            encoding="utf-8",
        )
        run_fixture(fixture, False)
        doc.write_text(
            doc_baseline.replace(
                "### 🟡 6.5 下一步执行队列",
                "### 🟡 6.5 下一步执行队列\n\n1. **PR-I** — stale queue entry",
                1,
            ),
            encoding="utf-8",
        )
        run_fixture(fixture, False)
        doc.write_text(doc_baseline, encoding="utf-8")

        anchor = fixture / "shared-types/src/live_trading.rs"
        anchor_baseline = anchor.read_text(encoding="utf-8")
        anchor.write_text(
            anchor_baseline.replace(
                "    #[test]\n    fn submit_order_request_keeps_flat_http_shape",
                "    #[test]\n    #[ignore]\n    fn submit_order_request_keeps_flat_http_shape",
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
                "scripts/check_pr_i_completion.sh\texact\t",
                "scripts/check_pr_i_completion.sh\tmissing\t",
                1,
            ),
            encoding="utf-8",
        )
        run_fixture(fixture, False)

    print("OK PR-I completion destructive self-test")


root = Path(sys.argv[1])
mode = sys.argv[2]
try:
    if mode == "--self-test":
        self_test(root)
    else:
        check(root)
        print(
            "OK PR-I completion gate "
            f"({len(EVIDENCE)} evidence types, {len(TEST_ANCHORS)} Rust anchors, "
            f"{len(UPSTREAM_GATES)} upstream completion gates)"
        )
except (OSError, ValueError) as exc:
    raise SystemExit(f"PR-I completion gate failed: {exc}") from exc
PY
