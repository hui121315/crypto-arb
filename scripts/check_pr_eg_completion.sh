#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
MATRIX="$ROOT/frontend/src/panels/modules/settings/tabs/diagnostics/runtime_matrix.rs"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

if [[ "$MODE" == "--self-test" ]]; then
  PR_EG_SKIP_TESTS=1 bash "$0" >/dev/null
  backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-eg.XXXXXX")"
  cp "$MATRIX" "$backup"
  restore() {
    cp "$backup" "$MATRIX"
    rm -f "$backup"
  }
  trap restore EXIT
  python3 - "$MATRIX" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = 'data-settings-table="venue-runtime-health"'
if marker not in source:
    raise SystemExit("PR-EG self-test setup failed: matrix marker missing")
path.write_text(source.replace(marker, 'data-settings-table="runtime-health-drift"', 1), encoding="utf-8")
PY
  if PR_EG_SKIP_TESTS=1 bash "$0" >/dev/null 2>&1; then
    printf 'PR-EG completion self-test failed: broken Settings matrix passed\n' >&2
    exit 1
  fi
  printf 'PR-EG completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from __future__ import annotations

import csv
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])
title = "PR-EG API Operation Health & Status Center Contract"
verify_anchor = "`bash scripts/check_pr_eg_completion.sh --self-test`"
evidence_contract = {
    "runtime-health-dto": "shared-types/src/venues/runtime_health.rs",
    "runtime-health-projection": "shared-types/src/venues/runtime_health_snapshot.rs",
    "runtime-health-tests": "shared-types/src/venues/tests_runtime_health.rs",
    "runtime-health-api": "crates/api/src/routers/system.rs",
    "operation-health-source": "crates/api/src/services/venue_operation_health/snapshot.rs",
    "frontend-runtime-client": "frontend/src/api/rest/portfolio_system.rs",
    "frontend-runtime-hook": "frontend/src/panels/modules/settings/data/resources/runtime_health.rs",
    "frontend-status-center": "frontend/src/panels/modules/settings/tabs/diagnostics/runtime_matrix.rs",
    "runtime-health-browser": "test/e2e/pr_eg_runtime_health.spec.ts",
    "completion-governance": "scripts/check_pr_eg_completion.sh",
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-EG completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be completed with no local remainder")
if row.count(verify_anchor) != 1:
    fail("roadmap verification must name the destructive completion gate")
queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-EG\b", queue):
    fail("completed PR-EG remains in the local queue")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-EG"]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for kind, artifact in evidence_contract.items():
    if indexed[kind]["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"{kind} artifact drifted or is missing")

markers = {
    "shared-types/src/venues/runtime_health.rs": (
        "pub struct VenueRuntimeOperationHealth",
        "pub latency_p95_ms: Option<u64>",
        "pub requested: Option<u64>",
        "pub problem: Option<ApiProblem>",
        "pub fn operations(&self)",
    ),
    "shared-types/src/venues/runtime_health_snapshot.rs": (
        "pub operation_count: usize",
        "pub currently_usable_count: usize",
        "pub attention_count: usize",
    ),
    "crates/api/src/routers/system.rs": (
        '"/api/system/venue-runtime-health"',
        "VenueRuntimeHealthSnapshot::from_operation_rows",
        "api_projects_existing_operation_snapshot_without_external_probes",
    ),
    "frontend/src/api/rest/portfolio_system.rs": (
        "pub async fn venue_runtime_health",
        'self.get_json("/api/system/venue-runtime-health")',
    ),
    "frontend/src/panels/modules/settings/data/resources/runtime_health.rs": (
        "fn use_venue_runtime_health",
        "client.venue_runtime_health().await",
    ),
    "frontend/src/panels/modules/settings/tabs/diagnostics/runtime_matrix.rs": (
        'data-settings-table="venue-runtime-health"',
        '"交易运行状态中心"',
        "runtime_operation_title",
        "operation.problem.as_ref()",
    ),
}
for path, required in markers.items():
    source = (root / path).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"missing {path} marker: {marker}")

browser = (root / evidence_contract["runtime-health-browser"]).read_text(encoding="utf-8")
if re.search(r"\btest\.(?:skip|fixme)\s*\(", browser):
    fail("browser fixture must not be skipped")
for marker in ("8 家交易所", "当前可用 9 / 55 项", "PRIVATE_READ_PARTIAL", "无证据", "request_id req-okx"):
    if marker not in browser:
        fail(f"browser fixture missing runtime-health marker: {marker}")

package = (root / "package.json").read_text(encoding="utf-8")
if '"test:e2e:pr-eg"' not in package or "pr_eg_runtime_health.spec.ts" not in package:
    fail("package script or product suite wiring is missing")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
for path in evidence_contract.values():
    if coverage.get(path, {}).get("coverage_status") != "exact":
        fail(f"exact coverage missing for {path}")

print(f"OK PR-EG contract ({len(evidence_contract)} evidence types; typed 8-venue status center)")
PY

bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"

if [[ "${PR_EG_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p shared-types --lib runtime_health --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api --bin crypto-arb-api routers::system::tests --no-fail-fast
  cargo test --manifest-path "$ROOT/frontend/Cargo.toml" runtime_health --no-fail-fast
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-eg
fi

printf 'OK PR-EG typed operation health and Settings status center contract\n'
