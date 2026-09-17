#!/usr/bin/env bash
set -euo pipefail

# PR-CL: Markdown actionable status marker gate.
#
# Enforces the status rule declared at the top of
# docs/PRODUCT_FULL_AUDIT_REFINEMENT.md: every actionable item must carry exactly
# one of ✅ 完成 / 🟡 部分完成 / ⏳ 未开始 / ➖ 吸收/不适用 / ❌ 阻塞.
#
# Two actionable shapes are checked:
#   1. Batch headings — any `### ` heading that references a `N.M` audit batch /
#      roadmap section must begin (right after the `### `) with a status marker.
#   2. Coded table rows — any Markdown table row whose first cell is a
#      `` `AUD-…` `` or `` `PR-…` `` code must carry exactly one status marker in
#      its status (second) cell.
#
# This closes the named PR-CL remaining gap ("完整 Markdown actionable status
# marker gate"). The progress/evidence gates assume these markers exist and are
# unique; this gate makes that assumption explicit and fail-closed so a future
# edit cannot silently add an actionable item without a status.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DOC="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
HIST="$ROOT/docs/audit_history/PRODUCT_AUDIT_HISTORY.md"

if [ ! -s "$DOC" ]; then
  printf 'actionable status marker gate failed: missing or empty %s\n' "${DOC#$ROOT/}" >&2
  exit 1
fi

python3 - "$DOC" "$HIST" <<'PY'
import re
import sys
from pathlib import Path

docs = [Path(arg) for arg in sys.argv[1:] if Path(arg).exists()]
markers = ("✅", "🟡", "⏳", "➖", "❌")
marker_set = set(markers)

problems = []
heading_re = re.compile(r"^###\s+(.*\S)\s*$")
batch_ref_re = re.compile(r"\d+\.\d+")
code_cell_re = re.compile(r"^`(AUD|PR)-")

heading_count = 0
row_count = 0
for doc in docs:
  rel = doc.name
  for lineno, raw in enumerate(doc.read_text(encoding="utf-8").splitlines(), 1):
    heading = heading_re.match(raw)
    if heading and batch_ref_re.search(heading.group(1)):
        heading_count += 1
        # The first grapheme after `### ` must be a status marker.
        if heading.group(1)[:1] not in marker_set:
            problems.append(
                f"{rel}:{lineno}: batch heading missing leading status marker -> {raw.strip()[:80]}"
            )
        continue

    if raw.startswith("|"):
        cells = [cell.strip() for cell in raw.strip().strip("|").split("|")]
        if len(cells) >= 2 and code_cell_re.match(cells[0]):
            row_count += 1
            found = sum(cells[1].count(m) for m in markers)
            if found != 1:
                problems.append(
                    f"{rel}:{lineno}: coded row status cell must hold exactly one "
                    f"status marker (found {found}) -> {raw.strip()[:80]}"
                )

if heading_count == 0 or row_count == 0:
    problems.append(
        f"actionable status marker gate found no actionable items "
        f"(headings={heading_count}, rows={row_count}); doc shape changed"
    )

if problems:
    sys.stderr.write("actionable status marker gate failed:\n")
    sys.stderr.write("\n".join("  " + p for p in problems) + "\n")
    raise SystemExit(1)

print(
    f"OK actionable status marker gate ({heading_count} batch headings, "
    f"{row_count} coded rows checked)"
)
PY
