#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
RUNTIME_WRAPPER="$ROOT/scripts/verify_runtime_contracts_with_api.sh"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

if [[ "$MODE" == "--self-test" ]]; then
  PR_DJ_SKIP_TESTS=1 bash "$0" >/dev/null
  backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-dj.XXXXXX")"
  cp "$RUNTIME_WRAPPER" "$backup"
  restore() {
    cp "$backup" "$RUNTIME_WRAPPER"
    chmod +x "$RUNTIME_WRAPPER"
    rm -f "$backup"
  }
  trap restore EXIT
  python3 - "$RUNTIME_WRAPPER" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "ALLOW_RUNTIME_SKIP=0"
if source.count(marker) != 2:
    raise SystemExit("PR-DJ self-test setup failed: no-skip marker missing")
path.write_text(source.replace(marker, "ALLOW_RUNTIME_SKIP=1"), encoding="utf-8")
PY
  if PR_DJ_SKIP_TESTS=1 bash "$0" >/dev/null 2>&1; then
    printf 'PR-DJ completion self-test failed: skippable runtime wrapper passed\n' >&2
    exit 1
  fi
  printf 'PR-DJ completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from __future__ import annotations

import csv
import json
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])
title = "PR-DJ Runtime Health Verification Gate & CI Evidence Contract"
verify_anchor = "`bash scripts/check_pr_dj_completion.sh --self-test`"
evidence_contract = {
    "typed-transport-health": "crates/api/src/services/venue_operation_health/snapshot/part_03.rs",
    "typed-transport-problems": "crates/api/src/services/venue_operation_health/snapshot/part_17.rs",
    "typed-transport-tests": "crates/api/src/services/venue_operation_health/snapshot/tests/part_12.rs",
    "runtime-required-wrapper": "scripts/verify_runtime_contracts_with_api.sh",
    "runtime-contract": "scripts/verify_runtime_contracts.sh",
    "local-verification": "scripts/verify_implementation.sh",
    "ci-runtime-gate": ".github/workflows/ci.yml",
    "top-scope-drilldown": "frontend/src/panels/status_bar/view.rs",
    "external-fixture-gate": "scripts/check_exchange_operation_evidence_matrix.sh",
    "product-browser": "test/e2e/pr_dj_runtime_gate.spec.ts",
    "completion-governance": "scripts/check_pr_dj_completion.sh",
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-DJ completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be completed with no local remainder")
if row.count(verify_anchor) != 1:
    fail("roadmap verification must name the destructive completion gate")
queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-DJ\b", queue):
    fail("completed PR-DJ remains in the local queue")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-DJ"]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for kind, artifact in evidence_contract.items():
    if indexed[kind]["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"{kind} artifact drifted or is missing")

markers = {
    "crates/api/src/services/venue_operation_health/snapshot/part_03.rs": (
        "host_gate_problem(&snapshot",
        "rate_limiter_problem(snapshot",
        "problem,",
    ),
    "crates/api/src/services/venue_operation_health/snapshot/part_17.rs": (
        '"HOST_GATE_RATE_LIMITED"',
        '"HOST_GATE_FAILURE_STREAK"',
        '"RATE_LIMITER_PRESSURE"',
        'codes::CIRCUIT_BREAKER_OPEN',
        '"circuit_open"',
        '"rate_limit_backoff"',
    ),
    "scripts/verify_runtime_contracts_with_api.sh": (
        "RUNTIME_DIR=\"$(mktemp -d",
        "trap cleanup EXIT",
        "start_api",
        "ALLOW_RUNTIME_SKIP=0",
        "verify_runtime_contracts.sh",
    ),
    "scripts/verify_implementation.sh": ("verify_runtime_contracts_with_api.sh",),
    ".github/workflows/ci.yml": (
        "runtime-contracts:",
        "RUNTIME_BUILD_API=0 bash scripts/verify_runtime_contracts_with_api.sh",
        "npm run test:e2e:product",
    ),
    "frontend/src/panels/status_bar/view.rs": (
        "MarketDataStatusSlot",
        "{api_status_slot(operation_health, operation_problem, environment)}",
        "WsStatusSlot",
        "AppWsStatusSlot",
    ),
    "scripts/check_exchange_operation_evidence_matrix.sh": (
        "exchange operation evidence matrix",
    ),
}
for path, required in markers.items():
    source = (root / path).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"missing {path} marker: {marker}")

runtime_wrapper = (root / "scripts/verify_runtime_contracts_with_api.sh").read_text(
    encoding="utf-8"
)
if runtime_wrapper.count("ALLOW_RUNTIME_SKIP=0") != 2:
    fail("runtime wrapper must force no-skip in both token branches")
if "ALLOW_RUNTIME_SKIP=1" in runtime_wrapper:
    fail("runtime wrapper contains a skippable contract branch")

runtime_contract = (root / "scripts/verify_runtime_contracts.sh").read_text(encoding="utf-8")
if 'ALLOW_RUNTIME_SKIP="${ALLOW_RUNTIME_SKIP:-0}"' not in runtime_contract:
    fail("runtime verifier no longer defaults to required")
if 'fail "base=$API_URL reason=health_unreachable"' not in runtime_contract:
    fail("unreachable runtime no longer fails closed")

browser_path = evidence_contract["product-browser"]
browser = (root / browser_path).read_text(encoding="utf-8")
if re.search(r"\btest\.(?:skip|fixme)\s*\(", browser):
    fail("browser fixture must not be skipped")
for marker in (
    "PR-DJ top status keeps four runtime scopes distinct",
    "PR-DJ Settings exposes typed HostGate and RateLimiter diagnostics",
    "CIRCUIT_BREAKER_OPEN",
    "RATE_LIMITER_PRESSURE",
    "retry 12000ms",
):
    if marker not in browser:
        fail(f"browser fixture missing runtime gate marker: {marker}")

package = json.loads((root / "package.json").read_text(encoding="utf-8"))
scripts = package.get("scripts", {})
if scripts.get("test:e2e:pr-dj") != "playwright test test/e2e/pr_dj_runtime_gate.spec.ts":
    fail("dedicated browser command is missing")
if scripts.get("test:e2e:product", "").count(browser_path) != 1:
    fail("product suite must include the PR-DJ fixture exactly once")
if scripts.get("verify:runtime") != "bash scripts/verify_runtime_contracts_with_api.sh":
    fail("package runtime verification must use the self-starting wrapper")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
for path in evidence_contract.values():
    if coverage.get(path, {}).get("coverage_status") != "exact":
        fail(f"exact coverage missing for {path}")

print(
    f"OK PR-DJ contract ({len(evidence_contract)} evidence types; "
    "runtime-required + typed transport + Chromium closure)"
)
PY

if ALLOW_RUNTIME_SKIP=0 API_URL=http://127.0.0.1:1 \
  bash "$ROOT/scripts/verify_runtime_contracts.sh" >/dev/null 2>&1; then
  printf 'PR-DJ completion gate failed: unreachable runtime passed without skip\n' >&2
  exit 1
fi

bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"

if [[ "${PR_DJ_SKIP_TESTS:-0}" != "1" ]]; then
  jobs="${CARGO_BUILD_JOBS:-8}"
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api host_gate_ --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api rate_limiter_ --no-fail-fast
  cargo test --manifest-path "$ROOT/frontend/Cargo.toml" status_bar --no-fail-fast
  bash "$ROOT/scripts/check_exchange_evidence_debt.sh"
  bash "$ROOT/scripts/check_exchange_operation_evidence_matrix.sh"
  bash "$ROOT/scripts/verify_runtime_contracts_with_api.sh"
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-dj -- --workers=1
fi

printf 'OK PR-DJ runtime health verification and CI evidence contract\n'
