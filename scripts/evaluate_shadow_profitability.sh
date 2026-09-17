#!/usr/bin/env bash
set -euo pipefail

API_BASE="${API_BASE:-http://127.0.0.1:8000}"
FROM_MS="${FROM_MS:?set FROM_MS to the Shadow acceptance window start in epoch milliseconds}"
STRATEGY="${STRATEGY:-}"
MIN_CLOSED_LOOPS="${MIN_CLOSED_LOOPS:-30}"
MIN_INDEPENDENT_PERIODS="${MIN_INDEPENDENT_PERIODS:-3}"
MIN_PROFITABLE_LOOPS="${MIN_PROFITABLE_LOOPS:-5}"
MIN_PROFIT_FACTOR="${MIN_PROFIT_FACTOR:-1.2}"
MAX_ERROR_RATE_PCT="${MAX_ERROR_RATE_PCT:-1}"
ARTIFACT_DIR="${ARTIFACT_DIR:-$(mktemp -d "${TMPDIR:-/tmp}/crossline-shadow-eval.XXXXXX")}"
KEEP_ARTIFACTS="${KEEP_ARTIFACTS:-0}"

cleanup() {
  if [[ "$KEEP_ARTIFACTS" == "0" ]]; then
    rm -rf "$ARTIFACT_DIR"
  fi
}
trap cleanup EXIT

request() {
  local path="$1" output="$2"
  curl -fsS --max-time 30 "$API_BASE$path" -o "$output"
}

command -v curl >/dev/null
command -v jq >/dev/null
mkdir -p "$ARTIFACT_DIR"

request '/api/trading/status' "$ARTIFACT_DIR/trading-status.json"
request '/api/trading/portfolio/snapshot' "$ARTIFACT_DIR/portfolio.json"
request '/api/review/executed?days=365&limit=100' "$ARTIFACT_DIR/review.json"

jq -e '.page.hasMore == false' "$ARTIFACT_DIR/review.json" >/dev/null || {
  printf 'shadow evaluation refused: review window exceeds one authoritative page\n' >&2
  exit 1
}

jq -n \
  --slurpfile trading "$ARTIFACT_DIR/trading-status.json" \
  --slurpfile portfolio "$ARTIFACT_DIR/portfolio.json" \
  --slurpfile review "$ARTIFACT_DIR/review.json" \
  --argjson from_ms "$FROM_MS" \
  --arg strategy "$STRATEGY" \
  --argjson min_closed "$MIN_CLOSED_LOOPS" \
  --argjson min_periods "$MIN_INDEPENDENT_PERIODS" \
  --argjson min_profitable "$MIN_PROFITABLE_LOOPS" \
  --argjson min_pf "$MIN_PROFIT_FACTOR" \
  --argjson max_error_pct "$MAX_ERROR_RATE_PCT" '
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
    def complete_trade:
      (.closedAtMs | type) == "number"
      and (.missingFields | length) == 0
      and ((.actualFields | sort) == (["fee", "funding", "slippage"] | sort))
      and ((.estimatedFields | sort) == (["gross", "net"] | sort))
      and (.evidence.closeRunEvidence | length) > 0
      and ([.evidence.closeRunEvidence[] | .status == "succeeded"] | all);
    def percentile($sorted; $p):
      if ($sorted | length) == 0 then null
      else $sorted[((($p * ($sorted | length)) | ceil) - 1) | if . < 0 then 0 else . end]
      end;
    def drawdown($rows):
      reduce ($rows | sort_by(.closedAtMs)[]) as $row
        ({equity: 0, peak: 0, maxUsd: 0};
          .equity += $row.netPnlUsd
          | .peak = ([.peak, .equity] | max)
          | .maxUsd = ([.maxUsd, (.peak - .equity)] | max)
        ) | .maxUsd;

    ($trading[0].risk.protectedPositions // []) as $fingerprints
    | [($portfolio[0].snapshot.positions // [])[] | select((is_protected($fingerprints)) | not)] as $unprotected
    | [($review[0].rows // [])[]
        | select(.openedAtMs >= $from_ms)
        | select($strategy == "" or .strategy == $strategy)] as $scoped
    | [$scoped[] | select(complete_trade)] as $complete
    | [$complete[] | .netPnlUsd | select(. > 0)] as $profits
    | [$complete[] | .netPnlUsd | select(. < 0) | -. ] as $losses
    | [$complete[] | .longOrders[], .shortOrders[]] as $orders
    | [$complete[] | .evidence.closeRunEvidence[]] as $close_runs
    | [$orders[] | (.updatedAtMs - .intent.createdAtMs) | select(. >= 0)] | sort as $latencies
    | ($profits | add // 0) as $gross_profit
    | ($losses | add // 0) as $gross_loss
    | ($complete | map(.netPnlUsd) | add // 0) as $net
    | ([$orders[] | select(.state == "rejected" or .state == "failed" or .state == "unknown")] | length) as $order_errors
    | ([$close_runs[] | select(.status != "succeeded")] | length) as $close_errors
    | (($orders | length) + ($close_runs | length)) as $terminal_samples
    | (if $terminal_samples == 0 then null else (($order_errors + $close_errors) * 100 / $terminal_samples) end) as $error_rate
    | (if $gross_loss > 0 then ($gross_profit / $gross_loss) else null end) as $profit_factor
    | (($gross_profit > 0 and $gross_loss == 0) or (($profit_factor // 0) > $min_pf)) as $pf_pass
    | {
        status: "evaluated",
        purpose: "shadow_profitability_acceptance",
        fromMs: $from_ms,
        strategy: (if $strategy == "" then "all" else $strategy end),
        thresholds: {
          closedLoops: $min_closed,
          independentPeriods: $min_periods,
          profitableLoops: $min_profitable,
          cumulativeNetPnlUsd: "> 0",
          profitFactor: ("> " + ($min_pf | tostring)),
          terminalErrorRatePct: ("< " + ($max_error_pct | tostring)),
          residualPositions: 0,
          openOrders: 0
        },
        metrics: {
          scopedTrades: ($scoped | length),
          completeClosedLoops: ($complete | length),
          excludedIncompleteLoops: (($scoped | length) - ($complete | length)),
          dataMissingRatePct: (if ($scoped | length) == 0 then null else ((($scoped | length) - ($complete | length)) * 100 / ($scoped | length)) end),
          independentPeriods: ([$complete[] | (.openedAtMs / 28800000 | floor)] | unique | length),
          profitableLoops: ($profits | length),
          losingLoops: ($losses | length),
          cumulativeNetPnlUsd: $net,
          grossProfitUsd: $gross_profit,
          grossLossUsd: $gross_loss,
          profitFactor: $profit_factor,
          profitFactorUnboundedNoLosses: ($gross_profit > 0 and $gross_loss == 0),
          expectancyUsd: (if ($complete | length) == 0 then null else ($net / ($complete | length)) end),
          hitRatePct: (if ($complete | length) == 0 then null else (($profits | length) * 100 / ($complete | length)) end),
          maxDrawdownUsd: drawdown($complete),
          tailLossP95Usd: percentile(($losses | sort); 0.95),
          terminalErrorRatePct: $error_rate,
          finalityLatencyMs: {
            p50: percentile($latencies; 0.50),
            p95: percentile($latencies; 0.95),
            max: ($latencies | max // null)
          },
          unprotectedPositionCount: ($unprotected | length),
          openOrderCount: $trading[0].openOrderCount
        }
      }
    | .gates = {
        closedLoops: (.metrics.completeClosedLoops >= $min_closed),
        independentPeriods: (.metrics.independentPeriods >= $min_periods),
        profitableLoops: (.metrics.profitableLoops >= $min_profitable),
        cumulativeNetPositive: (.metrics.cumulativeNetPnlUsd > 0),
        profitFactor: $pf_pass,
        noResidualPositions: (.metrics.unprotectedPositionCount == 0),
        noOpenOrders: (.metrics.openOrderCount == 0),
        terminalErrorRate: (.metrics.terminalErrorRatePct != null and .metrics.terminalErrorRatePct < $max_error_pct)
      }
    | .eligibleForUserLiveAcceptance = ([.gates[]] | all)
  ' | tee "$ARTIFACT_DIR/summary.json"

printf 'Shadow evaluation artifacts=%s\n' "$ARTIFACT_DIR"
