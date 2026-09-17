#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
POSITION_CONTRACT="$ROOT/shared-types/src/portfolio/positions.rs"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

if [[ "$MODE" == "--self-test" ]]; then
  PR_DZ_SKIP_TESTS=1 bash "$0" >/dev/null
  backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-dz.XXXXXX")"
  cp "$POSITION_CONTRACT" "$backup"
  restore() {
    cp "$backup" "$POSITION_CONTRACT"
    rm -f "$backup"
  }
  trap restore EXIT
  python3 - "$POSITION_CONTRACT" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "    Unknown,\n    Ok,"
if marker not in source:
    raise SystemExit("PR-DZ self-test setup failed: unknown severity marker missing")
path.write_text(source.replace(marker, "    DriftedUnknown,\n    Ok,", 1), encoding="utf-8")
PY
  if PR_DZ_SKIP_TESTS=1 bash "$0" >/dev/null 2>&1; then
    printf 'PR-DZ completion self-test failed: drifted position severity passed\n' >&2
    exit 1
  fi
  printf 'PR-DZ completion self-test passed\n'
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
title = "PR-DZ Portfolio AccountState & CloseRun Contract"
verify_anchor = "`bash scripts/check_pr_dz_completion.sh --self-test`"
evidence_contract = {
    "portfolio-envelope-contract": "shared-types/src/portfolio/snapshot.rs",
    "position-quality-contract": "shared-types/src/portfolio/positions.rs",
    "wallet-nav-risk-contract": "crates/portfolio/src/risk.rs",
    "wallet-nav-source": "crates/api/src/services/portfolio/risk.rs",
    "portfolio-partial-snapshot": "crates/api/src/services/portfolio/snapshot.rs",
    "portfolio-envelope-problems": "crates/api/src/services/portfolio_snapshot_envelope.rs",
    "position-quality-projection": "crates/api/src/services/portfolio/pricing.rs",
    "position-row-health": "crates/api/src/services/account_positions/health.rs",
    "frontend-position-evidence": "frontend/src/panels/modules/positions/components/positions_table/quality.rs",
    "frontend-risk-nav": "frontend/src/panels/modules/positions/components/risk_panel.rs",
    "frontend-ws-degraded": "frontend/src/panels/modules/positions/data/snapshot.rs",
    "auto-compensation-worker": "crates/api/src/services/close_runs/auto_compensation.rs",
    "durable-close-run-store": "crates/api/src/services/close_run_store.rs",
    "auto-compensation-tests": "crates/api/src/services/close_runs/tests/cases_c/compensation.rs",
    "durable-finality-tests": "crates/api/src/services/close_runs/tests/cases_c/runtime.rs",
    "manual-terminal-tests": "crates/api/src/services/close_runs/tests/cases_b/manual_terminal.rs",
    "portfolio-browser-contract": "test/e2e/pr_dz_portfolio_truth.spec.ts",
    "completion-governance": "scripts/check_pr_dz_completion.sh",
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-DZ completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be completed with no local remainder")
if row.count(verify_anchor) != 1:
    fail("roadmap verification must name the destructive completion gate")
queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-DZ\b", queue):
    fail("completed PR-DZ remains in the local queue")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-DZ"]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for kind, artifact in evidence_contract.items():
    if indexed[kind]["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"{kind} artifact drifted or is missing")

markers = {
    "shared-types/src/portfolio/snapshot.rs": (
        "pub struct PortfolioSnapshotEnvelope",
        "pub problems: Vec<ApiProblem>",
        "pub account_state: AccountStateSnapshot",
    ),
    "shared-types/src/portfolio/positions.rs": (
        "pub enum PositionSeverity",
        "    Unknown,",
        "pub nav_evidence: PortfolioNavEvidence",
    ),
    "crates/portfolio/src/risk.rs": (
        "pub total_nav_usd: f64",
        "let total_nav = valid_nav(inputs.total_nav_usd)",
        "fn valid_nav(value: f64) -> f64",
    ),
    "crates/api/src/services/portfolio/risk.rs": (
        "status: AccountFieldQualityStatus::Actual",
        "status: AccountFieldQualityStatus::Missing",
        'source: "account_state.account_summaries.total_equity_usd"',
        "missing_venues.is_empty()",
    ),
    "crates/api/src/services/portfolio/snapshot.rs": (
        "account_state.status == ListStatus::Degraded",
        "summary.nav_evidence.status != AccountFieldQualityStatus::Actual",
        "field_quality_degraded(&account_state.field_quality)",
        "account_api_problem(problem, summary.nav_evidence.observed_at_ms)",
    ),
    "crates/api/src/services/portfolio_snapshot_envelope.rs": (
        "fn snapshot_api_problems(snapshot: &PortfolioSnapshot)",
        "problems.extend(snapshot.problems.iter().map(runtime_api_problem))",
        "codes::PORTFOLIO_SNAPSHOT_DEGRADED",
        "push_primary_problem(&mut envelope",
    ),
    "crates/api/src/services/portfolio/pricing.rs": (
        "PositionSeverity::Unknown",
        '"fundingRate8h"',
        '"nextFundingMs"',
    ),
    "crates/api/src/services/account_positions/health.rs": (
        "data_health.freshness_ms = health.freshness_ms",
        "data_health.retry_after_ms",
        "evidence.request_id.clone()",
    ),
    "frontend/src/panels/modules/positions/components/positions_table/quality.rs": (
        "pub(super) fn funding_quality_rows",
        '"fundingRate8h" | "nextFundingMs"',
        '"nextFundingMs" => "结算时间缺证据"',
    ),
    "frontend/src/panels/modules/positions/components/risk_panel.rs": (
        '"权益占比缺证据"',
        "AccountFieldQualityStatus::Actual",
    ),
    "frontend/src/panels/modules/positions/data/snapshot.rs": (
        "pub(in crate::panels::modules::positions) fn apply_snapshot_update",
        "snapshot.set(LoadState::Stale",
    ),
    "crates/api/src/services/close_runs/auto_compensation.rs": (
        "pub(crate) async fn auto_submit_compensation_once",
        "AutoCompensationSkip::LiveMode",
        "begin_auto_compensation_action_run",
        "auto_compensation_key(submit)",
    ),
    "crates/api/src/services/close_run_store.rs": (
        "append_durable_jsonl",
        "replay_close_runs",
        "append_projected_replay",
    ),
    "crates/api/src/services/close_runs/tests/cases_c/compensation.rs": (
        "auto_compensation_worker_submits_single_candidate_with_action_run",
        "auto_compensation_worker_retries_single_failed_candidate_once",
        "compensation_cost_uses_durable_slippage_event_ids",
    ),
    "crates/api/src/services/close_runs/tests/cases_c/runtime.rs": (
        "compensation_submit_runtime_requires_fresh_orderbook",
        "durable_close_projection_retry_does_not_double_incremental_fill",
    ),
    "crates/api/src/services/close_runs/tests/cases_b/manual_terminal.rs": (
        "manual_terminal_ack_success_commits_once_with_manual_source",
        "manual_terminal_ack_failure_leaves_hot_and_durable_state_unmutated",
        "manual terminal evidence must survive CloseRunStore replay",
    ),
}
for path, required in markers.items():
    source = (root / path).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"missing {path} marker: {marker}")

browser = (root / evidence_contract["portfolio-browser-contract"]).read_text(encoding="utf-8")
if re.search(r"\btest\.(?:skip|fixme)\s*\(", browser):
    fail("browser fixture must not be skipped")
for marker in (
    "PR-DZ portfolio envelope keeps wallet NAV, position evidence, partial state, and durable compensation visible",
    'expect(envelope.status).toBe("degraded")',
    'severity).toBe("unknown")',
    'toHaveClass(/unknown-row/)',
    'toContainText("权益占比缺证据")',
    'toContainText("等待补偿终态")',
):
    if marker not in browser:
        fail(f"browser fixture missing marker: {marker}")

package = json.loads((root / "package.json").read_text(encoding="utf-8"))
scripts = package.get("scripts", {})
if scripts.get("test:e2e:pr-dz") != "playwright test test/e2e/pr_dz_portfolio_truth.spec.ts":
    fail("dedicated browser script is missing")
if scripts.get("test:e2e:product", "").count("pr_dz_portfolio_truth.spec.ts") != 1:
    fail("product suite wiring is missing or duplicated")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
for path in evidence_contract.values():
    if coverage.get(path, {}).get("coverage_status") != "exact":
        fail(f"exact coverage missing for {path}")

print(
    f"OK PR-DZ contract ({len(evidence_contract)} evidence types; "
    "portfolio truth, position quality, durable compensation)"
)
PY

bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"

if [[ "${PR_DZ_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p portfolio --lib --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api --bin crypto-arb-api services::portfolio --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api --bin crypto-arb-api services::close_runs --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib positions --no-fail-fast
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-dz -- --workers=1
fi

printf 'OK PR-DZ portfolio account-state truth and durable close-run contract\n'
