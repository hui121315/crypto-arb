#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
RESOURCE="$ROOT/shared-types/src/resource.rs"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

if [[ "$MODE" == "--self-test" ]]; then
  PR_EW_SKIP_TESTS=1 bash "$0" >/dev/null
  backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-ew.XXXXXX")"
  cp "$RESOURCE" "$backup"
  restore() {
    cp "$backup" "$RESOURCE"
    rm -f "$backup"
  }
  trap restore EXIT
  python3 - "$RESOURCE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "Self::Ready | Self::Degraded | Self::Partial"
if marker not in source:
    raise SystemExit("PR-EW self-test setup failed: usable-data marker missing")
path.write_text(source.replace(marker, "Self::Ready", 1), encoding="utf-8")
PY
  if PR_EW_SKIP_TESTS=1 bash "$0" >/dev/null 2>&1; then
    printf 'PR-EW completion self-test failed: partial data contract regression passed\n' >&2
    exit 1
  fi
  printf 'PR-EW completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from __future__ import annotations

import csv
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])
title = "PR-EW Shared Contract Envelope, ApiProblem & Runtime Evidence"
verify_anchor = "`bash scripts/check_pr_ew_completion.sh --self-test`"
evidence_contract = {
    "generic-resource-contract": "shared-types/src/resource.rs",
    "shared-action-state": "shared-types/src/actions/state.rs",
    "system-health-envelope": "crates/api/src/services/system_health.rs",
    "system-health-route": "crates/api/src/routers/system.rs",
    "action-run-envelope": "crates/api/src/services/action_runs/lifecycle.rs",
    "action-run-route": "crates/api/src/routers/trading/listing.rs",
    "frontend-envelope-consumer": "frontend/src/api/rest/portfolio_system.rs",
    "frontend-action-ledger-consumer": "frontend/src/api/rest/trading.rs",
    "paper-live-product-boundary": "frontend/src/panels/shared/execution_environment.rs",
    "resource-envelope-browser": "test/e2e/pr_ew_resource_envelope.spec.ts",
    "completion-governance": "scripts/check_pr_ew_completion.sh",
}
coverage_paths = (
    "shared-types/src/resource.rs",
    "shared-types/src/actions.rs",
    "shared-types/src/actions/state.rs",
    "shared-types/src/system.rs",
    "shared-types/src/lib.rs",
    "crates/api/src/services/system_health.rs",
    "crates/api/src/routers/system.rs",
    "crates/api/src/services/action_runs.rs",
    "crates/api/src/services/action_runs/lifecycle.rs",
    "crates/api/src/services/action_runs/tests/lifecycle.rs",
    "crates/api/src/routers/trading/listing.rs",
    "frontend/src/state/action_state.rs",
    "frontend/src/api/rest/portfolio_system.rs",
    "frontend/src/api/rest/trading.rs",
    "frontend/src/panels/shared/execution_environment.rs",
    "test/e2e/mock_api.mjs",
    "test/e2e/pr_ew_resource_envelope.spec.ts",
    "scripts/check_pr_ew_completion.sh",
    "scripts/check_release_qa_contract.sh",
    "scripts/verify_api_security_runtime_smoke.sh",
    "scripts/verify_repo_gates.sh",
)


def fail(message: str) -> None:
    raise SystemExit(f"PR-EW completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be completed with no local remainder")
if row.count(verify_anchor) != 1:
    fail("roadmap verification must name the destructive completion gate")
queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-EW\b", queue):
    fail("completed PR-EW remains in the local queue")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-EW"]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for kind, artifact in evidence_contract.items():
    if indexed[kind]["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"{kind} artifact drifted or is missing")

markers = {
    "shared-types/src/resource.rs": (
        "pub enum ResourceStatus",
        "Self::Ready | Self::Degraded | Self::Partial",
        "pub struct ResourceCoverage",
        "pub struct ResourceEnvelope<T>",
        "pub fn into_data(self) -> Result<T, Box<ApiProblem>>",
    ),
    "shared-types/src/actions/state.rs": (
        "pub enum ActionState",
        "problem: ApiProblem",
        "pub const fn is_pending",
    ),
    "shared-types/src/system.rs": (
        "pub fn to_api_problem(&self) -> crate::ApiProblem",
        '"operation": self.operation',
        '"venue": self.venue',
    ),
    "crates/api/src/services/system_health.rs": (
        "pub(crate) fn envelope(health: SystemHealth) -> SystemHealthEnvelope",
        "problem.to_api_problem()",
    ),
    "crates/api/src/services/action_runs/lifecycle.rs": (
        "pub(crate) fn recent_envelope",
        "ACTION_RUN_HISTORY_BOUNDED",
        "ResourceStatus::Partial",
    ),
    "frontend/src/api/rest/portfolio_system.rs": ("system_health_envelope", ".into_data()"),
    "frontend/src/api/rest/trading.rs": ("action_runs_envelope", ".into_data()"),
    "frontend/src/panels/shared/execution_environment.rs": (
        "ExecutionMode::DryRun",
        "ExecutionMode::Testnet",
        '"模拟"',
    ),
}
for path, required in markers.items():
    source = (root / path).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"missing {path} marker: {marker}")

frontend_state = (root / "frontend/src/state/action_state.rs").read_text(encoding="utf-8")
if "enum ActionState" in frontend_state or "pub use shared_types::ActionState;" not in frontend_state:
    fail("frontend ActionState must be a shared-types re-export")

browser = (root / "test/e2e/pr_ew_resource_envelope.spec.ts").read_text(encoding="utf-8")
if re.search(r"\btest\.(?:skip|fixme)\s*\(", browser):
    fail("browser fixture must not be skipped")
for marker in ("system-health-snapshot", "action-run-registry", "req-pr-ew-action-run"):
    if marker not in browser:
        fail(f"browser fixture missing envelope evidence: {marker}")

package = (root / "package.json").read_text(encoding="utf-8")
if '"test:e2e:pr-ew"' not in package or "pr_ew_resource_envelope.spec.ts" not in package:
    fail("package script or product suite wiring is missing")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
for path in coverage_paths:
    if coverage.get(path, {}).get("coverage_status") != "exact":
        fail(f"exact coverage missing for {path}")

print(f"OK PR-EW contract ({len(evidence_contract)} evidence types; {len(coverage_paths)} exact paths)")
PY

bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"

if [[ "${PR_EW_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p shared-types --lib resource::tests --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p shared-types --lib actions::state::tests --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api --bin crypto-arb-api system_health_route_returns_shared_resource_envelope --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api --bin crypto-arb-api recent_envelope_reports_bounded_history_as_partial --no-fail-fast
  cargo test --manifest-path "$ROOT/frontend/Cargo.toml" execution_environment --no-fail-fast
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-ew
fi

printf 'OK PR-EW shared resource, action state and runtime evidence contract\n'
