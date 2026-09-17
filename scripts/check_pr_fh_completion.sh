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
PR_FH_TITLE = "PR-FH Route Inventory, Legacy Feature Gate & Product Surface Contract"
REQUIRED_EVIDENCE = {
    "route-registry": (
        "crates/api/src/route_specs.rs",
        "route_registry_seeded_endpoint_metadata_matches_inventory",
    ),
    "route-inventory-frontend-contract": (
        "scripts/check_route_inventory.sh",
        "check_route_inventory.sh",
    ),
    "route-endpoint-metadata-contract": (
        "scripts/check_route_endpoint_metadata.sh",
        "check_route_endpoint_metadata.sh",
    ),
    "route-runtime-policy-contract": (
        "scripts/check_route_runtime_policy.sh",
        "check_route_runtime_policy.sh",
    ),
    "route-default-off-security-runtime": (
        "scripts/verify_api_security_runtime_smoke.sh",
        "verify_api_security_runtime_smoke.sh",
    ),
    "route-registry-browser": (
        "test/e2e/route_registry.spec.ts",
        "test:e2e:route-registry",
    ),
    "route-audit-browser": (
        "test/e2e/route_registry.spec.ts",
        "test:e2e:route-registry",
    ),
}
RUST_TEST_ANCHORS = (
    (
        "crates/api/src/app.rs",
        "route_registry_seeded_endpoint_metadata_matches_inventory",
    ),
    ("crates/api/src/app.rs", "route_registry_seeds_core_endpoints"),
    (
        "crates/api/src/app.rs",
        "route_registry_seeds_all_available_inventory_endpoints",
    ),
)
BROWSER_ANCHORS = (
    (
        "test/e2e/route_registry.spec.ts",
        "gated diagnostic routes stay absent even with local bearer credentials",
    ),
    (
        "test/e2e/route_registry.spec.ts",
        "high-risk action and secret mutation persist correlated redacted audit pairs",
    ),
)
SOURCE_MARKERS = {
    "docs/API_ROUTE_INVENTORY.tsv": "api_surface.strategy_v1",
    "crates/api/src/route_specs.rs": "RouteEndpointSpec",
    "crates/api/src/app.rs": "route_registry_seeds_all_available_inventory_endpoints",
    "crates/api/src/routers/mod.rs": "legacy-chat",
    "scripts/check_route_runtime_policy.sh": "default_off",
    "scripts/verify_api_security_runtime_smoke.sh": "assert_default_off_surface_routes",
    "test/e2e/fixtures/route_runtime_policy.mjs": "recordAuditPair",
}
CLOSURE_PATHS = (
    "scripts/check_pr_fh_completion.sh",
    "scripts/verify_repo_gates.sh",
    "scripts/check_route_inventory.sh",
    "scripts/check_route_endpoint_metadata.sh",
    "scripts/check_route_runtime_policy.sh",
    "scripts/verify_api_security_runtime_smoke.sh",
    "crates/api/src/app.rs",
    "crates/api/src/route_specs.rs",
    "crates/api/src/routers/mod.rs",
    "frontend/src/api/rest.rs",
    "test/e2e/fixtures/route_runtime_policy.mjs",
    "test/e2e/route_registry.spec.ts",
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
        if line.startswith("| " + BT + "PR-FH ")
    ]
    if len(rows) != 1 or len(rows[0]) != 4:
        raise ValueError("expected exactly one four-cell PR-FH roadmap row")
    return text, rows[0]


def completion_state(root: Path) -> bool:
    text, cells = roadmap_text(root)
    if cells[0].strip(BT) != PR_FH_TITLE:
        raise ValueError("PR-FH roadmap title drifted")
    if cells[1] == "🟡 部分完成":
        return False
    if cells[1] != "✅ 完成":
        raise ValueError(f"PR-FH status drifted: {cells[1]!r}")
    if "剩余：无。" not in cells[2] or "scripts/check_pr_fh_completion.sh" not in cells[2]:
        raise ValueError("PR-FH complete row must declare remaining-none and its gate")
    queue_start = text.find("### 🟡 6.5")
    queue_end = text.find("\n## ", queue_start + 1)
    queue = text[queue_start : queue_end if queue_end >= 0 else len(text)]
    if "PR-FH" in queue.split("\n2.", 1)[0]:
        raise ValueError("completed PR-FH must not remain at the active queue head")
    return True


def require_evidence(root: Path) -> None:
    rows = read_tsv(
        root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv",
        ("pr_id", "evidence_type", "artifact", "command", "notes"),
    )
    seen = {row["evidence_type"].strip(): row for row in rows if row["pr_id"].strip() == "PR-FH"}
    if len(seen) != sum(row["pr_id"].strip() == "PR-FH" for row in rows):
        raise ValueError("duplicate PR-FH evidence type")
    expected = set(REQUIRED_EVIDENCE)
    if set(seen) != expected:
        raise ValueError(
            f"PR-FH evidence type drift: missing={sorted(expected - set(seen))}, "
            f"extra={sorted(set(seen) - expected)}"
        )
    for name, (artifact, command_anchor) in REQUIRED_EVIDENCE.items():
        row = seen[name]
        if row["artifact"].strip() != artifact or command_anchor not in row["command"]:
            raise ValueError(f"PR-FH evidence anchor drift: {name}")


def strip_rust_comments(text: str) -> str:
    text = re.sub(r"/\*.*?\*/", "", text, flags=re.S)
    return re.sub(r"//[^\n]*", "", text)


def require_rust_anchors(root: Path) -> None:
    for relative_path, function_name in RUST_TEST_ANCHORS:
        path = root / relative_path
        if not path.is_file():
            raise ValueError(f"missing PR-FH anchor file: {relative_path}")
        text = strip_rust_comments(path.read_text(encoding="utf-8"))
        pattern = re.compile(
            r"(?m)^\s*#\[(?:tokio::)?test(?:\([^\]]*\))?\]\s*\n"
            r"(?P<attrs>(?:\s*#\[[^\n]+\]\s*\n)*)"
            rf"\s*(?:async\s+)?fn\s+{re.escape(function_name)}\s*\("
        )
        match = pattern.search(text)
        if match is None:
            raise ValueError(f"missing runnable PR-FH anchor: {relative_path}::{function_name}")
        if "ignore" in match.group("attrs"):
            raise ValueError(f"PR-FH anchor is ignored: {relative_path}::{function_name}")


def require_browser_anchors(root: Path) -> None:
    for relative_path, title in BROWSER_ANCHORS:
        text = (root / relative_path).read_text(encoding="utf-8")
        escaped = re.escape(title)
        if re.search(rf'(?m)^\s*test\.skip\(\s*["\']{escaped}["\']', text):
            raise ValueError(f"PR-FH browser anchor must not be skipped: {title}")
        if not re.search(rf'(?m)^\s*test\(\s*["\']{escaped}["\']', text):
            raise ValueError(f"missing non-skipping PR-FH browser anchor: {title}")


def require_source_markers(root: Path) -> None:
    for relative_path, marker in SOURCE_MARKERS.items():
        path = root / relative_path
        if not path.is_file() or marker not in path.read_text(encoding="utf-8"):
            raise ValueError(f"missing PR-FH source marker: {relative_path}:{marker}")


def require_exact_coverage(root: Path) -> None:
    rows = read_tsv(
        root / "docs/PRODUCT_AUDIT_COVERAGE.tsv",
        ("file", "coverage_status", "owner_surface", "risk", "suggested_pr", "evidence", "notes"),
    )
    by_path = {row["file"].strip(): row for row in rows}
    for relative_path in CLOSURE_PATHS:
        if not (root / relative_path).is_file():
            raise ValueError(f"missing PR-FH closure path: {relative_path}")
        row = by_path.get(relative_path)
        if row is None or row["coverage_status"].strip() != "exact":
            raise ValueError(f"PR-FH closure path is not exact: {relative_path}")
        if not row["evidence"].strip().startswith("line:"):
            raise ValueError(f"PR-FH closure path lacks line evidence: {relative_path}")


def validate(root: Path) -> bool:
    if not completion_state(root):
        return False
    require_evidence(root)
    require_rust_anchors(root)
    require_browser_anchors(root)
    require_source_markers(root)
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
        f"| {BT}{PR_FH_TITLE}{BT} | ✅ 完成 | 剩余：无。验证：{BT}scripts/check_pr_fh_completion.sh{BT}。 | static |\n\n"
        "### 🟡 6.4 其它\n\n"
        "### 🟡 6.5 下一步执行队列\n"
        "1. PR-FK next local task\n"
        "2. **外部等待池** — remote samples.\n",
        encoding="utf-8",
    )
    evidence = [["pr_id", "evidence_type", "artifact", "command", "notes"]]
    for name, (artifact, anchor) in REQUIRED_EVIDENCE.items():
        evidence.append(["PR-FH", name, artifact, f"cargo test {anchor}", "self-test"])
    (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").write_text(tsv(evidence), encoding="utf-8")
    coverage = [[
        "file", "coverage_status", "owner_surface", "risk", "suggested_pr", "evidence", "notes"
    ]]
    for index, relative_path in enumerate(CLOSURE_PATHS, start=1):
        path = root / relative_path
        path.parent.mkdir(parents=True, exist_ok=True)
        path.touch()
        coverage.append([relative_path, "exact", "self_test", "P0", "PR-FH", f"line:main:{index}", "exact"])
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
    browser_path = root / BROWSER_ANCHORS[0][0]
    browser_path.parent.mkdir(parents=True, exist_ok=True)
    browser_path.write_text(
        "".join(f'test("{title}", () => {{}});\n' for _, title in BROWSER_ANCHORS),
        encoding="utf-8",
    )
    for relative_path, marker in SOURCE_MARKERS.items():
        path = root / relative_path
        path.parent.mkdir(parents=True, exist_ok=True)
        existing = path.read_text(encoding="utf-8") if path.exists() else ""
        if marker not in existing:
            path.write_text(existing + f"\nconst marker = \"{marker}\";\n", encoding="utf-8")


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
    with tempfile.TemporaryDirectory(prefix="crossline-pr-fh-gate-") as temp:
        root = Path(temp)
        write_fixture(root)
        validate(root)
        doc = root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
        doc.write_text(doc.read_text(encoding="utf-8").replace("✅ 完成", "🟡 部分完成"), encoding="utf-8")
        if validate(root):
            raise AssertionError("partial PR-FH row must skip enforcement")
        write_fixture(root)
        evidence = root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv"
        evidence.write_text("\n".join(evidence.read_text(encoding="utf-8").splitlines()[:-1]) + "\n", encoding="utf-8")
        expect_failure(root, "evidence type drift")
        write_fixture(root)
        anchor_path = root / RUST_TEST_ANCHORS[0][0]
        anchor_path.write_text(f"/* #[test]\nfn {RUST_TEST_ANCHORS[0][1]}() {{}} */\n", encoding="utf-8")
        expect_failure(root, "missing runnable PR-FH anchor")
        write_fixture(root)
        anchor_path.write_text(f"#[test]\n#[ignore]\nfn {RUST_TEST_ANCHORS[0][1]}() {{}}\n", encoding="utf-8")
        expect_failure(root, "anchor is ignored")
        write_fixture(root)
        browser_path, title = BROWSER_ANCHORS[0]
        (root / browser_path).write_text(f'test.skip("{title}", () => {{}});\n', encoding="utf-8")
        expect_failure(root, "browser anchor must not be skipped")
        write_fixture(root)
        marker_path = root / "docs/API_ROUTE_INVENTORY.tsv"
        marker_path.write_text("", encoding="utf-8")
        expect_failure(root, "source marker")
        write_fixture(root)
        coverage = root / "docs/PRODUCT_AUDIT_COVERAGE.tsv"
        coverage.write_text(coverage.read_text(encoding="utf-8").replace("\texact\t", "\tbasename\t", 1), encoding="utf-8")
        expect_failure(root, "closure path is not exact")


root = Path(sys.argv[1])
mode = sys.argv[2]
try:
    if mode == "--self-test":
        self_test()
        print("OK PR-FH completion gate self-test")
    elif validate(root):
        print(
            "OK PR-FH completion gate "
            f"({len(REQUIRED_EVIDENCE)} evidence types; {len(RUST_TEST_ANCHORS)} Rust anchors; "
            f"{len(BROWSER_ANCHORS)} browser anchors; {len(CLOSURE_PATHS)} exact paths)"
        )
    else:
        print("SKIP PR-FH completion gate (roadmap row remains partial)")
except (AssertionError, ValueError) as error:
    raise SystemExit(f"PR-FH completion gate failed: {error}") from error
PY
