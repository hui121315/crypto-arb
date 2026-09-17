#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
PROFILE="$ROOT/Cargo.toml"
FUNDING="$ROOT/crates/api/src/lifecycle/funding.rs"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

if [[ "$MODE" == "--self-test" ]]; then
  PR_DR_SKIP_TESTS=1 bash "$0" >/dev/null
  profile_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-dr-profile.XXXXXX")"
  funding_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-dr-funding.XXXXXX")"
  cp "$PROFILE" "$profile_backup"
  cp "$FUNDING" "$funding_backup"
  restore() {
    cp "$profile_backup" "$PROFILE"
    cp "$funding_backup" "$FUNDING"
    rm -f "$profile_backup" "$funding_backup"
  }
  trap restore EXIT

  python3 - "$PROFILE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = 'panic = "unwind"'
if source.count(marker) != 1:
    raise SystemExit("PR-DR self-test setup failed: release panic marker drifted")
path.write_text(source.replace(marker, 'panic = "abort"', 1), encoding="utf-8")
PY
  if PR_DR_SKIP_TESTS=1 bash "$0" >/dev/null 2>&1; then
    printf 'PR-DR completion self-test failed: release panic abort regression passed\n' >&2
    exit 1
  fi

  cp "$profile_backup" "$PROFILE"
  python3 - "$FUNDING" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "combine_substep_results([rate_history, diff_history, diff_stats])"
if source.count(marker) != 1:
    raise SystemExit("PR-DR self-test setup failed: funding substep marker drifted")
path.write_text(source.replace(marker, "Ok(())", 1), encoding="utf-8")
PY
  if PR_DR_SKIP_TESTS=1 bash "$0" >/dev/null 2>&1; then
    printf 'PR-DR completion self-test failed: swallowed funding substeps passed\n' >&2
    exit 1
  fi

  printf 'PR-DR completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from __future__ import annotations

import csv
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])
title = "PR-DR Lifecycle Task Health & Shutdown Contract"
evidence_contract = {
    "bounded-restart-supervisor": "crates/api/src/lifecycle/tasks.rs",
    "release-panic-recovery": "Cargo.toml",
    "restart-runtime-registry": "crates/api/src/task_registry.rs",
    "restart-operation-health": "crates/api/src/services/venue_operation_health/snapshot/part_07.rs",
    "restart-prometheus-metrics": "crates/api/src/routers/metrics.rs",
    "restartable-lifecycle-factories": "crates/api/src/lifecycle.rs",
    "named-shutdown-drain-report": "crates/api/src/lifecycle/drain.rs",
    "runtime-shutdown-orchestration": "crates/api/src/main.rs",
    "funding-substep-problems": "crates/api/src/lifecycle/funding.rs",
    "watchlist-cap-problem": "crates/api/src/lifecycle/market_data/watchlist_runtime.rs",
    "completion-governance": "scripts/check_pr_dr_completion.sh",
}
supervised_files = (
    "crates/api/src/lifecycle/funding_payments.rs",
    "crates/api/src/lifecycle/instruments.rs",
    "crates/api/src/lifecycle/ledger_projection.rs",
    "crates/api/src/lifecycle/market_data.rs",
    "crates/api/src/lifecycle/portfolio.rs",
    "crates/api/src/lifecycle/private_ws.rs",
    "crates/api/src/lifecycle/reconciliation.rs",
    "crates/api/src/lifecycle/snapshot.rs",
    "crates/api/src/lifecycle/system.rs",
    "crates/api/src/lifecycle/funding.rs",
    "crates/api/src/lifecycle/ws_housekeeping.rs",
)
closure_paths = (
    "crates/api/src/lifecycle.rs",
    "crates/api/src/lifecycle/drain.rs",
    "crates/api/src/lifecycle/drain/tests.rs",
    "crates/api/src/lifecycle/tasks.rs",
    "crates/api/src/lifecycle/tasks/tests.rs",
    *supervised_files,
    "crates/api/src/lifecycle/market_data/watchlist_runtime.rs",
    "crates/api/src/lifecycle/market_data_tests/watchlist_runtime.rs",
    "crates/api/src/main.rs",
    "crates/api/src/task_registry.rs",
    "crates/api/src/routers/metrics.rs",
    "crates/api/src/services/venue_operation_health/snapshot/part_07.rs",
    "crates/api/src/services/venue_operation_health/snapshot/tests/part_05.rs",
    "scripts/check_pr_dr_completion.sh",
    "scripts/check_pr_fi_completion.sh",
    "scripts/check_pr_ga_completion.sh",
    "scripts/verify_repo_gates.sh",
)
test_anchors = (
    ("crates/api/src/lifecycle/tasks/tests.rs", "panicked_task_restarts_within_budget"),
    ("crates/api/src/lifecycle/tasks/tests.rs", "factory_panic_restarts_within_budget"),
    ("crates/api/src/lifecycle/tasks/tests.rs", "unexpected_returns_exhaust_restart_budget"),
    ("crates/api/src/lifecycle/tasks/tests.rs", "cooperative_shutdown_does_not_restart_task"),
    ("crates/api/src/lifecycle/tasks/tests.rs", "stubborn_task_hits_abort_fallback"),
    ("crates/api/src/task_registry.rs", "restart_keeps_exit_count_increments_restart_count_and_clears_dead"),
    ("crates/api/src/services/venue_operation_health/snapshot/tests/part_05.rs", "task_registry_rows_report_restart_count"),
    ("crates/api/src/routers/metrics.rs", "render_metrics_body_includes_snapshot_and_ws_channels"),
    ("crates/api/src/lifecycle/drain/tests.rs", "clean_report_accepts_drained_and_unbuffered_stores"),
    ("crates/api/src/lifecycle/drain/tests.rs", "failed_report_names_every_unclean_component"),
    ("crates/api/src/lifecycle/drain/tests.rs", "failed_producer_marks_synchronous_writes_unconfirmed"),
    ("crates/api/src/lifecycle/funding.rs", "funding_cycle_outcome_preserves_each_failed_substep"),
    ("crates/api/src/lifecycle/funding.rs", "funding_cycle_outcome_rejects_empty_rows_even_when_sinks_succeed"),
    ("crates/api/src/lifecycle/market_data_tests/watchlist_runtime.rs", "watchlist_runtime_plan_is_bounded_visible_and_private_ws_free"),
)


def fail(message: str) -> None:
    raise SystemExit(f"PR-DR completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be completed with no local remainder")
if "scripts/check_pr_dr_completion.sh --self-test" not in row:
    fail("roadmap verification must name the destructive completion gate")
queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-DR\b", queue):
    fail("completed PR-DR remains in the local queue")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-DR"]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for kind, artifact in evidence_contract.items():
    row = indexed[kind]
    if row["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"{kind} artifact drifted or is missing")
    if not row["command"].strip():
        fail(f"{kind} lacks a verification command")

profile = (root / "Cargo.toml").read_text(encoding="utf-8")
if not re.search(r"(?ms)^\[profile\.release\].*?^panic\s*=\s*\"unwind\"\s*$", profile):
    fail("release profile must preserve unwind for task panic recovery")

markers = {
    "crates/api/src/lifecycle/tasks.rs": (
        "DEFAULT_RESTART_MAX_ATTEMPTS: usize = 3",
        "std::panic::catch_unwind",
        ".catch_unwind()",
        "restart budget exhausted",
        "registry.record_restart(name)",
    ),
    "crates/api/src/task_registry.rs": (
        "restart_count: u32",
        "fn record_restart",
        "entry.restart_count = entry.restart_count.saturating_add(1)",
    ),
    "crates/api/src/routers/metrics.rs": (
        "crypto_arb_background_task_restart_total",
        "snapshot.restart_count",
    ),
    "crates/api/src/services/venue_operation_health/snapshot/part_07.rs": (
        'format!("restart_count={}", snapshot.restart_count)',
        'format!("，restart {}", snapshot.restart_count)',
    ),
    "crates/api/src/lifecycle/drain.rs": (
        '"portfolio_nav"',
        '"history_store"',
        '"trading_sql_journal"',
        '"audit_log_jsonl"',
        "drain_sql_ledger_and_shutdown()",
        "crate::middleware::audit::shutdown",
        "NoPendingBuffer",
        "in-flight synchronous write is unconfirmed",
    ),
    "crates/api/src/main.rs": (
        "lifecycle::drain_runtime(&state, &mut tasks)",
        ".ensure_clean()?",
    ),
    "crates/api/src/lifecycle/funding.rs": (
        "FUNDING_HISTORY_APPEND_FAILED",
        "FUNDING_DIFF_HISTORY_APPEND_FAILED",
        "FUNDING_DIFF_STATS_REFRESH_FAILED",
        "FUNDING_STREAM_SERIALIZE_FAILED",
        "combine_substep_results([rate_history, diff_history, diff_stats])",
    ),
    "crates/api/src/lifecycle/portfolio.rs": (
        "state.portfolio_snapshot().value_now()",
        "stale_snapshot_or_problem(last_snapshot.as_ref(), &error)",
    ),
    "crates/api/src/lifecycle/market_data/watchlist_runtime.rs": (
        "WATCHLIST_PREWARM_CAPPED",
        "WatchlistPrewarmStatus::Capped",
        ".with_source(\"market_prewarm\")",
    ),
}
for relative_path, required in markers.items():
    source = (root / relative_path).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"missing {relative_path} marker: {marker}")

for relative_path in supervised_files:
    source = (root / relative_path).read_text(encoding="utf-8")
    if "tasks.supervise(" not in source or "move ||" not in source:
        fail(f"lifecycle task is not backed by a restartable future factory: {relative_path}")

for relative_path, function_name in test_anchors:
    source = (root / relative_path).read_text(encoding="utf-8")
    pattern = re.compile(
        rf"(?m)((?:^\s*#\[[^\n]+\]\s*\n)+)"
        rf"\s*(?:async\s+)?fn\s+{re.escape(function_name)}\s*\("
    )
    match = pattern.search(source)
    if match is None or not re.search(r"#\[(?:tokio::)?test", match.group(1)):
        fail(f"missing runnable test anchor {relative_path}::{function_name}")
    if re.search(r"ignore|should_panic", match.group(1)):
        fail(f"test anchor is skipped: {relative_path}::{function_name}")

history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(encoding="utf-8")
if "## 2026-07-14 PR-DR Lifecycle Restart and Shutdown Closure" not in history:
    fail("PR-DR history appendix is missing")
for artifact in set(evidence_contract.values()):
    if artifact not in history:
        fail(f"PR-DR history appendix lacks closure path: {artifact}")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage_rows = list(csv.DictReader(handle, delimiter="\t"))
coverage = {row["file"]: row for row in coverage_rows}
if len(coverage) != len(coverage_rows):
    fail("coverage ledger contains duplicate paths")
for relative_path in closure_paths:
    if not (root / relative_path).is_file():
        fail(f"missing closure path: {relative_path}")
    if coverage.get(relative_path, {}).get("coverage_status") != "exact":
        fail(f"exact coverage missing for {relative_path}")

print(
    f"OK PR-DR contract ({len(evidence_contract)} evidence types; "
    f"{len(test_anchors)} non-skipping tests; {len(closure_paths)} exact paths)"
)
PY

bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"

if [[ "${PR_DR_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test \
    --manifest-path "$ROOT/Cargo.toml" -p api --bin crypto-arb-api \
    lifecycle::tasks --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test \
    --manifest-path "$ROOT/Cargo.toml" -p api --bin crypto-arb-api \
    lifecycle::drain --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test \
    --manifest-path "$ROOT/Cargo.toml" -p api --bin crypto-arb-api \
    lifecycle::funding::tests --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test \
    --manifest-path "$ROOT/Cargo.toml" -p api --bin crypto-arb-api \
    task_registry --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test \
    --manifest-path "$ROOT/Cargo.toml" -p api --bin crypto-arb-api \
    render_metrics_body_includes_snapshot_and_ws_channels --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test \
    --manifest-path "$ROOT/Cargo.toml" -p api --bin crypto-arb-api \
    watchlist_runtime_plan_is_bounded_visible_and_private_ws_free --no-fail-fast
fi

printf 'OK PR-DR lifecycle task health and shutdown contract\n'
