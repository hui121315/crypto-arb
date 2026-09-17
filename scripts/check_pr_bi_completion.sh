#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
HISTORY_SCHEMA="$ROOT/crates/realtime/src/history/postgres/schema.rs"
HISTORY_RUNTIME="$ROOT/crates/realtime/src/history/postgres.rs"
POSTGRES_GATE="$ROOT/scripts/verify_history_migration_contract.sh"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-BI completion gate failed: %s\n' "$1" >&2
  exit 1
}

if [[ "$MODE" == "--self-test" ]]; then
  schema_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-bi-schema.XXXXXX")"
  runtime_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-bi-runtime.XXXXXX")"
  gate_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-bi-gate.XXXXXX")"
  cp "$HISTORY_SCHEMA" "$schema_backup"
  cp "$HISTORY_RUNTIME" "$runtime_backup"
  cp "$POSTGRES_GATE" "$gate_backup"
  restore() {
    cp "$schema_backup" "$HISTORY_SCHEMA"
    cp "$runtime_backup" "$HISTORY_RUNTIME"
    cp "$gate_backup" "$POSTGRES_GATE"
    chmod +x "$POSTGRES_GATE"
    rm -f "$schema_backup" "$runtime_backup" "$gate_backup"
  }
  trap restore EXIT

  PR_BI_SKIP_TESTS=1 PR_BI_SKIP_UPSTREAM=1 bash "$0" >/dev/null

  python3 - "$HISTORY_SCHEMA" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "ON CONFLICT (migration_id) DO NOTHING"
if source.count(marker) != 1:
    raise SystemExit("PR-BI self-test setup failed: immutable insert marker drifted")
path.write_text(
    source.replace(marker, "ON CONFLICT (migration_id) DO UPDATE SET checksum = EXCLUDED.checksum", 1),
    encoding="utf-8",
)
PY
  if PR_BI_SKIP_TESTS=1 PR_BI_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted mutable history migration records"
  fi
  cp "$schema_backup" "$HISTORY_SCHEMA"

  python3 - "$HISTORY_RUNTIME" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "        self.validate_runtime_tables().await?;"
if source.count(marker) != 1:
    raise SystemExit("PR-BI self-test setup failed: runtime table validation marker drifted")
path.write_text(source.replace(marker, "        // validation removed by self-test", 1), encoding="utf-8")
PY
  if PR_BI_SKIP_TESTS=1 PR_BI_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted startup without required-table validation"
  fi
  cp "$runtime_backup" "$HISTORY_RUNTIME"

  python3 - "$HISTORY_RUNTIME" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "let existing_schema = presence.history_meta || presence.runtime_tables;"
if source.count(marker) != 1:
    raise SystemExit("PR-BI self-test setup failed: orphan-table authority marker drifted")
path.write_text(
    source.replace(marker, "let existing_schema = presence.history_meta;", 1),
    encoding="utf-8",
)
PY
  if PR_BI_SKIP_TESTS=1 PR_BI_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted orphan runtime tables as a fresh schema"
  fi
  cp "$runtime_backup" "$HISTORY_RUNTIME"

  python3 - "$POSTGRES_GATE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "  --ignored"
if source.count(marker) != 1:
    raise SystemExit("PR-BI self-test setup failed: non-skipping invocation drifted")
path.write_text(source.replace(marker, "  --list", 1), encoding="utf-8")
PY
  if PR_BI_SKIP_TESTS=1 PR_BI_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a skipped PostgreSQL drift acceptance"
  fi

  printf 'PR-BI completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from __future__ import annotations

import csv
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])
title = "PR-BI SQL Migration & Schema Drift Governance"
verify_anchor = "`bash scripts/check_pr_bi_completion.sh --self-test`"
evidence_contract = {
    "history-migration-source": "crates/realtime/migrations/20260701_history.sql",
    "history-migration-runtime": "crates/realtime/src/history/postgres.rs",
    "history-schema-inventory": "crates/realtime/src/history/postgres/inventory.rs",
    "history-schema-governance": "crates/realtime/src/history/postgres/schema.rs",
    "history-schema-unit-tests": "crates/realtime/src/history/postgres/tests.rs",
    "history-postgres-live-test": "crates/realtime/src/history/postgres/tests/live.rs",
    "history-runtime-table-tests": "crates/realtime/src/history/tests/migration.rs",
    "history-postgres-live-gate": "scripts/verify_history_migration_contract.sh",
    "nav-migration-source": "crates/api/migrations/20260605_portfolio_nav.sql",
    "nav-migration-runner": "crates/api/src/lifecycle/nav_persist.rs",
    "nav-migration-tests": "crates/api/src/lifecycle/nav_persist/tests.rs",
    "trading-migration-registry": "crates/trading/src/sql_ledger/migrations.rs",
    "trading-schema-drift-tests": "crates/trading/src/sql_ledger/migrations/tests.rs",
    "shared-storage-authority": "scripts/check_pr_ci_completion.sh",
    "durable-ledger-authority": "scripts/check_pr_dm_completion.sh",
    "trading-postgres-restart-gate": "scripts/verify_postgres_ledger_contract.sh",
    "completion-governance": "scripts/check_pr_bi_completion.sh",
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-BI completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(
    encoding="utf-8"
)
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be completed with no local remainder")
if row.count(verify_anchor) != 1:
    fail("roadmap verification must name the destructive completion gate")
queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-BI\b", queue):
    fail("completed PR-BI remains in the local queue")

for heading in (
    "订单 SQL durable ledger replayer/query 已成为运行时事实源",
    "Trading order SQL migration 已成为运行时事实源",
    "HistoryStore migration drift 与 review storage 已统一",
    "Storage health schema drift/NAV envelope 已统一",
):
    finding = next(
        (
            line
            for source in (doc, history)
            for line in source.splitlines()
            if heading in line
        ),
        None,
    )
    if finding is None or "✅ 完成" not in finding:
        fail(f"absorbed audit finding is missing or remains partial: {heading}")

for upstream in ("PR-CI Storage Health", "PR-DM Storage Health"):
    upstream_row = next(
        (line for line in doc.splitlines() if line.startswith(f"| `{upstream}")),
        None,
    )
    if upstream_row is None or "✅ 完成" not in upstream_row or "剩余：无。" not in upstream_row:
        fail(f"completed upstream authority drifted: {upstream}")

if "## 2026-07-15 PR-BI SQL Migration and Schema Drift Closure" not in history:
    fail("history closure appendix is missing")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(
    encoding="utf-8", newline=""
) as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-BI"]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for kind, artifact in evidence_contract.items():
    if indexed[kind]["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"{kind} artifact drifted or is missing")

markers = {
    "crates/realtime/src/history/postgres.rs": (
        "HISTORY_MIGRATION_ADVISORY_LOCK_SQL",
        "load_schema_presence().await?",
        "let existing_schema = presence.history_meta || presence.runtime_tables;",
        "None if existing_schema",
        "self.validate_runtime_tables().await?;",
        'row.try_get("value")',
        "load_schema_migration().await?",
    ),
    "crates/realtime/src/history/postgres/schema.rs": (
        "ON CONFLICT (migration_id) DO NOTHING",
        "HISTORY_MIGRATION_ADVISORY_LOCK_KEY",
        "pub(super) const HISTORY_RUNTIME_TABLES",
        "history_migration_column(row, \"migration_id\")?",
        "history_migration_status_from_applied",
        "is_runtime_schema_line",
    ),
    "crates/realtime/src/history/postgres/inventory.rs": (
        "runtime_tables: HISTORY_RUNTIME_TABLES[2..]",
        "load_schema_tables().await?",
        "history runtime tables are missing",
        "history table inventory row is invalid",
    ),
    "crates/realtime/src/history/postgres/tests.rs": (
        "applied_migration_history_is_insert_only",
        "applied_migration_identity_rejects_every_immutable_field_drift",
        "mod live;",
    ),
    "crates/realtime/src/history/postgres/tests/live.rs": (
        "history_postgres_restart_rejects_migration_and_table_drift",
        "CREATE TABLE funding_rates (orphan_id BIGINT)",
        "tokio::join!",
        "assert_history_connect_drift",
    ),
    "crates/realtime/src/history/tests/migration.rs": (
        "history_migration_contains_runtime_history_tables",
        "history_migration_includes_execution_ledger_tables",
    ),
    "crates/api/src/lifecycle/nav_persist.rs": (
        'include_str!("../../migrations/20260605_portfolio_nav.sql")',
        "NAV_SCHEMA_HASH_VALUE",
    ),
    "crates/api/src/lifecycle/nav_persist/tests.rs": (
        "nav_schema_migration_is_single_authority_with_stable_hash",
        "NAV_SCHEMA_MIGRATION_ID",
    ),
    "crates/trading/src/sql_ledger/migrations.rs": (
        "ON CONFLICT (migration_id) DO NOTHING",
        "Result<MigrationDisposition, MigrationFailure>",
        "reason: StorageDegradedReason::SchemaDrift,",
    ),
    "crates/trading/src/sql_ledger/migrations/tests.rs": (
        "registry_order_paths_versions_and_checksums_are_frozen",
        "existing_history_must_match_every_immutable_field",
        "history_sql_is_bootstrapped_and_never_rewritten",
    ),
}
for path, required in markers.items():
    source = (root / path).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"missing {path} marker: {marker}")

live_gate = (root / "scripts/verify_history_migration_contract.sh").read_text(
    encoding="utf-8"
)
for marker in (
    "mktemp -d /tmp/crossline-pr-bi-postgres.",
    "trap cleanup EXIT",
    "history_postgres_restart_rejects_migration_and_table_drift",
    "OK PR-BI disposable PostgreSQL history migration contract",
):
    if marker not in live_gate:
        fail(f"history PostgreSQL gate marker drifted: {marker}")
if live_gate.count("  --ignored") != 1 or live_gate.count("  --test-threads=1") != 1:
    fail("history PostgreSQL acceptance must run exactly once and cannot skip")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(
    encoding="utf-8", newline=""
) as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
for artifact in evidence_contract.values():
    if coverage.get(artifact, {}).get("coverage_status") != "exact":
        fail(f"exact coverage missing for {artifact}")

print(f"PR-BI static completion contract passed ({len(evidence_contract)} evidence rows)")
PY

bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"

if [[ "${PR_BI_SKIP_UPSTREAM:-0}" != "1" ]]; then
  PR_CI_SKIP_TESTS=1 PR_CI_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_ci_completion.sh"
  PR_DM_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dm_completion.sh"
fi

if [[ "${PR_BI_SKIP_TESTS:-0}" != "1" ]]; then
  jobs="${CARGO_BUILD_JOBS:-10}"
  CARGO_BUILD_JOBS="$jobs" cargo test -p realtime history --lib --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api nav_persist --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p trading sql_ledger::migrations --lib --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" bash "$POSTGRES_GATE"
  CARGO_BUILD_JOBS="$jobs" bash "$ROOT/scripts/verify_postgres_ledger_contract.sh"
fi

printf 'PR-BI completion gate passed\n'
