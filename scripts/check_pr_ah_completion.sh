#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
AUDIT="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
EVIDENCE="$ROOT/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
STATE="$ROOT/crates/api/src/state.rs"
TASK_REGISTRY="$ROOT/crates/api/src/task_registry.rs"
DRAIN="$ROOT/crates/api/src/lifecycle/drain.rs"
INDEX="$ROOT/crates/api/src/services/opportunity_index.rs"
REPO_GATE="$ROOT/scripts/verify_repo_gates.sh"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-AH completion gate failed: %s\n' "$1" >&2
  exit 1
}

run_static_gate() {
  PR_AH_SKIP_TESTS=1 PR_AH_SKIP_UPSTREAM=1 bash "$0" >/dev/null
}

expect_rejected() {
  local message="$1"
  if run_static_gate 2>/dev/null; then
    fail "$message"
  fi
}

if [[ "$MODE" == "--self-test" ]]; then
  audit_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-ah-audit.XXXXXX")"
  evidence_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-ah-evidence.XXXXXX")"
  state_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-ah-state.XXXXXX")"
  task_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-ah-task.XXXXXX")"
  drain_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-ah-drain.XXXXXX")"
  index_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-ah-index.XXXXXX")"
  repo_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-ah-repo.XXXXXX")"
  cp "$AUDIT" "$audit_backup"
  cp "$EVIDENCE" "$evidence_backup"
  cp "$STATE" "$state_backup"
  cp "$TASK_REGISTRY" "$task_backup"
  cp "$DRAIN" "$drain_backup"
  cp "$INDEX" "$index_backup"
  cp "$REPO_GATE" "$repo_backup"
  restore() {
    cp "$audit_backup" "$AUDIT"
    cp "$evidence_backup" "$EVIDENCE"
    cp "$state_backup" "$STATE"
    cp "$task_backup" "$TASK_REGISTRY"
    cp "$drain_backup" "$DRAIN"
    cp "$index_backup" "$INDEX"
    cp "$repo_backup" "$REPO_GATE"
    rm -f "$audit_backup" "$evidence_backup" "$state_backup" "$task_backup" \
      "$drain_backup" "$index_backup" "$repo_backup"
  }
  trap restore EXIT

  run_static_gate

  python3 - "$AUDIT" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "| `PR-AH API Gateway & Runtime State Boundary` | ✅ 完成 |"
if source.count(marker) != 1:
    raise SystemExit("PR-AH self-test setup failed: completed row drifted")
path.write_text(source.replace(marker, "| `PR-AH API Gateway & Runtime State Boundary` | 🟡 部分完成 |", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted a downgraded roadmap row"
  cp "$audit_backup" "$AUDIT"

  python3 - "$EVIDENCE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
lines = path.read_text(encoding="utf-8").splitlines()
for index, line in enumerate(lines):
    if line.startswith("PR-AH\truntime-state-domains\t"):
        del lines[index]
        break
else:
    raise SystemExit("PR-AH self-test setup failed: state evidence missing")
path.write_text("\n".join(lines) + "\n", encoding="utf-8")
PY
  expect_rejected "self-test accepted incomplete evidence"
  cp "$evidence_backup" "$EVIDENCE"

  python3 - "$STATE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "    market: MarketRuntimeState,"
if source.count(marker) != 1:
    raise SystemExit("PR-AH self-test setup failed: state boundary drifted")
path.write_text(source.replace(marker, "    aggregator: Arc<Aggregator>,", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted a flattened AppState boundary"
  cp "$state_backup" "$STATE"

  python3 - "$TASK_REGISTRY" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "    pub enabled: bool,"
if source.count(marker) != 1:
    raise SystemExit("PR-AH self-test setup failed: task enabled field drifted")
path.write_text(source.replace(marker, "    // enabled field removed", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted an incomplete task health contract"
  cp "$task_backup" "$TASK_REGISTRY"

  python3 - "$DRAIN" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "const SHUTDOWN_DRAIN_BUDGET: Duration = Duration::from_secs(5);"
if source.count(marker) != 1:
    raise SystemExit("PR-AH self-test setup failed: drain budget drifted")
path.write_text(source.replace(marker, "const SHUTDOWN_DRAIN_BUDGET: Duration = Duration::MAX;", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted an unbounded shutdown drain"
  cp "$drain_backup" "$DRAIN"

  python3 - "$INDEX" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "    current: ArcSwap<OpportunityIndexSnapshot>,"
if source.count(marker) != 1:
    raise SystemExit("PR-AH self-test setup failed: ArcSwap index drifted")
path.write_text(source.replace(marker, "    current: OpportunityIndexSnapshot,", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted a non-atomic opportunity index"
  cp "$index_backup" "$INDEX"

  python3 - "$AUDIT" <<'PY'
from pathlib import Path
import re
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "### 🟡 6.5 下一步执行队列"
if source.count(marker) != 1:
    raise SystemExit("PR-AH self-test setup failed: execution queue drifted")
injected = f"{marker}\n\n1. **PR-AH self-test reinsertion**"
path.write_text(source.replace(marker, injected, 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted PR-AH reinserted into the local queue"
  cp "$audit_backup" "$AUDIT"

  python3 - "$REPO_GATE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = 'PR_AH_SKIP_TESTS=1 PR_AH_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_ah_completion.sh"'
if source.count(marker) != 2:
    raise SystemExit("PR-AH self-test setup failed: repo wiring drifted")
path.write_text(source.replace(marker, "true # PR-AH gate removed", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted single-scope repository wiring"

  printf 'PR-AH completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from __future__ import annotations

import csv
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])
pr_id = "PR-AH"
title = "PR-AH API Gateway & Runtime State Boundary"
verify_anchor = "`bash scripts/check_pr_ah_completion.sh --self-test`"
evidence_contract = {
    "gateway-route-surface": "shared-types/src/route_surface.rs",
    "runtime-state-domains": "crates/api/src/state.rs",
    "lifecycle-state-boundary": "crates/api/src/lifecycle/snapshot.rs",
    "task-health-contract": "crates/api/src/task_registry.rs",
    "supervisor-retry-health": "crates/api/src/lifecycle/tasks.rs",
    "disabled-task-registration": "crates/api/src/lifecycle/ledger_projection.rs",
    "runtime-health-task-projection": "crates/api/src/services/venue_operation_health/snapshot/part_04.rs",
    "task-health-prometheus": "crates/api/src/routers/metrics.rs",
    "bounded-shutdown-drain": "crates/api/src/lifecycle/drain.rs",
    "opportunity-index-snapshot-swap": "crates/api/src/services/opportunity_index.rs",
    "task-health-tests": "crates/api/src/services/venue_operation_health/snapshot/tests/part_16.rs",
    "shutdown-drain-tests": "crates/api/src/lifecycle/drain/tests.rs",
    "completion-governance": "scripts/check_pr_ah_completion.sh",
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-AH completion gate failed: {message}")


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
if re.search(r"(?m)^\d+\.\s+\*\*PR-AH\b", queue):
    fail("completed PR-AH remains in the local queue")
successor_title = "PR-AI Lifecycle Data Pipeline & Task Health"
successor_row = next(
    (line for line in doc.splitlines() if line.startswith(f"| `{successor_title}`")),
    None,
)
if successor_row is None:
    fail("PR-AI successor roadmap row is missing")
successor_complete = "✅ 完成" in successor_row
successor_queued = re.search(r"(?m)^\d+\.\s+\*\*PR-AI\b", queue) is not None
successor_is_head = re.search(r"(?m)^1\.\s+\*\*PR-AI\b", queue) is not None
if successor_complete and successor_queued:
    fail("completed PR-AI successor remains in the local queue")
if not successor_complete and not successor_is_head:
    fail("unfinished PR-AI successor must be the local queue head")

history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(encoding="utf-8")
if "## 2026-07-17 PR-AH API Gateway & Runtime State Boundary Closure" not in history:
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
    item = coverage.get(artifact)
    if item is None or item["coverage_status"] != "exact" or not item["evidence"].startswith("line:"):
        fail(f"exact coverage missing for {artifact}")

state = (root / "crates/api/src/state.rs").read_text(encoding="utf-8")
for marker in (
    "market: MarketRuntimeState,",
    "trading: TradingRuntimeState,",
    "product: ProductRuntimeState,",
    "diagnostics: DiagnosticsRuntimeState,",
    "storage: StorageRuntimeState,",
    "struct MarketRuntimeState",
    "struct TradingRuntimeState",
    "struct ProductRuntimeState",
    "struct DiagnosticsRuntimeState",
    "struct StorageRuntimeState",
    "execution_run_hot_cache",
    "missed_opportunity_hot_cache",
):
    if marker not in state:
        fail(f"runtime state boundary marker missing: {marker}")
if "pub inner: Arc<AppStateInner>" in state:
    fail("AppState inner boundary is publicly bypassable")
for path in (root / "crates/api/src/lifecycle").rglob("*.rs"):
    if "state.inner." in path.read_text(encoding="utf-8"):
        fail(f"lifecycle bypasses AppState boundary: {path.relative_to(root)}")

task = (root / "crates/api/src/task_registry.rs").read_text(encoding="utf-8")
for marker in (
    "pub enabled: bool,",
    "pub lag_ms: i64,",
    "pub retry_after_ms: Option<u64>,",
    "pub(crate) fn register_disabled",
    "pub(crate) fn mark_retry_scheduled",
    "disabled_task_is_explicit_without_becoming_unhealthy",
    "scheduled_restart_exposes_retry_after_and_lag",
):
    if marker not in task:
        fail(f"task health contract marker missing: {marker}")

supervisor_paths = [root / "crates/api/src/lifecycle/tasks.rs"]
supervisor_paths.extend(sorted((root / "crates/api/src/lifecycle/tasks").rglob("*.rs")))
supervisor = "\n".join(path.read_text(encoding="utf-8") for path in supervisor_paths)
for marker in (
    "registry.mark_retry_scheduled",
    "tokio::time::timeout(drain_budget",
    "self.watchers.shutdown().await",
):
    if marker not in supervisor:
        fail(f"supervisor marker missing: {marker}")

health = (root / "crates/api/src/services/venue_operation_health/snapshot/part_04.rs").read_text(encoding="utf-8")
for marker in (
    "configured: Some(snapshot.enabled)",
    "retry_after_ms: snapshot.retry_after_ms",
    "then_some(snapshot.lag_ms.max(0))",
):
    if marker not in health:
        fail(f"runtime health task projection marker missing: {marker}")

metrics = (root / "crates/api/src/routers/metrics.rs").read_text(encoding="utf-8")
for marker in (
    "crypto_arb_background_tasks_enabled",
    "crypto_arb_background_tasks_disabled",
    "crypto_arb_background_task_lag_ms",
    "crypto_arb_background_task_retry_after_ms",
):
    if marker not in metrics:
        fail(f"task metric marker missing: {marker}")

drain = (root / "crates/api/src/lifecycle/drain.rs").read_text(encoding="utf-8")
for marker in (
    "const SHUTDOWN_DRAIN_BUDGET: Duration = Duration::from_secs(5);",
    "drain_sql_journal(state, sql_configured).await",
    "drain_audit_writer(audit_configured).await",
    "pub(crate) fn ensure_clean",
):
    if marker not in drain:
        fail(f"bounded drain marker missing: {marker}")

index = (root / "crates/api/src/services/opportunity_index.rs").read_text(encoding="utf-8")
for marker in (
    "current: ArcSwap<OpportunityIndexSnapshot>",
    "self.current.store(Arc::new(OpportunityIndexSnapshot",
    "publish_replaces_the_whole_generation_atomically",
    "stale_list_snapshot_fails_closed",
):
    if marker not in index:
        fail(f"opportunity snapshot swap marker missing: {marker}")

test_source = (root / "crates/api/src/services/venue_operation_health/snapshot/tests/part_16.rs").read_text(encoding="utf-8")
if "task_registry_rows_expose_enabled_lag_and_retry_contract" not in test_source or "#[ignore]" in test_source:
    fail("non-skipping task health projection test is missing")
drain_tests = (root / "crates/api/src/lifecycle/drain/tests.rs").read_text(encoding="utf-8")
for title in (
    "clean_report_accepts_drained_and_unbuffered_stores",
    "failed_report_names_every_unclean_component",
    "failed_producer_marks_synchronous_writes_unconfirmed",
):
    if title not in drain_tests:
        fail(f"shutdown drain test missing: {title}")

repo_gate = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
if repo_gate.count("check_pr_ah_completion.sh") != 2:
    fail("repo gate must execute PR-AH exactly once in docs and all scopes")

print(f"OK PR-AH static contract ({len(evidence_contract)} evidence types)")
PY

if [[ "${PR_AH_SKIP_UPSTREAM:-0}" != "1" ]]; then
  bash "$ROOT/scripts/check_product_audit_progress.sh"
  bash "$ROOT/scripts/check_product_audit_evidence.sh"
  bash "$ROOT/scripts/check_product_audit_evidence_index.sh"
  bash "$ROOT/scripts/check_product_audit_coverage.sh"
fi

if [[ "${PR_AH_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api task_registry --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api lifecycle::tasks::tests --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api lifecycle::drain::tests --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api opportunity_index --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api \
    unconfigured_projection_worker_is_reported_as_disabled --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api \
    render_metrics_body_includes_snapshot_and_ws_channels --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo check -p api
fi

printf 'PR-AH completion gate passed\n'
