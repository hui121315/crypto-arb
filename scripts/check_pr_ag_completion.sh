#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
AUDIT="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
EVIDENCE="$ROOT/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
CONTRACT_ROOT="$ROOT/shared-types/src/contracts.rs"
P0_CONTRACT="$ROOT/shared-types/src/contracts/p0_opportunity.rs"
REST_FIXTURE="$ROOT/shared-types/fixtures/p0_opportunity_list_v1.json"
WS_FIXTURE="$ROOT/shared-types/fixtures/p0_opportunity_stream_v1.json"
FIXTURE_TEST="$ROOT/shared-types/tests/p0_contract_fixtures.rs"
REPO_GATE="$ROOT/scripts/verify_repo_gates.sh"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-AG completion gate failed: %s\n' "$1" >&2
  exit 1
}

run_static_gate() {
  PR_AG_SKIP_TESTS=1 PR_AG_SKIP_UPSTREAM=1 bash "$0" >/dev/null
}

expect_rejected() {
  local message="$1"
  if run_static_gate 2>/dev/null; then
    fail "$message"
  fi
}

if [[ "$MODE" == "--self-test" ]]; then
  audit_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-ag-audit.XXXXXX")"
  evidence_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-ag-evidence.XXXXXX")"
  root_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-ag-root.XXXXXX")"
  p0_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-ag-p0.XXXXXX")"
  fixture_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-ag-fixture.XXXXXX")"
  test_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-ag-test.XXXXXX")"
  repo_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-ag-repo.XXXXXX")"
  cp "$AUDIT" "$audit_backup"
  cp "$EVIDENCE" "$evidence_backup"
  cp "$CONTRACT_ROOT" "$root_backup"
  cp "$P0_CONTRACT" "$p0_backup"
  cp "$REST_FIXTURE" "$fixture_backup"
  cp "$FIXTURE_TEST" "$test_backup"
  cp "$REPO_GATE" "$repo_backup"
  restore() {
    cp "$audit_backup" "$AUDIT"
    cp "$evidence_backup" "$EVIDENCE"
    cp "$root_backup" "$CONTRACT_ROOT"
    cp "$p0_backup" "$P0_CONTRACT"
    cp "$fixture_backup" "$REST_FIXTURE"
    cp "$test_backup" "$FIXTURE_TEST"
    cp "$repo_backup" "$REPO_GATE"
    rm -f "$audit_backup" "$evidence_backup" "$root_backup" "$p0_backup" \
      "$fixture_backup" "$test_backup" "$repo_backup"
  }
  trap restore EXIT

  run_static_gate

  python3 - "$AUDIT" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "| `PR-AG Shared Types Contract Stratification` | ✅ 完成 |"
if source.count(marker) != 1:
    raise SystemExit("PR-AG self-test setup failed: completed row drifted")
path.write_text(source.replace(marker, "| `PR-AG Shared Types Contract Stratification` | 🟡 部分完成 |", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted a downgraded roadmap row"
  cp "$audit_backup" "$AUDIT"

  python3 - "$EVIDENCE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
lines = path.read_text(encoding="utf-8").splitlines()
for index, line in enumerate(lines):
    if line.startswith("PR-AG\tp0-rest-fixture\t"):
        del lines[index]
        break
else:
    raise SystemExit("PR-AG self-test setup failed: REST fixture evidence missing")
path.write_text("\n".join(lines) + "\n", encoding="utf-8")
PY
  expect_rejected "self-test accepted incomplete evidence"
  cp "$evidence_backup" "$EVIDENCE"

  python3 - "$CONTRACT_ROOT" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "pub mod p0_opportunity;"
if source.count(marker) != 1:
    raise SystemExit("PR-AG self-test setup failed: P0 module anchor drifted")
path.write_text(source.replace(marker, "// P0 facade removed", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted a removed P0 contract facade"
  cp "$root_backup" "$CONTRACT_ROOT"

  python3 - "$P0_CONTRACT" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = ".filter(|kind| is_p0_executable_strategy(*kind))"
if source.count(marker) != 1:
    raise SystemExit("PR-AG self-test setup failed: allowlist projection drifted")
path.write_text(source.replace(marker, ".filter(|_| true)", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted a disabled P0 allowlist"
  cp "$p0_backup" "$P0_CONTRACT"

  python3 - "$REST_FIXTURE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = '"strategyKind": "perp_cross"'
if source.count(marker) != 1:
    raise SystemExit("PR-AG self-test setup failed: REST strategy fixture drifted")
path.write_text(source.replace(marker, '"strategyKind": "spot_cross"', 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted semantic fixture drift"
  cp "$fixture_backup" "$REST_FIXTURE"

  python3 - "$FIXTURE_TEST" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "fn p0_ws_fixture_uses_the_same_fail_closed_row_contract()"
if source.count(marker) != 1:
    raise SystemExit("PR-AG self-test setup failed: WS test anchor drifted")
path.write_text(source.replace(marker, "fn skipped_p0_ws_fixture_contract()", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted a missing non-skipping fixture test"
  cp "$test_backup" "$FIXTURE_TEST"

  python3 - "$AUDIT" <<'PY'
from pathlib import Path
import re
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
updated, count = re.subn(r"(?m)^1\. \*\*PR-AH\b", "1. **PR-AG", source, count=1)
if count != 1:
    raise SystemExit("PR-AG self-test setup failed: successor queue head drifted")
path.write_text(updated, encoding="utf-8")
PY
  expect_rejected "self-test accepted PR-AG reinserted into the local queue"
  cp "$audit_backup" "$AUDIT"

  python3 - "$REPO_GATE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = 'PR_AG_SKIP_TESTS=1 PR_AG_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_ag_completion.sh"'
if source.count(marker) != 2:
    raise SystemExit("PR-AG self-test setup failed: repo wiring drifted")
path.write_text(source.replace(marker, "true # PR-AG gate removed", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted single-scope repository wiring"

  printf 'PR-AG completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from __future__ import annotations

import csv
import hashlib
import json
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])
pr_id = "PR-AG"
title = "PR-AG Shared Types Contract Stratification"
verify_anchor = "`bash scripts/check_pr_ag_completion.sh --self-test`"
evidence_contract = {
    "p0-allowlist": "shared-types/src/strategy.rs",
    "contract-root": "shared-types/src/contracts.rs",
    "p0-contract-projection": "shared-types/src/contracts/p0_opportunity.rs",
    "execution-contract-layer": "shared-types/src/contracts/execution.rs",
    "market-data-contract-layer": "shared-types/src/contracts/market_data.rs",
    "account-health-contract-layer": "shared-types/src/contracts/account_health.rs",
    "diagnostics-legacy-layer": "shared-types/src/contracts/diagnostics.rs",
    "p0-rest-fixture": "shared-types/fixtures/p0_opportunity_list_v1.json",
    "p0-ws-fixture": "shared-types/fixtures/p0_opportunity_stream_v1.json",
    "p0-fixture-tests": "shared-types/tests/p0_contract_fixtures.rs",
    "backend-p0-contract-boundary": "crates/api/src/services/opportunity.rs",
    "frontend-p0-contract-boundary": "frontend/src/api/rest/dto.rs",
    "completion-governance": "scripts/check_pr_ag_completion.sh",
}
fixture_hashes = {
    "shared-types/fixtures/p0_opportunity_list_v1.json": "6065ee6ac9a8b176f96c68978a136fe23b5724766ff9a6a2e21506d2bab82892",
    "shared-types/fixtures/p0_opportunity_stream_v1.json": "dd87bfbc9177a95816d9db1b3c4c24abd9dab2964d44e0fc25eace80e23592cc",
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-AG completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be complete with no local remainder")
if row.count(verify_anchor) != 1:
    fail("roadmap verification must name the destructive gate once")

queue_start = doc.find("### 🟡 6.5")
if queue_start < 0:
    fail("local execution queue is missing")
queue = doc[queue_start:]
if re.search(r"(?m)^\d+\.\s+\*\*PR-AG\b", queue):
    fail("completed PR-AG remains in the local queue")
successor_title = "PR-AH API Gateway & Runtime State Boundary"
successor_row = next(
    (line for line in doc.splitlines() if line.startswith(f"| `{successor_title}`")),
    None,
)
if successor_row is None:
    fail("PR-AH successor roadmap row is missing")
successor_complete = "✅ 完成" in successor_row
successor_queued = re.search(r"(?m)^\d+\.\s+\*\*PR-AH\b", queue) is not None
successor_is_head = re.search(r"(?m)^1\.\s+\*\*PR-AH\b", queue) is not None
if successor_complete and successor_queued:
    fail("completed PR-AH successor remains in the local queue")
if not successor_complete and not successor_is_head:
    fail("unfinished PR-AH successor must be the local queue head")

history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(encoding="utf-8")
if "## 2026-07-17 PR-AG Shared Types Contract Stratification Closure" not in history:
    fail("history closure appendix is missing")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == pr_id]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for evidence_type, artifact in evidence_contract.items():
    evidence = indexed[evidence_type]
    if evidence["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"evidence artifact drifted or is missing: {evidence_type}")
    if not evidence["command"].strip() or not evidence["notes"].strip():
        fail(f"evidence lacks command or notes: {evidence_type}")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
for artifact in evidence_contract.values():
    if Path(artifact).suffix == ".json":
        continue
    item = coverage.get(artifact)
    if item is None or item["coverage_status"] != "exact" or not item["evidence"].startswith("line:"):
        fail(f"exact coverage missing for {artifact}")

markers = {
    "shared-types/src/contracts.rs": (
        "pub mod account_health;",
        "pub mod diagnostics;",
        "pub mod execution;",
        "pub mod market_data;",
        "pub mod p0_opportunity;",
        "pub use p0_opportunity as p0;",
    ),
    "shared-types/src/contracts/execution.rs": (
        "pub type CostEvidenceMeta = ProfitabilityEvidence;",
        "pub type FeeEvidenceMeta = FeeScheduleEvidence;",
        "pub type OrderFill = FillLedgerSnapshot;",
        "pub type LegFinality = ExecutionRunLegEvidence;",
        "VenueOrderIdentity",
    ),
    "shared-types/src/contracts/market_data.rs": (
        "MarketDataHealth",
        "MarketDataQuality",
        "MarketDataRowEvidence",
    ),
    "shared-types/src/contracts/account_health.rs": (
        "AccountDataHealth",
        "VenueRuntimeHealth",
        "VenueRuntimeOperationHealth",
    ),
    "shared-types/src/contracts/diagnostics.rs": (
        "pub mod legacy",
        "ArbitrageOpportunityDto",
        "ExecutionMode",
        "OnchainMetadata",
        "OptionMarketQuote",
    ),
    "shared-types/src/contracts/p0_opportunity.rs": (
        "pub struct OpportunityCore",
        "pub type OpportunityMetrics = OpportunityListMetrics;",
        "pub type OpportunityExecution = OpportunityListExecution;",
        "pub struct OpportunityEvidence",
        "pub struct P0OpportunityContract",
        ".filter(|kind| is_p0_executable_strategy(*kind))",
        "row.cost.fee_evidence_complete",
        "MarketDataQuality::Fresh",
    ),
    "crates/api/src/services/opportunity.rs": (
        "use shared_types::contracts::p0::{",
        "OpportunityListEnvelope",
        "OpportunityStreamEvent",
    ),
    "frontend/src/api/rest/dto.rs": (
        "shared_types::contracts::execution::{",
        "shared_types::contracts::p0::OpportunityListEnvelope",
        "shared_types::contracts::p0::OpportunityStreamPayload",
    ),
}
for relative, required in markers.items():
    source = (root / relative).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"source marker missing: {relative}:{marker}")

p0_source = (root / "shared-types/src/contracts/p0_opportunity.rs").read_text(encoding="utf-8")
for legacy_name in ("ExecutionMode", "OnchainMetadata", "OptionMarketQuote", "Simulation", "Llm"):
    if legacy_name in p0_source:
        fail(f"legacy contract leaked into P0 facade: {legacy_name}")

for relative, expected in fixture_hashes.items():
    raw = (root / relative).read_bytes()
    if hashlib.sha256(raw).hexdigest() != expected:
        fail(f"fixture hash drifted: {relative}")
    payload = json.loads(raw)
    rows = payload.get("rows", payload.get("changedRows", []))
    if payload.get("scope") != "main_p0" or len(rows) != 1:
        fail(f"fixture scope or row cardinality drifted: {relative}")
    row = rows[0]
    if row.get("strategyKind") != "perp_cross" or row.get("strategyCategory") != "futures":
        fail(f"fixture P0 strategy semantics drifted: {relative}")
    if row.get("execution", {}).get("eligible") is not True:
        fail(f"fixture no longer exercises executable evidence: {relative}")

test_source = (root / "shared-types/tests/p0_contract_fixtures.rs").read_text(encoding="utf-8")
test_titles = (
    "p0_rest_fixture_projects_into_four_stratified_contracts",
    "p0_ws_fixture_uses_the_same_fail_closed_row_contract",
    "p0_projection_rejects_non_p0_missing_category_and_false_execution_evidence",
    "contract_facades_keep_legacy_modes_outside_product_layers",
)
if "#[ignore]" in test_source:
    fail("fixture contract tests must not be ignored")
for title in test_titles:
    if len(re.findall(rf"(?m)^fn {re.escape(title)}\s*\(", test_source)) != 1:
        fail(f"non-skipping fixture test is missing: {title}")

repo_gate = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
if repo_gate.count("check_pr_ag_completion.sh") != 2:
    fail("repo gate must execute PR-AG exactly once in docs and all scopes")

print(f"OK PR-AG static contract ({len(evidence_contract)} evidence types; {len(test_titles)} fixture tests)")
PY

if [[ "${PR_AG_SKIP_UPSTREAM:-0}" != "1" ]]; then
  bash "$ROOT/scripts/check_product_audit_progress.sh"
  bash "$ROOT/scripts/check_product_audit_evidence.sh"
  bash "$ROOT/scripts/check_product_audit_evidence_index.sh"
  bash "$ROOT/scripts/check_product_audit_coverage.sh"
  bash "$ROOT/scripts/check_frontend_dto_mirror.sh"
  bash "$ROOT/scripts/check_frontend_module_boundaries.sh"
fi

if [[ "${PR_AG_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p shared-types \
    --test p0_contract_fixtures --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p shared-types \
    strategy --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo check -p api
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo check \
    --manifest-path "$ROOT/frontend/Cargo.toml" --target wasm32-unknown-unknown
fi

printf 'PR-AG completion gate passed\n'
