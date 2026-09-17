#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
API_BASE="${API_BASE:-http://127.0.0.1:8000}"
CAPITAL_USD="${CAPITAL_USD:-10}"
MIN_ONE_CYCLE_NET_BPS="${MIN_ONE_CYCLE_NET_BPS:-25}"
MIN_SCORE="${MIN_SCORE:-50}"
DISCOVERY_TIMEOUT_SECS="${DISCOVERY_TIMEOUT_SECS:-180}"
FLOW_TIMEOUT_SECS="${FLOW_TIMEOUT_SECS:-30}"
HTTP_TIMEOUT_SECS="${HTTP_TIMEOUT_SECS:-30}"
ARTIFACT_DIR="${ARTIFACT_DIR:-$(mktemp -d "${TMPDIR:-/tmp}/crossline-paper-flow.XXXXXX")}"
KEEP_ARTIFACTS="${KEEP_ARTIFACTS:-0}"

fail() {
  printf 'paper arbitrage flow failed: %s\n' "$*" >&2
  printf 'artifacts: %s\n' "$ARTIFACT_DIR" >&2
  KEEP_ARTIFACTS=1
  exit 1
}

cleanup() {
  if [[ "$KEEP_ARTIFACTS" == "0" ]]; then
    rm -rf "$ARTIFACT_DIR"
  fi
}
trap cleanup EXIT

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "missing command: $1"
}

request() {
  local method="$1"
  local path="$2"
  local output="$3"
  local body="${4:-}"
  local idempotency_key="${5:-}"
  local args=(
    -sS
    -m "$HTTP_TIMEOUT_SECS"
    -o "$output"
    -w '%{http_code}'
    -X "$method"
  )
  if [[ -n "$body" ]]; then
    args+=(-H 'Content-Type: application/json' --data-binary "@$body")
  fi
  if [[ -n "$idempotency_key" ]]; then
    args+=(-H "Idempotency-Key: $idempotency_key")
  fi
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

wait_for_eligible_opportunity() {
  local output="$1"
  local deadline=$((SECONDS + DISCOVERY_TIMEOUT_SECS))
  local path='/api/v3/arbitrage/opportunities/list?pageSize=200&fast=true&sortKey=score&strategy=perp_cross,perp_price_spread'
  while (( SECONDS < deadline )); do
    request GET "$path" "$output"
    OPPORTUNITY_ID="$(
      jq -r \
        --argjson min_net "$MIN_ONE_CYCLE_NET_BPS" \
        --argjson min_score "$MIN_SCORE" '
          [.rows[]? | select(
            .execution.eligible == true
            and .cost.verified == true
            and .metrics.oneCycleNetBps >= $min_net
            and .metrics.score >= $min_score
            and .longLeg.marketEvidence.health.quality == "fresh"
            and .shortLeg.marketEvidence.health.quality == "fresh"
          )][0].id // empty
        ' "$output"
    )"
    if [[ -n "$OPPORTUNITY_ID" ]]; then
      export OPPORTUNITY_ID
      return 0
    fi
    sleep 5
  done
  fail "no strict opportunity within ${DISCOVERY_TIMEOUT_SECS}s (minNet=${MIN_ONE_CYCLE_NET_BPS}bps minScore=${MIN_SCORE}); bilateral depth is proven on demand by preview"
}

wait_for_open_pair() {
  local output="$1"
  local run_id="$2"
  local deadline=$((SECONDS + FLOW_TIMEOUT_SECS))
  while (( SECONDS < deadline )); do
    request GET '/api/trading/portfolio/snapshot' "$output"
    if portfolio_shape_matches "$output" 2 "$run_id"; then
      return 0
    fi
    sleep 1
  done
  fail "paired paper positions did not materialize for run=$run_id"
}

wait_for_closed_pair() {
  local output="$1"
  local close_id="$2"
  local deadline=$((SECONDS + FLOW_TIMEOUT_SECS))
  while (( SECONDS < deadline )); do
    request GET '/api/trading/portfolio/snapshot' "$output"
    if portfolio_shape_matches "$output" 0 \
      && jq -e --arg close_id "$close_id" '
        any(.snapshot.recentCloseRuns[]?; .id == $close_id and .status == "succeeded")
      ' "$output" >/dev/null; then
      return 0
    fi
    sleep 1
  done
  fail "paper pair did not reach succeeded close state id=$close_id"
}

wait_for_review() {
  local output="$1"
  local run_id="$2"
  local deadline=$((SECONDS + FLOW_TIMEOUT_SECS))
  while (( SECONDS < deadline )); do
    request GET '/api/review/executed' "$output"
    if jq -e --arg run_id "$run_id" '
      any(.rows[]?;
        any(.evidence.closeRunEvidence[]?; .runId == $run_id)
        and (.missingFields | length) == 0
        and ((.estimatedFields | sort) == (["gross", "net"] | sort))
        and ((.actualFields | sort) == (["fee", "funding", "slippage"] | sort))
      )
    ' "$output" >/dev/null; then
      return 0
    fi
    sleep 1
  done
  fail "review row did not reach complete Paper evidence for run=$run_id"
}

wait_for_execution_closed() {
  local output="$1"
  local run_id="$2"
  local close_id="$3"
  local deadline=$((SECONDS + FLOW_TIMEOUT_SECS))
  while (( SECONDS < deadline )); do
    request GET '/api/trading/execution-runs?limit=32' "$output"
    if jq -e --arg run_id "$run_id" --arg close_id "$close_id" '
      any(.rows[]?;
        .runId == $run_id
        and .state == "closed"
        and .statusReason == "配对平仓已完成"
        and any(.evidence.events[]?;
          .eventId == ("close-run:" + $close_id + ":execution-closed")
          and .state == "closed"
        )
      )
    ' "$output" >/dev/null; then
      return 0
    fi
    sleep 1
  done
  fail "opening execution run did not close with pair close id=$close_id run=$run_id"
}

require_cmd curl
require_cmd jq
mkdir -p "$ARTIFACT_DIR"

request GET '/health' "$ARTIFACT_DIR/health.json"
jq -e '.status == "ok"' "$ARTIFACT_DIR/health.json" >/dev/null ||
  fail "API health is not ok"

request GET '/api/trading/status' "$ARTIFACT_DIR/trading-status.json"
jq -e '.environment == "paper" and .adapter == "mock"' \
  "$ARTIFACT_DIR/trading-status.json" >/dev/null ||
  fail "refusing to run unless trading environment=paper and adapter=mock"

request GET '/api/trading/portfolio/snapshot' "$ARTIFACT_DIR/portfolio-before.json"
portfolio_shape_matches "$ARTIFACT_DIR/portfolio-before.json" 0 \
  && jq -e '.snapshot.risk.hardLimits.openOrdersUsed == 0' \
    "$ARTIFACT_DIR/portfolio-before.json" >/dev/null ||
  fail "paper flow requires no unprotected starting positions and no open orders"

wait_for_eligible_opportunity "$ARTIFACT_DIR/opportunities.json"
OPPORTUNITY_URI="$(jq -rn --arg value "$OPPORTUNITY_ID" '$value | @uri')"

jq -n \
  --arg opportunity_id "$OPPORTUNITY_ID" \
  --arg capital_usd "$CAPITAL_USD" \
  '{
    opportunityId: $opportunity_id,
    capitalUsd: ($capital_usd | tonumber),
    leverage: 1
  }' >"$ARTIFACT_DIR/preview-request.json"
request POST \
  "/api/arbitrage/opportunities/$OPPORTUNITY_URI/preview" \
  "$ARTIFACT_DIR/preview.json" \
  "$ARTIFACT_DIR/preview-request.json"

jq -e '
  (.ticket.blockers | length) == 0
  and ([.ticket.guards[] | .passed] | all)
  and .ticket.ticketId
  and .idempotencyKey
  and .ticketOrderPlans.long.compilePlan
  and .ticketOrderPlans.short.compilePlan
  and .workflowView.longLeg.capability.status == "ready"
  and .workflowView.shortLeg.capability.status == "ready"
' "$ARTIFACT_DIR/preview.json" >/dev/null ||
  fail "preview ticket or workflow evidence is not executable"

TICKET_ID="$(jq -r '.ticket.ticketId' "$ARTIFACT_DIR/preview.json")"
HEDGE_KEY="$(jq -r '.idempotencyKey' "$ARTIFACT_DIR/preview.json")"
export TICKET_ID HEDGE_KEY
jq -n \
  --arg idempotency_key "$HEDGE_KEY" \
  --arg ticket_id "$TICKET_ID" \
  '{idempotencyKey: $idempotency_key, ticketId: $ticket_id}' \
  >"$ARTIFACT_DIR/confirm-request.json"
request POST \
  "/api/arbitrage/opportunities/$OPPORTUNITY_URI/confirm" \
  "$ARTIFACT_DIR/confirm.json" \
  "$ARTIFACT_DIR/confirm-request.json" \
  "$HEDGE_KEY"

jq -e '
  .executionRun.state == "hedged"
  and .executionRun.longLeg.state == "filled"
  and .executionRun.shortLeg.state == "filled"
  and (.executionRun.longLeg.confirmedFilledAtMs > 0)
  and (.executionRun.shortLeg.confirmedFilledAtMs > 0)
  and (.executionRun.longLeg.filledQuantity > 0)
  and (.executionRun.shortLeg.filledQuantity > 0)
  and (.executionRun.costReconciliation.actualOpenCostUsd >= 0)
' "$ARTIFACT_DIR/confirm.json" >/dev/null ||
  fail "hedge confirmation did not produce two final filled legs"

RUN_ID="$(jq -r '.executionRun.runId' "$ARTIFACT_DIR/confirm.json")"
export RUN_ID
wait_for_open_pair "$ARTIFACT_DIR/portfolio-open.json" "$RUN_ID"

CLOSE_VENUE="$(jq -r --arg run_id "$RUN_ID" '
  first(.snapshot.positions[]? | select(.pairEvidence.runId == $run_id)).venue // empty
' "$ARTIFACT_DIR/portfolio-open.json")"
CLOSE_SYMBOL="$(jq -r --arg run_id "$RUN_ID" '
  first(.snapshot.positions[]? | select(.pairEvidence.runId == $run_id)).symbol // empty
' "$ARTIFACT_DIR/portfolio-open.json")"
CLOSE_SIDE="$(jq -r --arg run_id "$RUN_ID" '
  first(.snapshot.positions[]? | select(.pairEvidence.runId == $run_id)).side // empty
' "$ARTIFACT_DIR/portfolio-open.json")"
[[ -n "$CLOSE_VENUE" && -n "$CLOSE_SYMBOL" && -n "$CLOSE_SIDE" ]] ||
  fail "paper pair close target is missing for run=$RUN_ID"
SNAPSHOT_VERSION="$(jq -r '.snapshot.snapshotVersion' "$ARTIFACT_DIR/portfolio-open.json")"
CLOSE_VENUE_URI="$(jq -rn --arg value "$CLOSE_VENUE" '$value | @uri')"
CLOSE_SYMBOL_URI="$(jq -rn --arg value "$CLOSE_SYMBOL" '$value | @uri')"
CLOSE_KEY="paper-flow-close-$(date +%s)-$$"

jq -n \
  --arg side "$CLOSE_SIDE" \
  --arg snapshot_version "$SNAPSHOT_VERSION" \
  '{
    side: $side,
    snapshotVersion: $snapshot_version,
    expectedLegCount: 2,
    reason: "automated paper arbitrage flow verification"
  }' >"$ARTIFACT_DIR/close-request.json"
request POST \
  "/api/trading/portfolio/positions/$CLOSE_VENUE_URI/$CLOSE_SYMBOL_URI/close-pair" \
  "$ARTIFACT_DIR/close.json" \
  "$ARTIFACT_DIR/close-request.json" \
  "$CLOSE_KEY"

CLOSE_ID="$(jq -r '.id // empty' "$ARTIFACT_DIR/close.json")"
[[ -n "$CLOSE_ID" ]] || fail "close response is missing close run id"
export CLOSE_ID
wait_for_closed_pair "$ARTIFACT_DIR/portfolio-closed.json" "$CLOSE_ID"
wait_for_execution_closed "$ARTIFACT_DIR/execution-closed.json" "$RUN_ID" "$CLOSE_ID"

jq -e --arg close_id "$CLOSE_ID" --arg run_id "$RUN_ID" '
  first(.snapshot.recentCloseRuns[] | select(.id == $close_id)) as $run
  | $run.status == "succeeded"
    and $run.failedLegCount == 0
    and $run.nakedExposureUsd == 0
    and ([ $run.legs[] |
      .status == "filled"
      and .pairEvidence.runId == $run_id
      and (.confirmedFilledAtMs > 0)
      and (.order.filledQuantity > 0)
      and (.order.filledPrice > 0)
      and (.order.filledFee >= 0)
    ] | all)
    and ($run.costReconciliation.closeFeeUsd >= 0)
    and ($run.costReconciliation.closeSlippageUsd >= 0)
    and ($run.costReconciliation.fundingUsd == 0)
    and (($run.costReconciliation.fundingEventIds | length) > 0)
    and (($run.costReconciliation.missingFields | length) == 0)
' "$ARTIFACT_DIR/portfolio-closed.json" >/dev/null ||
  fail "close run cost or finality evidence is incomplete"

wait_for_review "$ARTIFACT_DIR/review.json" "$RUN_ID"

jq -n \
  --arg opportunity_id "$OPPORTUNITY_ID" \
  --arg ticket_id "$TICKET_ID" \
  --arg run_id "$RUN_ID" \
  --arg close_id "$CLOSE_ID" \
  --arg capital_usd "$CAPITAL_USD" \
  --arg min_one_cycle_net_bps "$MIN_ONE_CYCLE_NET_BPS" \
  --arg min_score "$MIN_SCORE" \
  '{
    status: "ok",
    environment: "paper",
    adapter: "mock",
    opportunityId: $opportunity_id,
    capitalUsd: ($capital_usd | tonumber),
    minimumOneCycleNetBps: ($min_one_cycle_net_bps | tonumber),
    minimumScore: ($min_score | tonumber),
    ticketId: $ticket_id,
    executionRunId: $run_id,
    closeRunId: $close_id,
    checks: [
      "eligible opportunity",
      "preview guards",
      "compiled capability evidence",
      "two-leg fill finality",
      "paired portfolio projection",
      "pair close finality",
      "opening run closed projection",
      "fee/slippage/funding cost evidence",
      "complete Paper review PnL with estimated gross/net"
    ]
  }' | tee "$ARTIFACT_DIR/summary.json"

printf 'OK paper arbitrage flow artifacts=%s\n' "$ARTIFACT_DIR"
