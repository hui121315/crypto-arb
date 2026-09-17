#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
DOC="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
EVIDENCE="$ROOT/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
ENV_TEMPLATE="$ROOT/.env.example"
CONFIG="$ROOT/crates/common/src/config.rs"
MAIN="$ROOT/crates/api/src/main.rs"
DRAIN="$ROOT/crates/api/src/lifecycle/drain.rs"
AUDIT="$ROOT/crates/api/src/middleware/audit.rs"
WRITER="$ROOT/crates/api/src/middleware/audit/writer.rs"
AUTH="$ROOT/crates/api/src/middleware/auth.rs"
ACTION_AUDIT="$ROOT/crates/api/src/services/action_runs/audit_log.rs"
PRIVATE_WS="$ROOT/crates/api/src/trading_service/private_ws_events/apply.rs"
PRIVATE_WS_LIFECYCLE="$ROOT/crates/api/src/lifecycle/private_ws/apply.rs"
WS_PUBLISH="$ROOT/crates/api/src/services/ws_publish.rs"
CLOSE_EVENTS="$ROOT/crates/api/src/services/close_runs/project/events.rs"
CLOSE_ACTION="$ROOT/crates/api/src/services/close_runs/action.rs"
PROJECTION_TEST="$ROOT/crates/api/src/lifecycle/private_ws/tests/projection/close_run.rs"
CLOSE_TESTS="$ROOT/crates/api/src/services/close_runs/tests"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-GA completion gate failed: %s\n' "$1" >&2
  exit 1
}

must_match() {
  local label="$1"
  local pattern="$2"
  shift 2
  if ! rg -n -- "$pattern" "$@" >/dev/null; then
    fail "$label"
  fi
}

must_not_match() {
  local label="$1"
  local pattern="$2"
  shift 2
  if rg -n -- "$pattern" "$@" >/dev/null; then
    fail "$label"
  fi
}

require_evidence() {
  local evidence_type="$1"
  if ! awk -F '\t' -v evidence_type="$evidence_type" '
    $1 == "PR-GA" && $2 == evidence_type { found = 1 }
    END { exit(found ? 0 : 1) }
  ' "$EVIDENCE"; then
    fail "missing PR-GA evidence type $evidence_type"
  fi
}

if [[ "$MODE" == "--self-test" ]]; then
  env_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-ga-env.XXXXXX")"
  drain_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-ga-drain.XXXXXX")"
  cp "$ENV_TEMPLATE" "$env_backup"
  cp "$DRAIN" "$drain_backup"
  restore() {
    cp "$env_backup" "$ENV_TEMPLATE"
    cp "$drain_backup" "$DRAIN"
    rm -f "$env_backup" "$drain_backup"
  }
  trap restore EXIT

  printf '\nAPP_ORDER_AUDIT_PATH=legacy.jsonl\n' >>"$ENV_TEMPLATE"
  if bash "$0"; then
    fail "self-test unexpectedly accepted the retired audit environment key"
  fi

  cp "$env_backup" "$ENV_TEMPLATE"
  python3 - "$DRAIN" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "crate::middleware::audit::shutdown"
if source.count(marker) != 1:
    raise SystemExit("PR-GA self-test setup failed: audit drain marker drifted")
path.write_text(source.replace(marker, "crate::middleware::audit::shutdown_removed", 1), encoding="utf-8")
PY
  if bash "$0"; then
    fail "self-test unexpectedly accepted a disconnected audit shutdown drain"
  fi

  printf 'PR-GA completion self-test passed\n'
  exit 0
fi

must_match \
  "environment template lacks the authoritative audit path" \
  '^APP_SECURITY__AUDIT_LOG_PATH=' \
  "$ENV_TEMPLATE"
must_not_match \
  "retired order audit environment key is still exposed" \
  'APP_ORDER_AUDIT_PATH' \
  "$ENV_TEMPLATE"
must_match \
  "security config no longer owns the audit path" \
  'pub audit_log_path: Option<String>' \
  "$CONFIG"
must_match \
  "audit startup is not wired to the security configuration" \
  'audit::init\(config\.security\.audit_log_path\.as_deref\(\)\)' \
  "$MAIN"
must_match \
  "process shutdown no longer invokes the typed runtime drain" \
  'lifecycle::drain_runtime\(&state, &mut tasks\)' \
  "$MAIN"
must_match \
  "typed runtime drain no longer enforces audit writer shutdown" \
  'crate::middleware::audit::shutdown\(AUDIT_DRAIN_BUDGET\)' \
  "$DRAIN"
must_match \
  "audit actor is not HMAC-derived" \
  'hmac_sha256_hex' \
  "$AUDIT"
must_match \
  "audit actor does not support a trusted server-side label" \
  'actor_label' \
  "$AUDIT" \
  "$AUTH" \
  "$CONFIG"
must_match \
  "auth middleware does not clear client-forged actor context" \
  'audit::clear_verified_actor\(request\.headers_mut\(\)\)' \
  "$AUTH"
must_match \
  "auth middleware does not inject actor only after bearer verification" \
  'audit::insert_verified_bearer_actor' \
  "$AUTH"
must_match \
  "audit writer is not bounded and asynchronous" \
  'sync_channel' \
  "$WRITER"
must_match \
  "audit writer no longer durably acknowledges high-risk actions" \
  'write_durable' \
  "$WRITER"
must_match \
  "audit writer does not synchronize durable records" \
  'sync_data' \
  "$WRITER"
must_match \
  "audit health does not expose writer liveness" \
  'writer_alive' \
  "$AUDIT"
must_match \
  "ActionRun audit no longer waits for durable acknowledgement" \
  'audit::record_durable' \
  "$ACTION_AUDIT"
must_match \
  "ActionRun audit detail lost identity correlation" \
  '"actionRunId"|"requestId"|"idempotencyKey"' \
  "$ACTION_AUDIT"
must_match \
  "private WS cancel does not record finality proof" \
  'record_private_ws_cancel_finality_from_record' \
  "$PRIVATE_WS"
must_match \
  "private WS non-user cancel is not handled" \
  'PrivateWsEvent::NonUserCancel' \
  "$PRIVATE_WS"
must_match \
  "private WS lifecycle does not persist before publishing ledger projections" \
  'persist_then_publish_ledger_projected_runs' \
  "$PRIVATE_WS_LIFECYCLE"
must_match \
  "order publication no longer reaches CloseRun projection" \
  'project_order_update' \
  "$WS_PUBLISH"
must_match \
  "CloseRun finality is not appended from order evidence" \
  'append_close_run_finality_from_order' \
  "$CLOSE_EVENTS"
must_match \
  "CloseRun no longer refreshes its linked ActionRun payload" \
  'action_runs::finish_status_with_payload' \
  "$CLOSE_ACTION"
must_match \
  "private WS cancel to CloseRun regression is missing" \
  'private_ws_cancel_projects_close_run_failure_once_after_apply_ack' \
  "$PROJECTION_TEST"
must_match \
  "CloseRun durable replay regression is missing" \
  'close_cost_facts_replay_after_restart' \
  "$CLOSE_TESTS"
must_match \
  "CloseRun ActionRun terminal refresh regression is missing" \
  'compensation_order_finality_refreshes_close_run_and_action_payload' \
  "$CLOSE_TESTS"

roadmap_row="$(rg -F '| `PR-GA High-Risk Mutation Audit, RequestId & Actor Evidence Contract`' "$DOC" || true)"
[[ "$roadmap_row" == *"✅ 完成"* ]] || fail "roadmap row is not completed"
[[ "$roadmap_row" == *"剩余：无。"* ]] || fail "roadmap row still reports a remainder"
[[ "$roadmap_row" == *"check_pr_ga_completion.sh"* ]] || fail "roadmap row lacks the completion gate"
if rg -q '^1\. \*\*PR-GA ' "$DOC"; then
  fail "completed PR-GA remains at the queue head"
fi

for evidence_type in \
  audit-config-runtime \
  trusted-actor-contract \
  async-durable-writer \
  private-ws-cancel-close-run \
  action-run-close-run-linkage \
  completion-governance-gate; do
  require_evidence "$evidence_type"
done

CARGO_BUILD_JOBS=8 cargo test -p api --bin crypto-arb-api audit --no-fail-fast
CARGO_BUILD_JOBS=8 cargo test -p api --bin crypto-arb-api private_ws_cancel_projects_close_run_failure_once_after_apply_ack --no-fail-fast
CARGO_BUILD_JOBS=8 cargo test -p api --bin crypto-arb-api close_cost_facts_replay_after_restart --no-fail-fast
CARGO_BUILD_JOBS=8 cargo test -p api --bin crypto-arb-api compensation_order_finality_refreshes_close_run_and_action_payload --no-fail-fast
bash "$ROOT/scripts/verify_api_security_runtime_smoke.sh"

printf 'OK PR-GA high-risk mutation audit completion gate\n'
