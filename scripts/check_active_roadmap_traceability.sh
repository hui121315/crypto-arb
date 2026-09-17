#!/usr/bin/env bash
set -euo pipefail

# PR-CL: every current 6.3 roadmap row, including partial and pending rows, must
# have at least one repo-local code/test/runtime trace or an HTTPS official-doc
# trace in PRODUCT_AUDIT_EVIDENCE.tsv. Historical batch headings remain archive
# context; section 6.3 is the authoritative actionable state surface.

SCRIPT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"

case "$MODE" in
  check) ;;
  --root)
    [[ $# -eq 2 ]] || {
      printf 'usage: %s [--self-test|--root <repo-root>]\n' "$0" >&2
      exit 2
    }
    ROOT="$(cd "$2" && pwd)"
    ;;
  --self-test)
    fixture="$(mktemp -d "${TMPDIR:-/tmp}/crossline-roadmap-trace.XXXXXX")"
    trap 'rm -rf "$fixture"' EXIT
    mkdir -p "$fixture/docs" "$fixture/scripts"
    printf '#!/usr/bin/env bash\nexit 0\n' >"$fixture/scripts/proof.sh"
    chmod +x "$fixture/scripts/proof.sh"
    cat >"$fixture/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md" <<'EOF'
# Audit

### 🟡 6.3 Roadmap
| PR | Status | Work | Evidence |
|---|---|---|---|
| `PR-AA Alpha` | 🟡 部分完成 | active | remaining |
| `PR-AB Beta` | ⏳ 未开始 | queued | remaining |

### 🟡 6.4 Other
EOF
    printf 'pr_id\tevidence_type\tartifact\tcommand\tnotes\n' \
      >"$fixture/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
    printf 'PR-AA\tcode\tscripts/proof.sh\tbash scripts/proof.sh\tlocal code proof\n' \
      >>"$fixture/docs/PRODUCT_AUDIT_EVIDENCE.tsv"

    if bash "$SCRIPT" --root "$fixture" >/dev/null 2>&1; then
      printf 'active roadmap traceability self-test failed: missing partial trace passed\n' >&2
      exit 1
    fi

    printf 'PR-AB\ttest\tscripts/\tbash scripts/proof.sh\tlocal directory proof\n' \
      >>"$fixture/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
    bash "$SCRIPT" --root "$fixture" >/dev/null

    perl -0pi -e 's#bash scripts/proof\.sh#gh pr view 1#' \
      "$fixture/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
    if bash "$SCRIPT" --root "$fixture" >/dev/null 2>&1; then
      printf 'active roadmap traceability self-test failed: non-local command passed\n' >&2
      exit 1
    fi

    printf 'OK active roadmap traceability self-test\n'
    exit 0
    ;;
  *)
    printf 'usage: %s [--self-test|--root <repo-root>]\n' "$0" >&2
    exit 2
    ;;
esac

DOC="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
LEDGER="$ROOT/docs/PRODUCT_AUDIT_EVIDENCE.tsv"

python3 - "$ROOT" "$DOC" "$LEDGER" <<'PY'
from __future__ import annotations

import csv
import re
import sys
from collections import Counter, defaultdict
from pathlib import Path, PurePosixPath

root = Path(sys.argv[1])
doc = Path(sys.argv[2])
ledger = Path(sys.argv[3])
statuses = {
    "✅ 完成",
    "🟡 部分完成",
    "⏳ 未开始",
    "➖ 吸收/不适用",
    "❌ 阻塞",
}
header = ["pr_id", "evidence_type", "artifact", "command", "notes"]


def fail(message: str) -> None:
    raise SystemExit(f"active roadmap traceability gate failed: {message}")


if not doc.is_file() or not ledger.is_file():
    fail("missing product audit document or evidence ledger")

text = doc.read_text(encoding="utf-8")
start = text.find("### 🟡 6.3")
end = text.find("### 🟡 6.4", start)
if start < 0 or end < 0:
    fail("missing authoritative 6.3 roadmap section")

active: dict[str, str] = {}
for line in text[start:end].splitlines():
    if not line.startswith("| `PR-"):
        continue
    cells = [cell.strip() for cell in line.strip().strip("|").split("|")]
    if len(cells) < 4:
        fail(f"malformed roadmap row: {line[:100]}")
    match = re.fullmatch(r"`(PR-[A-Z0-9]+)(?: [^`]*)?`", cells[0])
    if match is None:
        fail(f"malformed roadmap identity: {cells[0]}")
    pr_id = match.group(1)
    if pr_id in active:
        fail(f"duplicate roadmap identity: {pr_id}")
    if cells[1] not in statuses:
        fail(f"{pr_id} has unsupported status marker: {cells[1]}")
    active[pr_id] = cells[1]

if not active:
    fail("roadmap contains no active PR rows")

with ledger.open(encoding="utf-8", newline="") as handle:
    reader = csv.DictReader(handle, delimiter="\t")
    if reader.fieldnames != header:
        fail(f"unexpected evidence header: {reader.fieldnames}")
    rows = list(reader)

by_pr: dict[str, list[dict[str, str]]] = defaultdict(list)
classes: Counter[str] = Counter()
non_local = re.compile(r"(^|[;&|]\s*)gh(\s|$)")
runtime_words = re.compile(r"runtime|smoke|capture|live[_-]sample", re.IGNORECASE)
test_words = re.compile(r"(^|[\s/:_-])(test|tests|fixture|playwright)([\s/:_.-]|$)", re.IGNORECASE)

for index, row in enumerate(rows, start=2):
    pr_id = row["pr_id"].strip()
    artifact = row["artifact"].strip()
    command = row["command"].strip()
    notes = row["notes"].strip()
    if pr_id not in active:
        fail(f"{ledger.name}:{index} references non-roadmap {pr_id}")
    if not artifact or not command or not notes:
        fail(f"{ledger.name}:{index} must contain artifact, command, and notes")
    if non_local.search(command):
        fail(f"{ledger.name}:{index} uses non-local GitHub CLI evidence")

    if artifact.startswith("https://"):
        trace_class = "official"
    elif artifact.startswith("http://"):
        fail(f"{ledger.name}:{index} official evidence must use HTTPS")
    else:
        path = PurePosixPath(artifact)
        if path.is_absolute() or ".." in path.parts:
            fail(f"{ledger.name}:{index} artifact must be repo-local: {artifact}")
        if not (root / artifact).exists():
            fail(f"{ledger.name}:{index} artifact is missing: {artifact}")
        combined = f"{artifact} {command}"
        if runtime_words.search(combined):
            trace_class = "runtime"
        elif test_words.search(combined):
            trace_class = "test"
        else:
            trace_class = "code"

    classes[trace_class] += 1
    by_pr[pr_id].append(row)

missing = sorted(set(active).difference(by_pr))
if missing:
    fail("active roadmap rows lack direct evidence: " + ", ".join(missing))

status_counts = Counter(active.values())
class_summary = ", ".join(f"{kind}={classes[kind]}" for kind in ("code", "test", "runtime", "official"))
print(
    "OK active roadmap traceability gate "
    f"({len(active)} PRs; completed={status_counts['✅ 完成']}; "
    f"partial={status_counts['🟡 部分完成']}; {class_summary})"
)
PY
