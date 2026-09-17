#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DOC="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
HIST="$ROOT/docs/audit_history/PRODUCT_AUDIT_HISTORY.md"
EVIDENCE="$ROOT/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
LEDGER="$ROOT/docs/PRODUCT_AUDIT_COVERAGE.tsv"
MODE="${1:-check}"

if [ "$MODE" != "check" ] && [ "$MODE" != "--write" ]; then
  printf 'usage: %s [--write]\n' "$0" >&2
  exit 2
fi

python3 - "$ROOT" "$DOC" "$HIST" "$EVIDENCE" "$LEDGER" "$MODE" <<'PY'
from collections import Counter
from pathlib import Path
import sys

root = Path(sys.argv[1])
doc = Path(sys.argv[2])
hist = Path(sys.argv[3])
evidence_ledger = Path(sys.argv[4])
ledger = Path(sys.argv[5])
mode = sys.argv[6]

target_roots = ("crates", "frontend", "shared-types", "scripts", "test", ".github")
target_exts = {
    ".css",
    ".html",
    ".js",
    ".mjs",
    ".rs",
    ".sh",
    ".sql",
    ".toml",
    ".ts",
    ".tsv",
    ".txt",
    ".yaml",
    ".yml",
}
ignored_parts = {
    ".generated",
    "dist",
    "node_modules",
    "playwright-report",
    "target",
    "test-results",
}


def collect_files() -> list[str]:
    files: list[str] = []
    for target_root in target_roots:
        base = root / target_root
        if not base.exists():
            continue
        for path in base.rglob("*"):
            rel = path.relative_to(root)
            if not path.is_file():
                continue
            if path.suffix not in target_exts:
                continue
            if ignored_parts.intersection(rel.parts):
                continue
            files.append(rel.as_posix())
    return sorted(files)


def line_index(sources: list[tuple[str, list[str]]], needle: str) -> str | None:
    for label, lines in sources:
        for index, line in enumerate(lines, start=1):
            if needle in line:
                return f"{label}:{index}"
    return None


def owner_surface(path: str) -> str:
    if path.startswith("crates/exchange/src/adapters/"):
        return "exchange_adapter"
    if path.startswith("crates/exchange/tests/"):
        return "exchange_tests"
    if path.startswith("crates/api/src/routers/"):
        return "api_router"
    if path.startswith("crates/api/src/services/"):
        return "api_service"
    if path.startswith("crates/api/src/lifecycle/"):
        return "api_lifecycle"
    if path.startswith("crates/arbitrage/src/"):
        return "arbitrage_domain"
    if path.startswith("crates/trading/src/"):
        return "trading_domain"
    if path.startswith("crates/realtime/src/"):
        return "realtime_domain"
    if path.startswith("frontend/src/panels/modules/"):
        return "frontend_module"
    if path.startswith("frontend/src/api/"):
        return "frontend_transport"
    if path.startswith("frontend/src/state/"):
        return "frontend_runtime"
    if path.startswith("frontend/styles/"):
        return "frontend_style"
    if path.startswith("shared-types/"):
        return "shared_contract"
    if path.startswith("scripts/"):
        return "verification_script"
    if path.startswith("test/e2e/"):
        return "e2e_fixture"
    if path.startswith(".github/"):
        return "ci"
    return path.split("/", 1)[0]


def risk_and_pr(surface: str, path: str) -> tuple[str, str]:
    if surface in {"exchange_adapter", "exchange_tests"}:
        return "P0", "PR-FW/PR-GI"
    if surface in {"api_router", "api_service", "api_lifecycle"}:
        return "P0", "PR-GI/PR-FT"
    if surface in {"arbitrage_domain", "trading_domain", "shared_contract"}:
        return "P0", "PR-GI/PR-DD"
    if surface.startswith("frontend_"):
        return "P0", "PR-GI/PR-FS"
    if surface in {"verification_script", "ci", "e2e_fixture"}:
        return "P0", "PR-GI/PR-AW"
    if path.startswith("crates/options/") or path.startswith("crates/llm/"):
        return "P1", "PR-GI/PR-CP"
    return "P1", "PR-GI"


def build_rows() -> list[list[str]]:
    sources = [("main", doc.read_text(encoding="utf-8").splitlines())]
    if hist.exists():
        sources.append(("hist", hist.read_text(encoding="utf-8").splitlines()))
    if evidence_ledger.exists():
        sources.append(
            ("evidence", evidence_ledger.read_text(encoding="utf-8").splitlines())
        )
    rows: list[list[str]] = []
    for file in collect_files():
        exact_line = line_index(sources, file)
        basename_line = line_index(sources, Path(file).name) if exact_line is None else None
        if exact_line is not None:
            status = "exact"
            evidence = f"line:{exact_line}"
            notes = "exact path referenced in product audit"
        elif basename_line is not None:
            status = "basename"
            evidence = f"line:{basename_line}"
            notes = "basename-only audit proxy; needs exact path"
        else:
            status = "missing"
            evidence = ""
            notes = "needs audit row or explicit non-mainline disposition"
        surface = owner_surface(file)
        risk, suggested_pr = risk_and_pr(surface, file)
        rows.append([file, status, surface, risk, suggested_pr, evidence, notes])
    return rows


header = ["file", "coverage_status", "owner_surface", "risk", "suggested_pr", "evidence", "notes"]
rows = build_rows()
content = "\t".join(header) + "\n" + "\n".join("\t".join(row) for row in rows) + "\n"

if mode == "--write":
    ledger.write_text(content, encoding="utf-8")
else:
    if not ledger.exists():
        raise SystemExit(
            "product audit coverage gate failed: missing docs/PRODUCT_AUDIT_COVERAGE.tsv; "
            "run scripts/check_product_audit_coverage.sh --write"
        )
    current = ledger.read_text(encoding="utf-8")
    if current != content:
        raise SystemExit(
            "product audit coverage gate failed: docs/PRODUCT_AUDIT_COVERAGE.tsv is stale; "
            "run scripts/check_product_audit_coverage.sh --write"
        )

counts = Counter(row[1] for row in rows)
print(
    "OK product audit coverage ledger "
    f"({len(rows)} files; exact={counts['exact']}; "
    f"basename={counts['basename']}; missing={counts['missing']})"
)
PY
