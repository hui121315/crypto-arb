#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
AUDIT="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
EVIDENCE="$ROOT/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
WS_OUTCOME="$ROOT/crates/api/src/lifecycle/market_data/ws_touch/outcome.rs"
FUNDING="$ROOT/crates/api/src/lifecycle/funding.rs"
STATS="$ROOT/crates/realtime/src/history/funding_stats/projector.rs"
STATS_TESTS="$ROOT/crates/realtime/src/history/funding_stats/tests.rs"
PRIVATE_TYPES="$ROOT/crates/api/src/trading_service/private_ws_events/types.rs"
REPO_GATE="$ROOT/scripts/verify_repo_gates.sh"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-AI completion gate failed: %s\n' "$1" >&2
  exit 1
}

run_static_gate() {
  PR_AI_SKIP_TESTS=1 PR_AI_SKIP_UPSTREAM=1 bash "$0" >/dev/null
}

expect_rejected() {
  local message="$1"
  if run_static_gate 2>/dev/null; then
    fail "$message"
  fi
}

if [[ "$MODE" == "--self-test" ]]; then
  audit_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-ai-audit.XXXXXX")"
  evidence_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-ai-evidence.XXXXXX")"
  ws_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-ai-ws.XXXXXX")"
  funding_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-ai-funding.XXXXXX")"
  stats_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-ai-stats.XXXXXX")"
  stats_tests_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-ai-stats-tests.XXXXXX")"
  private_types_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-ai-private-types.XXXXXX")"
  repo_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-ai-repo.XXXXXX")"
  cp "$AUDIT" "$audit_backup"
  cp "$EVIDENCE" "$evidence_backup"
  cp "$WS_OUTCOME" "$ws_backup"
  cp "$FUNDING" "$funding_backup"
  cp "$STATS" "$stats_backup"
  cp "$STATS_TESTS" "$stats_tests_backup"
  cp "$PRIVATE_TYPES" "$private_types_backup"
  cp "$REPO_GATE" "$repo_backup"
  restore() {
    cp "$audit_backup" "$AUDIT"
    cp "$evidence_backup" "$EVIDENCE"
    cp "$ws_backup" "$WS_OUTCOME"
    cp "$funding_backup" "$FUNDING"
    cp "$stats_backup" "$STATS"
    cp "$stats_tests_backup" "$STATS_TESTS"
    cp "$private_types_backup" "$PRIVATE_TYPES"
    cp "$repo_backup" "$REPO_GATE"
    rm -f "$audit_backup" "$evidence_backup" "$ws_backup" "$funding_backup" \
      "$stats_backup" "$stats_tests_backup" "$private_types_backup" "$repo_backup"
  }
  trap restore EXIT

  run_static_gate

  python3 - "$AUDIT" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "| `PR-AI Lifecycle Data Pipeline & Task Health` | ✅ 完成 |"
if source.count(marker) != 1:
    raise SystemExit("PR-AI self-test setup failed: completed row drifted")
path.write_text(source.replace(marker, "| `PR-AI Lifecycle Data Pipeline & Task Health` | 🟡 部分完成 |", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted a downgraded roadmap row"
  cp "$audit_backup" "$AUDIT"

  python3 - "$EVIDENCE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
lines = path.read_text(encoding="utf-8").splitlines()
for index, line in enumerate(lines):
    if line.startswith("PR-AI\tpublic-ws-subscribe-contract\t"):
        del lines[index]
        break
else:
    raise SystemExit("PR-AI self-test setup failed: public WS evidence missing")
path.write_text("\n".join(lines) + "\n", encoding="utf-8")
PY
  expect_rejected "self-test accepted incomplete evidence"
  cp "$evidence_backup" "$EVIDENCE"

  python3 - "$WS_OUTCOME" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "PublicWsSubscribeOutcome::Requested => runtime.market_data.record_runtime_pending("
if source.count(marker) != 1:
    raise SystemExit("PR-AI self-test setup failed: requested outcome drifted")
path.write_text(source.replace(marker, "PublicWsSubscribeOutcome::Requested => runtime.market_data.record_runtime_success(", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted a false-green requested subscription"
  cp "$ws_backup" "$WS_OUTCOME"

  python3 - "$FUNDING" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = ".query_funding_diffs(realtime::FundingDiffQuery {"
if source.count(marker) != 1:
    raise SystemExit("PR-AI self-test setup failed: funding bootstrap drifted")
path.write_text(source.replace(marker, ".query_funding_diff_stats(realtime::FundingDiffStatsQuery {", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted a recurring full funding stats query"
  cp "$funding_backup" "$FUNDING"

  python3 - "$STATS" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = ".insert(row.occurred_at_ms, row);"
if source.count(marker) != 1:
    raise SystemExit("PR-AI self-test setup failed: incremental merge drifted")
path.write_text(source.replace(marker, ".or_insert(row);", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted a non-replacing incremental projector"
  cp "$stats_backup" "$STATS"

  python3 - "$STATS_TESTS" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "#[test]\nfn projector_merges_only_new_rows_and_replaces_duplicate_timestamps()"
if source.count(marker) != 1:
    raise SystemExit("PR-AI self-test setup failed: projector test drifted")
path.write_text(source.replace(marker, "#[test]\n#[ignore]\nfn projector_merges_only_new_rows_and_replaces_duplicate_timestamps()", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted a skipped incremental projector fixture"
  cp "$stats_tests_backup" "$STATS_TESTS"

  python3 - "$PRIVATE_TYPES" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "    Positions(PrivatePositionsSnapshot),"
if source.count(marker) != 1:
    raise SystemExit("PR-AI self-test setup failed: private event contract drifted")
path.write_text(source.replace(marker, "    // positions event removed", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted an incomplete private event contract"
  cp "$private_types_backup" "$PRIVATE_TYPES"

  python3 - "$AUDIT" <<'PY'
from pathlib import Path
import re
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
updated, count = re.subn(
    r"(?m)^1\. \*\*(?!PR-AI\b)PR-[A-Z]+\b",
    "1. **PR-AI",
    source,
    count=1,
)
if count != 1:
    raise SystemExit("PR-AI self-test setup failed: successor queue head drifted")
path.write_text(updated, encoding="utf-8")
PY
  expect_rejected "self-test accepted PR-AI reinserted into the local queue"
  cp "$audit_backup" "$AUDIT"

  python3 - "$REPO_GATE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = 'PR_AI_SKIP_TESTS=1 PR_AI_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_ai_completion.sh"'
if source.count(marker) != 2:
    raise SystemExit("PR-AI self-test setup failed: repo wiring drifted")
path.write_text(source.replace(marker, "true # PR-AI gate removed", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted single-scope repository wiring"

  printf 'PR-AI completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from __future__ import annotations

import csv
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])
pr_id = "PR-AI"
title = "PR-AI Lifecycle Data Pipeline & Task Health"
verify_anchor = "`bash scripts/check_pr_ai_completion.sh --self-test`"
evidence_contract = {
    "lifecycle-task-health-authority": "scripts/check_pr_dr_completion.sh",
    "market-task-health": "crates/api/src/lifecycle/market_data.rs",
    "funding-task-and-incremental-stats": "crates/api/src/lifecycle/funding.rs",
    "opportunity-task-health": "crates/api/src/lifecycle/snapshot.rs",
    "public-ws-subscribe-contract": "crates/exchange/src/adapter.rs",
    "public-ws-runtime-outcome": "crates/api/src/lifecycle/market_data/ws_touch/outcome.rs",
    "public-ws-pending-fixture": "crates/api/src/lifecycle/market_data_tests/recovery.rs",
    "public-ws-unsupported-fixture": "crates/api/src/lifecycle/market_data_tests.rs",
    "funding-stats-projector": "crates/realtime/src/history/funding_stats/projector.rs",
    "funding-stats-projector-tests": "crates/realtime/src/history/funding_stats/tests.rs",
    "market-evidence-envelope": "shared-types/src/market.rs",
    "market-evidence-envelope-tests": "crates/api/src/services/market_data/envelope/tests/funding.rs",
    "opportunity-stream-envelope": "shared-types/src/arbitrage.rs",
    "opportunity-stream-envelope-tests": "crates/api/src/services/opportunity/tests/stale.rs",
    "private-event-contract": "crates/api/src/trading_service/private_ws_events/types.rs",
    "private-event-dispatch": "crates/api/src/lifecycle/private_ws/apply.rs",
    "private-event-batch-fixture": "crates/api/src/lifecycle/private_ws/tests/projection/event_bus.rs",
    "completion-governance": "scripts/check_pr_ai_completion.sh",
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-AI completion gate failed: {message}")


def source(relative: str) -> str:
    return (root / relative).read_text(encoding="utf-8")


def require_markers(relative: str, markers: tuple[str, ...]) -> str:
    text = source(relative)
    for marker in markers:
        if marker not in text:
            fail(f"{relative} marker missing: {marker}")
    return text


def require_non_skipping_test(relative: str, name: str) -> None:
    text = source(relative)
    match = re.search(
        rf"(?P<attrs>(?:\s*#\[[^\n]+\]\s*)+)(?:async\s+)?fn\s+{re.escape(name)}\b",
        text,
    )
    if match is None or "test" not in match.group("attrs"):
        fail(f"runnable test missing: {relative}::{name}")
    attrs = match.group("attrs")
    if any(marker in attrs for marker in ("ignore", "should_panic", "cfg(")):
        fail(f"evidence test is skippable: {relative}::{name}")


doc = source("docs/PRODUCT_FULL_AUDIT_REFINEMENT.md")
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be complete with no local remainder")
if row.count(verify_anchor) != 1:
    fail("roadmap verification must name the destructive gate once")

queue_start = doc.find("### 🟡 6.5")
if queue_start < 0:
    fail("local execution queue is missing")
queue = doc[queue_start:]
if re.search(r"(?m)^\d+\.\s+\*\*PR-AI\b", queue):
    fail("completed PR-AI remains in the local queue")
successor_title = "PR-AJ Trading HTTP Contract & Execution Problem Model"
successor_row = next(
    (line for line in doc.splitlines() if line.startswith(f"| `{successor_title}`")),
    None,
)
if successor_row is None:
    fail("PR-AJ successor roadmap row is missing")
successor_complete = "✅ 完成" in successor_row
successor_queued = re.search(r"(?m)^\d+\.\s+\*\*PR-AJ\b", queue) is not None
successor_is_head = re.search(r"(?m)^1\.\s+\*\*PR-AJ\b", queue) is not None
if successor_complete and successor_queued:
    fail("completed PR-AJ successor remains in the local queue")
if not successor_complete and not successor_is_head:
    fail("unfinished PR-AJ successor must be the local queue head")

history = source("docs/audit_history/PRODUCT_AUDIT_HISTORY.md")
if "## 2026-07-17 PR-AI Lifecycle Data Pipeline and Task Health Closure" not in history:
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
for artifact in set(evidence_contract.values()):
    item = coverage.get(artifact)
    if item is None or item["coverage_status"] != "exact" or not item["evidence"].startswith("line:"):
        fail(f"exact coverage missing for {artifact}")

require_markers(
    "crates/exchange/src/adapter.rs",
    (
        "pub enum PublicWsSubscribeOutcome",
        "pub fn subscribe_outcome",
        "Self::Pending => PublicWsSubscribeOutcome::Requested",
        "public_ws_snapshot_exposes_explicit_subscribe_outcome",
    ),
)
require_markers(
    "crates/api/src/lifecycle/market_data/ws_touch.rs",
    (
        "let subscribe_outcome = snapshot.subscribe_outcome();",
        "record_ws_pending(",
        "record_ws_unsupported(",
        "ticker_fallback(runtime, venue, adapter, symbols).await",
        "funding_fallback(runtime, venue, adapter, symbols).await",
    ),
)
ws = require_markers(
    "crates/api/src/lifecycle/market_data/ws_touch/outcome.rs",
    (
        "PublicWsSubscribeOutcome::Confirmed => runtime.market_data.record_runtime_success(",
        "PublicWsSubscribeOutcome::Requested => runtime.market_data.record_runtime_pending(",
        "PublicWsSubscribeOutcome::Unsupported => runtime.market_data.record_runtime_unsupported(",
        "SUBSCRIBE_REQUESTED",
    ),
)
if "PublicWsSubscribeOutcome::Requested => runtime.market_data.record_runtime_success(" in ws:
    fail("requested public WS subscription is falsely reported as fresh")

funding = require_markers(
    "crates/api/src/lifecycle/funding.rs",
    (
        "const FUNDING_DIFF_STATS_BOOTSTRAP_LIMIT: usize = 5_000;",
        "bootstrap_funding_diff_stats(&mut ctx).await",
        ".query_funding_diffs(realtime::FundingDiffQuery {",
        "ctx.diff_stats_projector.replace(rows);",
        ".apply(funding_diffs, common::time::now_ms());",
        "combine_substep_results([rate_history, diff_history, diff_stats])",
        "registry.record_result_timed(\"funding\"",
    ),
)
if ".query_funding_diff_stats(" in funding or "limit: 50_000" in funding:
    fail("funding lifecycle reintroduced a recurring full stats scan")

require_markers(
    "crates/realtime/src/history/funding_stats/projector.rs",
    (
        "pub struct FundingDiffStatsProjector",
        "pub fn replace(&mut self, rows: Vec<FundingDiffRow>)",
        "pub fn apply(",
        ".insert(row.occurred_at_ms, row);",
        "fn prune_pair_history",
        "MAX_CYCLE_BUCKET",
    ),
)
for relative, task_name in (
    ("crates/api/src/lifecycle/market_data.rs", "market_prewarm"),
    ("crates/api/src/lifecycle/funding.rs", "funding"),
    ("crates/api/src/lifecycle/snapshot.rs", "arbitrage_snapshot"),
):
    require_markers(relative, (f'record_result_timed("{task_name}"',))

require_markers(
    "shared-types/src/market.rs",
    (
        "pub struct MarketDataHealth",
        "pub source: MarketDataSourceKind",
        "pub freshness_ms: Option<i64>",
        "pub retry_after_ms: Option<u64>",
        "pub coverage: Option<MarketDataCoverage>",
        "pub problem: Option<ApiProblem>",
    ),
)
require_markers(
    "shared-types/src/arbitrage.rs",
    (
        "pub struct OpportunityStreamEvent",
        "pub snapshot_id: String",
        "pub source: String",
        "pub freshness_ms: Option<i64>",
        "pub retry_after_ms: Option<u64>",
        "pub error: Option<ApiProblem>",
    ),
)
require_markers(
    "crates/api/src/trading_service/private_ws_events/types.rs",
    (
        "Order(PrivateOrderDelta)",
        "Positions(PrivatePositionsSnapshot)",
        "Balances(Box<PrivateBalancesSnapshot>)",
    ),
)
require_markers(
    "crates/api/src/lifecycle/private_ws/apply.rs",
    (
        "private_ws_mapper::apply_events(state.trading_service(), events)",
        "for outcome in outcomes",
        "apply_outcome(state, venue, outcome).await",
        "publish_order(state, &record, outcome.order_projection_handled_by_ledger)",
        'publish(state, "private_ws_order_update", record)',
    ),
)

for relative, test_name in (
    ("crates/exchange/src/adapter.rs", "public_ws_snapshot_exposes_explicit_subscribe_outcome"),
    ("crates/api/src/lifecycle/market_data_tests/recovery.rs", "ws_touch_failure_records_market_runtime_health"),
    ("crates/api/src/lifecycle/market_data_tests.rs", "unsupported_ws_uses_rest_fallback_without_false_ws_success"),
    ("crates/realtime/src/history/funding_stats/tests.rs", "projector_merges_only_new_rows_and_replaces_duplicate_timestamps"),
    ("crates/realtime/src/history/funding_stats/tests.rs", "projector_retains_only_the_longest_funding_window"),
    ("crates/api/src/lifecycle/private_ws/tests/projection/event_bus.rs", "private_ws_event_bus_projects_order_position_and_balance_batch"),
    ("crates/api/src/lifecycle/private_ws/tests/projection.rs", "private_fill_chain_projects_execution_slippage_once"),
    ("crates/api/src/services/market_data/envelope/tests/funding.rs", "funding_rates_envelope_carries_row_evidence"),
    ("crates/api/src/services/opportunity/tests/stale.rs", "stale_stream_event_retains_snapshot_ids_and_reports_problem"),
    ("crates/api/src/task_registry.rs", "task_snapshots_include_healthy_and_unhealthy_rows"),
):
    require_non_skipping_test(relative, test_name)

repo_gate = source("scripts/verify_repo_gates.sh")
if repo_gate.count("check_pr_ai_completion.sh") != 2:
    fail("repo gate must execute PR-AI exactly once in docs and all scopes")

print(f"OK PR-AI static contract ({len(evidence_contract)} evidence types)")
PY

if [[ "${PR_AI_SKIP_UPSTREAM:-0}" != "1" ]]; then
  PR_DR_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dr_completion.sh"
  PR_DS_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_ds_completion.sh"
  PR_EC_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_ec_completion.sh"
  PR_EG_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_eg_completion.sh"
  bash "$ROOT/scripts/check_product_audit_progress.sh"
  bash "$ROOT/scripts/check_product_audit_evidence.sh"
  bash "$ROOT/scripts/check_product_audit_evidence_index.sh"
  bash "$ROOT/scripts/check_product_audit_coverage.sh"
fi

if [[ "${PR_AI_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p exchange \
    public_ws_snapshot_exposes_explicit_subscribe_outcome --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p realtime \
    history::funding_stats::tests --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api \
    ws_touch_failure_records_market_runtime_health --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api \
    unsupported_ws_uses_rest_fallback_without_false_ws_success --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api \
    private_ws_event_bus_projects_order_position_and_balance_batch --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api \
    private_fill_chain_projects_execution_slippage_once --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api \
    funding_rates_envelope_carries_row_evidence --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api \
    stale_stream_event_retains_snapshot_ids_and_reports_problem --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api \
    task_snapshots_include_healthy_and_unhealthy_rows --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo check -p api
fi

printf 'PR-AI completion gate passed\n'
