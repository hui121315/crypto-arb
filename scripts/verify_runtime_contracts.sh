#!/usr/bin/env bash
set -euo pipefail

API_URL="${API_URL:-http://127.0.0.1:8000}"
API_TIMEOUT_SECS="${API_TIMEOUT_SECS:-30}"
ALLOW_RUNTIME_SKIP="${ALLOW_RUNTIME_SKIP:-0}"
TMP_DIR="${TMPDIR:-/tmp}/crossline-runtime-contracts.$$"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

HEALTH_MAX_BYTES="${HEALTH_MAX_BYTES:-32768}"
METRICS_MAX_BYTES="${METRICS_MAX_BYTES:-1048576}"
PORTFOLIO_SNAPSHOT_MAX_BYTES="${PORTFOLIO_SNAPSHOT_MAX_BYTES:-524288}"
# ResourceEnvelope<SystemHealth> carries both typed ApiProblem rows and the
# runtime-native problem list; 68 KiB keeps the configured venue matrix bounded.
SYSTEM_HEALTH_MAX_BYTES="${SYSTEM_HEALTH_MAX_BYTES:-69632}"
VENUE_OPERATION_HEALTH_MAX_BYTES="${VENUE_OPERATION_HEALTH_MAX_BYTES:-524288}"
VENUE_QUALITY_MAX_BYTES="${VENUE_QUALITY_MAX_BYTES:-262144}"
P0_OPPORTUNITIES_LIST_MAX_BYTES="${P0_OPPORTUNITIES_LIST_MAX_BYTES:-184320}"
P0_OPPORTUNITIES_LIST_GZIP_MAX_BYTES="${P0_OPPORTUNITIES_LIST_GZIP_MAX_BYTES:-102400}"
P0_OPPORTUNITIES_LIST_GZIP_MAX_RATIO_PERCENT="${P0_OPPORTUNITIES_LIST_GZIP_MAX_RATIO_PERCENT:-90}"
LEGACY_OPPORTUNITIES_WIDE_MAX_BYTES="${LEGACY_OPPORTUNITIES_WIDE_MAX_BYTES:-1600000}"
OPPORTUNITY_DETAIL_SEED_MAX_BYTES="${OPPORTUNITY_DETAIL_SEED_MAX_BYTES:-524288}"
OPPORTUNITY_COMBINED_DETAIL_MAX_BYTES="${OPPORTUNITY_COMBINED_DETAIL_MAX_BYTES:-786432}"
POSITIONS_MAX_BYTES="${POSITIONS_MAX_BYTES:-524288}"
BALANCES_MAX_BYTES="${BALANCES_MAX_BYTES:-262144}"
ORDER_LIST_MAX_BYTES="${ORDER_LIST_MAX_BYTES:-184320}"
REVIEW_EXECUTED_MAX_BYTES="${REVIEW_EXECUTED_MAX_BYTES:-368640}"
REVIEW_MISSED_MAX_BYTES="${REVIEW_MISSED_MAX_BYTES:-98304}"
REVIEW_STRATEGY_MAX_BYTES="${REVIEW_STRATEGY_MAX_BYTES:-262144}"
HISTORY_FUNDING_DIFFS_MAX_BYTES="${HISTORY_FUNDING_DIFFS_MAX_BYTES:-524288}"

if [[ -n "${API_BEARER_TOKEN:-}" ]]; then
  AUTH_ARGS=(-H "Authorization: Bearer $API_BEARER_TOKEN")
fi

mkdir -p "$TMP_DIR"
trap 'rm -rf "$TMP_DIR"' EXIT

fail() {
  printf 'runtime contracts failed: %s\n' "$*" >&2
  exit 1
}

if ! curl -fsS -m 2 "$API_URL/health" >/dev/null; then
  if [[ "$ALLOW_RUNTIME_SKIP" == "1" ]]; then
    printf 'runtime contracts skipped base=%s reason=health_unreachable\n' "$API_URL"
    exit 0
  fi
  fail "base=$API_URL reason=health_unreachable"
fi

probe_json() {
  local label="$1"
  local path="$2"
  local jq_filter="$3"
  probe_json_budget "$label" "$path" "$jq_filter" 0
}

probe_json_budget() {
  local label="$1"
  local path="$2"
  local jq_filter="$3"
  local max_bytes="$4"
  local body="$TMP_DIR/$label.json"
  local meta code time_total bytes
  local curl_args=(-sS -m "$API_TIMEOUT_SECS")

  if [[ -n "${API_BEARER_TOKEN:-}" ]]; then
    curl_args+=(-H "Authorization: Bearer $API_BEARER_TOKEN")
  fi
  meta="$(curl "${curl_args[@]}" -o "$body" -w '%{http_code} %{time_total}' "$API_URL$path")"
  code="${meta%% *}"
  time_total="${meta#* }"
  [[ "$code" == "200" ]] || fail "$label status=$code path=$path"
  bytes="$(wc -c <"$body" | tr -d '[:space:]')"
  if (( max_bytes > 0 && bytes > max_bytes )); then
    fail "$label payload_bytes=$bytes budget=$max_bytes path=$path"
  fi
  jq -e "$jq_filter" "$body" >/dev/null || fail "$label contract_mismatch path=$path"
  printf 'runtime %s ok status=200 bytes=%s time=%ss\n' "$label" "$bytes" "$time_total"
}

probe_status_budget() {
  local label="$1"
  local path="$2"
  local max_bytes="$3"
  local body="$TMP_DIR/$label.txt"
  local meta code time_total bytes
  local curl_args=(-sS -m "$API_TIMEOUT_SECS")

  if [[ -n "${API_BEARER_TOKEN:-}" ]]; then
    curl_args+=(-H "Authorization: Bearer $API_BEARER_TOKEN")
  fi
  meta="$(curl "${curl_args[@]}" -o "$body" -w '%{http_code} %{time_total}' "$API_URL$path")"
  code="${meta%% *}"
  time_total="${meta#* }"
  [[ "$code" == "200" ]] || fail "$label status=$code path=$path"
  bytes="$(wc -c <"$body" | tr -d '[:space:]')"
  if (( max_bytes > 0 && bytes > max_bytes )); then
    fail "$label payload_bytes=$bytes budget=$max_bytes path=$path"
  fi
  printf 'runtime %s ok status=200 bytes=%s time=%ss\n' "$label" "$bytes" "$time_total"
}

probe_metrics_contract() {
  local body="$TMP_DIR/prometheus_metrics.txt"
  local meta code time_total bytes marker
  local curl_args=(-sS -m "$API_TIMEOUT_SECS")
  local required=(
    '# TYPE crypto_arb_http_request_outcome_total counter'
    '# TYPE crypto_arb_http_request_last_latency_ms gauge'
    '# TYPE crypto_arb_http_request_last_retry_after_ms gauge'
    '# TYPE crypto_arb_http_request_last_observed_at_ms gauge'
    '# TYPE crypto_arb_http_request_latency_ms_p95_bucket gauge'
    '# TYPE crypto_arb_http_host_gate_state gauge'
    '# TYPE crypto_arb_rate_limiter_wait_ms_total counter'
  )

  if [[ -n "${API_BEARER_TOKEN:-}" ]]; then
    curl_args+=(-H "Authorization: Bearer $API_BEARER_TOKEN")
  fi
  meta="$(curl "${curl_args[@]}" -o "$body" -w '%{http_code} %{time_total}' "$API_URL/metrics")"
  code="${meta%% *}"
  time_total="${meta#* }"
  [[ "$code" == "200" ]] || fail "prometheus_metrics status=$code path=/metrics"
  bytes="$(wc -c <"$body" | tr -d '[:space:]')"
  (( bytes <= METRICS_MAX_BYTES )) \
    || fail "prometheus_metrics payload_bytes=$bytes budget=$METRICS_MAX_BYTES path=/metrics"
  for marker in "${required[@]}"; do
    grep -Fqx -- "$marker" "$body" \
      || fail "prometheus_metrics missing_marker=$marker path=/metrics"
  done
  printf 'runtime prometheus_metrics ok status=200 bytes=%s time=%ss markers=%s\n' \
    "$bytes" "$time_total" "${#required[@]}"
}

probe_gzip_budget() {
  local label="$1"
  local path="$2"
  local max_bytes="$3"
  local max_ratio_percent="$4"
  local raw_body="$TMP_DIR/$label.raw.json"
  local gzip_body="$TMP_DIR/$label.json.gz"
  local headers="$TMP_DIR/$label.headers"
  local raw_bytes gzip_bytes encoding
  local curl_args=(-fsS -m "$API_TIMEOUT_SECS")

  if [[ -n "${API_BEARER_TOKEN:-}" ]]; then
    curl_args+=(-H "Authorization: Bearer $API_BEARER_TOKEN")
  fi
  curl "${curl_args[@]}" -o "$raw_body" "$API_URL$path"
  curl "${curl_args[@]}" -H 'Accept-Encoding: gzip' -D "$headers" -o "$gzip_body" "$API_URL$path"
  encoding="$(awk 'tolower($1)=="content-encoding:" {gsub(/\r/, "", $2); print tolower($2); exit}' "$headers")"
  [[ "$encoding" == "gzip" ]] || fail "$label content_encoding=${encoding:-missing} expected=gzip path=$path"
  gzip -t "$gzip_body" || fail "$label invalid_gzip_stream path=$path"
  gzip -dc "$gzip_body" | jq -e 'has("rows") and (.rows|type=="array")' >/dev/null \
    || fail "$label decompressed_contract_mismatch path=$path"

  raw_bytes="$(wc -c <"$raw_body" | tr -d '[:space:]')"
  gzip_bytes="$(wc -c <"$gzip_body" | tr -d '[:space:]')"
  (( gzip_bytes <= max_bytes )) \
    || fail "$label gzip_bytes=$gzip_bytes budget=$max_bytes path=$path"
  if (( raw_bytes >= 1024 && gzip_bytes * 100 > raw_bytes * max_ratio_percent )); then
    fail "$label gzip_bytes=$gzip_bytes raw_bytes=$raw_bytes ratio_budget_percent=$max_ratio_percent path=$path"
  fi
  printf 'runtime %s ok encoding=gzip gzip_bytes=%s raw_bytes=%s ratio_budget_percent=%s\n' \
    "$label" "$gzip_bytes" "$raw_bytes" "$max_ratio_percent"
}

probe_optional_opportunity_error_contract() {
  local label="opportunity_rate_limit_error"
  local path="/api/v3/arbitrage/opportunities/list?pageSize=50&fast=true&sortKey=score&strategy=perp_cross,perp_price_spread,spot_perp,cross_spot_perp,spot_cross&e2eScenario=typed-rate-limit"
  local body="$TMP_DIR/$label.json"
  local headers="$TMP_DIR/$label.headers"
  local meta code time_total bytes retry_after
  local curl_args=(-sS -m "$API_TIMEOUT_SECS")

  if [[ -n "${API_BEARER_TOKEN:-}" ]]; then
    curl_args+=(-H "Authorization: Bearer $API_BEARER_TOKEN")
  fi
  meta="$(curl "${curl_args[@]}" -D "$headers" -o "$body" -w '%{http_code} %{time_total}' "$API_URL$path")"
  code="${meta%% *}"
  time_total="${meta#* }"
  if [[ "$code" != "429" ]]; then
    printf 'runtime %s skipped status=%s reason=scenario_unsupported\n' "$label" "$code"
    return 0
  fi
  retry_after="$(awk 'tolower($1)=="retry-after:" {gsub(/\r/, "", $2); print $2; exit}' "$headers")"
  [[ "$retry_after" == "2" ]] || fail "$label retry_after=$retry_after expected=2"
  bytes="$(wc -c <"$body" | tr -d '[:space:]')"
  jq -e '
    .error.code=="MARKET_DATA_RATE_LIMITED"
    and .error.status==429
    and .error.requestId=="e2e-rate-limit-1"
    and .error.retryAfterMs==2000
    and .error.source=="e2e-fixture"
    and .error.details.venue=="mock"
    and .error.details.operation=="opportunity_list"
    and .error.details.path=="/api/v3/arbitrage/opportunities/list"
    and .error.details.status==429
  ' "$body" >/dev/null || fail "$label contract_mismatch path=$path"
  printf 'runtime %s ok status=429 bytes=%s time=%ss retry_after=%s\n' "$label" "$bytes" "$time_total" "$retry_after"
}

probe_opportunity_detail_seed() {
  local list_body="$TMP_DIR/p0_opportunities.json"
  local id encoded_id

  id="$(jq -r '.rows[0].id // empty' "$list_body")"
  if [[ -z "$id" ]]; then
    printf 'runtime opportunity_detail_seed skipped reason=no_opportunity_id\n'
    return 0
  fi
  encoded_id="$(jq -rn --arg id "$id" '$id|@uri')"
  probe_json_budget \
    opportunity_detail_seed \
    "/api/v3/arbitrage/opportunities/$encoded_id" \
    'has("id") and has("symbol") and has("strategyKind") and has("longExchange") and has("shortExchange")' \
    "$OPPORTUNITY_DETAIL_SEED_MAX_BYTES"
}

probe_opportunity_combined_detail() {
  local list_body="$TMP_DIR/p0_opportunities.json"
  local id encoded_id

  id="$(jq -r '.rows[0].id // empty' "$list_body")"
  if [[ -z "$id" ]]; then
    printf 'runtime opportunity_combined_detail skipped reason=no_opportunity_id\n'
    return 0
  fi
  encoded_id="$(jq -rn --arg id "$id" '$id|@uri')"
  probe_json_budget \
    opportunity_combined_detail \
    "/api/v3/arbitrage/opportunities/$encoded_id/detail?depth=5&historyLimit=6" \
    'has("opportunity") and has("longOrderbook") and has("shortOrderbook") and has("history") and has("longIndexComposition") and has("shortIndexComposition") and has("status") and has("source") and has("observedAtMs") and ((.partialFailures // [])|type=="array") and (.opportunity|has("id") and has("symbol") and has("strategyKind")) and (.longOrderbook|has("data") and has("health") and (.health|has("quality") and has("source"))) and (.shortOrderbook|has("data") and has("health") and (.health|has("quality") and has("source"))) and (.history|has("rows") and has("source")) and (.longIndexComposition|has("data") and has("health") and (.health|has("quality") and has("source"))) and (.shortIndexComposition|has("data") and has("health") and (.health|has("quality") and has("source")))' \
    "$OPPORTUNITY_COMBINED_DETAIL_MAX_BYTES"
}

probe_status_budget health '/health' "$HEALTH_MAX_BYTES"
probe_metrics_contract

probe_json_budget \
  portfolio_snapshot \
  '/api/trading/portfolio/snapshot' \
  'has("status") and has("source") and has("observedAtMs") and has("operationHealth") and ((has("snapshot") and (.snapshot|has("summary") and has("positions") and has("balances") and has("risk") and has("serverNowMs") and (.summary|has("totalNavUsd") and has("netDeltaUsd") and has("realizedPnlTodayUsd")) and (.risk|has("var991dUsd") and has("fundingClustering")))) or (has("problem") and .status=="error"))' \
  "$PORTFOLIO_SNAPSHOT_MAX_BYTES"

probe_json_budget \
  system_health \
  '/api/system/health' \
  'has("data") and has("status") and has("source") and has("observedAtMs") and has("problems") and (.problems|type=="array") and (.data|has("api") and has("ws") and has("orderElapsedMs") and has("risk") and has("updatedAtMs") and has("problems")) and (.data.api|has("healthy") and has("total")) and (.data.ws|has("channels")) and (.data.problems|type=="array")' \
  "$SYSTEM_HEALTH_MAX_BYTES"

probe_json_budget \
  venue_operation_health \
  '/api/system/venue-operation-health' \
  'has("rows") and has("generatedAtMs") and has("rowCount") and (.rows|type=="array")' \
  "$VENUE_OPERATION_HEALTH_MAX_BYTES"

probe_json_budget \
  positions \
  '/api/trading/positions' \
  'has("rows") and has("rowCount") and has("status") and has("source") and has("observedAtMs") and has("problems") and has("operationHealth") and has("fieldQuality") and has("rowHealth") and has("accountBindings") and (.rows|type=="array") and (.problems|type=="array") and (.operationHealth|type=="array") and (.fieldQuality|type=="array") and (.rowHealth|type=="array") and (.accountBindings|type=="array") and ((.rowCount == 0) or (.accountBindings|length > 0)) and ([.accountBindings[] | select((has("venue") and has("status") and has("source") and ((.status == "verified" and (.accountScope|type=="string") and (.credentialFingerprint|startswith("hmac-sha256:"))) or (.status != "verified" and (.problem.code|type=="string"))))|not)]|length == 0) and ((.rowCount > 0) or (.problems|length > 0) or ([.operationHealth[] | select(.status != "ok")]|length > 0))' \
  "$POSITIONS_MAX_BYTES"

probe_json_budget \
  balances \
  '/api/trading/balances' \
  'has("rows") and has("rowCount") and has("status") and has("source") and has("observedAtMs") and has("problems") and has("operationHealth") and has("fieldQuality") and has("rowHealth") and has("accountBindings") and (.rows|type=="array") and (.problems|type=="array") and (.operationHealth|type=="array") and (.fieldQuality|type=="array") and (.rowHealth|type=="array") and (.accountBindings|type=="array") and ((.rowCount == 0) or (.accountBindings|length > 0)) and ([.accountBindings[] | select((has("venue") and has("status") and has("source") and ((.status == "verified" and (.accountScope|type=="string") and (.credentialFingerprint|startswith("hmac-sha256:"))) or (.status != "verified" and (.problem.code|type=="string"))))|not)]|length == 0) and ((.rowCount > 0) or (.problems|length > 0) or ([.operationHealth[] | select(.status != "ok")]|length > 0))' \
  "$BALANCES_MAX_BYTES"

probe_json_budget \
  venue_quality \
  '/api/trading/venues/quality' \
  'has("rows") and has("rowCount") and has("sampledCount") and (.rows|type=="array")' \
  "$VENUE_QUALITY_MAX_BYTES"

probe_json_budget \
  p0_opportunities \
  '/api/v3/arbitrage/opportunities/list?pageSize=50&fast=true&sortKey=score&strategy=perp_cross,perp_price_spread,spot_perp,cross_spot_perp,spot_cross' \
  'has("rows") and has("page") and has("scopeMeta") and has("meta") and has("status") and has("scope") and has("source") and (.rows|type=="array") and (.page|has("returnedCount") and has("totalRows") and has("hasNextPage"))' \
  "$P0_OPPORTUNITIES_LIST_MAX_BYTES"

probe_gzip_budget \
  p0_opportunities_gzip \
  '/api/v3/arbitrage/opportunities/list?pageSize=50&fast=true&sortKey=score&strategy=perp_cross,perp_price_spread,spot_perp,cross_spot_perp,spot_cross' \
  "$P0_OPPORTUNITIES_LIST_GZIP_MAX_BYTES" \
  "$P0_OPPORTUNITIES_LIST_GZIP_MAX_RATIO_PERCENT"

probe_optional_opportunity_error_contract

probe_opportunity_detail_seed

probe_opportunity_combined_detail

probe_json_budget \
  legacy_opportunities_wide \
  '/api/v3/arbitrage/opportunities?limit=500&fast=true&strategy=perp_cross,perp_price_spread,spot_perp,cross_spot_perp,spot_cross' \
  'has("opportunities") and has("totalCount") and has("returnedCount") and has("status") and (.opportunities|type=="array")' \
  "$LEGACY_OPPORTUNITIES_WIDE_MAX_BYTES"

probe_json_budget \
  trading_orders \
  '/api/trading/orders?limit=50' \
  'has("rows") and has("page") and has("status") and has("source") and has("observedAtMs") and (.rows|type=="array") and (.page|has("returnedCount") and has("totalRows") and has("hasMore"))' \
  "$ORDER_LIST_MAX_BYTES"

probe_json_budget \
  review_executed \
  '/api/review/executed?days=7&limit=50' \
  'has("rows") and has("page") and has("rowCount") and has("status") and has("source") and (.rows|type=="array") and (.page|has("returnedCount") and has("totalRows") and has("hasMore"))' \
  "$REVIEW_EXECUTED_MAX_BYTES"

probe_json_budget \
  review_missed \
  '/api/review/missed?days=7&limit=50' \
  'has("rows") and has("page") and has("rowCount") and has("status") and has("source") and (.rows|type=="array") and (.page|has("returnedCount") and has("totalRows") and has("hasMore"))' \
  "$REVIEW_MISSED_MAX_BYTES"

probe_json_budget \
  review_strategy_performance \
  '/api/review/strategy-performance' \
  'has("rows") and has("page") and has("rowCount") and has("status") and has("source") and (.rows|type=="array") and (.page|has("returnedCount") and has("totalRows") and has("hasMore"))' \
  "$REVIEW_STRATEGY_MAX_BYTES"

probe_json_budget \
  history_funding_diffs \
  '/api/history/funding-diffs?limit=100' \
  'has("count") and has("rows") and has("page") and has("source") and has("observedAtMs") and (.rows|type=="array") and (.page|has("returnedCount") and has("hasMore"))' \
  "$HISTORY_FUNDING_DIFFS_MAX_BYTES"

bash "$ROOT/scripts/verify_security_contract.sh"

printf 'OK runtime contracts verified\n'
