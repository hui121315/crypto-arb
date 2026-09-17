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
import subprocess
import sys
import tempfile


PR_TITLE = "PR-ET Frontend Boundary, DTO Single Source & Hard-Gate Contract"
VERIFY_ANCHOR = "`bash scripts/check_pr_et_completion.sh --self-test`"
REQUIRED_EVIDENCE = {
    "simulation-shared-boundary-gate": (
        "shared-types/src/simulation.rs",
        "contract_types_remain_in_the_simulation_namespace",
    ),
    "opportunity-detail-load-state-gate": (
        "frontend/src/panels/modules/opportunities/data/tests/detail_problem.rs",
        "segment_problem_keeps_partial_detail_stale_instead_of_blank",
    ),
    "execution-preview-selection-gate": (
        "frontend/src/panels/modules/execution/data/preview_tests/state.rs",
        "execution_preview_refresh_keeps_same_query_as_explicit_stale",
    ),
    "funding-single-flight-health-gate": (
        "crates/api/src/data_source_tests.rs",
        "concurrent_cold_reads_start_one_funding_refresh",
    ),
    "funding-product-browser-gate": (
        "test/e2e/pr_et_runtime.spec.ts",
        "PR-ET funding cold start stays warming instead of becoming a healthy empty state",
    ),
    "dto-duplication-regression-gate": (
        "scripts/check_frontend_module_boundaries_self_test.sh",
        "expect_failure bad_dto_mirror",
    ),
    "direct-client-module-boundary-regression-gate": (
        "scripts/check_frontend_module_boundaries_self_test.sh",
        "expect_failure bad_component_client",
    ),
    "crate-root-boundary-regression-gate": (
        "scripts/check_crate_root_boundaries_self_test.sh",
        "expect_failure bad_logic",
    ),
    "completion-governance-gate": (
        "scripts/check_pr_et_completion.sh",
        "bash scripts/check_pr_et_completion.sh --self-test",
    ),
}
TEST_ANCHORS = (
    (
        "shared-types/src/simulation.rs",
        "contract_types_remain_in_the_simulation_namespace",
    ),
    (
        "frontend/src/panels/modules/opportunities/data/tests/detail_problem.rs",
        "segment_problem_keeps_partial_detail_stale_instead_of_blank",
    ),
    (
        "frontend/src/panels/modules/execution/data/preview_tests/state.rs",
        "execution_preview_refresh_keeps_same_query_as_explicit_stale",
    ),
    (
        "crates/api/src/data_source_tests.rs",
        "concurrent_cold_reads_start_one_funding_refresh",
    ),
)
BROWSER_ANCHORS = (
    "PR-ET funding cold start stays warming instead of becoming a healthy empty state",
    "PR-ET funding degradation exposes typed request source and retry context",
)
BOUNDARY_GATES = (
    "scripts/check_frontend_dto_mirror.sh",
    "scripts/check_frontend_module_boundaries.sh",
    "scripts/check_frontend_module_boundaries_self_test.sh",
    "scripts/check_crate_root_boundaries.sh",
    "scripts/check_crate_root_boundaries_self_test.sh",
)
DTO_NAMES = {
    "PortfolioSummary",
    "PositionRow",
    "RiskSnapshot",
    "SystemHealth",
    "VenueQuality",
    "StrategyPerformance",
    "ExecutedTrade",
    "MissedOpportunity",
    "SimulationRuntimeMeta",
    "SimulationPersistenceMode",
    "SimulationPortfolioSummary",
    "SimulationPosition",
    "SimulationOpenRequest",
    "SimulationOpenResponse",
    "SimulationCloseResponse",
    "ExecutionRunEvent",
    "CloseRunEvent",
}
CLOSURE_PATHS = tuple(
    """
scripts/check_pr_et_completion.sh
scripts/check_product_audit_evidence_index.sh
scripts/verify_repo_gates.sh
scripts/check_release_qa_contract.sh
scripts/check_frontend_dto_mirror.sh
scripts/check_frontend_module_boundaries.sh
scripts/check_frontend_module_boundaries_self_test.sh
scripts/check_crate_root_boundaries.sh
scripts/check_crate_root_boundaries_self_test.sh
scripts/fixtures/frontend_module_boundaries/good/frontend/src/panels/modules/futures/view.rs
scripts/fixtures/frontend_module_boundaries/good/frontend/src/panels/modules/opportunities/view.rs
scripts/fixtures/frontend_module_boundaries/bad_component_client/frontend/src/panels/modules/futures/view.rs
scripts/fixtures/frontend_module_boundaries/bad_component_client/frontend/src/panels/modules/opportunities/view.rs
scripts/fixtures/frontend_module_boundaries/bad_dto_mirror/frontend/src/panels/modules/futures/view.rs
scripts/fixtures/frontend_module_boundaries/bad_dto_mirror/frontend/src/panels/modules/opportunities/view.rs
scripts/fixtures/frontend_module_boundaries/bad_module_root/frontend/src/panels/modules/futures/view.rs
scripts/fixtures/frontend_module_boundaries/bad_module_root/frontend/src/panels/modules/opportunities/view.rs
scripts/fixtures/frontend_module_boundaries/bad_selection_owner/frontend/src/panels/modules/futures/view.rs
scripts/fixtures/frontend_module_boundaries/bad_selection_owner/frontend/src/panels/modules/opportunities/view.rs
crates/api/src/data_source.rs
crates/api/src/data_source_tests.rs
crates/api/src/services/market_data/cache/helpers3.rs
crates/api/src/services/market_data/cache/runtime.rs
crates/api/src/services/market_data/cache/tests/cases_4.rs
frontend/src/api/rest.rs
frontend/src/api/rest/dto.rs
shared-types/src/simulation.rs
frontend/src/panels/modules/execution/components/leg_panel.rs
frontend/src/panels/modules/execution/components/risk_preview.rs
frontend/src/panels/modules/execution/data/preview.rs
frontend/src/panels/modules/execution/data/preview_tests.rs
frontend/src/panels/modules/execution/data/preview_tests/state.rs
frontend/src/panels/modules/execution/data/runtime.rs
frontend/src/panels/modules/execution/draft.rs
frontend/src/panels/modules/execution/mod.rs
frontend/src/panels/modules/execution/selection.rs
frontend/src/panels/modules/futures/data/model.rs
frontend/src/panels/modules/futures/data/tests/selection.rs
frontend/src/panels/modules/futures/view.rs
frontend/src/panels/modules/futures/view/sections.rs
frontend/src/panels/modules/futures/view/sections_tests.rs
frontend/src/panels/modules/mod.rs
frontend/src/panels/modules/opportunity_toolbar_state.rs
frontend/src/panels/modules/opportunities/components/detail_panel.rs
frontend/src/panels/modules/opportunities/data/detail.rs
frontend/src/panels/modules/opportunities/data/runtime.rs
frontend/src/panels/modules/opportunities/data/tests/detail.rs
frontend/src/panels/modules/opportunities/data/tests/detail_problem.rs
frontend/src/panels/modules/opportunities/view.rs
frontend/src/panels/modules/opportunities/view/sections.rs
frontend/src/panels/modules/settings/tabs/diagnostics/market.rs
frontend/src/panels/modules/settings/tabs/diagnostics/tests/message.rs
test/e2e/data_pipeline.spec.ts
test/e2e/pr_et_runtime.spec.ts
""".split()
)
LINE_EVIDENCE = re.compile(r"line:[A-Za-z0-9_.-]+:[1-9][0-9]*(?:-[1-9][0-9]*)?")

if len(CLOSURE_PATHS) != 54 or len(set(CLOSURE_PATHS)) != 54:
    raise SystemExit("PR-ET completion gate internal error: expected 54 unique closure paths")


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
    rows = [
        [cell.strip() for cell in line.strip().strip("|").split("|")]
        for line in bounded_section(path.read_text(encoding="utf-8"), "6.3").splitlines()
        if line.startswith("| `PR-ET ")
    ]
    if len(rows) != 1 or len(rows[0]) != 4:
        raise ValueError(f"expected one four-cell PR-ET roadmap row, found {len(rows)}")
    cells = rows[0]
    if cells[0].strip("`") != PR_TITLE:
        raise ValueError("PR-ET roadmap title drifted")
    if cells[1] == "🟡 部分完成":
        return False
    if cells[1] != "✅ 完成":
        raise ValueError(f"unsupported PR-ET status: {cells[1]!r}")
    if "剩余：无。" not in cells[2]:
        raise ValueError("completed PR-ET row must declare no remainder")
    if cells[3].count(VERIFY_ANCHOR) != 1:
        raise ValueError(f"completed PR-ET row must record exact anchor {VERIFY_ANCHOR}")
    return True


def require_queue_clear(root: Path) -> None:
    text = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
    if re.search(r"(?m)^\d+\.\s+\*\*PR-ET\*\*(?:\s|$)", bounded_section(text, "6.5")):
        raise ValueError("completed PR-ET remains in the 6.5 local queue")


def require_evidence(root: Path) -> None:
    rows = read_tsv(
        root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv",
        ("pr_id", "evidence_type", "artifact", "command", "notes"),
    )
    selected = [row for row in rows if row["pr_id"].strip() == "PR-ET"]
    by_type: dict[str, dict[str, str]] = {}
    for row in selected:
        evidence_type = row["evidence_type"].strip()
        if evidence_type in by_type:
            raise ValueError(f"duplicate PR-ET evidence type: {evidence_type}")
        by_type[evidence_type] = row
    expected = set(REQUIRED_EVIDENCE)
    if set(by_type) != expected:
        raise ValueError(
            f"PR-ET evidence type drift: missing={sorted(expected - set(by_type))}, "
            f"extra={sorted(set(by_type) - expected)}"
        )
    for evidence_type, (artifact, anchor) in REQUIRED_EVIDENCE.items():
        row = by_type[evidence_type]
        if row["artifact"].strip() != artifact or anchor not in row["command"]:
            raise ValueError(f"PR-ET {evidence_type} artifact or command anchor drifted")
        if not (root / artifact).is_file():
            raise ValueError(f"missing PR-ET evidence artifact: {artifact}")


def strip_comments(text: str) -> str:
    text = re.sub(r"/\*.*?\*/", lambda match: "\n" * match.group(0).count("\n"), text, flags=re.S)
    return re.sub(r"(?m)//.*$", "", text)


def require_test_anchors(root: Path) -> None:
    for relative_path, name in TEST_ANCHORS:
        path = root / relative_path
        if not path.is_file():
            raise ValueError(f"missing PR-ET anchor file: {relative_path}")
        text = strip_comments(path.read_text(encoding="utf-8"))
        pattern = re.compile(
            rf"(?m)((?:^\s*#\[[^\n]+\]\s*\n)+)"
            rf"\s*(?:async\s+)?fn\s+{re.escape(name)}\s*\("
        )
        match = pattern.search(text)
        if not match or not re.search(r"#\[(?:tokio::)?test", match.group(1)):
            raise ValueError(f"missing runnable PR-ET test anchor {relative_path}::{name}")
        if re.search(r"\b(?:skip|ignore|should_panic)\b", match.group(1)):
            raise ValueError(
                f"PR-ET test anchor is skipped, ignored, or should_panic: {relative_path}::{name}"
            )


def require_browser_anchors(root: Path) -> None:
    path = root / "test/e2e/pr_et_runtime.spec.ts"
    if not path.is_file():
        raise ValueError("missing PR-ET browser artifact")
    text = path.read_text(encoding="utf-8")
    if re.search(r"\btest\.(?:skip|fixme)\s*\(", text):
        raise ValueError("PR-ET browser anchor is skipped or fixed out")
    for title in BROWSER_ANCHORS:
        pattern = re.compile(
            rf'(?m)^\s*test\(\s*["\']{re.escape(title)}["\']\s*,\s*async\b'
        )
        if not pattern.search(text):
            raise ValueError(f"missing runnable PR-ET browser anchor: {title}")


def rust_sources(root: Path, base: str) -> list[Path]:
    path = root / base
    return sorted(path.rglob("*.rs")) if path.is_dir() else []


def require_no_dto_mirrors(root: Path) -> None:
    declaration = re.compile(r"(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?(?:struct|enum)\s+([A-Za-z_][A-Za-z0-9_]*)\b")
    for path in rust_sources(root, "frontend/src"):
        for name in declaration.findall(strip_comments(path.read_text(encoding="utf-8"))):
            if name in DTO_NAMES:
                raise ValueError(f"frontend DTO duplication regression: {path.relative_to(root)}::{name}")


def require_no_direct_client_calls(root: Path) -> None:
    pattern = re.compile(
        r"use_global\(\)\.(?:client|settings_client)|"
        r"\b(?:client|settings_client)\.[A-Za-z_][A-Za-z0-9_]*\s*\(|"
        r"\bspawn_local\s*\("
    )
    modules = root / "frontend/src/panels/modules"
    if not modules.is_dir():
        raise ValueError("missing frontend module tree")
    for path in sorted(modules.rglob("*.rs")):
        if "data" in path.relative_to(modules).parts:
            continue
        if path.name != "view.rs" and "components" not in path.parts and "tabs" not in path.parts:
            continue
        if pattern.search(strip_comments(path.read_text(encoding="utf-8"))):
            raise ValueError(f"frontend direct client regression: {path.relative_to(root)}")


def require_no_root_logic(root: Path) -> None:
    logic = re.compile(
        r"(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?"
        r"(?:const|static|struct|enum|trait|impl|fn|type|macro_rules!)\b"
    )
    module_root = root / "frontend/src/panels/modules"
    roots = sorted(module_root.glob("*/mod.rs")) if module_root.is_dir() else []
    roots.extend(sorted((root / "crates").glob("*/src/lib.rs")))
    shared_root = root / "shared-types/src/lib.rs"
    if shared_root.is_file():
        roots.append(shared_root)
    for path in roots:
        if logic.search(strip_comments(path.read_text(encoding="utf-8"))):
            raise ValueError(f"module/crate root logic regression: {path.relative_to(root)}")


def require_boundary_gates(root: Path) -> None:
    for relative_path in BOUNDARY_GATES:
        path = root / relative_path
        if not path.is_file():
            raise ValueError(f"missing PR-ET boundary gate: {relative_path}")
        result = subprocess.run(
            ["bash", str(path)], cwd=root, text=True, capture_output=True, check=False
        )
        if result.returncode != 0:
            detail = (result.stderr or result.stdout).strip().splitlines()
            suffix = f": {detail[0]}" if detail else ""
            raise ValueError(f"PR-ET boundary gate failed: {relative_path}{suffix}")


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
            raise ValueError(f"missing designated PR-ET closure path: {relative_path}")
        row = by_path.get(relative_path)
        if row is None or row["coverage_status"].strip() != "exact":
            raise ValueError(f"PR-ET closure path is not exact: {relative_path}")
        if not LINE_EVIDENCE.fullmatch(row["evidence"].strip()):
            raise ValueError(f"PR-ET closure path lacks line-level exact evidence: {relative_path}")


def validate(root: Path) -> bool:
    if not completion_state(root):
        return False
    require_queue_clear(root)
    require_evidence(root)
    require_test_anchors(root)
    require_browser_anchors(root)
    require_no_dto_mirrors(root)
    require_no_direct_client_calls(root)
    require_no_root_logic(root)
    require_boundary_gates(root)
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
        "### active 6.3 roadmap\n\n"
        f"| `{PR_TITLE}` | ✅ 完成 | boundary closure；剩余：无。 | 验证：{VERIFY_ANCHOR}。 |\n\n"
        "### active 6.4 other\n\n### active 6.5 queue\n\n"
        "1. **PR-NEXT** - next lane.\n\n### active 6.6 other\n",
        encoding="utf-8",
    )
    evidence = [["pr_id", "evidence_type", "artifact", "command", "notes"]]
    for evidence_type, (artifact, anchor) in REQUIRED_EVIDENCE.items():
        evidence.append(["PR-ET", evidence_type, artifact, f"verify {anchor}", "self-test"])
    (docs / "PRODUCT_AUDIT_EVIDENCE.tsv").write_text(fixture_tsv(evidence), encoding="utf-8")

    coverage = [["file", "coverage_status", "owner_surface", "risk", "suggested_pr", "evidence", "notes"]]
    for index, relative_path in enumerate(CLOSURE_PATHS, 1):
        path = root / relative_path
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("", encoding="utf-8")
        coverage.append([relative_path, "exact", "self_test", "P0", "PR-ET", f"line:self:{index}", "exact"])
    (docs / "PRODUCT_AUDIT_COVERAGE.tsv").write_text(fixture_tsv(coverage), encoding="utf-8")

    for relative_path, name in TEST_ANCHORS:
        with (root / relative_path).open("a", encoding="utf-8") as handle:
            handle.write(f"\n#[test]\nfn {name}() {{}}\n")
    browser = root / "test/e2e/pr_et_runtime.spec.ts"
    with browser.open("a", encoding="utf-8") as handle:
        for title in BROWSER_ANCHORS:
            handle.write(f'\ntest("{title}", async () => {{}});\n')
    for relative_path in BOUNDARY_GATES:
        path = root / relative_path
        path.write_text("#!/usr/bin/env bash\nexit 0\n", encoding="utf-8")
    (root / "frontend/src/panels/modules/example/mod.rs").parent.mkdir(parents=True, exist_ok=True)
    (root / "frontend/src/panels/modules/example/mod.rs").write_text("pub mod view;\n", encoding="utf-8")
    (root / "frontend/src/panels/modules/example/view.rs").write_text("pub fn render() {}\n", encoding="utf-8")
    (root / "crates/example/src/lib.rs").parent.mkdir(parents=True, exist_ok=True)
    (root / "crates/example/src/lib.rs").write_text("pub mod model;\n", encoding="utf-8")


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
    with tempfile.TemporaryDirectory(prefix="crossline-pr-et-gate-") as temp:
        root = Path(temp)
        write_fixture(root)
        assert validate(root)

        doc = root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
        doc.write_text(doc.read_text(encoding="utf-8").replace("✅ 完成", "🟡 部分完成"), encoding="utf-8")
        if validate(root):
            raise AssertionError("partial PR-ET row must skip completion enforcement")

        write_fixture(root)
        evidence = root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv"
        lines = evidence.read_text(encoding="utf-8").splitlines()
        evidence.write_text("\n".join(lines[:-1]) + "\n", encoding="utf-8")
        expect_failure(root, "evidence type drift")

        for attribute in ("#[ignore]", "#[should_panic]", "#[cfg_attr(test, ignore)]"):
            write_fixture(root)
            path = root / TEST_ANCHORS[0][0]
            name = TEST_ANCHORS[0][1]
            text = path.read_text(encoding="utf-8")
            path.write_text(
                text.replace(f"#[test]\nfn {name}", f"#[test]\n{attribute}\nfn {name}", 1),
                encoding="utf-8",
            )
            expect_failure(root, "skipped, ignored, or should_panic")

        write_fixture(root)
        browser = root / "test/e2e/pr_et_runtime.spec.ts"
        browser.write_text(
            browser.read_text(encoding="utf-8").replace("test(\"PR-ET", "test.skip(\"PR-ET", 1),
            encoding="utf-8",
        )
        expect_failure(root, "browser anchor is skipped")

        write_fixture(root)
        dto = root / "frontend/src/panels/modules/example/view.rs"
        dto.write_text("pub struct SimulationPortfolioSummary;\n", encoding="utf-8")
        expect_failure(root, "DTO duplication regression")

        write_fixture(root)
        view = root / "frontend/src/panels/modules/example/view.rs"
        view.write_text("fn render() { client.submit(); }\n", encoding="utf-8")
        expect_failure(root, "direct client regression")

        write_fixture(root)
        module_root = root / "frontend/src/panels/modules/example/mod.rs"
        module_root.write_text("pub mod view;\nfn leaked_logic() {}\n", encoding="utf-8")
        expect_failure(root, "module/crate root logic regression")

        write_fixture(root)
        crate_root = root / "crates/example/src/lib.rs"
        crate_root.write_text("pub mod model;\npub struct Leaked;\n", encoding="utf-8")
        expect_failure(root, "module/crate root logic regression")

        write_fixture(root)
        coverage = root / "docs/PRODUCT_AUDIT_COVERAGE.tsv"
        coverage.write_text(
            coverage.read_text(encoding="utf-8").replace(
                "scripts/check_pr_et_completion.sh\texact",
                "scripts/check_pr_et_completion.sh\tbasename",
                1,
            ),
            encoding="utf-8",
        )
        expect_failure(root, "closure path is not exact")

        write_fixture(root)
        doc.write_text(doc.read_text(encoding="utf-8").replace("1. **PR-NEXT**", "1. **PR-ET**"), encoding="utf-8")
        expect_failure(root, "remains in the 6.5 local queue")


root = Path(sys.argv[1])
mode = sys.argv[2]
try:
    if mode == "--self-test":
        self_test()
        print("OK PR-ET completion gate self-test")
    elif validate(root):
        print(
            "OK PR-ET completion gate "
            f"({len(REQUIRED_EVIDENCE)} evidence types; {len(TEST_ANCHORS)} Rust/frontend anchors; "
            f"{len(BROWSER_ANCHORS)} browser anchors; {len(CLOSURE_PATHS)} exact paths)"
        )
    else:
        print("SKIP PR-ET completion gate (roadmap row remains partial)")
except (AssertionError, OSError, ValueError) as error:
    raise SystemExit(f"PR-ET completion gate failed: {error}") from error
PY
