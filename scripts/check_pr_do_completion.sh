#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
INVENTORY="$ROOT/docs/API_ROUTE_INVENTORY.tsv"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

if [[ "$MODE" == "--self-test" ]]; then
  PR_DO_SKIP_TESTS=1 bash "$0" >/dev/null
  backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-do.XXXXXX")"
  cp "$INVENTORY" "$backup"
  restore() {
    cp "$backup" "$INVENTORY"
    rm -f "$backup"
  }
  trap restore EXIT
  python3 - "$INVENTORY" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "/api/v1/spot/ticks\tGET\tspot\tdiagnostic\tdefault_off\t"
if source.count(marker) != 1:
    raise SystemExit("PR-DO self-test setup failed: spot v1 inventory marker drifted")
path.write_text(source.replace(marker, marker.replace("default_off", "always")), encoding="utf-8")
PY
  if PR_DO_SKIP_TESTS=1 bash "$0" >/dev/null 2>&1; then
    printf 'PR-DO completion self-test failed: default-on spot v1 regression passed\n' >&2
    exit 1
  fi
  printf 'PR-DO completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from __future__ import annotations

import csv
import json
import re
import sys
from collections import Counter
from pathlib import Path

root = Path(sys.argv[1])
title = "PR-DO API Surface Legacy Route Classification & Exposure Gate"
verify_anchor = "`bash scripts/check_pr_do_completion.sh --self-test`"
browser_command = "playwright test test/e2e/route_registry.spec.ts"
evidence_contract = {
    "route-inventory-authority": "docs/API_ROUTE_INVENTORY.tsv",
    "route-spec-registry": "crates/api/src/route_specs.rs",
    "route-runtime-assembly": "crates/api/src/app.rs",
    "route-static-parity": "scripts/check_route_inventory.sh",
    "route-endpoint-metadata": "scripts/check_route_endpoint_metadata.sh",
    "route-runtime-policy": "scripts/check_route_runtime_policy.sh",
    "high-risk-audit-matrix": "scripts/check_mutation_audit_contract.sh",
    "real-api-security": "scripts/verify_api_security_runtime_smoke.sh",
    "browser-route-policy": "test/e2e/route_registry.spec.ts",
    "upstream-route-governance": "scripts/check_pr_fh_completion.sh",
    "upstream-security-governance": "scripts/check_pr_dk_completion.sh",
    "product-suite-contract": "package.json",
    "completion-governance": "scripts/check_pr_do_completion.sh",
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-DO completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be completed with no local remainder")
if row.count(verify_anchor) != 1:
    fail("roadmap verification must name the destructive completion gate")
queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-DO\b", queue):
    fail("completed PR-DO remains in the local queue")
for upstream in (
    "PR-FH Route Inventory, Legacy Feature Gate & Product Surface Contract",
    "PR-DK API Security Surface & High-Risk Audit Contract",
):
    upstream_row = next(
        (line for line in doc.splitlines() if line.startswith(f"| `{upstream}`")),
        None,
    )
    if upstream_row is None or "✅ 完成" not in upstream_row:
        fail(f"upstream completed contract drifted: {upstream}")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(
    encoding="utf-8", newline=""
) as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-DO"]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for kind, artifact in evidence_contract.items():
    if indexed[kind]["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"{kind} artifact drifted or is missing")

inventory_path = root / "docs/API_ROUTE_INVENTORY.tsv"
with inventory_path.open(encoding="utf-8", newline="") as handle:
    inventory = list(csv.DictReader(handle, delimiter="\t"))
expected_header = (
    "path", "methods", "router", "class", "default_exposure", "risk",
    "auth_policy", "audit_policy", "feature_flag", "shared_dto",
    "frontend_client", "owner", "notes",
)
if not inventory or tuple(inventory[0]) != expected_header:
    fail("route inventory header drifted")
endpoint_keys = [
    (method, route["path"])
    for route in inventory
    for method in route["methods"].split(",")
]
if len(endpoint_keys) != 95 or len(set(endpoint_keys)) != 95:
    fail(f"route endpoint inventory must remain unique and complete at 95, got {len(endpoint_keys)}")
exposure = Counter(route["default_exposure"] for route in inventory for _ in route["methods"].split(","))
if exposure != Counter({"always": 78, "default_off": 17}):
    fail(f"route exposure matrix drifted: {dict(exposure)}")
optional = [route for route in inventory if route["feature_flag"].startswith("api_surface.")]
if len(optional) != 17 or any(route["default_exposure"] != "default_off" for route in optional):
    fail("all 17 optional route endpoints must remain default-off")
spot = [route for route in inventory if route["path"] == "/api/v1/spot/ticks"]
if len(spot) != 1:
    fail("spot v1 route must have exactly one inventory row")
for field, expected in {
    "methods": "GET",
    "router": "spot",
    "class": "diagnostic",
    "default_exposure": "default_off",
    "risk": "low",
    "auth_policy": "bearer",
    "audit_policy": "read",
    "feature_flag": "api_surface.spot_v1",
}.items():
    if spot[0][field] != expected:
        fail(f"spot v1 {field} drifted: expected={expected}, actual={spot[0][field]}")
protected = [
    route for route in inventory
    if route["audit_policy"] in {"action_run", "secret_mutation"}
]
if len(protected) != 20 or any(
    route["class"] != "main_p0"
    or route["risk"] != "high"
    or route["auth_policy"] != "bearer"
    for route in protected
):
    fail("all 20 ActionRun/secret mutation routes must remain main-P0 high-risk bearer endpoints")

markers = {
    "crates/api/src/route_specs.rs": (
        "pub(crate) struct RouteSpec",
        "pub(crate) struct RouteEndpointSpec",
        "pub(crate) const ROUTE_SPECS",
        "pub(crate) fn build_surface_router",
    ),
    "scripts/check_route_inventory.sh": (
        "extract_route_methods",
        "frontend ApiClient method lacks inventory frontend_client coverage",
    ),
    "scripts/check_route_endpoint_metadata.sh": (
        "len(registry_rows)",
        "RouteEndpointSpec metadata drifted from inventory",
    ),
    "scripts/check_route_runtime_policy.sh": (
        "ALWAYS_ON_BEARER_ROUTE_RUNTIME_POLICY",
        "binary default-off matrix",
    ),
    "scripts/check_mutation_audit_contract.sh": (
        "inventory_high_risk_count",
        "runtime and auth-denial audit actions",
    ),
    "scripts/verify_api_security_runtime_smoke.sh": (
        "assert_default_off_surface_routes",
        "assert_always_on_bearer_auth_matrix",
        "assert_always_on_bearer_cors_matrix",
        "assert_always_on_bearer_read_matrix",
    ),
}
for path, required in markers.items():
    source = (root / path).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"missing {path} marker: {marker}")

app_source = (root / "crates/api/src/app.rs").read_text(encoding="utf-8")
for test_name in (
    "legacy_routes_gated_off_by_default",
    "gated_routes_register_when_enabled",
    "route_inventory_default_exposure_matches_build_router",
    "route_inventory_auth_policy_matches_runtime_security",
    "route_registry_seeded_endpoint_metadata_matches_inventory",
    "route_inventory_action_run_policies_match_typed_runtime_registry",
):
    pattern = re.compile(
        rf"(?m)^\s*#\[(?:tokio::)?test(?:\([^\]]*\))?\]\s*\n"
        rf"(?P<attrs>(?:\s*#\[[^\n]+\]\s*\n)*)"
        rf"\s*(?:async\s+)?fn\s+{re.escape(test_name)}\s*\("
    )
    match = pattern.search(app_source)
    if match is None or "ignore" in match.group("attrs"):
        fail(f"missing non-ignored app route test: {test_name}")

browser_path = root / "test/e2e/route_registry.spec.ts"
browser_source = browser_path.read_text(encoding="utf-8")
for title_anchor in (
    "every always-on bearer route preserves browser auth and typed error boundaries",
    "gated diagnostic routes stay absent even with local bearer credentials",
    "high-risk action and secret mutation persist correlated redacted audit pairs",
):
    escaped = re.escape(title_anchor)
    if re.search(rf"(?m)^\s*test\.(?:skip|fixme)\(\s*['\"]{escaped}['\"]", browser_source):
        fail(f"browser anchor must not be skipped: {title_anchor}")
    if not re.search(rf"(?m)^\s*test\(\s*['\"]{escaped}['\"]", browser_source):
        fail(f"missing non-skipping browser anchor: {title_anchor}")

package = json.loads((root / "package.json").read_text(encoding="utf-8"))
scripts = package.get("scripts", {})
if scripts.get("test:e2e:pr-do") != browser_command:
    fail("dedicated PR-DO browser command drifted")
if scripts.get("test:e2e:product", "").count("test/e2e/route_registry.spec.ts") != 1:
    fail("product browser suite must include route_registry.spec.ts exactly once")
release_fixture = json.loads(
    (root / "scripts/fixtures/release_qa_contract/package.json").read_text(encoding="utf-8")
)
release_product = release_fixture.get("scripts", {}).get("test:e2e:product", "")
if release_product.count("test/e2e/route_registry.spec.ts") != 1:
    fail("release QA browser suite must include route_registry.spec.ts exactly once")

history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(encoding="utf-8")
if "## 2026-07-14 PR-DO API Surface Closure" not in history:
    fail("PR-DO history appendix is missing")
for path in evidence_contract.values():
    if path not in history:
        fail(f"PR-DO history appendix lacks closure path: {path}")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(
    encoding="utf-8", newline=""
) as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
for path in evidence_contract.values():
    if path == "package.json" or path.startswith("docs/"):
        continue
    if coverage.get(path, {}).get("coverage_status") != "exact":
        fail(f"exact coverage missing for {path}")

print(
    f"OK PR-DO contract ({len(evidence_contract)} evidence types; "
    "95 routes + 17 default-off endpoints + 20 audited mutations)"
)
PY

bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"
bash "$ROOT/scripts/check_route_inventory.sh"
bash "$ROOT/scripts/check_route_endpoint_metadata.sh"
bash "$ROOT/scripts/check_route_runtime_policy.sh"
bash "$ROOT/scripts/check_mutation_audit_contract.sh"
PR_DK_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dk_completion.sh"
bash "$ROOT/scripts/check_release_qa_contract.sh"

if [[ "${PR_DO_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test \
    --manifest-path "$ROOT/Cargo.toml" \
    -p api \
    --bin crypto-arb-api \
    route_inventory_ \
    --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test \
    --manifest-path "$ROOT/Cargo.toml" \
    -p api \
    --bin crypto-arb-api \
    route_registry_ \
    --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" bash "$ROOT/scripts/verify_security_contract.sh"
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-do -- --workers=1
fi

printf 'OK PR-DO API surface classification and exposure contract\n'
