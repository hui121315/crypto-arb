#!/usr/bin/env bash
set -euo pipefail

API_BASE="${API_BASE:-http://127.0.0.1:8000}"
CAPITAL_USD="${CAPITAL_USD:-10}"
MIN_ONE_CYCLE_NET_BPS="${MIN_ONE_CYCLE_NET_BPS:-25}"
MIN_SCORE="${MIN_SCORE:-50}"
DISCOVERY_TIMEOUT_SECS="${DISCOVERY_TIMEOUT_SECS:-180}"
FLOW_TIMEOUT_SECS="${FLOW_TIMEOUT_SECS:-60}"
ARTIFACT_DIR="${ARTIFACT_DIR:-$(mktemp -d "${TMPDIR:-/tmp}/crossline-auto-flow.XXXXXX")}"
KEEP_ARTIFACTS="${KEEP_ARTIFACTS:-0}"
RISK_CAPTURED=0

fail() {
  printf 'automated arbitrage flow failed: %s\nartifacts: %s\n' "$*" "$ARTIFACT_DIR" >&2
  KEEP_ARTIFACTS=1
  exit 1
}

request() {
  local method="$1" path="$2" output="$3" body="${4:-}" key="${5:-}"
  local args=(-sS -m 30 -o "$output" -w '%{http_code}' -X "$method")
  [[ -z "$body" ]] || args+=(-H 'Content-Type: application/json' --data-binary "@$body")
  [[ -z "$key" ]] || args+=(-H "Idempotency-Key: $key")
  local code
  code="$(curl "${args[@]}" "$API_BASE$path")" || fail "$method $path transport error"
  [[ "$code" =~ ^2 ]] || fail "$method $path returned HTTP $code"
}

portfolio_shape_matches() {
  local output="$1"
  local expected_unprotected="$2"
  local run_id="${3:-}"
  jq -e \
    --slurpfile trading "$ARTIFACT_DIR/trading-status.json" \
    --argjson expected "$expected_unprotected" \
    --arg run_id "$run_id" '
      def normalized:
        tostring | ascii_downcase | gsub("[^a-z0-9]"; "");
      def magnitude:
        if . < 0 then -. else . end;
      def is_protected($fingerprints):
        . as $position
        | any($fingerprints[]?; . as $fingerprint |
            ($position.venue | normalized) == ($fingerprint.venue | normalized)
            and (
              ($position.symbol | normalized) == ($fingerprint.canonicalSymbol | normalized)
              or ($position.symbol | normalized) == ($fingerprint.nativeSymbol | normalized)
            )
            and ($position.side | normalized) == ($fingerprint.side | normalized)
            and ($position.quantity | type) == "number"
            and ($position.entryPrice | type) == "number"
            and ((($position.quantity - $fingerprint.quantity) | magnitude) <= 0.00000001)
            and ((($position.entryPrice - $fingerprint.entryPrice) | magnitude) <= 0.01)
          );
      ($trading[0].risk.protectedPositions // []) as $fingerprints
      | [.snapshot.positions[]? | select((is_protected($fingerprints)) | not)] as $unprotected
      | ($unprotected | length) == $expected
        and (
          if $run_id == "" then true
          else
            ([$unprotected[] | .pairEvidence.runId == $run_id] | all)
            and ([$unprotected[] | (.pairedWith | length) > 0] | all)
          end
        )
    ' "$output" >/dev/null
}

best_effort_cleanup() {
  local body="$ARTIFACT_DIR/cleanup-control.json"
  printf '%s\n' '{"action":"emergency_stop"}' >"$body"
  curl -sS -m 5 -X POST -H 'Content-Type: application/json' \
    -H "Idempotency-Key: auto-flow-stop-$$" --data-binary "@$body" \
    "$API_BASE/api/automation/control" >/dev/null 2>&1 || true
  if [[ "$RISK_CAPTURED" == "1" ]]; then
    curl -sS -m 5 -X PATCH -H 'Content-Type: application/json' \
      -H "Idempotency-Key: auto-flow-risk-restore-$$" \
      --data-binary "@$ARTIFACT_DIR/risk-restore.json" \
      "$API_BASE/api/trading/risk-config" >/dev/null 2>&1 || true
  fi
  if [[ "$KEEP_ARTIFACTS" == "0" ]]; then
    rm -rf "$ARTIFACT_DIR"
  fi
}
trap best_effort_cleanup EXIT

wait_for_candidate() {
  local deadline=$((SECONDS + DISCOVERY_TIMEOUT_SECS))
  while ((SECONDS < deadline)); do
    request GET '/api/v3/arbitrage/opportunities/list?pageSize=200&fast=true&sortKey=score&strategy=perp_cross,perp_price_spread' "$ARTIFACT_DIR/opportunities.json"
    if jq -e \
      --argjson min_net "$MIN_ONE_CYCLE_NET_BPS" \
      --argjson min_score "$MIN_SCORE" '
      any(.rows[]?;
        .execution.eligible == true
        and .cost.verified == true
        and .metrics.oneCycleNetBps >= $min_net
        and .metrics.score >= $min_score
        and .longLeg.marketEvidence.health.quality == "fresh"
        and .shortLeg.marketEvidence.health.quality == "fresh"
      )
    ' "$ARTIFACT_DIR/opportunities.json" >/dev/null; then
      return
    fi
    sleep 5
  done
  fail "no strict opportunity within ${DISCOVERY_TIMEOUT_SECS}s (minNet=${MIN_ONE_CYCLE_NET_BPS}bps minScore=${MIN_SCORE})"
}

wait_for_run() {
  local deadline=$((SECONDS + FLOW_TIMEOUT_SECS))
  while ((SECONDS < deadline)); do
    request GET '/api/automation/status' "$ARTIFACT_DIR/automation-runtime.json"
    RUN_ID="$(jq -r '.lastDecision.executionRunId // empty' "$ARTIFACT_DIR/automation-runtime.json")"
    if [[ -n "$RUN_ID" ]]; then
      export RUN_ID
      return
    fi
    sleep 1
  done
  fail "automation did not submit an execution run"
}

wait_for_open_pair() {
  local deadline=$((SECONDS + FLOW_TIMEOUT_SECS))
  while ((SECONDS < deadline)); do
    request GET '/api/trading/portfolio/snapshot' "$ARTIFACT_DIR/portfolio-open.json"
    if portfolio_shape_matches "$ARTIFACT_DIR/portfolio-open.json" 2 "$RUN_ID"; then
      return
    fi
    sleep 1
  done
  fail "automatic run did not produce a paired paper position"
}

wait_for_close() {
  local deadline=$((SECONDS + FLOW_TIMEOUT_SECS))
  while ((SECONDS < deadline)); do
    request GET '/api/trading/portfolio/snapshot' "$ARTIFACT_DIR/portfolio-closed.json"
    CLOSE_ID="$(jq -r --arg run "$RUN_ID" '
      first(.snapshot.recentCloseRuns[]?
        | select(.status == "succeeded")
        | select(any(.legs[]?.pairEvidence?; .runId == $run))
      ).id // empty
    ' "$ARTIFACT_DIR/portfolio-closed.json")"
    if [[ -n "$CLOSE_ID" ]] \
      && portfolio_shape_matches "$ARTIFACT_DIR/portfolio-closed.json" 0; then
      export CLOSE_ID
      return
    fi
    sleep 1
  done
  fail "automatic exit did not produce a succeeded pair CloseRun"
}

wait_for_review() {
  local deadline=$((SECONDS + FLOW_TIMEOUT_SECS))
  while ((SECONDS < deadline)); do
    request GET '/api/trading/execution-runs?limit=32' "$ARTIFACT_DIR/execution-runs.json"
    request GET '/api/review/executed' "$ARTIFACT_DIR/review.json"
    if jq -e --arg run "$RUN_ID" 'any(.rows[]?; .runId == $run and .state == "closed")' "$ARTIFACT_DIR/execution-runs.json" >/dev/null \
      && jq -e --arg run "$RUN_ID" 'any(.rows[]?; any(.evidence.closeRunEvidence[]?; .runId == $run))' "$ARTIFACT_DIR/review.json" >/dev/null; then
      return
    fi
    sleep 1
  done
  fail "closed automatic run did not reach review"
}

command -v curl >/dev/null || fail 'missing curl'
command -v jq >/dev/null || fail 'missing jq'
mkdir -p "$ARTIFACT_DIR"

request GET '/api/trading/status' "$ARTIFACT_DIR/trading-status.json"
jq -e '.environment == "paper" and .adapter == "mock"' "$ARTIFACT_DIR/trading-status.json" >/dev/null || fail 'requires Paper mock runtime'
jq '{autoProfitClose: .risk.autoProfitClose}' "$ARTIFACT_DIR/trading-status.json" >"$ARTIFACT_DIR/risk-restore.json"
RISK_CAPTURED=1

request GET '/api/trading/portfolio/snapshot' "$ARTIFACT_DIR/portfolio-before.json"
portfolio_shape_matches "$ARTIFACT_DIR/portfolio-before.json" 0 \
  || fail 'requires no unprotected starting positions'
wait_for_candidate

jq -n '{autoProfitClose: {
  enabled: false,
  stopLossEnabled: true,
  maxNetLossUsd: 0.01,
  maxLossRoiBps: 0.01,
  liquidationGuardEnabled: false,
  confirmationSamples: 2,
  cooldownSecs: 10
}}' >"$ARTIFACT_DIR/risk-enable.json"
request PATCH '/api/trading/risk-config' "$ARTIFACT_DIR/risk-enabled.json" "$ARTIFACT_DIR/risk-enable.json" "auto-flow-risk-$$"

jq -n \
  --argjson capital "$CAPITAL_USD" \
  --argjson min_net "$MIN_ONE_CYCLE_NET_BPS" \
  --argjson min_score "$MIN_SCORE" '{
  enabled: true,
  paused: false,
  environment: "paper",
  capitalUsd: $capital,
  leverage: 1,
  minOneCycleNetBps: $min_net,
  minScore: $min_score,
  minDepthUsd: $capital,
  maxConcurrentRuns: 1,
  cooldownSecs: 10
}' >"$ARTIFACT_DIR/automation-config.json"
request PATCH '/api/automation/config' "$ARTIFACT_DIR/automation-enabled.json" "$ARTIFACT_DIR/automation-config.json" "auto-flow-config-$$"

wait_for_run
wait_for_open_pair
wait_for_close
wait_for_review

jq -n \
  --arg run "$RUN_ID" \
  --arg close "$CLOSE_ID" \
  --arg min_one_cycle_net_bps "$MIN_ONE_CYCLE_NET_BPS" \
  --arg min_score "$MIN_SCORE" '{
  status: "ok",
  environment: "paper",
  entry: "automated",
  exit: "automatic_stop_loss",
  purpose: "functional_flow_only",
  profitabilityEvidence: false,
  minimumOneCycleNetBps: ($min_one_cycle_net_bps | tonumber),
  minimumScore: ($min_score | tonumber),
  executionRunId: $run,
  closeRunId: $close,
  checks: ["candidate selection", "shared hedge safety path", "paired position", "automatic pair exit", "execution closed", "review projection"]
}' | tee "$ARTIFACT_DIR/summary.json"
printf 'OK automated paper arbitrage flow artifacts=%s\n' "$ARTIFACT_DIR"
