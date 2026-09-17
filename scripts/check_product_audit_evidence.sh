#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DOC="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"

if [ ! -s "$DOC" ]; then
  printf 'product audit evidence gate failed: missing or empty %s\n' "${DOC#$ROOT/}" >&2
  exit 1
fi

python3 - "$DOC" <<'PY'
from pathlib import Path
import re
import sys

doc = Path(sys.argv[1])
text = doc.read_text(encoding="utf-8")

start = text.find("### 🟡 6.3")
end = text.find("### 🟡 6.4", start)
if start == -1 or end == -1:
    raise SystemExit("product audit evidence gate failed: missing 6.3 roadmap section")

roadmap = text[start:end]
roadmap_start_line = text[:start].count("\n") + 1
evidence_pattern = re.compile(
    r"(验证|测试|单测|回归|gate|CI|cargo\s|bash\s|rg\s|trunk\s|npm\s|"
    r"Playwright|fixture|clippy|fmt)",
    re.IGNORECASE,
)

checked = 0
failures = []
for offset, line in enumerate(roadmap.splitlines(), start=roadmap_start_line):
    if not line.startswith("| `PR-"):
        continue
    cells = [cell.strip() for cell in line.strip().strip("|").split("|")]
    if len(cells) < 4 or "✅ 完成" not in cells[1]:
        continue
    checked += 1
    if not evidence_pattern.search(line):
        failures.append(f"{doc.name}:{offset} {cells[0]} lacks verification evidence")

if failures:
    raise SystemExit(
        "product audit evidence gate failed: completed roadmap rows need verification evidence\n"
        + "\n".join(f"  {failure}" for failure in failures)
    )

print(f"OK product audit evidence gate ({checked} completed roadmap rows checked)")
PY
