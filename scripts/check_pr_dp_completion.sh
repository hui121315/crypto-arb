#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
MIGRATION="$ROOT/crates/realtime/migrations/20260714_watchlist_alerts.sql"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

if [[ "$MODE" == "--self-test" ]]; then
  PR_DP_SKIP_TESTS=1 bash "$0" >/dev/null
  backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-dp.XXXXXX")"
  cp "$MIGRATION" "$backup"
  restore() {
    cp "$backup" "$MIGRATION"
    rm -f "$backup"
  }
  trap restore EXIT
  python3 - "$MIGRATION" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "payload_hash TEXT NOT NULL"
if source.count(marker) != 1:
    raise SystemExit("PR-DP self-test setup failed: payload hash marker drifted")
path.write_text(source.replace(marker, "payload_hash TEXT", 1), encoding="utf-8")
PY
  if PR_DP_SKIP_TESTS=1 bash "$0" >/dev/null 2>&1; then
    printf 'PR-DP completion self-test failed: nullable payload hash regression passed\n' >&2
    exit 1
  fi
  printf 'PR-DP completion self-test passed\n'
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
title = "PR-DP Watchlist Alert Persistence & Runtime Status Contract"
verify_anchor = "`bash scripts/check_pr_dp_completion.sh --self-test`"
browser_path = "test/e2e/watchlist_alerts_runtime.spec.ts"
browser_titles = (
    "settings exposes bounded watchlist prewarm and truthful toast queue runtime",
    "settings exposes durable watchlist storage and delivery provenance",
)
evidence_contract = {
    "shared-persistence-dto": "shared-types/src/alerts.rs",
    "sqlite-authoritative-migration": "crates/realtime/migrations/20260714_watchlist_alerts.sql",
    "sqlite-snapshot-store": "crates/realtime/src/alerts/storage.rs",
    "sqlite-corruption-fixtures": "crates/realtime/src/alerts/storage/tests.rs",
    "atomic-mutation-service": "crates/api/src/services/watchlist_alerts.rs",
    "mutation-rollback-gate": "crates/api/src/routers/watchlist/tests.rs",
    "startup-replay-cooldown": "crates/api/src/state.rs",
    "durable-delivery-state": "crates/realtime/src/alerts/evaluation.rs",
    "runtime-envelope-contract": "crates/realtime/src/alerts.rs",
    "operation-health-registry": "crates/api/src/services/venue_operation_health/snapshot/part_19.rs",
    "operation-health-gate": "crates/api/src/services/venue_operation_health/snapshot/tests/part_13.rs",
    "runtime-state-inventory": "crates/api/src/services/runtime_state.rs",
    "bounded-prewarm-runtime": "crates/api/src/lifecycle/market_data/watchlist_runtime.rs",
    "settings-diagnostics": "frontend/src/panels/modules/settings/tabs/diagnostics/watchlist_alerts.rs",
    "browser-runtime-gate": browser_path,
    "webhook-diagnostic-boundary": "shared-types/src/alerts.rs",
    "upstream-pr-fl-governance": "scripts/check_pr_fl_completion.sh",
    "completion-governance": "scripts/check_pr_dp_completion.sh",
}
closure_paths = (
    "shared-types/src/alerts.rs",
    "shared-types/src/lib.rs",
    "shared-types/src/venues/operation.rs",
    "shared-types/src/venues/operation_kind.rs",
    "shared-types/src/venues/operation_kind_labels.rs",
    "shared-types/src/venues/runtime_health_snapshot.rs",
    "crates/common/src/config.rs",
    "crates/realtime/migrations/20260714_watchlist_alerts.sql",
    "crates/realtime/src/alerts.rs",
    "crates/realtime/src/alerts/storage.rs",
    "crates/realtime/src/alerts/storage/tests.rs",
    "crates/realtime/src/alerts/evaluation.rs",
    "crates/realtime/src/alerts/mutations.rs",
    "crates/api/src/services/watchlist_alerts.rs",
    "crates/api/src/state.rs",
    "crates/api/src/routers/watchlist.rs",
    "crates/api/src/routers/watchlist/tests.rs",
    "crates/api/src/routers/alerts.rs",
    "crates/api/src/lifecycle/snapshot/alerts.rs",
    "crates/api/src/lifecycle/market_data/watchlist_runtime.rs",
    "crates/api/src/services/runtime_state.rs",
    "crates/api/src/services/runtime_state/tests.rs",
    "crates/api/src/services/ws_replay.rs",
    "crates/api/src/services/venue_operation_health/snapshot/part_19.rs",
    "crates/api/src/services/venue_operation_health/snapshot/tests/part_13.rs",
    "frontend/src/panels/modules/settings/tabs/diagnostics/watchlist_alerts.rs",
    "frontend/src/panels/modules/settings/tabs/diagnostics/watchlist_alerts/format.rs",
    "frontend/src/panels/modules/settings/tabs/diagnostics/watchlist_alerts/tests.rs",
    "frontend/src/state/watchlist_alerts.rs",
    browser_path,
    "scripts/check_pr_eh_completion.sh",
    "scripts/check_pr_dp_completion.sh",
)
test_anchors = (
    ("shared-types/src/alerts.rs", "watchlist_item_serde_uses_camel_case_and_defaults"),
    ("shared-types/src/alerts.rs", "alert_rule_serde_applies_defaults"),
    ("shared-types/src/alerts.rs", "webhook_channel_is_fail_closed_even_with_https_url"),
    (
        "crates/realtime/src/alerts/storage/tests.rs",
        "sqlite_snapshot_restores_config_delivery_and_cooldown_without_stale_prewarm",
    ),
    ("crates/realtime/src/alerts/storage/tests.rs", "corrupted_snapshot_hash_blocks_restore_and_future_mutation"),
    ("crates/realtime/src/alerts/storage/tests.rs", "schema_identity_drift_blocks_restore_and_future_mutation"),
    ("crates/realtime/src/alerts/tests/cases.rs", "toast_queue_requires_real_subscriber_before_cooldown_and_count"),
    ("crates/api/src/routers/watchlist/tests.rs", "unavailable_durable_storage_rolls_back_watchlist_mutation"),
    ("crates/api/src/state.rs", "app_state_restores_watchlist_alert_snapshot_and_active_cooldown"),
    (
        "crates/api/src/services/runtime_state/tests.rs",
        "persisted_watchlist_state_is_not_reported_as_volatile",
    ),
    (
        "crates/api/src/services/venue_operation_health/snapshot/tests/part_13.rs",
        "snapshot_exposes_watchlist_storage_and_bounded_prewarm_evidence",
    ),
    (
        "crates/api/src/lifecycle/market_data_tests/watchlist_runtime.rs",
        "watchlist_runtime_plan_is_bounded_visible_and_private_ws_free",
    ),
    (
        "frontend/src/panels/modules/settings/tabs/diagnostics/watchlist_alerts/tests.rs",
        "runtime_labels_distinguish_queued_blocked_and_risk_alert_channels",
    ),
)


def fail(message: str) -> None:
    raise SystemExit(f"PR-DP completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be completed with no local remainder")
if row.count(verify_anchor) != 1:
    fail("roadmap verification must name the destructive completion gate")
queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-DP\b", queue):
    fail("completed PR-DP remains in the local queue")
upstream = "PR-FL Watchlist Alerts State, Prewarm Side-Effect & Notification Contract"
upstream_row = next((line for line in doc.splitlines() if line.startswith(f"| `{upstream}`")), None)
if upstream_row is None or "✅ 完成" not in upstream_row:
    fail("upstream PR-FL completion contract drifted")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-DP"]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for kind, artifact in evidence_contract.items():
    if indexed[kind]["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"{kind} artifact drifted or is missing")
    if not indexed[kind]["command"].strip():
        fail(f"{kind} lacks verification command")

markers = {
    ".env.example": (
        "APP_STORAGE__WATCHLIST_ALERTS_PATH=watchlist_alerts.sqlite",
    ),
    "crates/common/src/config.rs": (
        "DEFAULT_WATCHLIST_ALERTS_FILE",
        "watchlist_alerts_path",
    ),
    "crates/realtime/migrations/20260714_watchlist_alerts.sql": (
        "singleton INTEGER PRIMARY KEY CHECK (singleton = 1)",
        "schema_version INTEGER NOT NULL",
        "revision INTEGER NOT NULL CHECK (revision >= 0)",
        "payload_hash TEXT NOT NULL",
    ),
    "crates/realtime/src/alerts/storage.rs": (
        "tokio::task::spawn_blocking",
        "verify_or_initialize_schema_identity",
        "payload_hash(&payload)",
        "load_blocked",
        "WatchlistPersistStatus::Persisted",
    ),
    "crates/api/src/services/watchlist_alerts.rs": (
        "WATCHLIST_STORAGE_UNAVAILABLE",
        "persist_rows",
        "snapshot-then-commit",
        "watchlist_alert_mutation_lock",
    ),
    "crates/api/src/state.rs": (
        "init_watchlist_alert_runtime",
        "alert_cooldowns_from_replay",
        "watchlist_alert_mutation_lock",
    ),
    "crates/api/src/services/venue_operation_health/snapshot/part_19.rs": (
        "OP_STORAGE_WATCHLIST_ALERTS",
        "OP_WATCHLIST_PREWARM",
        "private_ws_symbols=0",
    ),
    "frontend/src/panels/modules/settings/tabs/diagnostics/watchlist_alerts.rs": (
        "storage_summary",
        "persistence_label",
        "delivery_status_label",
    ),
    "frontend/src/panels/modules/settings/tabs/diagnostics/watchlist_alerts/format.rs": (
        "SQLite rev",
        "config_source_label",
        "delivery_status_label",
        "persist_status_label",
    ),
}
for relative_path, required in markers.items():
    source = (root / relative_path).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"missing {relative_path} marker: {marker}")

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

browser = (root / browser_path).read_text(encoding="utf-8")
for browser_title in browser_titles:
    escaped = re.escape(browser_title)
    if re.search(rf"(?m)^\s*test\.(?:skip|fixme)\(\s*['\"]{escaped}['\"]", browser):
        fail(f"browser anchor must not be skipped: {browser_title}")
    if not re.search(rf"(?m)^\s*test\(\s*['\"]{escaped}['\"]", browser):
        fail(f"missing runnable browser anchor: {browser_title}")

package = json.loads((root / "package.json").read_text(encoding="utf-8"))
if package.get("scripts", {}).get("test:e2e:pr-dp") != (
    "playwright test test/e2e/watchlist_alerts_runtime.spec.ts"
):
    fail("dedicated PR-DP browser command drifted")

history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(encoding="utf-8")
if "## 2026-07-14 PR-DP Watchlist Alert Persistence Closure" not in history:
    fail("PR-DP history appendix is missing")
for artifact in set(evidence_contract.values()):
    if artifact not in history:
        fail(f"PR-DP history appendix lacks closure path: {artifact}")

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
    f"OK PR-DP contract ({len(evidence_contract)} evidence types; "
    f"{len(test_anchors)} Rust tests; {len(browser_titles)} browser tests; "
    f"{len(closure_paths)} exact paths)"
)
PY

bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"
bash "$ROOT/scripts/check_pr_fl_completion.sh"

if [[ "${PR_DP_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test \
    --manifest-path "$ROOT/Cargo.toml" -p shared-types alerts:: --lib --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test \
    --manifest-path "$ROOT/Cargo.toml" -p realtime alerts:: --lib --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test \
    --manifest-path "$ROOT/Cargo.toml" -p api --bin crypto-arb-api watchlist --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test \
    --manifest-path "$ROOT/frontend/Cargo.toml" --lib watchlist_alert --no-fail-fast
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-dp -- --workers=1
fi

printf 'OK PR-DP watchlist alert persistence and runtime status contract\n'
