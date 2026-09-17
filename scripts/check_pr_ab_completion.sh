#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

python3 - "$ROOT" "$MODE" <<'PY'
from pathlib import Path
import csv
import re
import shutil
import sys
import tempfile

root = Path(sys.argv[1])
mode = sys.argv[2]

PR_TITLE = "PR-AB Trading Ledger Persistence"
VERIFY_ANCHOR = "`bash scripts/check_pr_ab_completion.sh --self-test`"
HISTORY_HEADING = "## 2026-07-16 PR-AB Trading Ledger Persistence Closure"

EVIDENCE = {
    "successor-pr-dm": ("scripts/check_pr_dm_completion.sh", "check_pr_dm_completion.sh"),
    "successor-pr-ci": ("scripts/check_pr_ci_completion.sh", "check_pr_ci_completion.sh"),
    "successor-pr-dh": ("scripts/check_pr_dh_completion.sh", "check_pr_dh_completion.sh"),
    "successor-pr-ea": ("scripts/check_pr_ea_completion.sh", "check_pr_ea_completion.sh"),
    "successor-pr-bw": ("scripts/check_pr_bw_completion.sh", "check_pr_bw_completion.sh"),
    "migration-registry": ("crates/trading/src/sql_ledger/migrations.rs", "registry_order_paths_versions_and_checksums_are_frozen"),
    "normalized-ledger-schema": ("crates/trading/migrations/20260601_orders.sql", "verify_postgres_ledger_contract.sh"),
    "run-cost-schema": ("crates/trading/migrations/20260710_run_cost_integrity.sql", "integrity_migration_contains_required_tables_keys_and_indexes"),
    "run-cost-source-links": ("crates/trading/migrations/20260710_run_cost_sources.sql", "run_cost_sources_migration_normalizes_sources_and_rebuild_receipts"),
    "transactional-writer": ("crates/trading/src/sql_ledger/writer.rs", "sql_ledger::writer"),
    "durable-projection-jobs": ("crates/trading/src/sql_ledger/projection_jobs.rs", "verify_postgres_ledger_contract.sh"),
    "normalized-run-costs": ("crates/trading/src/sql_ledger/run_cost.rs", "non_usd_funding_keeps_native_amount_without_fabricated_usd"),
    "bounded-run-cost-rebuild": ("crates/trading/src/sql_ledger/run_cost_rebuild.rs", "verify_postgres_ledger_contract.sh"),
    "execution-run-jsonl": ("crates/api/src/services/execution_run_store.rs", "execution_run_store::tests"),
    "execution-run-replay-fixture": ("crates/api/src/services/execution_run_store/tests.rs", "durable_run_snapshot_replays_embedded_timeline"),
    "close-run-jsonl": ("crates/api/src/services/close_run_store.rs", "close_run_store::tests"),
    "close-run-durable-ledger": ("crates/api/src/services/close_run_store/ledger.rs", "close_run_store::tests"),
    "close-run-replay-fixture": ("crates/api/src/services/close_run_store/tests.rs", "replays_projected_snapshot_and_receipt_after_bare_row"),
    "app-restart-seed": ("crates/api/src/state.rs", "execution_runs_from_replay"),
    "runtime-state-health": ("crates/api/src/services/runtime_state.rs", "runtime_state::tests"),
    "system-health-query": ("crates/api/src/services/system_health.rs", "runtime_state::inventory"),
    "review-sql-consumer": ("crates/api/src/services/review/ledger.rs", "executed_envelope_uses_sql_replayed_events_and_order_snapshots"),
    "portfolio-sql-consumer": ("crates/api/src/services/portfolio_pnl.rs", "sql_realized_window_close_run_costs_reduce_today_and_history_pnl"),
    "postgres-restart-gate": ("scripts/verify_postgres_ledger_contract.sh", "verify_postgres_ledger_contract.sh"),
    "execution-product-recovery": ("test/e2e/pr_ea_execution_finality.spec.ts", "test:e2e:pr-ea"),
    "review-product-evidence": ("test/e2e/pr_dh_review_ledger.spec.ts", "test:e2e:pr-dh"),
    "portfolio-product-evidence": ("test/e2e/pr_dz_portfolio_truth.spec.ts", "test:e2e:pr-dz"),
    "completion-governance": ("scripts/check_pr_ab_completion.sh", "check_pr_ab_completion.sh --self-test"),
}

RUNNABLE = (
    ("crates/trading/src/sql_ledger/migrations/tests.rs", "registry_order_paths_versions_and_checksums_are_frozen"),
    ("crates/trading/src/sql_ledger/migrations/tests.rs", "integrity_migration_contains_required_tables_keys_and_indexes"),
    ("crates/trading/src/sql_ledger/migrations/tests.rs", "run_cost_sources_migration_normalizes_sources_and_rebuild_receipts"),
    ("crates/api/src/services/execution_run_store/tests.rs", "replays_old_bare_snapshots"),
    ("crates/api/src/services/execution_run_store/tests.rs", "replays_projected_snapshot_and_receipt_after_bare_row"),
    ("crates/api/src/services/execution_run_store/tests.rs", "durable_run_snapshot_replays_embedded_timeline"),
    ("crates/api/src/services/execution_run_store/tests.rs", "load_retains_replay_failure_health"),
    ("crates/api/src/services/close_run_store/tests.rs", "replays_old_bare_snapshots"),
    ("crates/api/src/services/close_run_store/tests.rs", "replays_projected_snapshot_and_receipt_after_bare_row"),
    ("crates/api/src/services/close_run_store/tests.rs", "load_repairs_malformed_rows_and_preserves_backup"),
    ("crates/api/src/services/close_run_store/tests.rs", "concurrent_bare_appends_replay_without_interleaving"),
    ("crates/api/src/services/runtime_state/tests.rs", "unconfigured_close_runs_are_reported_as_volatile_runtime_state"),
    ("crates/api/src/services/runtime_state/tests.rs", "configured_close_runs_are_reported_as_durable_runtime_state"),
    ("crates/api/src/services/runtime_state/tests.rs", "degraded_execution_run_store_becomes_runtime_problem"),
    ("crates/api/src/services/review/tests/sql_replay.rs", "executed_envelope_uses_sql_replayed_events_and_order_snapshots"),
    ("crates/api/src/services/portfolio_pnl/tests.rs", "sql_realized_window_order_snapshots_drive_pnl"),
    ("crates/api/src/services/portfolio_pnl/tests.rs", "sql_realized_window_close_run_costs_reduce_today_and_history_pnl"),
)

BROWSERS = {
    "test/e2e/pr_ea_execution_finality.spec.ts": "PR-EA renders ticket evidence and replayable finality timeline",
    "test/e2e/pr_dh_review_ledger.spec.ts": "PR-DH review ledger keeps explicit PnL quality and snapshot-bound 50-row budget",
    "test/e2e/pr_dz_portfolio_truth.spec.ts": "PR-DZ portfolio envelope keeps wallet NAV, position evidence, partial state, and durable compensation visible",
}

SUCCESSORS = (
    "PR-DM Storage Health & Ledger Persistence Contract",
    "PR-CI Storage Health & Migration Authority Contract",
    "PR-DH Review Ledger Truth Source & Estimated PnL Contract",
    "PR-EA ExecutionRun Finality & ActionState Contract",
    "PR-BW ExecutionRun Finality & ActionState Snapshot",
)


def fail(message: str) -> None:
    raise ValueError(message)


def rows(path: Path) -> list[dict[str, str]]:
    with path.open(encoding="utf-8", newline="") as handle:
        return list(csv.DictReader(handle, delimiter="\t"))


def require_markers(relative: str, *markers: str) -> str:
    source = (root / relative).read_text(encoding="utf-8")
    for marker in markers:
        if marker not in source:
            fail(f"missing {relative} marker: {marker}")
    return source


def require_runnable(relative: str, test_name: str) -> None:
    source = (root / relative).read_text(encoding="utf-8")
    pattern = re.compile(
        rf"(?ms)(?P<attrs>(?:^\s*#\[[^\n]+\]\s*\n)+)\s*(?:async\s+)?fn\s+{re.escape(test_name)}\s*\("
    )
    match = pattern.search(source)
    if match is None:
        fail(f"runnable test is missing: {relative}:{test_name}")
    attrs = match.group("attrs")
    if "#[test]" not in attrs and "#[tokio::test" not in attrs:
        fail(f"test attribute is missing: {relative}:{test_name}")
    if re.search(r"#\[(?:ignore|should_panic)|#\[cfg(?:_attr)?\(", attrs):
        fail(f"runnable test is skipped or cfg-disabled: {relative}:{test_name}")


def require_incomplete_queue_head(doc: str, queue: str) -> None:
    matched = re.search(r"(?m)^1\.\s+\*\*(PR-[A-Z]+)\b", queue)
    if not matched:
        fail("local queue must retain a PR roadmap item at its head")
    head = matched.group(1)
    row = next((line for line in doc.splitlines() if line.startswith(f"| `{head} ")), None)
    if row is None or "✅ 完成" in row:
        fail(f"local queue head must be an incomplete roadmap row: {head}")


def check(current_root: Path) -> None:
    global root
    root = current_root
    doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
    row = next((line for line in doc.splitlines() if line.startswith(f"| `{PR_TITLE}`")), None)
    if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
        fail("roadmap row must be completed with no local remainder")
    if row.count(VERIFY_ANCHOR) != 1:
        fail("roadmap row must name the destructive completion gate")
    queue = doc[doc.index("### 🟡 6.5"):]
    if re.search(r"(?m)^\d+\.\s+\*\*PR-AB\b", queue):
        fail("completed PR-AB remains in the local queue")
    require_incomplete_queue_head(doc, queue)
    for title in SUCCESSORS:
        successor = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
        if successor is None or "✅ 完成" not in successor or "剩余：无" not in successor:
            fail(f"completed successor authority drifted: {title}")

    history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(encoding="utf-8")
    if HISTORY_HEADING not in history:
        fail("PR-AB closure appendix is missing")

    selected = [item for item in rows(root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv") if item["pr_id"] == "PR-AB"]
    indexed = {item["evidence_type"]: item for item in selected}
    if len(indexed) != len(selected) or set(indexed) != set(EVIDENCE):
        fail(f"evidence type drift: expected={sorted(EVIDENCE)}, actual={sorted(indexed)}")
    for evidence_type, (artifact, anchor) in EVIDENCE.items():
        item = indexed[evidence_type]
        if item["artifact"] != artifact or anchor not in item["command"] or not item["notes"].strip():
            fail(f"evidence anchor drifted: {evidence_type}")
        if not (root / artifact).is_file():
            fail(f"evidence artifact missing: {artifact}")

    coverage = {item["file"]: item for item in rows(root / "docs/PRODUCT_AUDIT_COVERAGE.tsv")}
    for artifact, _ in EVIDENCE.values():
        item = coverage.get(artifact)
        if item is None or item["coverage_status"] != "exact" or not item["evidence"].startswith("line:"):
            fail(f"exact coverage missing for {artifact}")

    require_markers(
        "crates/trading/migrations/20260601_orders.sql",
        "CREATE TABLE IF NOT EXISTS order_events",
        "CREATE TABLE IF NOT EXISTS fills",
        "CREATE TABLE IF NOT EXISTS fees",
        "CREATE TABLE IF NOT EXISTS funding_payments",
        "CREATE TABLE IF NOT EXISTS slippage_events",
        "CREATE TABLE IF NOT EXISTS balance_events",
        "CREATE TABLE IF NOT EXISTS order_snapshots",
        "CREATE TABLE IF NOT EXISTS run_finality_events",
    )
    require_markers(
        "crates/trading/migrations/20260710_run_cost_integrity.sql",
        "CREATE TABLE IF NOT EXISTS ledger_projection_jobs",
        "CREATE TABLE IF NOT EXISTS run_cost_facts",
    )
    require_markers(
        "crates/trading/migrations/20260710_run_cost_sources.sql",
        "source_order_event_id",
        "source_run_finality_event_id",
        "CREATE TABLE IF NOT EXISTS run_cost_rebuild_receipts",
    )
    require_markers(
        "crates/trading/src/sql_ledger/migrations.rs",
        'include_str!("../../migrations/20260601_orders.sql")',
        'include_str!("../../migrations/20260710_run_cost_integrity.sql")',
        'include_str!("../../migrations/20260710_run_cost_sources.sql")',
    )
    require_markers(
        "crates/trading/src/sql_ledger/writer.rs",
        "protocol::persist_event_group",
        "claim_projection_jobs",
        "protocol::persist_run_finality",
        "complete_projection_job",
    )
    require_markers(
        "crates/trading/src/sql_ledger/projection_jobs.rs",
        'EXECUTION_RUN_PROJECTOR: &str = "execution_run_v1"',
        'CLOSE_RUN_PROJECTOR: &str = "close_run_v1"',
        'RUN_COST_PROJECTOR: &str = "run_cost_facts_v1"',
    )
    require_markers(
        "crates/trading/src/sql_ledger/run_cost_rebuild.rs",
        "order_high_water",
        "finality_high_water",
        "run_cost_rebuild_receipts",
    )
    require_markers(
        "crates/api/src/services/execution_run_store.rs",
        "file.sync_data()?",
        "replay_failures",
        "pub(crate) fn persistence_configured(&self) -> bool",
        "pub(crate) fn persistence_degraded(&self) -> bool",
    )
    require_markers(
        "crates/api/src/services/close_run_store.rs",
        "mod ledger;",
        "append_durable_jsonl(path, &envelope)",
        "replay_failures",
        "pub(crate) fn persistence_configured(&self) -> bool",
        "pub(crate) fn persistence_degraded(&self) -> bool",
    )
    require_markers(
        "crates/api/src/services/close_run_store/ledger.rs",
        "pub(super) fn append_durable_jsonl",
        "file.sync_data()?",
        "repair_replay_failures",
    )
    require_markers(
        "crates/api/src/state.rs",
        "fn init_run_runtime(",
        "ExecutionRunStore::load(config)",
        "execution_runs_from_replay(",
        "CloseRunStore::load(config)",
        "close_runs_from_replay(",
    )
    require_markers(
        "crates/api/src/services/runtime_state.rs",
        'const JSONL_DEGRADED: &str = "jsonl_degraded"',
        'const STORE_DEGRADED: &str = "DEGRADED_STATE_STORE"',
        "execution_store.persistence_configured()",
        "execution_store.persistence_degraded()",
        "close_store.persistence_configured()",
        "close_store.persistence_degraded()",
        "(true, true) => JSONL_DEGRADED",
    )
    require_markers(
        "crates/api/src/services/system_health.rs",
        "runtime_state::inventory(state).await",
        "problems.extend(runtime_state::problems(&runtime_state, now_ms))",
    )
    require_markers(
        "crates/api/src/services/review/ledger.rs",
        "service.list_sql_realized_window(from_ms, to_ms).await",
    )
    require_markers(
        "crates/api/src/services/portfolio_pnl.rs",
        "service.list_sql_realized_window(from_ms, to_ms).await",
    )

    postgres_gate = require_markers(
        "scripts/verify_postgres_ledger_contract.sh",
        "--test sql_ledger_postgres",
        "run_cost_facts_rebuild_execution_and_close_reconciliation",
        "startup_projection_worker_catches_pending_fill_once",
        "--ignored",
        "--test-threads=1",
    )
    if postgres_gate.count("  --ignored") != 3:
        fail("all PostgreSQL live groups must explicitly execute ignored tests")
    require_markers(
        "crates/trading/tests/sql_ledger_postgres.rs",
        "projection_jobs_survive_restart_and_duplicate_persist_does_not_reset_them",
        "sql_ledger_postgres_roundtrip_restarts_from_committed_facts",
    )

    for relative, test_name in RUNNABLE:
        require_runnable(relative, test_name)
    for relative, title in BROWSERS.items():
        source = (root / relative).read_text(encoding="utf-8")
        escaped = re.escape(title)
        if re.search(rf'test\.(?:skip|fixme)\(\s*["\']{escaped}["\']', source):
            fail(f"browser proof is skipped: {title}")
        if not re.search(rf'test\(\s*["\']{escaped}["\']', source):
            fail(f"browser proof is missing: {title}")

    repo_gate = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
    if repo_gate.count("check_pr_ab_completion.sh") != 2:
        fail("repo gate must execute PR-AB in documentation and full scopes")


def assert_rejected(current_root: Path, relative: str, transform, label: str) -> None:
    path = current_root / relative
    baseline = path.read_text(encoding="utf-8")
    changed = transform(baseline)
    if changed == baseline:
        fail(f"self-test setup drifted: {label}")
    path.write_text(changed, encoding="utf-8")
    try:
        check(current_root)
    except ValueError:
        pass
    else:
        fail(f"self-test accepted {label}")
    finally:
        path.write_text(baseline, encoding="utf-8")


def self_test(source_root: Path) -> None:
    paths = {
        "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md",
        "docs/PRODUCT_AUDIT_EVIDENCE.tsv",
        "docs/PRODUCT_AUDIT_COVERAGE.tsv",
        "docs/audit_history/PRODUCT_AUDIT_HISTORY.md",
        "scripts/verify_repo_gates.sh",
        "crates/trading/src/sql_ledger/migrations/tests.rs",
        "crates/trading/tests/sql_ledger_postgres.rs",
        "crates/api/src/services/review/tests/sql_replay.rs",
        "crates/api/src/services/portfolio_pnl/tests.rs",
    }
    paths.update(artifact for artifact, _ in EVIDENCE.values())
    paths.update(relative for relative, _ in RUNNABLE)
    paths.update(BROWSERS)
    with tempfile.TemporaryDirectory(prefix="crossline-pr-ab-completion-") as temp:
        test_root = Path(temp) / "repo"
        for relative in paths:
            target = test_root / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source_root / relative, target)
        check(test_root)
        assert_rejected(test_root, "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md", lambda text: text.replace(f"| `{PR_TITLE}` | ✅ 完成 |", f"| `{PR_TITLE}` | 🟡 部分完成 |", 1), "a downgraded roadmap row")
        assert_rejected(test_root, "docs/PRODUCT_AUDIT_EVIDENCE.tsv", lambda text: "\n".join(line for line in text.splitlines() if not line.startswith("PR-AB\truntime-state-health\t")) + "\n", "an incomplete evidence matrix")
        assert_rejected(test_root, "crates/trading/migrations/20260601_orders.sql", lambda text: text.replace("CREATE TABLE IF NOT EXISTS run_finality_events", "CREATE TABLE IF NOT EXISTS removed_run_finality_events", 1), "a missing run-finality source table")
        assert_rejected(test_root, "crates/api/src/services/execution_run_store.rs", lambda text: text.replace("file.sync_data()?", "file.flush()?", 1), "a non-durable execution-run append")
        assert_rejected(test_root, "crates/api/src/services/close_run_store/ledger.rs", lambda text: text.replace("file.sync_data()?", "file.flush()?", 1), "a non-durable close-run append")
        assert_rejected(test_root, "crates/api/src/services/close_run_store.rs", lambda text: text.replace("pub(crate) fn persistence_degraded", "pub(crate) fn removed_persistence_degraded", 1), "detached close-run degradation health")
        assert_rejected(test_root, "crates/api/src/services/runtime_state.rs", lambda text: text.replace("(true, true) => JSONL_DEGRADED", "(true, true) => JSONL_SNAPSHOT", 1), "a false-green degraded runtime store")
        assert_rejected(test_root, "crates/api/src/services/runtime_state/tests.rs", lambda text: text.replace("#[tokio::test]\nasync fn degraded_execution_run_store_becomes_runtime_problem", "#[tokio::test]\n#[ignore]\nasync fn degraded_execution_run_store_becomes_runtime_problem", 1), "a skipped runtime degradation test")
        assert_rejected(test_root, "crates/api/src/services/review/ledger.rs", lambda text: text.replace("service.list_sql_realized_window(from_ms, to_ms).await", "None", 1), "a detached Review SQL consumer")
        assert_rejected(test_root, "crates/api/src/services/portfolio_pnl.rs", lambda text: text.replace("service.list_sql_realized_window(from_ms, to_ms).await", "None", 1), "a detached Portfolio SQL consumer")
        assert_rejected(test_root, "test/e2e/pr_ea_execution_finality.spec.ts", lambda text: text.replace('test("PR-EA renders ticket evidence', 'test.skip("PR-EA renders ticket evidence', 1), "a skipped execution recovery browser proof")
        assert_rejected(test_root, "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md", lambda text: text.replace("### 🟡 6.5 下一步执行队列", "### 🟡 6.5 下一步执行队列\n\n1. **PR-AB stale**", 1), "completed PR-AB returned to the queue")
        assert_rejected(
            test_root,
            "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md",
            lambda text: re.sub(r"(?m)^1\. \*\*PR-[A-Z]+", "1. **PR-ZZ", text, count=1),
            "an untracked local queue head",
        )
        assert_rejected(test_root, "scripts/verify_repo_gates.sh", lambda text: text.replace("check_pr_ab_completion.sh", "removed_pr_ab_completion.sh", 1), "single-scope repository wiring")
    print("PR-AB completion destructive self-test passed")


try:
    if mode == "--self-test":
        self_test(root)
    else:
        check(root)
        print(
            f"OK PR-AB static contract ({len(EVIDENCE)} evidence types; "
            f"{len(RUNNABLE)} runnable anchors; {len(BROWSERS)} browser anchors)"
        )
except (OSError, KeyError, ValueError) as exc:
    raise SystemExit(f"PR-AB completion gate failed: {exc}") from exc
PY

if [[ "$MODE" == "--self-test" ]]; then
  exit 0
fi

if [[ "${PR_AB_SKIP_BROWSER_LIST:-0}" != "1" ]]; then
  npx playwright test \
    "$ROOT/test/e2e/pr_ea_execution_finality.spec.ts" \
    "$ROOT/test/e2e/pr_dh_review_ledger.spec.ts" \
    "$ROOT/test/e2e/pr_dz_portfolio_truth.spec.ts" \
    --list >/dev/null
fi

if [[ "${PR_AB_SKIP_UPSTREAM:-0}" != "1" ]]; then
  PR_C_SKIP_TESTS=1 PR_FZ_SKIP_TESTS=1 PR_DM_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dm_completion.sh"
  PR_CI_SKIP_TESTS=1 PR_CI_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_ci_completion.sh"
  PR_DH_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dh_completion.sh"
  PR_EA_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_ea_completion.sh"
  PR_BW_SKIP_TESTS=1 PR_BW_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_bw_completion.sh"
  bash "$ROOT/scripts/check_product_audit_progress.sh"
  bash "$ROOT/scripts/check_product_audit_evidence.sh"
  bash "$ROOT/scripts/check_product_audit_evidence_index.sh"
  bash "$ROOT/scripts/check_product_audit_coverage.sh"
fi

if [[ "${PR_AB_SKIP_TESTS:-0}" != "1" ]]; then
  jobs="${CARGO_BUILD_JOBS:-10}"
  CARGO_BUILD_JOBS="$jobs" cargo test -p trading sql_ledger::migrations::tests --lib --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api run_store::tests --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api runtime_state::tests --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api executed_envelope_uses_sql_replayed_events_and_order_snapshots --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api sql_realized_window --no-fail-fast
  if [[ "${PR_AB_SKIP_BROWSER:-0}" != "1" ]]; then
    CI=1 npx playwright test \
      "$ROOT/test/e2e/pr_ea_execution_finality.spec.ts" \
      "$ROOT/test/e2e/pr_dh_review_ledger.spec.ts" \
      "$ROOT/test/e2e/pr_dz_portfolio_truth.spec.ts" \
      --grep 'PR-EA renders ticket evidence and replayable finality timeline|PR-DH review ledger keeps explicit PnL quality and snapshot-bound 50-row budget|PR-DZ portfolio envelope keeps wallet NAV, position evidence, partial state, and durable compensation visible' \
      --workers=1
  fi
fi

if [[ "${PR_AB_SKIP_POSTGRES:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-10}" bash "$ROOT/scripts/verify_postgres_ledger_contract.sh"
fi

printf 'PR-AB trading ledger persistence completion passed\n'
