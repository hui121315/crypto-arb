#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
DOC="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
EVIDENCE="$ROOT/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
ACTIONS="$ROOT/shared-types/src/actions.rs"
STATUS="$ROOT/shared-types/src/live_trading.rs"
TRADING="$ROOT/crates/api/src/routers/trading.rs"
ACTION_MUTATE="$ROOT/crates/api/src/services/action_runs/mutate.rs"
ACTION_AUDIT="$ROOT/crates/api/src/services/action_runs/audit_log.rs"
AUDIT_REPLAY="$ROOT/crates/api/src/middleware/audit/replay.rs"
ACTION_TESTS="$ROOT/crates/api/src/routers/trading/tests/cases_action_runs.rs"
RISK_TESTS="$ROOT/crates/api/src/routers/trading/tests/cases_risk_config.rs"
KILL_TESTS="$ROOT/crates/api/src/routers/trading/tests/cases_kill.rs"
FRONTEND_ACTIONS="$ROOT/frontend/src/panels/modules/settings/data/actions.rs"
FRONTEND_LABELS="$ROOT/frontend/src/panels/modules/settings/tabs/action_runs/labels.rs"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-EI completion gate failed: %s\n' "$1" >&2
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
  shift 2
  local status
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
    $1 == "PR-EI" && $2 == evidence_type { found = 1 }
    END { exit(found ? 0 : 1) }
  ' "$EVIDENCE" || fail "missing PR-EI evidence type $evidence_type"
}

if [[ "$MODE" == "--self-test" ]]; then
  backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-ei-actions.XXXXXX")"
  cp "$ACTIONS" "$backup"
  restore() {
    cp "$backup" "$ACTIONS"
    rm -f "$backup"
  }
  trap restore EXIT

  perl -0pi -e 's/pub struct ActionMutationDiff/pub struct RemovedActionMutationDiff/' "$ACTIONS"
  if PR_EI_SKIP_TESTS=1 bash "$0"; then
    fail "self-test unexpectedly accepted a missing typed mutation diff"
  fi
  printf 'PR-EI completion self-test passed\n'
  exit 0
fi

must_match "typed action mutation diff is missing" 'pub struct ActionMutationDiff' "$ACTIONS"
must_match "action mutation fields are not enum constrained" 'pub enum ActionMutationChange' "$ACTIONS"
must_match "ActionRun does not persist mutation evidence" 'pub mutation: Option<ActionMutationDiff>' "$ACTIONS"
mutation_enum="$(sed -n '/pub enum ActionMutationChange/,/^}/p' "$ACTIONS")"
if [[ "$mutation_enum" =~ Credential|Secret|ApiKey|Passphrase|PrivateKey ]]; then
  fail "credential or secret values can enter the typed mutation enum"
fi
must_match "trading mutation response lacks action receipt" 'pub action_run_id: Option<String>' "$STATUS"
must_match "trading mutation response lacks request correlation" 'pub request_id: Option<String>' "$STATUS"
must_match "trading mutation response lacks idempotency correlation" 'pub idempotency_key: Option<String>' "$STATUS"
must_match "trading mutation response lacks old/new diff" 'pub mutation: Option<crate::actions::ActionMutationDiff>' "$STATUS"
must_match "trading status diff builder is missing" 'fn trading_mutation_diff' \
  "$ROOT/crates/api/src/routers/trading/mutation.rs"
must_match "trading action receipt builder is missing" 'fn attach_action_receipt' \
  "$ROOT/crates/api/src/routers/trading/mutation.rs"
must_match "terminal action update does not persist mutation" 'finish_result_with_payload_and_mutation' "$ACTION_MUTATE"
must_match "durable audit does not retain mutation" '"mutation": run.mutation' "$ACTION_AUDIT"
must_match "restart replay does not prove mutation retention" \
  'replay_turns_interrupted_accepted_run_into_fail_closed_terminal' "$AUDIT_REPLAY"
must_match "risk mutation diff route test is missing" \
  'risk_config_action_run_detail_restores_status_payload' "$RISK_TESTS"
must_match "adapter idempotency replay test is missing" \
  'adapter_select_replays_explicit_idempotency_key_without_second_action_run' "$ACTION_TESTS"
must_match "kill switch old/new test is missing" \
  'ActionMutationChange::KillSwitchActive' "$KILL_TESTS"
must_match "risk config UI does not reuse a business idempotency key" \
  'risk_config_replay_slot' "$FRONTEND_ACTIONS"
must_match "Settings ActionRun detail does not display old/new diff" \
  'pub\(super\) fn mutation_detail' "$FRONTEND_LABELS"
must_match "public submit does not force a business idempotency key" \
  'clientOrderId is required as the public idempotency key' \
  "$ROOT/crates/api/src/routers/trading/types.rs"
must_match "hedge confirm replay contract is missing" \
  'confirm_replay_with_hot_run_preserves_stored_partial_outcome' \
  "$ROOT/crates/api/src/routers/arbitrage/hedge_tests/pricing_confirm_partial_outcome.rs"
must_match "cancel replay key is missing" 'fn cancel_idempotency_key' \
  "$ROOT/crates/api/src/routers/trading/order_replay.rs"
must_match "portfolio close replay key is missing" 'close_action_idempotency_key' \
  "$ROOT/crates/api/src/routers/portfolio.rs"
must_match "credential replay contract is missing" \
  'update_credentials_replays_same_request_without_new_action_run' \
  "$ROOT/crates/api/src/routers/exchanges.rs"
must_match "close finality does not update ActionRun payload" \
  'close_order_finality_updates_action_run_payload' "$ROOT/crates/api/src/routers/portfolio.rs"
must_match "compensation finality does not update CloseRun and ActionRun" \
  'compensation_order_finality_refreshes_close_run_and_action_payload' \
  "$ROOT/crates/api/src/services/close_runs/tests/cases_b/finality.rs"
must_match "route policy matrix does not lock ActionRun kinds" \
  'route_inventory_action_run_policies_match_typed_runtime_registry' "$ROOT/crates/api/src/app.rs"

for evidence_type in \
  typed-mutation-diff \
  mutation-http-receipt \
  durable-mutation-audit-replay \
  idempotent-risk-adapter \
  core-live-idempotency \
  close-cancel-finality \
  frontend-action-diff \
  completion-governance; do
  require_evidence "$evidence_type"
done

roadmap_row="$(rg -F '| `PR-EI Mutation Audit, Idempotency & ActionRun Contract`' "$DOC" || true)"
[[ "$roadmap_row" == *"✅ 完成"* ]] || fail "roadmap row is not completed"
[[ "$roadmap_row" == *"剩余：无。"* ]] || fail "roadmap row still reports a remainder"
queue_section="$(sed -n '/^### 🟡 6\.5/,/^## /p' "$DOC")"
[[ "$queue_section" != *'**PR-EI Mutation Audit'* ]] || fail "completed PR-EI remains in the queue"

if [[ "${PR_EI_SKIP_TESTS:-0}" == "1" ]]; then
  printf 'OK PR-EI static completion contract\n'
  exit 0
fi

JOBS="${CARGO_BUILD_JOBS:-8}"
CARGO_BUILD_JOBS="$JOBS" cargo test -p shared-types action_mutation --lib --no-fail-fast
CARGO_BUILD_JOBS="$JOBS" cargo test -p api --bin crypto-arb-api action_run --no-fail-fast
CARGO_BUILD_JOBS="$JOBS" cargo test -p api --bin crypto-arb-api kill_switch --no-fail-fast
CARGO_BUILD_JOBS="$JOBS" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib risk_config --no-fail-fast
CARGO_BUILD_JOBS="$JOBS" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib action_mutation --no-fail-fast
bash "$ROOT/scripts/check_mutation_audit_contract.sh"

printf 'OK PR-EI mutation audit, idempotency, ActionRun and finality completion contract\n'
