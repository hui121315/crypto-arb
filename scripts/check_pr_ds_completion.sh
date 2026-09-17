#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
REPLAY="$ROOT/crates/api/src/services/ws_replay.rs"
WEBSOCKET="$ROOT/crates/api/src/routers/websocket.rs"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

if [[ "$MODE" == "--self-test" ]]; then
  PR_DS_SKIP_TESTS=1 bash "$0" >/dev/null
  replay_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-ds-replay.XXXXXX")"
  websocket_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-ds-websocket.XXXXXX")"
  cp "$REPLAY" "$replay_backup"
  cp "$WEBSOCKET" "$websocket_backup"
  restore() {
    cp "$replay_backup" "$REPLAY"
    cp "$websocket_backup" "$WEBSOCKET"
    rm -f "$replay_backup" "$websocket_backup"
  }
  trap restore EXIT

  python3 - "$REPLAY" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "WsChannelReplaySource::FundingRatesSnapshot => funding_rate_payloads(state),"
if source.count(marker) != 1:
    raise SystemExit("PR-DS self-test setup failed: funding replay marker drifted")
path.write_text(source.replace(marker, "WsChannelReplaySource::FundingRatesSnapshot => Ok(Vec::new()),", 1), encoding="utf-8")
PY
  if PR_DS_SKIP_TESTS=1 bash "$0" >/dev/null 2>&1; then
    printf 'PR-DS completion self-test failed: disconnected funding replay passed\n' >&2
    exit 1
  fi

  cp "$replay_backup" "$REPLAY"
  python3 - "$WEBSOCKET" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "hub.record_lag(&channel, skipped, common::time::now_ms());"
if source.count(marker) != 1:
    raise SystemExit("PR-DS self-test setup failed: lag runtime marker drifted")
path.write_text(source.replace(marker, "let _ = (&hub, skipped);", 1), encoding="utf-8")
PY
  if PR_DS_SKIP_TESTS=1 bash "$0" >/dev/null 2>&1; then
    printf 'PR-DS completion self-test failed: disconnected lag runtime passed\n' >&2
    exit 1
  fi

  printf 'PR-DS completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from __future__ import annotations

import csv
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])
title = "PR-DS Realtime WS Multiplexer & Replay Contract"
evidence_contract = {
    "product-channel-registry": "crates/realtime/src/channels.rs",
    "channel-replay-dispatch": "crates/api/src/services/ws_replay.rs",
    "replay-source-fixtures": "crates/api/src/services/ws_replay/tests/replay_sources.rs",
    "lag-runtime-counters": "crates/realtime/src/hub.rs",
    "lag-forwarder-contract": "crates/api/src/routers/websocket.rs",
    "lag-operation-health": "crates/api/src/services/venue_operation_health/snapshot/part_20.rs",
    "shared-operation-kind": "shared-types/src/venues/operation_kind.rs",
    "status-bar-lag-evidence": "frontend/src/panels/status_bar/slots/app_ws.rs",
    "settings-lag-diagnostics": "frontend/src/panels/modules/settings/tabs/diagnostics/ws_rtt.rs",
    "browser-runtime-evidence": "test/e2e/pr_ds_ws_runtime.spec.ts",
    "completion-governance": "scripts/check_pr_ds_completion.sh",
}
closure_paths = (
    "crates/realtime/src/channels.rs",
    "crates/realtime/src/hub.rs",
    "crates/realtime/src/lib.rs",
    "crates/api/src/routers/websocket.rs",
    "crates/api/src/services/ws_replay.rs",
    "crates/api/src/services/ws_replay/tests.rs",
    "crates/api/src/services/ws_replay/tests/replay_sources.rs",
    "crates/api/src/services/venue_operation_health/snapshot.rs",
    "crates/api/src/services/venue_operation_health/snapshot/part_01.rs",
    "crates/api/src/services/venue_operation_health/snapshot/part_20.rs",
    "crates/api/src/services/venue_operation_health/snapshot/tests.rs",
    "crates/api/src/services/venue_operation_health/snapshot/tests/part_14.rs",
    "shared-types/src/lib.rs",
    "shared-types/src/venues/operation.rs",
    "shared-types/src/venues/operation_kind.rs",
    "shared-types/src/venues/operation_kind_labels.rs",
    "shared-types/src/venues/runtime_health_snapshot.rs",
    "shared-types/src/venues/tests_operation_kind.rs",
    "frontend/src/panels/status_bar/slots/app_ws.rs",
    "frontend/src/panels/status_bar/slots/tests/cases_ws.rs",
    "frontend/src/panels/status_bar/view.rs",
    "frontend/src/panels/modules/settings/tabs/diagnostics/ws_rtt.rs",
    "frontend/src/panels/modules/settings/tabs/diagnostics/tests/ws_rtt.rs",
    "test/e2e/pr_ds_ws_runtime.spec.ts",
    "scripts/check_pr_ds_completion.sh",
    "scripts/check_release_qa_contract.sh",
    "scripts/verify_repo_gates.sh",
)
test_anchors = (
    ("crates/realtime/src/channels.rs", "product_channel_registry_is_unique_authenticated_and_replayable"),
    ("crates/realtime/src/channels.rs", "legacy_dynamic_topics_are_not_product_channels_without_live_publishers"),
    ("crates/realtime/src/hub.rs", "runtime_snapshot_accumulates_lag_and_keeps_active_zero_lag_channels"),
    ("crates/api/src/services/ws_replay/tests.rs", "execution_channel_replays_recent_runs"),
    ("crates/api/src/services/ws_replay/tests.rs", "arbitrage_channel_replays_warming_opportunity_envelope"),
    ("crates/api/src/services/ws_replay/tests.rs", "orders_channel_replays_recent_order_records"),
    ("crates/api/src/services/ws_replay/tests.rs", "portfolio_channel_replays_latest_cached_snapshot"),
    ("crates/api/src/services/ws_replay/tests.rs", "system_channel_replays_current_health_snapshot"),
    ("crates/api/src/services/ws_replay/tests.rs", "risk_alerts_channel_replays_current_risk_snapshot"),
    ("crates/api/src/services/ws_replay/tests/watchlist_alerts.rs", "watchlist_and_alert_channels_replay_current_volatile_state"),
    ("crates/api/src/services/ws_replay/tests/replay_sources.rs", "funding_rates_channel_replays_latest_cached_envelope"),
    ("crates/api/src/services/ws_replay/tests/replay_sources.rs", "every_registered_channel_has_an_explicit_replay_source"),
    ("crates/api/src/routers/websocket.rs", "unknown_channel_is_rejected"),
    ("crates/api/src/routers/websocket.rs", "lagged_broadcast_sends_typed_error_and_keeps_forwarder_alive"),
    ("crates/api/src/services/venue_operation_health/snapshot/tests/part_14.rs", "snapshot_exposes_app_ws_lag_counts_by_channel"),
    ("crates/api/src/services/venue_operation_health/snapshot/tests/part_14.rs", "old_app_ws_lag_keeps_cumulative_counts_without_current_warning"),
    ("frontend/src/panels/status_bar/slots/tests/cases_ws.rs", "app_ws_surfaces_backend_broadcast_lag_counts"),
    ("frontend/src/panels/modules/settings/tabs/diagnostics/tests/ws_rtt.rs", "app_ws_rows_and_scope_summary_keep_lag_counts_visible"),
)


def fail(message: str) -> None:
    raise SystemExit(f"PR-DS completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be completed with no local remainder")
if "scripts/check_pr_ds_completion.sh --self-test" not in row:
    fail("roadmap verification must name the destructive completion gate")
queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-DS\b", queue):
    fail("completed PR-DS remains in the local queue")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-DS"]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for kind, artifact in evidence_contract.items():
    row = indexed[kind]
    if row["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"{kind} artifact drifted or is missing")
    if not row["command"].strip():
        fail(f"{kind} lacks a verification command")

markers = {
    "crates/realtime/src/channels.rs": (
        "pub const WS_CHANNEL_SPECS: [WsChannelSpec; 10]",
        "WsChannelReplaySource::FundingRatesSnapshot",
        "pub fn ws_channel_spec(channel: &str)",
    ),
    "crates/api/src/services/ws_replay.rs": (
        "let Some(spec) = channels::ws_channel_spec(channel)",
        "WsChannelReplaySource::FundingRatesSnapshot => funding_rate_payloads(state),",
        "market_data::envelope::funding_rates_envelope(",
    ),
    "crates/realtime/src/hub.rs": (
        "pub fn record_lag(&self, channel: &str, skipped_messages: u64, observed_at_ms: i64)",
        "pub fn runtime_snapshots(&self) -> Vec<WsChannelRuntimeSnapshot>",
        "counters.skipped_messages.saturating_add(skipped_messages)",
    ),
    "crates/api/src/routers/websocket.rs": (
        "realtime::channels::ws_channel_spec(channel).is_none()",
        "hub.record_lag(&channel, skipped, common::time::now_ms());",
        "ChannelDecision::Reject(\"unknown channel\")",
    ),
    "crates/api/src/services/venue_operation_health/snapshot/part_20.rs": (
        ".runtime_snapshots()",
        "requested: Some(snapshot.lag_events)",
        "rows: Some(snapshot.skipped_messages)",
        "WS_BROADCAST_LAGGED",
    ),
    "shared-types/src/venues/operation_kind.rs": (
        "Self::AppWsBroadcast",
        "operation.starts_with(OP_APP_WS_BROADCAST_PREFIX)",
        "VenueOperationClass::AppWs",
    ),
    "frontend/src/panels/status_bar/slots/app_ws.rs": (
        "app_ws_lag_summary(operation_health)",
        'format!("丢帧 {}", lag.recent_skipped_messages)',
        "累计丢帧 {}",
    ),
    "frontend/src/panels/modules/settings/tabs/diagnostics/ws_rtt.rs": (
        "app_ws_broadcast_rows(&snapshot.rows)",
        "AppWS channels {} · lag {} · 丢帧 {} · 近期异常 {}",
        "app_ws_broadcast",
    ),
}
for relative_path, required in markers.items():
    source = (root / relative_path).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"missing {relative_path} marker: {marker}")

channels = (root / "crates/realtime/src/channels.rs").read_text(encoding="utf-8")
if channels.count("authenticated: true") != 10:
    fail("every product channel must remain authenticated")
websocket = (root / "crates/api/src/routers/websocket.rs").read_text(encoding="utf-8")
if "parse_topic" in websocket or "Topic::Custom" in websocket:
    fail("legacy dynamic topics must not bypass the product channel registry")

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

browser = (root / "test/e2e/pr_ds_ws_runtime.spec.ts").read_text(encoding="utf-8")
browser_anchor = 'test("PR-DS surfaces app WS lag in the top status and Settings diagnostics"'
if browser_anchor not in browser or "test.skip" in browser or ".skip(" in browser:
    fail("non-skipping PR-DS browser anchor is missing")

package = (root / "package.json").read_text(encoding="utf-8")
if '"test:e2e:pr-ds": "playwright test test/e2e/pr_ds_ws_runtime.spec.ts"' not in package:
    fail("package.json lacks the dedicated PR-DS browser command")
if "test/e2e/pr_ds_ws_runtime.spec.ts" not in package.split('"test:e2e:product"', 1)[1]:
    fail("PR-DS browser fixture is not in the product suite")

repo_gate = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
gate_call = 'PR_DS_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_ds_completion.sh"'
if repo_gate.count(gate_call) < 2:
    fail("docs and full repo scopes must both invoke the PR-DS gate")
if 'test/e2e/pr_ds_ws_runtime.spec.ts" --list' not in repo_gate:
    fail("repo gate must statically enumerate the PR-DS browser fixture")

history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(encoding="utf-8")
if "## 2026-07-14 PR-DS Realtime Replay and Lag Closure" not in history:
    fail("PR-DS history appendix is missing")
for artifact in set(evidence_contract.values()):
    if artifact not in history:
        fail(f"PR-DS history appendix lacks closure path: {artifact}")

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
    f"OK PR-DS contract ({len(evidence_contract)} evidence types; "
    f"{len(test_anchors)} non-skipping Rust/Wasm tests; 1 browser fixture; "
    f"{len(closure_paths)} exact paths)"
)
PY

bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"

if [[ "${PR_DS_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-10}" cargo test \
    --manifest-path "$ROOT/Cargo.toml" -p realtime channels::tests --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-10}" cargo test \
    --manifest-path "$ROOT/Cargo.toml" -p realtime hub::tests --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-10}" cargo test \
    --manifest-path "$ROOT/Cargo.toml" -p api --bin crypto-arb-api \
    services::ws_replay::tests --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-10}" cargo test \
    --manifest-path "$ROOT/Cargo.toml" -p api --bin crypto-arb-api \
    routers::websocket::tests --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-10}" cargo test \
    --manifest-path "$ROOT/Cargo.toml" -p api --bin crypto-arb-api \
    app_ws_lag --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-10}" cargo test \
    --manifest-path "$ROOT/Cargo.toml" -p shared-types tests_operation_kind --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-10}" cargo test \
    --manifest-path "$ROOT/frontend/Cargo.toml" --lib app_ws --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-10}" cargo test \
    --manifest-path "$ROOT/frontend/Cargo.toml" --lib ws_rtt --no-fail-fast
  npm --prefix "$ROOT" run test:e2e:pr-ds
fi

printf 'OK PR-DS realtime WS replay and lag contract\n'
