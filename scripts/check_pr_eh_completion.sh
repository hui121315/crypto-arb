#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
DOC="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
EVIDENCE="$ROOT/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
WS="$ROOT/frontend/src/api/ws.rs"
WS_RUNTIME="$ROOT/frontend/src/api/ws_runtime.rs"
ARBITRAGE="$ROOT/frontend/src/state/arbitrage_stream.rs"
POLLING="$ROOT/frontend/src/state/polling.rs"
CHANNEL_UI="$ROOT/frontend/src/panels/shared/ws_channel.rs"
REPO_GATE="$ROOT/scripts/verify_repo_gates.sh"
MOCK_API="$ROOT/test/e2e/mock_api.mjs"
DATA_PIPELINE="$ROOT/test/e2e/data_pipeline.spec.ts"
WATCHLIST_BROWSER="$ROOT/test/e2e/watchlist_alerts_runtime.spec.ts"
WATCHLIST_UI="$ROOT/frontend/src/panels/modules/settings/tabs/diagnostics/watchlist_alerts.rs"
WATCHLIST_FORMAT="$ROOT/frontend/src/panels/modules/settings/tabs/diagnostics/watchlist_alerts/format.rs"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-EH completion gate failed: %s\n' "$1" >&2
  exit 1
}

must_match() {
  local label="$1"
  local pattern="$2"
  shift 2
  rg -n -- "$pattern" "$@" >/dev/null || fail "$label"
}

must_not_match() {
  local label="$1"
  local pattern="$2"
  local status
  shift 2

  set +e
  rg -n -- "$pattern" "$@" >/dev/null
  status=$?
  set -e

  case "$status" in
    0) fail "$label" ;;
    1) ;;
    *) fail "$label search failed" ;;
  esac
}

require_evidence() {
  local evidence_type="$1"
  awk -F '\t' -v evidence_type="$evidence_type" '
    $1 == "PR-EH" && $2 == evidence_type { found = 1 }
    END { exit(found ? 0 : 1) }
  ' "$EVIDENCE" || fail "missing PR-EH evidence type $evidence_type"
}

if [[ "$MODE" == "--self-test" ]]; then
  backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-eh-ws-channel.XXXXXX")"
  cp "$CHANNEL_UI" "$backup"
  restore() {
    cp "$backup" "$CHANNEL_UI"
    rm -f "$backup"
  }
  trap restore EXIT

  rm "$CHANNEL_UI"
  if PR_EH_SKIP_TESTS=1 bash "$0"; then
    fail "self-test unexpectedly accepted missing channel activity UI"
  fi

  printf 'PR-EH completion self-test passed\n'
  exit 0
fi

[[ -s "$CHANNEL_UI" ]] || fail "shared channel activity UI is missing"
must_match "WsChannelState lacks message counter" 'pub message_count: u64' "$WS"
must_match "WsChannelState lacks problem counter" 'pub problem_count: u64' "$WS"
must_match "WsChannelState lacks last problem timestamp" 'pub last_problem_at_ms: Option<u64>' "$WS"
must_match "typed decoder does not return ApiProblem to runtime" 'Result<\(\), ApiProblem>' "$WS"
must_match "runtime does not persist payload decoder failures" 'runtime_payload_problem_persists_for_late_subscriber' "$WS_RUNTIME"
must_match "runtime does not count successful payload once" 'runtime_successful_payload_is_counted_once' "$WS_RUNTIME"
must_match "inactive retained channels do not prove resubscription" 'inactive_channel_with_retained_state_resubscribes' "$WS_RUNTIME"
must_match "WS auth ticket does not preserve typed problem context" 'ws_auth_ticket_problem' "$WS_RUNTIME"
must_match "WS command write failure is not typed" 'WS_WRITE_ERROR' "$WS_RUNTIME"
must_match "arbitrage REST fallback does not preserve WS problem in LoadState" 'preserved_problem_after_event' "$ARBITRAGE"
must_match "shared channel activity label is missing" 'ws_channel_activity_label' "$CHANNEL_UI"
must_match "repo async state gate is missing" 'run_frontend_async_state_gates' "$REPO_GATE"
must_match "browser WS error fixture lacks request id" 'requestId: "e2e-ws-error"' "$MOCK_API"
must_match "browser WS error does not assert channel counters" '帧 0 · 错误 1' "$DATA_PIPELINE"
must_match "watchlist browser does not assert channel counters" 'watchlist 已订阅 · 帧 1 · 错误 0' "$WATCHLIST_BROWSER"
must_match "watchlist channel summary is not wired into diagnostics" 'transport_summary' "$WATCHLIST_UI"

must_not_match "parallel RwSignal<WsStatus> remains" 'RwSignal<WsStatus>' "$ROOT/frontend/src"
must_not_match "optional WsChannelState allows hidden channel health" 'Option<RwSignal<WsChannelState>>' "$ROOT/frontend/src"
must_not_match "arbitrage stream mirrors LoadState" 'pub\s+(response|problem|ws_status)\s*:' "$ARBITRAGE"
must_not_match "legacy status-only WS starter remains" 'pub fn start_[a-z_]+_stream\(' "$WS"
must_not_match "obsolete status-only polling remains" 'use_ws_fallback_polling' "$POLLING"
must_not_match "frontend async result is discarded" 'let\s+_\s*=.*\.await' "$ROOT/frontend/src"
must_not_match "frontend async failure is reduced to is_ok/is_err" '\.await\.is_(ok|err)\(' "$ROOT/frontend/src"

for surface in \
  "$ROOT/frontend/src/panels/status_bar/slots/app_ws.rs" \
  "$ROOT/frontend/src/panels/modules/positions/components/snapshot_transport.rs" \
  "$ROOT/frontend/src/panels/shared/orders_list/labels.rs" \
  "$ROOT/frontend/src/panels/modules/execution/components/execution_status_bar/state.rs" \
  "$WATCHLIST_FORMAT"; do
  must_match "channel counters are not visible in ${surface#$ROOT/}" '(ws_channel_activity_label|传输事件)' "$surface"
done

for evidence_type in \
  loadstate-single-source \
  ws-channel-state-contract \
  ws-runtime-error-propagation \
  ws-auth-write-problem \
  channel-consumer-counters \
  async-anti-swallow-governance \
  browser-channel-fixtures \
  completion-governance; do
  require_evidence "$evidence_type"
done

roadmap_row="$(rg -F '| `PR-EH ApiProblem RequestId & Frontend LoadState Contract`' "$DOC" || true)"
[[ "$roadmap_row" == *"✅ 完成"* ]] || fail "roadmap row is not completed"
[[ "$roadmap_row" == *"剩余：无。"* ]] || fail "roadmap row still reports a remainder"
queue_section="$(sed -n '/^### 🟡 6\.5/,/^## /p' "$DOC")"
[[ "$queue_section" != *'**PR-EH ApiProblem RequestId'* ]] || fail "completed PR-EH remains in the queue"

if [[ "${PR_EH_SKIP_TESTS:-0}" == "1" ]]; then
  printf 'OK PR-EH static completion contract\n'
  exit 0
fi

JOBS="${CARGO_BUILD_JOBS:-8}"
CARGO_BUILD_JOBS="$JOBS" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib ws_runtime --no-fail-fast
CARGO_BUILD_JOBS="$JOBS" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib arbitrage_stream --no-fail-fast
CARGO_BUILD_JOBS="$JOBS" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib ws_channel --no-fail-fast
CARGO_BUILD_JOBS="$JOBS" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib snapshot_transport --no-fail-fast
CARGO_BUILD_JOBS="$JOBS" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib status_bar --no-fail-fast
CARGO_BUILD_JOBS="$JOBS" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib watchlist_alerts --no-fail-fast
CI=1 npx playwright test "$ROOT/test/e2e/data_pipeline.spec.ts" --grep "opportunities page surfaces websocket" --reporter=list
CI=1 npx playwright test "$WATCHLIST_BROWSER" --reporter=list
VERIFY_REPO_GATES_SCOPE=debt bash "$REPO_GATE"

printf 'OK PR-EH ApiProblem, LoadState and WS channel completion contract\n'
