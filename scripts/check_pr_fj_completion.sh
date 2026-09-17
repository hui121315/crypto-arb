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
import io
import re
import sys
import tempfile


PR_TITLE = "PR-FJ Security Verification, CI Gate & Runtime Smoke Contract"
ROUTE_BROWSER_PATH = "test/e2e/route_registry.spec.ts"
ROUTE_BROWSER_TITLE = (
    "high-risk action and secret mutation persist correlated redacted audit pairs"
)
PRODUCT_BROWSER_PATH = "test/e2e/data_pipeline.spec.ts"
PRODUCT_BROWSER_TITLES = (
    "settings risk kill switch surfaces extractor 422 typed problem",
    "positions page surfaces portfolio snapshot LoadState failure",
    "positions close denial keeps the position and typed retry context",
    "positions compensation cancel denial keeps CloseRun accepted and retryable",
    "hedge confirm policy denial preserves typed error without a fake run state",
    "execution order cancel denial keeps the pending run and typed retry context",
    "settings credential save denial redacts secret and shows no success feedback",
)
REQUIRED_EVIDENCE = {
    "mutation-route-matrix-gate": (
        "scripts/check_mutation_audit_contract.sh",
        "bash scripts/check_mutation_audit_contract.sh",
    ),
    "runtime-audit-persistence-restart-redaction-gate": (
        "scripts/verify_api_security_runtime_smoke.sh",
        "bash scripts/verify_api_security_runtime_smoke.sh",
    ),
    "route-registry-browser-gate": (
        ROUTE_BROWSER_PATH,
        "playwright test test/e2e/route_registry.spec.ts",
    ),
    "product-security-browser-gate": (
        PRODUCT_BROWSER_PATH,
        "CI=1 npm run test:e2e:data-pipeline",
    ),
    "action-run-terminal-correlation-gate": (
        "crates/api/src/services/action_runs/tests/terminal_contract.rs",
        "core_high_risk_mutations_preserve_identity_across_terminal_outcomes",
    ),
    "completion-governance-gate": (
        "scripts/check_pr_fj_completion.sh",
        "bash scripts/check_pr_fj_completion.sh --self-test",
    ),
}
RUST_TEST_ANCHORS = (
    ("crates/api/src/app.rs", "route_inventory_high_risk_routes_declare_mutation_audit_policy"),
    ("crates/api/src/app.rs", "route_inventory_action_run_policies_match_typed_runtime_registry"),
    ("crates/api/src/routers/extractors.rs", "json_data_error_returns_typed_problem_with_request_id"),
    (
        "crates/api/src/routers/trading/tests/cases_risk_config.rs",
        "risk_config_replay_returns_first_payload_without_overwriting_newer_config",
    ),
    (
        "crates/api/src/routers/trading/tests/cases_kill.rs",
        "set_kill_switch_rejects_missing_reason_and_records_failed_action",
    ),
    (
        "crates/api/src/routers/trading/tests/cases_submit.rs",
        "submit_order_requires_client_order_id",
    ),
    (
        "crates/api/src/routers/arbitrage/hedge_tests/pricing_confirm_context.rs",
        "confirm_missing_preview_terminalizes_action_run_and_preserves_identity",
    ),
    (
        "crates/api/src/services/action_runs/tests/terminal_contract.rs",
        "core_high_risk_mutations_preserve_identity_across_terminal_outcomes",
    ),
)
STATIC_MARKERS = {
    "scripts/check_mutation_audit_contract.sh": (
        "require_audit_action",
        "PortfolioCloseManualTerminal",
        "inventory_high_risk_count",
    ),
    "scripts/check_route_runtime_policy.sh": (
        "high_risk_secret",
        "actionRunId",
        "idempotencyKey",
        "CREDENTIAL_PERMISSION_DENIED",
    ),
    "scripts/verify_api_security_runtime_smoke.sh": (
        "stop_api",
        "start_api",
        "credential_run_id",
        "cross-route accepted/terminal audit pairs",
        "audit log did not append correlated evidence across restart",
        "restarted audit log leaked a credential sentinel",
    ),
}
CLOSURE_PATHS = tuple("""
.github/workflows/ci.yml
crates/api/src/app.rs
crates/api/src/route_specs.rs
crates/api/src/services/hedge_confirm/confirm.rs
crates/api/src/routers/arbitrage/hedge_tests/pricing_confirm_context.rs
crates/api/src/routers/exchanges.rs
crates/api/src/routers/extractors.rs
crates/api/src/routers/mod.rs
crates/api/src/routers/trading.rs
crates/api/src/routers/trading/account.rs
crates/api/src/routers/trading/kill_switch.rs
crates/api/src/routers/trading/orders.rs
crates/api/src/routers/trading/tests/cases_risk_config.rs
crates/api/src/routers/trading/tests/cases_kill.rs
crates/api/src/routers/trading/tests/cases_submit.rs
crates/api/src/routers/trading/tests/fixtures.rs
crates/api/src/services/action_runs/audit_log.rs
crates/api/src/services/action_runs/mutate.rs
crates/api/src/services/action_runs/tests.rs
crates/api/src/services/action_runs/tests/terminal_contract.rs
scripts/check_mutation_audit_contract.sh
scripts/check_pr_fj_completion.sh
scripts/check_product_audit_evidence_index.sh
scripts/check_route_runtime_policy.sh
scripts/verify_api_security_runtime_smoke.sh
scripts/verify_repo_gates.sh
scripts/verify_runtime_contracts.sh
scripts/verify_security_contract.sh
shared-types/src/problem.rs
test/e2e/data_pipeline.spec.ts
test/e2e/fixtures/route_runtime_policy.mjs
test/e2e/mock_api.mjs
test/e2e/route_registry.spec.ts
""".split())
LINE_EVIDENCE = re.compile(r"line:[A-Za-z0-9_.-]+:[1-9][0-9]*(?:-[1-9][0-9]*)?")

if len(CLOSURE_PATHS) != 33 or len(CLOSURE_PATHS) != len(set(CLOSURE_PATHS)):
    raise SystemExit("PR-FJ completion gate internal error: expected 33 unique closure paths")


def read_tsv(path: Path, header: tuple[str, ...]) -> list[dict[str, str]]:
    if not path.is_file():
        raise ValueError(f"missing {path.name}")
    with path.open(encoding="utf-8", newline="") as handle:
        reader = csv.DictReader(handle, delimiter="\t")
        if tuple(reader.fieldnames or ()) != header:
            raise ValueError(f"{path.name} header drifted")
        return list(reader)


def bounded_section(text: str, number: str) -> str:
    match = re.search(rf"(?m)^### .*\b{re.escape(number)}\b.*$", text)
    if not match:
        raise ValueError(f"missing bounded {number} section")
    following = re.search(r"(?m)^### ", text[match.end():])
    end = match.end() + following.start() if following else len(text)
    return text[match.start():end]


def completion_state(root: Path) -> bool:
    path = root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
    if not path.is_file():
        raise ValueError("missing docs/PRODUCT_FULL_AUDIT_REFINEMENT.md")
    section = bounded_section(path.read_text(encoding="utf-8"), "6.3")
    rows = [
        [cell.strip() for cell in line.strip().strip("|").split("|")]
        for line in section.splitlines()
        if line.startswith("| `PR-FJ ")
    ]
    if len(rows) != 1 or len(rows[0]) != 4:
        raise ValueError(f"expected one four-cell PR-FJ roadmap row, found {len(rows)}")
    cells = rows[0]
    if cells[0].strip("`") != PR_TITLE:
        raise ValueError("PR-FJ roadmap title drifted")
    if cells[1] == "🟡 部分完成":
        return False
    if cells[1] != "✅ 完成":
        raise ValueError(f"unsupported PR-FJ status: {cells[1]!r}")
    if "剩余：无。" not in cells[2]:
        raise ValueError("completed PR-FJ row must declare 剩余：无。")
    if "scripts/check_pr_fj_completion.sh" not in cells[3]:
        raise ValueError("completed PR-FJ row must record its completion gate")
    return True


def require_queue_clear(root: Path) -> None:
    text = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
    if re.search(r"(?m)^\d+\.\s+\*\*PR-FJ\*\*(?:\s|$)", bounded_section(text, "6.5")):
        raise ValueError("completed PR-FJ remains in the 6.5 local queue")


def require_evidence(root: Path) -> None:
    rows = read_tsv(
        root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv",
        ("pr_id", "evidence_type", "artifact", "command", "notes"),
    )
    selected = [row for row in rows if row["pr_id"].strip() == "PR-FJ"]
    by_type: dict[str, dict[str, str]] = {}
    for row in selected:
        evidence_type = row["evidence_type"].strip()
        if evidence_type in by_type:
            raise ValueError(f"duplicate PR-FJ evidence type: {evidence_type}")
        by_type[evidence_type] = row
    expected = set(REQUIRED_EVIDENCE)
    if set(by_type) != expected:
        raise ValueError(
            f"PR-FJ evidence type drift: missing={sorted(expected - set(by_type))}, "
            f"extra={sorted(set(by_type) - expected)}"
        )
    for evidence_type, (artifact, anchor) in REQUIRED_EVIDENCE.items():
        row = by_type[evidence_type]
        if row["artifact"].strip() != artifact or anchor not in row["command"]:
            raise ValueError(f"PR-FJ {evidence_type} artifact or command anchor drifted")
        if not (root / artifact).is_file():
            raise ValueError(f"missing PR-FJ evidence artifact: {artifact}")


def strip_comments(text: str) -> str:
    output: list[str] = []
    index = 0
    depth = 0
    quote: str | None = None
    escaped = False
    while index < len(text):
        char = text[index]
        pair = text[index:index + 2]
        if depth:
            if pair == "/*":
                depth += 1
                output.extend("  ")
                index += 2
            elif pair == "*/":
                depth -= 1
                output.extend("  ")
                index += 2
            else:
                output.append("\n" if char == "\n" else " ")
                index += 1
        elif quote:
            output.append(char)
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == quote:
                quote = None
            index += 1
        elif pair == "//":
            newline = text.find("\n", index + 2)
            if newline < 0:
                output.extend(" " * (len(text) - index))
                break
            output.extend(" " * (newline - index))
            output.append("\n")
            index = newline + 1
        elif pair == "/*":
            depth = 1
            output.extend("  ")
            index += 2
        else:
            output.append(char)
            if char in {'"', "'", "`"}:
                quote = char
            index += 1
    return "".join(output)


def require_runnable_anchors(root: Path) -> None:
    for relative_path, name in RUST_TEST_ANCHORS:
        path = root / relative_path
        if not path.is_file():
            raise ValueError(f"missing PR-FJ anchor file: {relative_path}")
        text = strip_comments(path.read_text(encoding="utf-8"))
        pattern = re.compile(
            rf"(?m)((?:^\s*#\[[^\n]+\]\s*\n)+)"
            rf"\s*(?:async\s+)?fn\s+{re.escape(name)}\s*\("
        )
        match = pattern.search(text)
        if not match or not re.search(r"#\[(?:tokio::)?test", match.group(1)):
            raise ValueError(f"missing runnable PR-FJ test anchor {relative_path}::{name}")
        if re.search(r"\b(?:ignore|should_panic)\b", match.group(1)):
            raise ValueError(f"PR-FJ test anchor is ignored or should_panic: {relative_path}::{name}")

    browser_anchors = (
        (ROUTE_BROWSER_PATH, ROUTE_BROWSER_TITLE),
        *((PRODUCT_BROWSER_PATH, title) for title in PRODUCT_BROWSER_TITLES),
    )
    checked_browser_files: set[str] = set()
    for relative_path, title in browser_anchors:
        text = strip_comments((root / relative_path).read_text(encoding="utf-8"))
        if relative_path not in checked_browser_files:
            checked_browser_files.add(relative_path)
            if re.search(r"\btest\.(?:skip|fixme)\b|\btest\.describe\.skip\b", text):
                raise ValueError(
                    f"PR-FJ browser anchor file contains forbidden skip/fixme: {relative_path}"
                )
        runnable = re.search(rf"(?m)^\s*test\(\s*['\"]{re.escape(title)}['\"]", text)
        skipped = re.search(
            rf"(?m)^\s*test\.(?:skip|fixme)\(\s*['\"]{re.escape(title)}['\"]", text
        )
        if skipped or not runnable:
            raise ValueError(f"missing runnable PR-FJ browser anchor: {title}")


def require_static_contracts(root: Path) -> None:
    for relative_path, markers in STATIC_MARKERS.items():
        text = (root / relative_path).read_text(encoding="utf-8")
        for marker in markers:
            if marker not in text:
                raise ValueError(f"PR-FJ static contract marker drifted: {relative_path}::{marker}")


def require_exact_coverage(root: Path) -> None:
    rows = read_tsv(
        root / "docs/PRODUCT_AUDIT_COVERAGE.tsv",
        ("file", "coverage_status", "owner_surface", "risk", "suggested_pr", "evidence", "notes"),
    )
    by_path: dict[str, dict[str, str]] = {}
    for row in rows:
        relative_path = row["file"].strip()
        if relative_path in by_path:
            raise ValueError(f"duplicate coverage path: {relative_path}")
        by_path[relative_path] = row
    for relative_path in CLOSURE_PATHS:
        if not (root / relative_path).is_file():
            raise ValueError(f"missing designated PR-FJ closure path: {relative_path}")
        row = by_path.get(relative_path)
        if row is None or row["coverage_status"].strip() != "exact":
            raise ValueError(f"PR-FJ closure path is not exact: {relative_path}")
        if not LINE_EVIDENCE.fullmatch(row["evidence"].strip()):
            raise ValueError(f"PR-FJ closure path lacks line-level exact evidence: {relative_path}")


def validate(root: Path) -> bool:
    if not completion_state(root):
        return False
    require_queue_clear(root)
    require_evidence(root)
    require_runnable_anchors(root)
    require_static_contracts(root)
    require_exact_coverage(root)
    return True


def fixture_tsv(rows: list[list[str]]) -> str:
    output = io.StringIO()
    csv.writer(output, delimiter="\t", lineterminator="\n").writerows(rows)
    return output.getvalue()


def write_fixture(root: Path) -> None:
    docs = root / "docs"
    docs.mkdir(parents=True, exist_ok=True)
    (docs / "PRODUCT_FULL_AUDIT_REFINEMENT.md").write_text(
        "### 🟡 6.3 建议整改顺序\n\n"
        f"| `{PR_TITLE}` | ✅ 完成 | runtime closure；剩余：无。 | "
        "验证：`scripts/check_pr_fj_completion.sh`。 |\n\n"
        "### 🟡 6.4 其它\n\n### 🟡 6.5 下一步执行队列\n\n"
        "1. **PR-ET** — next local lane.\n\n### 🟡 6.6 其它\n",
        encoding="utf-8",
    )
    evidence = [["pr_id", "evidence_type", "artifact", "command", "notes"]]
    for evidence_type, (artifact, anchor) in REQUIRED_EVIDENCE.items():
        evidence.append(["PR-FJ", evidence_type, artifact, f"verify {anchor}", "self-test"])
    (docs / "PRODUCT_AUDIT_EVIDENCE.tsv").write_text(fixture_tsv(evidence), encoding="utf-8")

    coverage = [["file", "coverage_status", "owner_surface", "risk", "suggested_pr", "evidence", "notes"]]
    for index, relative_path in enumerate(CLOSURE_PATHS, 1):
        path = root / relative_path
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("", encoding="utf-8")
        coverage.append([relative_path, "exact", "self_test", "P0", "PR-FJ", f"line:self:{index}", "exact"])
    (docs / "PRODUCT_AUDIT_COVERAGE.tsv").write_text(fixture_tsv(coverage), encoding="utf-8")

    for relative_path, name in RUST_TEST_ANCHORS:
        with (root / relative_path).open("a", encoding="utf-8") as handle:
            handle.write(f"\n#[test]\nfn {name}() {{}}\n")
    (root / ROUTE_BROWSER_PATH).write_text(
        f'test("{ROUTE_BROWSER_TITLE}", async () => {{}});\n', encoding="utf-8"
    )
    (root / PRODUCT_BROWSER_PATH).write_text(
        "".join(f'test("{title}", async () => {{}});\n' for title in PRODUCT_BROWSER_TITLES),
        encoding="utf-8",
    )
    for relative_path, markers in STATIC_MARKERS.items():
        (root / relative_path).write_text("\n".join(markers) + "\n", encoding="utf-8")


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
    with tempfile.TemporaryDirectory(prefix="crossline-pr-fj-gate-") as temp:
        root = Path(temp)
        write_fixture(root)
        assert validate(root)

        doc = root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
        doc.write_text(doc.read_text(encoding="utf-8").replace("✅ 完成", "🟡 部分完成"), encoding="utf-8")
        if validate(root):
            raise AssertionError("partial PR-FJ row must skip completion enforcement")

        write_fixture(root)
        evidence = root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv"
        lines = evidence.read_text(encoding="utf-8").splitlines()
        evidence.write_text("\n".join(lines[:-1]) + "\n", encoding="utf-8")
        expect_failure(root, "evidence type drift")

        for modifier in ("skip", "fixme"):
            write_fixture(root)
            browser = root / ROUTE_BROWSER_PATH
            browser.write_text(
                f'test.{modifier}("{ROUTE_BROWSER_TITLE}", async () => {{}});\n',
                encoding="utf-8",
            )
            expect_failure(root, "browser anchor")

        for attribute in ("#[ignore]", "#[should_panic]"):
            write_fixture(root)
            path = root / RUST_TEST_ANCHORS[0][0]
            name = RUST_TEST_ANCHORS[0][1]
            text = path.read_text(encoding="utf-8")
            path.write_text(
                text.replace(f"#[test]\nfn {name}", f"#[test]\n{attribute}\nfn {name}", 1),
                encoding="utf-8",
            )
            expect_failure(root, "ignored or should_panic")

        write_fixture(root)
        runtime = root / "scripts/verify_api_security_runtime_smoke.sh"
        runtime.write_text(runtime.read_text(encoding="utf-8").replace("credential_run_id", "run_id", 1), encoding="utf-8")
        expect_failure(root, "static contract marker drifted")

        write_fixture(root)
        coverage = root / "docs/PRODUCT_AUDIT_COVERAGE.tsv"
        coverage.write_text(
            coverage.read_text(encoding="utf-8").replace("scripts/check_pr_fj_completion.sh\texact", "check_pr_fj_completion.sh\texact", 1),
            encoding="utf-8",
        )
        expect_failure(root, "closure path is not exact")

        write_fixture(root)
        doc.write_text(doc.read_text(encoding="utf-8").replace("1. **PR-ET**", "1. **PR-FJ**"), encoding="utf-8")
        expect_failure(root, "remains in the 6.5 local queue")


root = Path(sys.argv[1])
mode = sys.argv[2]
try:
    if mode == "--self-test":
        self_test()
        print("OK PR-FJ completion gate self-test")
    elif validate(root):
        print(
            "OK PR-FJ completion gate "
            f"({len(REQUIRED_EVIDENCE)} evidence types; {len(RUST_TEST_ANCHORS)} Rust anchors; "
            f"{len(PRODUCT_BROWSER_TITLES) + 1} browser anchors; "
            f"{len(CLOSURE_PATHS)} exact paths)"
        )
    else:
        print("SKIP PR-FJ completion gate (roadmap row remains partial)")
except (AssertionError, OSError, ValueError) as error:
    raise SystemExit(f"PR-FJ completion gate failed: {error}") from error
PY
