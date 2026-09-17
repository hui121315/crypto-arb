#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
INVENTORY="$ROOT/docs/API_ROUTE_INVENTORY.tsv"
REGISTRY="$ROOT/crates/api/src/route_specs.rs"

python3 - "$INVENTORY" "$REGISTRY" <<'PY'
import csv
import re
import sys
from pathlib import Path

inventory_path = Path(sys.argv[1])
registry_path = Path(sys.argv[2])

if not inventory_path.is_file():
    print(f"route endpoint metadata gate failed: missing {inventory_path}", file=sys.stderr)
    sys.exit(1)
if not registry_path.is_file():
    print(f"route endpoint metadata gate failed: missing {registry_path}", file=sys.stderr)
    sys.exit(1)

inventory_rows = {}
with inventory_path.open(newline="") as handle:
    for row in csv.DictReader(handle, delimiter="\t"):
        for method in row["methods"].split(","):
            key = (row["path"], method)
            if key in inventory_rows:
                print(
                    f"route endpoint metadata gate failed: duplicate inventory row {method} {row['path']}",
                    file=sys.stderr,
                )
                sys.exit(1)
            inventory_rows[key] = {
                "class": row["class"],
                "default_exposure": row["default_exposure"],
                "risk": row["risk"],
                "auth_policy": row["auth_policy"],
                "audit_policy": row["audit_policy"],
            }

registry_text = registry_path.read_text()
endpoint_re = re.compile(
    r"RouteEndpointSpec::(get|post|patch|delete)\(\s*"
    r'"([^"]+)"\s*,\s*'
    r'"([^"]+)"\s*,\s*'
    r'"([^"]+)"\s*,\s*'
    r'"([^"]+)"\s*,\s*'
    r'"([^"]+)"\s*,\s*'
    r'"([^"]+)"\s*,\s*\)',
    re.S,
)
action_run_re = re.compile(
    r"RouteEndpointSpec::(post|patch)_action_run\(\s*"
    r'"([^"]+)"\s*,\s*'
    r'"([^"]+)"\s*,\s*'
    r'"([^"]+)"\s*,\s*'
    r'"([^"]+)"\s*,\s*'
    r'"([^"]+)"\s*,\s*'
    r"ActionRunKind::[A-Za-z0-9_]+\s*,\s*\)",
    re.S,
)
struct_re = re.compile(
    r"RouteEndpointSpec\s*\{\s*"
    r'path:\s*"([^"]+)"\s*,\s*'
    r'methods:\s*"([^"]+)"\s*,\s*'
    r'class:\s*"([^"]+)"\s*,\s*'
    r'default_exposure:\s*"([^"]+)"\s*,\s*'
    r'risk:\s*"([^"]+)"\s*,\s*'
    r'auth_policy:\s*"([^"]+)"\s*,\s*'
    r'audit_policy:\s*"([^"]+)"\s*,\s*'
    r"action_run_kind:\s*(?:Some\(ActionRunKind::[A-Za-z0-9_]+\)|None)\s*,\s*\}",
    re.S,
)

method_names = {
    "get": "GET",
    "post": "POST",
    "patch": "PATCH",
    "delete": "DELETE",
}
registry_rows = {}

def add_registry_row(path, method, route_class, exposure, risk, auth_policy, audit_policy):
    key = (path, method)
    if key in registry_rows:
        print(
            f"route endpoint metadata gate failed: duplicate registry endpoint {method} {path}",
            file=sys.stderr,
        )
        sys.exit(1)
    registry_rows[key] = {
        "class": route_class,
        "default_exposure": exposure,
        "risk": risk,
        "auth_policy": auth_policy,
        "audit_policy": audit_policy,
    }

for match in endpoint_re.finditer(registry_text):
    add_registry_row(
        match.group(2),
        method_names[match.group(1)],
        *match.groups()[2:],
    )

for match in action_run_re.finditer(registry_text):
    method, path, route_class, exposure, risk, audit_policy = match.groups()
    add_registry_row(
        path,
        method_names[method],
        route_class,
        exposure,
        risk,
        "bearer",
        audit_policy,
    )

for match in struct_re.finditer(registry_text):
    path, method, route_class, exposure, risk, auth_policy, audit_policy = match.groups()
    add_registry_row(path, method, route_class, exposure, risk, auth_policy, audit_policy)

missing = sorted(set(inventory_rows) - set(registry_rows))
extra = sorted(set(registry_rows) - set(inventory_rows))
if missing:
    print("route endpoint metadata gate failed: inventory rows missing RouteEndpointSpec metadata", file=sys.stderr)
    for path, method in missing:
        print(f"  {method}\t{path}", file=sys.stderr)
if extra:
    print("route endpoint metadata gate failed: RouteEndpointSpec metadata not present in inventory", file=sys.stderr)
    for path, method in extra:
        print(f"  {method}\t{path}", file=sys.stderr)
if missing or extra:
    sys.exit(1)

drift = []
for key in sorted(inventory_rows):
    expected = inventory_rows[key]
    actual = registry_rows[key]
    for field in ("class", "default_exposure", "risk", "auth_policy", "audit_policy"):
        if expected[field] != actual[field]:
            path, method = key
            drift.append((method, path, field, expected[field], actual[field]))

if drift:
    print("route endpoint metadata gate failed: RouteEndpointSpec metadata drifted from inventory", file=sys.stderr)
    for method, path, field, expected, actual in drift:
        print(
            f"  {method}\t{path}\t{field}: inventory={expected} registry={actual}",
            file=sys.stderr,
        )
    sys.exit(1)

print(f"OK route endpoint metadata gate ({len(registry_rows)}/{len(inventory_rows)})")
PY
