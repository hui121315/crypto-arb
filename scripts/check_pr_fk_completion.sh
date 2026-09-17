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


PR_TITLE = "PR-FK LLM External Payload, Redaction, Feature Gate & Provider Evidence Contract"
EVIDENCE = {
    "external-payload-contract": (
        "shared-types/src/llm.rs",
        "cargo test -p shared-types --lib llm",
    ),
    "provider-evidence-registry": (
        "crates/llm/src/evidence.rs",
        "cargo test -p llm --all-targets",
    ),
    "deepseek-official-path-fixture": (
        "crates/llm/tests/providers_test.rs",
        "deepseek_uses_official_chat_completions_path_with_own_provider_label",
    ),
    "outbound-audit-readiness": (
        "crates/api/src/routers/chat.rs",
        "cargo test -p api --features legacy-chat routers::chat",
    ),
    "frontend-wrapper-cleanup": (
        "scripts/check_frontend_dto_mirror.sh",
        "bash scripts/check_frontend_dto_mirror.sh",
    ),
    "completion-gate": (
        "scripts/check_pr_fk_completion.sh",
        "bash scripts/check_pr_fk_completion.sh",
    ),
}
TEST_ANCHORS = (
    ("shared-types/src/llm.rs", "sensitive_markers_fail_closed_before_external_serialization"),
    ("shared-types/src/llm.rs", "unknown_raw_fields_cannot_deserialize_into_allowlisted_contract"),
    ("shared-types/src/llm.rs", "multibyte_summary_respects_byte_cap"),
    ("crates/llm/src/evidence.rs", "deepseek_uses_its_official_non_v1_endpoint"),
    (
        "crates/llm/tests/providers_test.rs",
        "deepseek_uses_official_chat_completions_path_with_own_provider_label",
    ),
    ("crates/api/src/routers/chat.rs", "outbound_requires_auth_and_a_live_audit_writer"),
    ("crates/api/src/routers/chat.rs", "legacy_raw_chat_message_body_is_not_a_supported_contract"),
    ("crates/api/src/routers/chat.rs", "provider_rate_limit_keeps_provider_evidence_in_typed_problem"),
)
SOURCE_MARKERS = {
    "shared-types/src/llm.rs": ("pub struct LlmExternalPayload",),
    "crates/llm/src/evidence.rs": ('endpoint_path: "/chat/completions"',),
    "crates/llm/src/providers/deepseek.rs": ("DEEPSEEK_EVIDENCE",),
    "crates/api/src/routers/chat.rs": (
        "ensure_external_readiness",
        "record_llm_durable_audit",
    ),
}
CLOSURE_PATHS = (
    "scripts/check_pr_fk_completion.sh",
    "scripts/verify_repo_gates.sh",
    "scripts/check_frontend_dto_mirror.sh",
    "shared-types/src/lib.rs",
    "shared-types/src/llm.rs",
    "shared-types/src/problem.rs",
    "crates/llm/src/evidence.rs",
    "crates/llm/src/error.rs",
    "crates/llm/src/lib.rs",
    "crates/llm/src/provider.rs",
    "crates/llm/src/router.rs",
    "crates/llm/src/providers/openai.rs",
    "crates/llm/src/providers/claude.rs",
    "crates/llm/src/providers/gemini.rs",
    "crates/llm/src/providers/deepseek.rs",
    "crates/llm/tests/providers_test.rs",
    "crates/api/src/routers/chat.rs",
    "frontend/src/api/rest.rs",
    "frontend/src/api/rest/dto.rs",
)


def read_tsv(path: Path, header: tuple[str, ...]) -> list[dict[str, str]]:
    with path.open(encoding="utf-8", newline="") as handle:
        reader = csv.DictReader(handle, delimiter="\t")
        if tuple(reader.fieldnames or ()) != header:
            raise ValueError(f"{path.name} header drifted")
        return list(reader)


def roadmap_row(root: Path) -> tuple[str, list[str]]:
    text = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
    start = text.find("### 🟡 6.3")
    end = text.find("### 🟡 6.4", start)
    if start < 0 or end < 0:
        raise ValueError("missing bounded 6.3 roadmap section")
    rows = [
        [cell.strip() for cell in line.strip().strip("|").split("|")]
        for line in text[start:end].splitlines()
        if line.startswith("| `PR-FK ")
    ]
    if len(rows) != 1 or len(rows[0]) != 4:
        raise ValueError("expected exactly one four-cell PR-FK roadmap row")
    return text, rows[0]


def completion_state(root: Path) -> bool:
    text, row = roadmap_row(root)
    if row[0].strip("`") != PR_TITLE:
        raise ValueError("PR-FK roadmap title drifted")
    if row[1] == "🟡 部分完成":
        return False
    if row[1] != "✅ 完成":
        raise ValueError(f"PR-FK status drifted: {row[1]!r}")
    if "剩余：无。" not in row[2] or "scripts/check_pr_fk_completion.sh" not in row[2]:
        raise ValueError("PR-FK complete row must declare remaining-none and its gate")
    queue_start = text.find("### 🟡 6.5")
    queue_end = text.find("\n## ", queue_start + 1)
    queue = text[queue_start : queue_end if queue_end >= 0 else len(text)]
    head = queue.split("\n2.", 1)[0]
    if "PR-FK" in head:
        raise ValueError("completed PR-FK must not remain at the active queue head")
    return True


def require_evidence(root: Path) -> None:
    rows = read_tsv(
        root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv",
        ("pr_id", "evidence_type", "artifact", "command", "notes"),
    )
    seen = {row["evidence_type"].strip(): row for row in rows if row["pr_id"].strip() == "PR-FK"}
    expected = set(EVIDENCE)
    if set(seen) != expected:
        raise ValueError(
            f"evidence type drift: missing={sorted(expected - set(seen))}, "
            f"extra={sorted(set(seen) - expected)}"
        )
    for name, (artifact, command_anchor) in EVIDENCE.items():
        row = seen[name]
        if row["artifact"].strip() != artifact or command_anchor not in row["command"]:
            raise ValueError(f"evidence anchor drift: {name}")


def strip_comments(text: str) -> str:
    text = re.sub(r"/\*.*?\*/", "", text, flags=re.S)
    return re.sub(r"//[^\n]*", "", text)


def require_test_anchors(root: Path) -> None:
    for relative_path, function_name in TEST_ANCHORS:
        path = root / relative_path
        if not path.is_file():
            raise ValueError(f"missing test anchor file: {relative_path}")
        text = strip_comments(path.read_text(encoding="utf-8"))
        pattern = re.compile(
            r"(?m)^\s*#\[(?:tokio::)?test(?:\([^\]]*\))?\]\s*\n"
            r"(?P<attrs>(?:\s*#\[[^\n]+\]\s*\n)*)"
            rf"\s*(?:async\s+)?fn\s+{re.escape(function_name)}\s*\("
        )
        match = pattern.search(text)
        if match is None:
            raise ValueError(f"missing runnable anchor: {relative_path}::{function_name}")
        if "ignore" in match.group("attrs"):
            raise ValueError(f"ignored anchor: {relative_path}::{function_name}")


def require_source_contract(root: Path) -> None:
    for relative_path, markers in SOURCE_MARKERS.items():
        path = root / relative_path
        text = path.read_text(encoding="utf-8") if path.is_file() else ""
        for marker in markers:
            if marker not in text:
                raise ValueError(f"missing source marker: {relative_path}:{marker}")
    rest = (root / "frontend/src/api/rest.rs").read_text(encoding="utf-8")
    dto = (root / "frontend/src/api/rest/dto.rs").read_text(encoding="utf-8")
    if (root / "frontend/src/api/rest/chat.rs").exists() or "mod chat;" in rest:
        raise ValueError("frontend legacy chat wrapper remains")
    if re.search(r"pub\s+(struct|enum)\s+", dto):
        raise ValueError("frontend REST DTO mirror remains")


def require_exact_coverage(root: Path) -> None:
    rows = read_tsv(
        root / "docs/PRODUCT_AUDIT_COVERAGE.tsv",
        ("file", "coverage_status", "owner_surface", "risk", "suggested_pr", "evidence", "notes"),
    )
    by_path = {row["file"].strip(): row for row in rows}
    for relative_path in CLOSURE_PATHS:
        path = root / relative_path
        row = by_path.get(relative_path)
        if not path.is_file() or row is None or row["coverage_status"].strip() != "exact":
            raise ValueError(f"closure path is not exact: {relative_path}")
        if not row["evidence"].strip().startswith("line:"):
            raise ValueError(f"closure path lacks line evidence: {relative_path}")


def validate(root: Path) -> bool:
    if not completion_state(root):
        return False
    require_evidence(root)
    require_test_anchors(root)
    require_source_contract(root)
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
        f"| `{PR_TITLE}` | ✅ 完成 | 剩余：无。验证：`scripts/check_pr_fk_completion.sh`。 | static |\n\n"
        "### 🟡 6.4 其它\n\n"
        "### 🟡 6.5 下一步执行队列\n"
        "1. PR-FL next local task\n"
        "2. **外部等待池** — live proof.\n",
        encoding="utf-8",
    )
    evidence_rows = [["pr_id", "evidence_type", "artifact", "command", "notes"]]
    for name, (artifact, command_anchor) in EVIDENCE.items():
        evidence_rows.append(["PR-FK", name, artifact, command_anchor, "fixture"])
    (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").write_text(tsv(evidence_rows), encoding="utf-8")
    coverage_rows = [[
        "file", "coverage_status", "owner_surface", "risk", "suggested_pr", "evidence", "notes"
    ]]
    for index, relative_path in enumerate(CLOSURE_PATHS, start=1):
        path = root / relative_path
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("", encoding="utf-8")
        coverage_rows.append([relative_path, "exact", "fixture", "P0", "PR-FK", f"line:main:{index}", "fixture"])
    (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").write_text(tsv(coverage_rows), encoding="utf-8")
    for relative_path, markers in SOURCE_MARKERS.items():
        path = root / relative_path
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("\n".join(markers) + "\n", encoding="utf-8")
    grouped: dict[str, list[str]] = {}
    for relative_path, function_name in TEST_ANCHORS:
        grouped.setdefault(relative_path, []).append(function_name)
    for relative_path, names in grouped.items():
        path = root / relative_path
        path.parent.mkdir(parents=True, exist_ok=True)
        existing = path.read_text(encoding="utf-8") if path.exists() else ""
        path.write_text(
            existing + "".join(f"\n#[test]\nfn {name}() {{}}\n" for name in names),
            encoding="utf-8",
        )
    (root / "frontend/src/api/rest.rs").write_text("mod dto;\n", encoding="utf-8")
    (root / "frontend/src/api/rest/dto.rs").write_text("pub type Shared = u8;\n", encoding="utf-8")


def expect_failure(root: Path, expected: str) -> None:
    try:
        validate(root)
    except ValueError as error:
        if expected not in str(error):
            raise AssertionError(f"expected {expected!r}, got {error!r}") from error
        return
    raise AssertionError(f"expected validation failure containing {expected!r}")


def self_test() -> None:
    with tempfile.TemporaryDirectory(prefix="crossline-pr-fk-gate-") as temp:
        root = Path(temp)
        write_fixture(root)
        validate(root)
        source = root / "crates/llm/src/providers/deepseek.rs"
        source.write_text("", encoding="utf-8")
        expect_failure(root, "missing source marker")
        write_fixture(root)
        coverage = root / "docs/PRODUCT_AUDIT_COVERAGE.tsv"
        coverage.write_text(
            coverage.read_text(encoding="utf-8").replace("\texact\t", "\tbasename\t", 1),
            encoding="utf-8",
        )
        expect_failure(root, "closure path is not exact")
        write_fixture(root)
        doc = root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
        doc.write_text(
            doc.read_text(encoding="utf-8").replace("✅ 完成", "🟡 部分完成"),
            encoding="utf-8",
        )
        if validate(root):
            raise AssertionError("partial PR-FK row must skip enforcement")


root = Path(sys.argv[1])
mode = sys.argv[2]
try:
    if mode == "--self-test":
        self_test()
        print("OK PR-FK completion gate self-test")
    elif validate(root):
        print(
            "OK PR-FK completion gate "
            f"({len(EVIDENCE)} evidence types; {len(TEST_ANCHORS)} non-skipping Rust anchors; "
            f"{len(CLOSURE_PATHS)} exact paths)"
        )
    else:
        print("SKIP PR-FK completion gate (roadmap row remains partial)")
except (AssertionError, ValueError) as error:
    raise SystemExit(f"PR-FK completion gate failed: {error}") from error
PY
