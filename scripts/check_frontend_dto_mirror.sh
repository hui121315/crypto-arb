#!/usr/bin/env bash
set -euo pipefail

# Frontend REST modules may own transport-only errors and mutation context, but
# backend request/response DTOs must remain aliases or re-exports of shared-types.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REST_DIR="$ROOT/frontend/src/api/rest"

python3 - "$REST_DIR" <<'PY'
from __future__ import annotations

import re
import sys
from pathlib import Path

rest_dir = Path(sys.argv[1])
allowed_transport_types = {
    ("error.rs", "struct", "ApiError"),
    ("transport.rs", "struct", "MutationRequestContext"),
}
declaration = re.compile(
    r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?P<kind>struct|enum)\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)\b"
)
alias = re.compile(
    r"^\s*pub\s+type\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*(?:<[^>]+>)?)\s*=\s*(?P<target>.+);\s*$"
)

unexpected: list[str] = []
aliases = 0
for path in sorted(rest_dir.rglob("*.rs")):
    relative = path.relative_to(rest_dir).as_posix()
    for line_no, line in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
        match = declaration.match(line)
        if match is not None:
            key = (relative, match.group("kind"), match.group("name"))
            if key not in allowed_transport_types:
                unexpected.append(
                    f"{relative}:{line_no}: local {match.group('kind')} {match.group('name')}"
                )
        match = alias.match(line)
        if match is not None:
            aliases += 1
            if not match.group("target").strip().startswith("shared_types::"):
                unexpected.append(
                    f"{relative}:{line_no}: alias {match.group('name')} does not target shared_types"
                )

if unexpected:
    print(
        "repo gate failed: frontend REST backend DTOs must come from shared-types; "
        "only ApiError and MutationRequestContext may be transport-local",
        file=sys.stderr,
    )
    for item in unexpected:
        print(f"  {item}", file=sys.stderr)
    raise SystemExit(1)

print(
    "OK frontend dto mirror gate "
    f"({len(allowed_transport_types)} transport-only types; {aliases} shared aliases)"
)
PY
