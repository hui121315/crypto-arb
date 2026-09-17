#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
STORAGE="$ROOT/shared-types/src/storage.rs"
HISTORY_ROUTER="$ROOT/crates/api/src/routers/history.rs"
TRADING_MIGRATIONS="$ROOT/crates/trading/src/sql_ledger/migrations.rs"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-CI completion gate failed: %s\n' "$1" >&2
  exit 1
}

if [[ "$MODE" == "--self-test" ]]; then
  storage_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-ci-storage.XXXXXX")"
  history_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-ci-history.XXXXXX")"
  migration_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-ci-migration.XXXXXX")"
  cp "$STORAGE" "$storage_backup"
  cp "$HISTORY_ROUTER" "$history_backup"
  cp "$TRADING_MIGRATIONS" "$migration_backup"
  restore() {
    cp "$storage_backup" "$STORAGE"
    cp "$history_backup" "$HISTORY_ROUTER"
    cp "$migration_backup" "$TRADING_MIGRATIONS"
    rm -f "$storage_backup" "$history_backup" "$migration_backup"
  }
  trap restore EXIT

  PR_CI_SKIP_TESTS=1 PR_CI_SKIP_UPSTREAM=1 bash "$0" >/dev/null

  python3 - "$STORAGE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "    SchemaDrift,"
if source.count(marker) != 1:
    raise SystemExit("PR-CI self-test setup failed: schema drift enum marker drifted")
path.write_text(source.replace(marker, "    SchemaMismatch,", 1), encoding="utf-8")
PY
  if PR_CI_SKIP_TESTS=1 PR_CI_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted removal of the shared schema-drift reason"
  fi
  cp "$storage_backup" "$STORAGE"

  python3 - "$HISTORY_ROUTER" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = ".push(StorageDegradedReason::Stale);"
if source.count(marker) != 1:
    raise SystemExit("PR-CI self-test setup failed: stale projection marker drifted")
path.write_text(source.replace(marker, ".clear();", 1), encoding="utf-8")
PY
  if PR_CI_SKIP_TESTS=1 PR_CI_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a history envelope without typed stale evidence"
  fi
  cp "$history_backup" "$HISTORY_ROUTER"

  python3 - "$TRADING_MIGRATIONS" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "            reason: StorageDegradedReason::SchemaDrift,"
if source.count(marker) != 1:
    raise SystemExit("PR-CI self-test setup failed: typed migration drift marker drifted")
path.write_text(
    source.replace(
        marker,
        "            reason: StorageDegradedReason::MigrationFailed,",
        1,
    ),
    encoding="utf-8",
)
PY
  if PR_CI_SKIP_TESTS=1 PR_CI_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted migration drift downgraded to a generic failure"
  fi

  printf 'PR-CI completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from __future__ import annotations

import csv
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])
title = "PR-CI Storage Health & Migration Authority Contract"
verify_anchor = "`bash scripts/check_pr_ci_completion.sh --self-test`"
evidence_contract = {
    "shared-storage-contract": "shared-types/src/storage.rs",
    "history-runtime-contract": "crates/realtime/src/history.rs",
    "history-schema-drift": "crates/realtime/src/history/postgres/schema.rs",
    "history-envelope-contract": "crates/api/src/routers/history.rs",
    "history-operation-health": "crates/api/src/services/venue_operation_health/snapshot/part_10.rs",
    "nav-migration-authority": "crates/api/src/services/portfolio/nav.rs",
    "nav-operation-health": "crates/api/src/services/venue_operation_health/snapshot/part_14.rs",
    "trading-migration-contract": "crates/trading/src/sql_ledger/migrations.rs",
    "trading-storage-contract": "crates/trading/src/sql_ledger.rs",
    "trading-operation-health": "crates/api/src/services/venue_operation_health/snapshot/part_12.rs",
    "durable-ledger-authority": "scripts/check_pr_dm_completion.sh",
    "completion-governance": "scripts/check_pr_ci_completion.sh",
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-CI completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be completed with no local remainder")
if row.count(verify_anchor) != 1:
    fail("roadmap verification must name the destructive completion gate")
queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-CI\b", queue):
    fail("completed PR-CI remains in the local queue")
dm_row = next(
    (line for line in doc.splitlines() if line.startswith("| `PR-DM Storage Health")),
    None,
)
if dm_row is None or "✅ 完成" not in dm_row or "剩余：无。" not in dm_row:
    fail("durable trading ledger authority must remain completed under PR-DM")

history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(
    encoding="utf-8"
)
if "## 2026-07-14 PR-CI Storage Health and Migration Authority Closure" not in history:
    fail("history closure appendix is missing")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(
    encoding="utf-8", newline=""
) as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-CI"]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for kind, artifact in evidence_contract.items():
    if indexed[kind]["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"{kind} artifact drifted or is missing")

markers = {
    "shared-types/src/storage.rs": (
        "pub enum StorageBackendKind",
        "pub enum StorageDegradedReason",
        "SchemaDrift,",
        "pub struct StorageMigrationAuthority",
        "pub struct StorageRuntimeContract",
    ),
    "shared-types/src/history.rs": (
        "pub type HistoryMigrationStatus = crate::StorageMigrationAuthority;",
        "pub storage_contract: crate::StorageRuntimeContract",
    ),
    "crates/realtime/src/history.rs": (
        "mod health;",
        "mod telemetry;",
        "telemetry: Arc<HistoryStoreTelemetry>",
    ),
    "crates/realtime/src/history/telemetry.rs": (
        "last_error_reason: arc_swap::ArcSwapOption<StorageDegradedReason>",
        "error.storage_degraded_reason(operation_reason)",
    ),
    "crates/realtime/src/history/postgres/schema.rs": (
        "HistoryError::SchemaDrift(error.to_string())",
        "history migration row mismatch",
    ),
    "crates/api/src/routers/history.rs": (
        ".push(StorageDegradedReason::Stale);",
        "health.backend_status()",
        '"storageContract": &health.storage_contract',
    ),
    "crates/api/src/services/portfolio/nav.rs": (
        "pub(crate) fn nav_storage_contract",
        "pub(crate) fn nav_migration_authority",
        "migration_status: Some(migration_authority)",
    ),
    "crates/trading/src/sql_ledger/migrations.rs": (
        "reason: StorageDegradedReason::SchemaDrift,",
        "Result<MigrationDisposition, MigrationFailure>",
    ),
    "crates/trading/src/sql_ledger.rs": (
        "pub degraded_reason: Option<StorageDegradedReason>",
        "pub fn migration_authority(&self) -> StorageMigrationAuthority",
        "pub fn storage_contract(&self) -> StorageRuntimeContract",
    ),
    "crates/api/src/services/venue_operation_health/snapshot/part_12.rs": (
        "health.degraded_reason == Some(StorageDegradedReason::SchemaDrift)",
        '"storageContract": health.storage_contract()',
    ),
}
for path, required in markers.items():
    source = (root / path).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"missing {path} marker: {marker}")

for path in (
    "crates/api/src/routers/history.rs",
    "crates/api/src/services/venue_operation_health/snapshot/part_10.rs",
    "crates/api/src/services/venue_operation_health/snapshot/part_12.rs",
):
    source = (root / path).read_text(encoding="utf-8")
    if re.search(r'contains\("(?:history )?schema drift|contains\("schema version mismatch|contains\("checksum mismatch', source):
        fail(f"{path} infers schema drift from error prose")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(
    encoding="utf-8", newline=""
) as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
for artifact in evidence_contract.values():
    row = coverage.get(artifact)
    if row is None or row["coverage_status"] != "exact":
        fail(f"coverage is not exact for {artifact}")

print(f"PR-CI static completion contract passed ({len(evidence_contract)} evidence rows)")
PY

if [[ "${PR_CI_SKIP_UPSTREAM:-0}" != "1" ]]; then
  PR_DM_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dm_completion.sh"
fi

if [[ "${PR_CI_SKIP_TESTS:-0}" != "1" ]]; then
  jobs="${CARGO_BUILD_JOBS:-10}"
  CARGO_BUILD_JOBS="$jobs" cargo test -p shared-types storage --lib --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p shared-types history_response --lib --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p realtime history --lib --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p trading existing_history_must_match_every_immutable_field --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api routers::history --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api storage_row --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api nav_history_response_uses_memory_rows_and_storage_envelope --no-fail-fast
fi

printf 'PR-CI completion gate passed\n'
