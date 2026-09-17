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
PR_TITLE = "PR-FA Runtime Smoke, Partial Failure Isolation & Payload Budget Gate"
VERIFY_ANCHOR = "`bash scripts/check_pr_fa_completion.sh --self-test`"
BROWSER_PATH = "test/e2e/pr_fa_runtime.spec.ts"
BROWSER_TITLES = (
    "account evidence isolates section freshness and rejects trusted warming empties",
    "gzip hot-path transfer records browser, Wasm, and bounded render metrics",
)
REQUIRED_EVIDENCE = {
    "account-binding-credential-generation-gate": ("crates/api/src/services/account_binding.rs", "verified_scope_requires_probe_and_current_credential_fingerprint"),
    "partial-failure-isolation-gate": ("crates/api/src/services/account_positions/tests/error_paths.rs", "partial_position_envelope_keeps_rows_and_binds_each_venue"),
    "typed-problem-browser-gate": (BROWSER_PATH, BROWSER_TITLES[0]),
    "runtime-contract": ("scripts/verify_runtime_contracts.sh", "bash scripts/verify_runtime_contracts.sh"),
    "frontend-source-freshness-gate": ("frontend/src/panels/modules/positions/components/account_evidence.rs", "binding_detail_keeps_scope_fingerprint_and_problem_context"),
    "wasm-browser-performance-gate": (BROWSER_PATH, BROWSER_TITLES[1]),
    "release-runtime-shutdown": ("crates/api/src/lifecycle/shutdown.rs", "check_release_qa_contract.sh --release"),
    "completion-governance-gate": ("scripts/check_pr_fa_completion.sh", "bash scripts/check_pr_fa_completion.sh --self-test"),
}
RUST_TEST_ANCHORS = (
    ("crates/api/src/app.rs", "json_payloads_negotiate_gzip_transfer_encoding"),
    ("crates/api/src/services/account_binding.rs", "verified_scope_requires_probe_and_current_credential_fingerprint"),
    ("crates/api/src/services/account_binding.rs", "rotated_credential_rejects_stale_scope_probe"),
    ("crates/api/src/services/account_binding.rs", "stale_scope_without_current_credential_binding_stays_unverified"),
    ("crates/api/src/services/account_binding.rs", "unknown_scope_keeps_request_context_in_typed_problem"),
    ("crates/api/src/services/account_balances/tests.rs", "partial_fanout_keeps_rows_and_surfaces_route_problem"),
    ("crates/api/src/services/account_positions/tests/error_paths.rs", "partial_position_envelope_keeps_rows_and_binds_each_venue"),
    ("crates/api/src/services/account_positions/tests/error_paths.rs", "position_success_timestamp_requires_freshness_evidence"),
    ("crates/api/src/trading_service/cache.rs", "credential_invalidation_advances_epoch_and_clears_account_backoff"),
    ("frontend/src/panels/modules/positions/components/account_evidence.rs", "binding_detail_keeps_scope_fingerprint_and_problem_context"),
    ("frontend/src/panels/modules/positions/components/account_evidence.rs", "observed_age_is_distinct_and_saturating"),
    ("frontend/src/panels/modules/positions/components/runtime_problems_tests.rs", "unknown_operation_health_keeps_typed_problem_drilldown"),
    ("frontend/src/panels/modules/positions/view/tests.rs", "fresh_balance_envelope_is_not_staled_by_position_problem"),
)
RUNTIME_BUDGET_LINES = (
    'P0_OPPORTUNITIES_LIST_MAX_BYTES="${P0_OPPORTUNITIES_LIST_MAX_BYTES:-184320}"',
    'P0_OPPORTUNITIES_LIST_GZIP_MAX_BYTES="${P0_OPPORTUNITIES_LIST_GZIP_MAX_BYTES:-102400}"',
    'P0_OPPORTUNITIES_LIST_GZIP_MAX_RATIO_PERCENT="${P0_OPPORTUNITIES_LIST_GZIP_MAX_RATIO_PERCENT:-90}"',
)
BROWSER_BUDGET_SNIPPETS = (
    "expect(transferBytes.encoded).toBeLessThanOrEqual(64 * 1024);",
    "expect(metrics.transfer.decodedBytes).toBeLessThanOrEqual(180 * 1024);",
    "expect(metrics.transfer.cloneReadMs).toBeLessThanOrEqual(750);",
    "expect(metrics.transfer.jsonParseMs).toBeLessThanOrEqual(100);",
    "expect(wasm[0].decodeMs).toBeLessThanOrEqual(750);",
)
CLOSURE_PATHS = tuple("""
.github/workflows/ci.yml frontend/Cargo.toml shared-types/Cargo.toml
scripts/check_pr_fa_completion.sh scripts/check_product_audit_evidence_index.sh scripts/check_wasm_budget.sh
scripts/check_release_qa_contract.sh scripts/dependency_feature_budget.tsv scripts/verify_repo_gates.sh scripts/verify_runtime_contracts.sh
crates/api/src/app.rs crates/api/src/lifecycle/shutdown.rs crates/api/src/routers/exchanges.rs
crates/api/src/services/account_binding.rs crates/api/src/services/account_balances.rs crates/api/src/services/account_balances/tests.rs
crates/api/src/services/account_positions.rs crates/api/src/services/account_positions/tests.rs crates/api/src/services/account_positions/tests/error_paths.rs
crates/api/src/services/mod.rs crates/api/src/services/trading_credentials.rs crates/api/src/services/venue_credentials.rs crates/api/src/trading_service/cache.rs
crates/api/src/services/venue_operation_health/snapshot/tests/part_01.rs crates/api/src/services/venue_operation_health/tests/safe_probe_endpoint_tests.rs
frontend/src/panels/modules/positions/components/account_evidence.rs frontend/src/panels/modules/positions/components/balance_panel.rs
frontend/src/panels/modules/positions/components/balance_panel/derive.rs frontend/src/panels/modules/positions/components/mod.rs
frontend/src/panels/modules/positions/components/balance_panel/testing.rs frontend/src/panels/modules/positions/components/positions_table.rs
frontend/src/panels/modules/positions/components/positions_table/quality.rs frontend/src/panels/modules/positions/components/positions_table/testing.rs
frontend/src/panels/modules/positions/components/positions_table/quality/account_quality.rs frontend/src/panels/modules/positions/components/positions_table/testing/account_quality.rs
frontend/src/panels/modules/positions/components/runtime_problems.rs frontend/src/panels/modules/positions/components/runtime_problems_tests.rs
frontend/src/panels/modules/positions/components/snapshot_transport.rs
frontend/src/panels/modules/positions/view.rs frontend/src/panels/modules/positions/view/tests.rs shared-types/src/lib.rs shared-types/src/orders.rs shared-types/src/problem.rs
frontend/src/panels/modules/settings/tabs/diagnostics/watchlist_alerts.rs frontend/src/panels/modules/settings/tabs/diagnostics/watchlist_alerts/table.rs
""".split()) + (BROWSER_PATH,)
LINE_EVIDENCE = re.compile(r"line:[A-Za-z0-9_.-]+:[1-9][0-9]*(?:-[1-9][0-9]*)?")
if len(CLOSURE_PATHS) != len(set(CLOSURE_PATHS)):
    raise SystemExit("PR-FA completion gate internal error: duplicate closure path")


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
    following = re.search(r"(?m)^### ", text[match.end() :])
    end = match.end() + following.start() if following else len(text)
    return text[match.start() : end]

def completion_state(root: Path) -> bool:
    path = root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
    if not path.is_file():
        raise ValueError("missing docs/PRODUCT_FULL_AUDIT_REFINEMENT.md")
    section = bounded_section(path.read_text(encoding="utf-8"), "6.3")
    rows = [
        [cell.strip() for cell in line.strip().strip("|").split("|")]
        for line in section.splitlines()
        if line.startswith("| `PR-FA ")
    ]
    if len(rows) != 1 or len(rows[0]) != 4:
        raise ValueError(f"expected one four-cell PR-FA roadmap row, found {len(rows)}")
    cells = rows[0]
    if cells[0].strip("`") != PR_TITLE:
        raise ValueError("PR-FA roadmap title drifted")
    if cells[1] == "🟡 部分完成":
        return False
    if cells[1] != "✅ 完成":
        raise ValueError(f"unsupported PR-FA status: {cells[1]!r}")
    if not cells[2].endswith("剩余：无。"):
        raise ValueError("completed PR-FA row must end its scope with 剩余：无。")
    if cells[3].count(VERIFY_ANCHOR) != 1:
        raise ValueError(f"completed PR-FA row must record exact verification anchor {VERIFY_ANCHOR}")
    return True

def require_queue_clear(root: Path) -> None:
    text = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
    queue = bounded_section(text, "6.5")
    if re.search(r"(?m)^\d+\.\s+\*\*PR-FA\*\*(?:\s|$)", queue):
        raise ValueError("completed PR-FA remains in the 6.5 local queue")

def require_evidence(root: Path) -> None:
    rows = read_tsv(
        root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv",
        ("pr_id", "evidence_type", "artifact", "command", "notes"),
    )
    selected = [row for row in rows if row["pr_id"].strip() == "PR-FA"]
    by_type: dict[str, dict[str, str]] = {}
    for row in selected:
        evidence_type = row["evidence_type"].strip()
        if evidence_type in by_type:
            raise ValueError(f"duplicate PR-FA evidence type: {evidence_type}")
        by_type[evidence_type] = row
    expected = set(REQUIRED_EVIDENCE)
    if set(by_type) != expected:
        raise ValueError(
            f"PR-FA evidence type drift: missing={sorted(expected - set(by_type))}, "
            f"extra={sorted(set(by_type) - expected)}"
        )
    for evidence_type, (artifact, anchor) in REQUIRED_EVIDENCE.items():
        row = by_type[evidence_type]
        if row["artifact"].strip() != artifact or anchor not in row["command"]:
            raise ValueError(f"PR-FA {evidence_type} artifact or command anchor drifted")
        if not (root / artifact).is_file():
            raise ValueError(f"missing PR-FA evidence artifact: {artifact}")

def strip_comments(text: str) -> str:
    output: list[str] = []
    index = 0
    depth = 0
    quote: str | None = None
    escaped = False
    while index < len(text):
        char = text[index]
        pair = text[index : index + 2]
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
            raise ValueError(f"missing PR-FA anchor file: {relative_path}")
        text = strip_comments(path.read_text(encoding="utf-8"))
        pattern = re.compile(
            rf"(?m)((?:^\s*#\[[^\n]+\]\s*\n)+)"
            rf"\s*(?:async\s+)?fn\s+{re.escape(name)}\s*\("
        )
        match = pattern.search(text)
        if not match or not re.search(r"#\[(?:tokio::)?test", match.group(1)):
            raise ValueError(f"missing runnable PR-FA test anchor {relative_path}::{name}")
        if re.search(r"\b(?:ignore|should_panic)\b", match.group(1)):
            raise ValueError(f"PR-FA test anchor is ignored or should_panic: {relative_path}::{name}")

    browser = strip_comments((root / BROWSER_PATH).read_text(encoding="utf-8"))
    for title in BROWSER_TITLES:
        skipped = re.search(rf"(?m)^\s*test\.(?:skip|fixme)\(\s*['\"]{re.escape(title)}['\"]", browser)
        runnable = re.search(rf"(?m)^\s*test\(\s*['\"]{re.escape(title)}['\"]", browser)
        if skipped or not runnable:
            raise ValueError(f"missing runnable PR-FA browser anchor: {title}")

def require_budget_anchors(root: Path) -> None:
    runtime = (root / "scripts/verify_runtime_contracts.sh").read_text(encoding="utf-8")
    for line in RUNTIME_BUDGET_LINES:
        if not re.search(rf"(?m)^{re.escape(line)}$", runtime):
            raise ValueError(f"PR-FA runtime budget anchor drifted: {line}")
    if not re.search(r"(?m)^probe_gzip_budget \\$", runtime) or not re.search(
        r"(?m)^\s+p0_opportunities_gzip \\$", runtime
    ):
        raise ValueError("PR-FA runtime gzip probe anchor drifted")
    browser = strip_comments((root / BROWSER_PATH).read_text(encoding="utf-8"))
    for snippet in BROWSER_BUDGET_SNIPPETS:
        if snippet not in browser:
            raise ValueError(f"PR-FA browser budget anchor drifted: {snippet}")

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
            raise ValueError(f"missing designated PR-FA closure path: {relative_path}")
        row = by_path.get(relative_path)
        if row is None:
            raise ValueError(f"coverage ledger lacks exact PR-FA closure path: {relative_path}")
        if row["coverage_status"].strip() != "exact":
            raise ValueError(f"PR-FA closure path is not exact: {relative_path}")
        if not LINE_EVIDENCE.fullmatch(row["evidence"].strip()):
            raise ValueError(f"PR-FA closure path lacks line-level exact evidence: {relative_path}")


def validate(root: Path) -> bool:
    if not completion_state(root):
        return False
    require_queue_clear(root)
    require_evidence(root)
    require_runnable_anchors(root)
    require_budget_anchors(root)
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
        f"| `{PR_TITLE}` | ✅ 完成 | runtime closure；剩余：无。 | 验证：{VERIFY_ANCHOR}。 |\n\n"
        "### 🟡 6.4 其它\n\n### 🟡 6.5 下一步执行队列\n\n"
        "1. **PR-FJ** — next local lane.\n\n### 🟡 6.6 其它\n",
        encoding="utf-8",
    )
    evidence = [["pr_id", "evidence_type", "artifact", "command", "notes"]]
    for evidence_type, (artifact, anchor) in REQUIRED_EVIDENCE.items():
        evidence.append(["PR-FA", evidence_type, artifact, f"verify {anchor}", "self-test"])
    (docs / "PRODUCT_AUDIT_EVIDENCE.tsv").write_text(fixture_tsv(evidence), encoding="utf-8")

    coverage = [["file", "coverage_status", "owner_surface", "risk", "suggested_pr", "evidence", "notes"]]
    for index, relative_path in enumerate(CLOSURE_PATHS, 1):
        path = root / relative_path
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("", encoding="utf-8")
        coverage.append([relative_path, "exact", "self_test", "P0", "PR-FA", f"line:self:{index}", "exact"])
    (docs / "PRODUCT_AUDIT_COVERAGE.tsv").write_text(fixture_tsv(coverage), encoding="utf-8")

    for relative_path, name in RUST_TEST_ANCHORS:
        with (root / relative_path).open("a", encoding="utf-8") as handle:
            handle.write(f"\n#[test]\nfn {name}() {{}}\n")
    (root / "scripts/verify_runtime_contracts.sh").write_text(
        "\n".join(RUNTIME_BUDGET_LINES)
        + "\nprobe_gzip_budget \\\n  p0_opportunities_gzip \\\n  '/api/v3/arbitrage/opportunities/list'\n",
        encoding="utf-8",
    )
    browser_body = "\n  ".join(BROWSER_BUDGET_SNIPPETS)
    (root / BROWSER_PATH).write_text(
        f'test("{BROWSER_TITLES[0]}", async () => {{}});\n'
        f'test("{BROWSER_TITLES[1]}", async () => {{\n  {browser_body}\n}});\n',
        encoding="utf-8",
    )


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
    with tempfile.TemporaryDirectory(prefix="crossline-pr-fa-gate-") as temp:
        root = Path(temp)
        write_fixture(root)
        assert validate(root)

        doc = root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
        doc.write_text(doc.read_text(encoding="utf-8").replace("✅ 完成", "🟡 部分完成"), encoding="utf-8")
        if validate(root):
            raise AssertionError("partial PR-FA row must skip completion enforcement")

        write_fixture(root)
        evidence = root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv"
        lines = evidence.read_text(encoding="utf-8").splitlines()
        evidence.write_text("\n".join(lines[:-1]) + "\n", encoding="utf-8")
        expect_failure(root, "evidence type drift")

        write_fixture(root)
        lines = evidence.read_text(encoding="utf-8").splitlines()
        evidence.write_text("\n".join(lines + [lines[1]]) + "\n", encoding="utf-8")
        expect_failure(root, "duplicate PR-FA evidence type")

        for modifier in ("skip", "fixme"):
            write_fixture(root)
            browser = root / BROWSER_PATH
            text = browser.read_text(encoding="utf-8")
            browser.write_text(text.replace('test("account', f'test.{modifier}("account', 1), encoding="utf-8")
            expect_failure(root, "browser anchor")

        for attribute in ("#[ignore]", "#[should_panic]"):
            write_fixture(root)
            path = root / RUST_TEST_ANCHORS[0][0]
            name = RUST_TEST_ANCHORS[0][1]
            text = path.read_text(encoding="utf-8")
            path.write_text(text.replace(f"#[test]\nfn {name}", f"#[test]\n{attribute}\nfn {name}", 1), encoding="utf-8")
            expect_failure(root, "ignored or should_panic")

        write_fixture(root)
        runtime = root / "scripts/verify_runtime_contracts.sh"
        runtime.write_text(runtime.read_text(encoding="utf-8").replace(":-102400}", ":-102401}", 1), encoding="utf-8")
        expect_failure(root, "runtime budget anchor drifted")

        write_fixture(root)
        coverage = root / "docs/PRODUCT_AUDIT_COVERAGE.tsv"
        text = coverage.read_text(encoding="utf-8")
        coverage.write_text(text.replace("scripts/check_pr_fa_completion.sh\texact", "check_pr_fa_completion.sh\texact", 1), encoding="utf-8")
        expect_failure(root, "lacks exact PR-FA closure path")

        write_fixture(root)
        text = coverage.read_text(encoding="utf-8")
        coverage.write_text(text.replace("\tline:self:1\t", "\tbasename:self\t", 1), encoding="utf-8")
        expect_failure(root, "lacks line-level exact evidence")

        write_fixture(root)
        doc.write_text(doc.read_text(encoding="utf-8").replace("1. **PR-FJ**", "1. **PR-FA**"), encoding="utf-8")
        expect_failure(root, "remains in the 6.5 local queue")


root = Path(sys.argv[1])
mode = sys.argv[2]
try:
    if mode == "--self-test":
        self_test()
        print("OK PR-FA completion gate self-test")
    elif validate(root):
        print(
            "OK PR-FA completion gate "
            f"({len(REQUIRED_EVIDENCE)} evidence types; {len(RUST_TEST_ANCHORS)} Rust anchors; "
            f"{len(BROWSER_TITLES)} browser anchors; {len(CLOSURE_PATHS)} exact paths)"
        )
    else:
        print("SKIP PR-FA completion gate (roadmap row remains partial)")
except (AssertionError, OSError, ValueError) as error:
    raise SystemExit(f"PR-FA completion gate failed: {error}") from error
PY
