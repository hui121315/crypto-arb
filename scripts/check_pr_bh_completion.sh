#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
IGNORE="$ROOT/.gitignore"
FIXTURE_README="$ROOT/crates/api/fixtures/README.md"
SQL_LEDGER="$ROOT/crates/trading/src/sql_ledger.rs"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-BH completion gate failed: %s\n' "$1" >&2
  exit 1
}

if [[ "$MODE" == "--self-test" ]]; then
  ignore_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-bh-ignore.XXXXXX")"
  fixture_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-bh-fixture.XXXXXX")"
  ledger_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-bh-ledger.XXXXXX")"
  cp "$IGNORE" "$ignore_backup"
  cp "$FIXTURE_README" "$fixture_backup"
  cp "$SQL_LEDGER" "$ledger_backup"
  restore() {
    cp "$ignore_backup" "$IGNORE"
    cp "$fixture_backup" "$FIXTURE_README"
    cp "$ledger_backup" "$SQL_LEDGER"
    rm -f "$ignore_backup" "$fixture_backup" "$ledger_backup"
  }
  trap restore EXIT

  PR_BH_SKIP_TESTS=1 PR_BH_SKIP_UPSTREAM=1 bash "$0" >/dev/null

  python3 - "$IGNORE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "data/*.sqlite\n"
lines = source.splitlines(keepends=True)
if lines.count(marker) != 1:
    raise SystemExit("PR-BH self-test setup failed: runtime ignore marker drifted")
lines.remove(marker)
path.write_text("".join(lines), encoding="utf-8")
PY
  if PR_BH_SKIP_TESTS=1 PR_BH_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted removal of the runtime sqlite ignore boundary"
  fi
  cp "$ignore_backup" "$IGNORE"

  python3 - "$FIXTURE_README" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "Runtime data must not be committed here."
if source.count(marker) != 1:
    raise SystemExit("PR-BH self-test setup failed: fixture boundary marker drifted")
path.write_text(source.replace(marker, "Fixture data may be committed here.", 1), encoding="utf-8")
PY
  if PR_BH_SKIP_TESTS=1 PR_BH_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a fixture directory without a runtime-data prohibition"
  fi
  cp "$fixture_backup" "$FIXTURE_README"

  python3 - "$SQL_LEDGER" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "    pub async fn query_realized_window("
if source.count(marker) != 1:
    raise SystemExit("PR-BH self-test setup failed: SQL query marker drifted")
path.write_text(
    source.replace(marker, "    pub async fn query_realized_window_removed(", 1),
    encoding="utf-8",
)
PY
  if PR_BH_SKIP_TESTS=1 PR_BH_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted removal of the durable SQL realized-window query"
  fi

  printf 'PR-BH completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from __future__ import annotations

import csv
import re
import subprocess
import sys
from pathlib import Path

root = Path(sys.argv[1])
title = "PR-BH Runtime Data Artifact & Storage Boundary"
verify_anchor = "`bash scripts/check_pr_bh_completion.sh --self-test`"
evidence_contract = {
    "runtime-data-dir": "crates/common/src/config.rs",
    "runtime-env-contract": ".env.example",
    "runtime-artifact-ignore": ".gitignore",
    "runtime-artifact-gate": "scripts/verify_repo_gates.sh",
    "fixture-boundary": "scripts/verify_repo_gates.sh",
    "nav-migration-storage": "crates/api/src/lifecycle/nav_persist.rs",
    "order-snapshot-jsonl-replay": "crates/trading/src/journal/projection/tests/part_05.rs",
    "sql-ledger-replay-query": "crates/trading/src/sql_ledger.rs",
    "sql-ledger-postgres-restart": "crates/trading/tests/sql_ledger_postgres.rs",
    "sql-review-portfolio-consumer": "crates/api/src/services/portfolio_pnl.rs",
    "storage-runtime-health": "crates/api/src/services/venue_operation_health/snapshot/tests/part_08.rs",
    "migration-authority": "scripts/check_pr_ci_completion.sh",
    "durable-ledger-authority": "scripts/check_pr_dm_completion.sh",
    "completion-governance": "scripts/check_pr_bh_completion.sh",
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-BH completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be completed with no local remainder")
if row.count(verify_anchor) != 1:
    fail("roadmap verification must name the destructive completion gate")
queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-BH\b", queue):
    fail("completed PR-BH remains in the local queue")

for upstream in (
    "PR-CI Storage Health & Migration Authority Contract",
    "PR-DM Storage Health & Ledger Persistence Contract",
    "PR-FZ Runtime Artifact, Storage Path & Data Hygiene Contract",
):
    upstream_row = next(
        (line for line in doc.splitlines() if line.startswith(f"| `{upstream}`")),
        None,
    )
    if upstream_row is None or "✅ 完成" not in upstream_row or "剩余：无。" not in upstream_row:
        fail(f"upstream storage authority drifted: {upstream}")

history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(
    encoding="utf-8"
)
if "## 2026-07-15 PR-BH Runtime Artifact and Durable Replay Closure" not in history:
    fail("history closure appendix is missing")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(
    encoding="utf-8", newline=""
) as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-BH"]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for kind, artifact in evidence_contract.items():
    if indexed[kind]["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"{kind} artifact drifted or is missing")

markers = {
    "crates/common/src/config.rs": (
        'DEFAULT_RUNTIME_DIR_NAME: &str = "crossline-omni"',
        "pub fn resolve_runtime_path(&self, value: &str) -> PathBuf",
        "fn default_nav_storage_uses_ignored_runtime_dir()",
        "fn relative_runtime_paths_are_resolved_under_data_dir()",
    ),
    ".env.example": (
        "APP_STORAGE__DATA_DIR=",
        "APP_STORAGE__PORTFOLIO_NAV_PATH=portfolio_nav.sqlite",
        "APP_STORAGE__ORDER_SNAPSHOT_PATH=order_snapshots.jsonl",
    ),
    "crates/api/fixtures/README.md": (
        "Runtime data must not be committed here.",
    ),
    "crates/api/fixtures/portfolio_nav/README.md": (
        "crates/api/migrations/20260605_portfolio_nav.sql",
        "nav_persist::nav_schema_hash()",
        "temporary directory",
    ),
    "crates/api/src/lifecycle/nav_persist.rs": (
        'NAV_SCHEMA_MIGRATION_ID: &str = "20260605_portfolio_nav"',
        'include_str!("../../migrations/20260605_portfolio_nav.sql")',
        "pub(crate) fn nav_schema_hash() -> String",
    ),
    "crates/trading/src/journal/projection/tests/part_05.rs": (
        "fn order_snapshot_jsonl_replays_order_records_after_restart()",
        "fn order_snapshot_replay_keeps_latest_record_for_same_order()",
    ),
    "crates/trading/src/sql_ledger.rs": (
        "pub async fn query_realized_window(",
        "async fn replay_sql_ledger_inner(url: &str) -> SqlLedgerReplay",
        "read_replay_order_snapshots(&client, &mut replay).await;",
        "fn sql_order_snapshot_from_replay_row(",
        "decode_typed_replay_payload::<OrderRecord>(",
    ),
    "crates/trading/tests/sql_ledger_postgres.rs": (
        "sql_ledger_postgres_roundtrip_restarts_from_committed_facts",
        "stored_normalized_cost_sources",
    ),
    "crates/api/src/services/portfolio_pnl.rs": (
        "list_sql_realized_window(from_ms, to_ms).await",
        "realized_pnl_by_group_with_close_runs",
    ),
    "crates/api/src/services/venue_operation_health/snapshot/tests/part_08.rs": (
        "fn trading_sql_ledger_storage_row_reports_replay_query_health()",
        "fn trading_sql_ledger_storage_row_blocks_replay_decode_failures()",
    ),
}
for path, required in markers.items():
    source = (root / path).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"missing {path} marker: {marker}")

ignore = (root / ".gitignore").read_text(encoding="utf-8")
ignore_lines = set(ignore.splitlines())
for pattern in (
    ".crossline-runtime/",
    "data/*.sqlite",
    "data/*.db",
    "data/*.jsonl",
    "crates/api/data/*.sqlite",
    "frontend/data/*.sqlite",
):
    if pattern not in ignore_lines:
        fail(f"runtime ignore pattern is missing: {pattern}")

tracked = subprocess.run(
    [
        "git", "-C", str(root), "ls-files", "--",
        "*.sqlite", "*.sqlite-*", "*.sqlite3", "*.db", "*.db-*", "*.db3", "*.jsonl",
    ],
    check=True,
    capture_output=True,
    text=True,
).stdout.splitlines()
tracked_runtime = [path for path in tracked if "/fixtures/" not in f"/{path}"]
if tracked_runtime:
    fail(f"tracked runtime artifacts remain: {tracked_runtime}")

repo_gate = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
for marker in ("fail_if_tracked_runtime_artifacts", "require_api_fixture_boundary_docs"):
    if marker not in repo_gate:
        fail(f"repo storage boundary marker drifted: {marker}")
if repo_gate.count(
    'PR_BH_SKIP_TESTS=1 PR_BH_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_bh_completion.sh"'
) != 2:
    fail("PR-BH completion gate must run in both documentation and full repo scopes")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(
    encoding="utf-8", newline=""
) as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
for artifact in set(evidence_contract.values()) - {".env.example", ".gitignore"}:
    if coverage.get(artifact, {}).get("coverage_status") != "exact":
        fail(f"exact coverage missing for {artifact}")

print(
    f"OK PR-BH contract ({len(evidence_contract)} evidence types; "
    "fixed runtime paths + fixture boundary + JSONL/SQL restart replay/query)"
)
PY

bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"

if [[ "${PR_BH_SKIP_UPSTREAM:-0}" != "1" ]]; then
  PR_CI_SKIP_TESTS=1 PR_CI_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_ci_completion.sh"
  PR_DM_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dm_completion.sh"
fi

if [[ "${PR_BH_SKIP_TESTS:-0}" != "1" ]]; then
  jobs="${CARGO_BUILD_JOBS:-10}"
  CARGO_BUILD_JOBS="$jobs" cargo test -p common --lib --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p trading order_snapshot_jsonl --lib --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p trading sql_replay --lib --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api trading_sql_ledger_storage_row --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api sql_replay --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" bash "$ROOT/scripts/verify_postgres_ledger_contract.sh"
fi

printf 'PR-BH completion gate passed\n'
