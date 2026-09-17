#!/usr/bin/env bash
# Fail-closed gate: backticked repo file paths referenced in docs must exist.
# Historical archives under docs/audit_history/ are exempt (they faithfully
# record deleted/renamed files).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

python3 - "$ROOT" <<'PY'
import glob
import os
import re
import sys

root = sys.argv[1]
pat = re.compile(
    r"`((?:crates|frontend|shared-types|scripts|docs|test|deploy)/"
    r"[A-Za-z0-9_\-./]+\.(?:rs|sh|css|tsv|md|toml|sql|json|yml|yaml|mjs|ts))`"
)

docs = sorted(glob.glob(os.path.join(root, "docs", "*.md")))
docs += [os.path.join(root, name) for name in ("README.md", "AGENTS.md", "CHANGELOG.md")]

bad = []
for doc in docs:
    if not os.path.exists(doc):
        continue
    with open(doc, encoding="utf-8") as fh:
        for lineno, line in enumerate(fh, 1):
            for match in pat.finditer(line):
                path = match.group(1)
                if not os.path.exists(os.path.join(root, path)):
                    bad.append(f"{os.path.relpath(doc, root)}:{lineno}: {path}")

if bad:
    print("doc path reference gate failed: referenced files do not exist", file=sys.stderr)
    for entry in bad:
        print(f"  {entry}", file=sys.stderr)
    sys.exit(1)

print(f"OK doc path references gate ({len(docs)} docs scanned)")
PY
