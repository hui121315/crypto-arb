#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUNTIME_DIR="$(mktemp -d "${TMPDIR:-/tmp}/crossline-api-security.XXXXXX")"
API_HOST="${API_HOST:-127.0.0.1}"
API_PORT="${API_PORT:-}"
API_TOKEN="${API_BEARER_TOKEN:-runtime-smoke-secret-token}"
API_ORIGIN="${API_ORIGIN:-http://127.0.0.1:8080}"
EVIL_ORIGIN="${EVIL_ORIGIN:-https://evil.example}"
CREDENTIAL_KEY="runtime-security-api-key-sentinel"
CREDENTIAL_SECRET="runtime-security-api-secret-sentinel"
STARTUP_TIMEOUT_SECS="${STARTUP_TIMEOUT_SECS:-90}"
API_LOG="$RUNTIME_DIR/api.log"
ROUTE_INVENTORY="$ROOT/docs/API_ROUTE_INVENTORY.tsv"
API_PID=""
SUCCESS=0
AUTH_DENIAL_EVENT_COUNT=0

fail() {
  printf 'api security runtime smoke failed: %s\n' "$*" >&2
  printf 'runtime_dir=%s\n' "$RUNTIME_DIR" >&2
  if [[ -f "$API_LOG" ]]; then
    tail -120 "$API_LOG" >&2 || true
  fi
  exit 1
}

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "missing command: $1"
}

cleanup() {
  stop_api
  if [[ "$SUCCESS" == "1" || "${KEEP_RUNTIME_DIR_ON_FAILURE:-0}" != "1" ]]; then
    rm -rf "$RUNTIME_DIR"
  else
    printf 'kept runtime_dir=%s\n' "$RUNTIME_DIR" >&2
  fi
}
trap cleanup EXIT

find_port() {
  python3 - <<'PY'
import socket
with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
    sock.bind(("127.0.0.1", 0))
    print(sock.getsockname()[1])
PY
}

wait_for_health() {
  local deadline=$((SECONDS + STARTUP_TIMEOUT_SECS))
  local code

  while (( SECONDS < deadline )); do
    if [[ -n "$API_PID" ]] && ! kill -0 "$API_PID" 2>/dev/null; then
      local status=0
      wait "$API_PID" 2>/dev/null || status="$?"
      fail "api process exited early status=$status"
    fi
    code="$(curl_status "$RUNTIME_DIR/health.json" "$API_URL/health")"
    if [[ "${SMOKE_DEBUG:-0}" == "1" ]]; then
      printf 'api security wait pid=%s code=%s elapsed=%ss\n' "$API_PID" "$code" "$SECONDS" >&2
    fi
    if [[ "$code" == "200" ]]; then
      return 0
    fi
    sleep 1
  done
  fail "api did not become healthy within ${STARTUP_TIMEOUT_SECS}s"
}

header_value() {
  local headers="$1"
  local name="$2"

  awk -v name="$name" '
    BEGIN { target = tolower(name) ":" }
    tolower($1) == target {
      $1 = ""
      sub(/^[ \t]+/, "")
      sub(/\r$/, "")
      print
      exit
    }
  ' "$headers"
}

curl_status() {
  local output="$1"
  shift

  curl -sS -m 10 -o "$output" -w '%{http_code}' "$@" 2>/dev/null || printf '000'
}

stop_api() {
  if [[ -z "$API_PID" ]] || ! kill -0 "$API_PID" 2>/dev/null; then
    API_PID=""
    return
  fi
  kill "$API_PID" 2>/dev/null || true
  for _ in {1..20}; do
    kill -0 "$API_PID" 2>/dev/null || break
    sleep 0.1
  done
  if kill -0 "$API_PID" 2>/dev/null; then
    kill -9 "$API_PID" 2>/dev/null || true
  fi
  wait "$API_PID" 2>/dev/null || true
  API_PID=""
}

start_api() {
  (
    cd "$RUNTIME_DIR"
    env \
      -u APP_API_SURFACE__CHAT \
      -u APP_API_SURFACE__LLM_DIAGNOSTICS \
      -u APP_API_SURFACE__OPTIONS \
      -u APP_API_SURFACE__WATCHLIST_ALERTS \
      -u APP_API_SURFACE__SPOT_V1 \
      -u APP_API_SURFACE__STRATEGY_V1 \
      APP_HOST="$API_HOST" \
      APP_PORT="$API_PORT" \
      APP_LOG_LEVEL="${APP_LOG_LEVEL:-info}" \
      RUST_LOG="${SMOKE_RUST_LOG:-info}" \
      APP_SECURITY__AUTH_TOKEN="$API_TOKEN" \
      APP_SECURITY__AUDIT_LOG_PATH="$AUDIT_LOG" \
      APP_STORAGE__DATA_DIR="$RUNTIME_DIR/data" \
      "$API_BIN" >>"$API_LOG" 2>&1 &
    printf '%s\n' "$!" >"$RUNTIME_DIR/api.pid"
  )
  API_PID="$(cat "$RUNTIME_DIR/api.pid")"
  wait_for_health
}

sample_inventory_path() {
  local path="$1"

  printf '%s\n' "$path" | sed -E 's/:[[:alpha:]_][[:alnum:]_]*/runtime-smoke-id/g'
}

always_on_bearer_inventory_rows() {
  awk -F '\t' '
    NR > 1 && $5 == "always" && $7 == "bearer" { print }
  ' "$ROUTE_INVENTORY"
}

audit_line_count() {
  if [[ -f "$AUDIT_LOG" ]]; then
    awk 'END { print NR + 0 }' "$AUDIT_LOG"
  else
    printf '0\n'
  fi
}

assert_typed_unauthorized() {
  local body="$1"
  local headers="$2"
  local request_id="$3"
  local route_label="$4"

  jq -e --arg request_id "$request_id" '
    .error.code == "UNAUTHORIZED"
    and .error.requestId == $request_id
  ' "$body" >/dev/null || fail "$route_label did not return a typed correlated UNAUTHORIZED problem"
  [[ "$(header_value "$headers" "x-request-id")" == "$request_id" ]] \
    || fail "$route_label did not echo x-request-id"
}

assert_auth_denial_event() {
  local method="$1"
  local route_path="$2"
  local request_path="$3"
  local request_id="$4"
  local expected_lines="$5"

  [[ "$(audit_line_count)" == "$expected_lines" ]] \
    || fail "auth denial audit count drifted method=$method path=$request_path expected=$expected_lines"
  jq -s -e \
    --arg method "$method" \
    --arg route_path "$route_path" \
    --arg request_path "$request_path" \
    --arg request_id "$request_id" '
    [.[] | select(.requestId == $request_id)] as $events
    | ($events | length) == 1
      and $events[0].actor == "unknown"
      and $events[0].actorKind == "unknown"
      and $events[0].outcome == "denied"
      and $events[0].method == $method
      and $events[0].path == $route_path
      and $events[0].resource == $route_path
      and $events[0].status == 401
      and $events[0].problemCode == "UNAUTHORIZED"
      and $events[0].detail.requestId == $request_id
      and $events[0].detail.requestPath == $request_path
      and $events[0].detail.problem.code == "UNAUTHORIZED"
      and $events[0].detail.problem.status == 401
      and ($events[0].action | type == "string" and length > 0)
      and ($events[0].actionKind | type == "string" and length > 0)
      and ($events[0].resourceKind | type == "string" and length > 0)
  ' "$AUDIT_LOG" >/dev/null \
    || fail "auth denial audit evidence drifted method=$method path=$request_path request_id=$request_id"
}

assert_always_on_bearer_auth_matrix() {
  local path methods router route_class exposure risk auth_policy audit_policy feature_flag dto client owner notes
  local sample_path method request_id body headers status route_index=0 protected_route_count=0
  local audit_lines_before expected_audit_lines
  local -a route_methods

  audit_lines_before="$(audit_line_count)"

  while IFS=$'\t' read -r path methods router route_class exposure risk auth_policy audit_policy feature_flag dto client owner notes; do
    sample_path="$(sample_inventory_path "$path")"
    IFS=',' read -r -a route_methods <<<"$methods"
    for method in "${route_methods[@]}"; do
      route_index=$((route_index + 1))
      request_id="runtime-always-on-auth-$route_index"
      body="$RUNTIME_DIR/always_on_auth_$route_index.body"
      headers="$RUNTIME_DIR/always_on_auth_$route_index.headers"
      status="$(curl -sS -m 10 \
        -X "$method" \
        -H "x-request-id: $request_id" \
        -D "$headers" \
        -o "$body" \
        -w '%{http_code}' \
        "$API_URL$sample_path" 2>/dev/null || printf '000')"
      [[ "$status" == "401" ]] \
        || fail "always-on bearer route accepted missing auth method=$method path=$sample_path class=$route_class status=$status"
      assert_typed_unauthorized "$body" "$headers" "$request_id" "always-on bearer route method=$method path=$sample_path"

      if [[ "$audit_policy" == "action_run" || "$audit_policy" == "secret_mutation" ]]; then
        protected_route_count=$((protected_route_count + 1))
        expected_audit_lines=$((audit_lines_before + protected_route_count))
        assert_auth_denial_event \
          "$method" "$path" "$sample_path" "$request_id" "$expected_audit_lines"
      else
        expected_audit_lines=$((audit_lines_before + protected_route_count))
        [[ "$(audit_line_count)" == "$expected_audit_lines" ]] \
          || fail "unauthorized read route wrote audit evidence method=$method path=$sample_path"
      fi
    done
  done < <(always_on_bearer_inventory_rows)

  (( route_index > 0 )) || fail "route inventory has no always-on bearer routes"
  (( protected_route_count > 0 )) || fail "route inventory has no always-on ActionRun or secret mutation routes"
  AUTH_DENIAL_EVENT_COUNT="$protected_route_count"
  expected_audit_lines=$((audit_lines_before + AUTH_DENIAL_EVENT_COUNT))
  [[ "$(audit_line_count)" == "$expected_audit_lines" ]] \
    || fail "always-on bearer auth denial matrix lost durable audit evidence"
  printf 'api security always-on bearer auth matrix ok routes=%s audit_denials=%s\n' \
    "$route_index" "$AUTH_DENIAL_EVENT_COUNT"
}

assert_always_on_bearer_cors_matrix() {
  local path methods router route_class exposure risk auth_policy audit_policy feature_flag dto client owner notes
  local sample_path method allowed_headers allowed_methods evil_origin evil_status allowed_status route_index=0
  local -a route_methods

  while IFS=$'\t' read -r path methods router route_class exposure risk auth_policy audit_policy feature_flag dto client owner notes; do
    sample_path="$(sample_inventory_path "$path")"
    IFS=',' read -r -a route_methods <<<"$methods"
    for method in "${route_methods[@]}"; do
      route_index=$((route_index + 1))
      evil_origin="$RUNTIME_DIR/always_on_cors_evil_$route_index.headers"
      evil_status="$(curl -sS -m 10 \
        -X OPTIONS \
        -H "Origin: $EVIL_ORIGIN" \
        -H "Access-Control-Request-Method: $method" \
        -H 'Access-Control-Request-Headers: authorization,content-type,idempotency-key,x-idempotency-key,x-request-id' \
        -D "$evil_origin" \
        -o "$RUNTIME_DIR/always_on_cors_evil_$route_index.body" \
        -w '%{http_code}' \
        "$API_URL$sample_path" 2>/dev/null || printf '000')"
      [[ "$evil_status" =~ ^(200|204)$ ]] \
        || fail "always-on bearer evil CORS preflight status=$evil_status method=$method path=$sample_path"
      [[ "$(header_value "$evil_origin" "access-control-allow-origin")" != "$EVIL_ORIGIN" ]] \
        || fail "always-on bearer route echoed an evil CORS origin method=$method path=$sample_path"

      allowed_status="$(curl -sS -m 10 \
        -X OPTIONS \
        -H "Origin: $API_ORIGIN" \
        -H "Access-Control-Request-Method: $method" \
        -H 'Access-Control-Request-Headers: authorization,content-type,idempotency-key,x-idempotency-key,x-request-id' \
        -D "$RUNTIME_DIR/always_on_cors_allowed_$route_index.headers" \
        -o "$RUNTIME_DIR/always_on_cors_allowed_$route_index.body" \
        -w '%{http_code}' \
        "$API_URL$sample_path" 2>/dev/null || printf '000')"
      [[ "$allowed_status" =~ ^(200|204)$ ]] \
        || fail "always-on bearer allowed CORS preflight status=$allowed_status method=$method path=$sample_path"
      [[ "$(header_value "$RUNTIME_DIR/always_on_cors_allowed_$route_index.headers" "access-control-allow-origin")" == "$API_ORIGIN" ]] \
        || fail "always-on bearer route did not echo the configured CORS origin method=$method path=$sample_path"
      allowed_headers="$(header_value "$RUNTIME_DIR/always_on_cors_allowed_$route_index.headers" "access-control-allow-headers" | tr '[:upper:]' '[:lower:]')"
      allowed_methods="$(header_value "$RUNTIME_DIR/always_on_cors_allowed_$route_index.headers" "access-control-allow-methods" | tr '[:upper:]' '[:lower:]')"
      [[ "$allowed_headers" == *"authorization"* && "$allowed_headers" == *"idempotency-key"* && "$allowed_headers" == *"x-idempotency-key"* && "$allowed_headers" == *"x-request-id"* ]] \
        || fail "always-on bearer CORS headers drifted method=$method path=$sample_path headers=$allowed_headers"
      [[ "$allowed_methods" == *"$(printf '%s' "$method" | tr '[:upper:]' '[:lower:]')"* ]] \
        || fail "always-on bearer CORS method is absent method=$method path=$sample_path methods=$allowed_methods"
    done
  done < <(always_on_bearer_inventory_rows)

  (( route_index > 0 )) || fail "route inventory has no always-on bearer routes for CORS matrix"
  printf 'api security always-on bearer CORS matrix ok routes=%s\n' "$route_index"
}

assert_always_on_bearer_read_matrix() {
  local path methods router route_class exposure risk auth_policy audit_policy feature_flag dto client owner notes
  local sample_path request_id body headers status route_index=0

  while IFS=$'\t' read -r path methods router route_class exposure risk auth_policy audit_policy feature_flag dto client owner notes; do
    [[ "$methods" == "GET" ]] || continue
    sample_path="$(sample_inventory_path "$path")"
    route_index=$((route_index + 1))
    request_id="runtime-always-on-read-$route_index"
    body="$RUNTIME_DIR/always_on_read_$route_index.body"
    headers="$RUNTIME_DIR/always_on_read_$route_index.headers"
    status="$(curl -sS -m 10 \
      -H "Authorization: Bearer $API_TOKEN" \
      -H "x-request-id: $request_id" \
      -D "$headers" \
      -o "$body" \
      -w '%{http_code}' \
      "$API_URL$sample_path" 2>/dev/null || printf '000')"
    [[ "$(header_value "$headers" "x-request-id")" == "$request_id" ]] \
      || fail "always-on bearer read did not echo x-request-id path=$sample_path status=$status"

    case "$status" in
      2??)
        ;;
      4??|503)
        jq -e --arg request_id "$request_id" --argjson status "$status" '
          .error.requestId == $request_id
          and .error.status == $status
        ' "$body" >/dev/null || fail "always-on bearer read error was not typed and correlated path=$sample_path status=$status"
        ;;
      *)
        fail "always-on bearer read returned an unexpected status path=$sample_path class=$route_class status=$status"
        ;;
    esac
  done < <(always_on_bearer_inventory_rows)

  (( route_index > 0 )) || fail "route inventory has no always-on bearer GET routes"
  printf 'api security always-on bearer read matrix ok routes=%s\n' "$route_index"
}

assert_default_off_surface_routes() {
  local path methods router route_class exposure risk auth_policy audit_policy feature_flag dto client owner notes
  local sample_path method request_id body status route_index=0
  local -a route_methods

  [[ -s "$ROUTE_INVENTORY" ]] || fail "missing route inventory $ROUTE_INVENTORY"

  while IFS=$'\t' read -r path methods router route_class exposure risk auth_policy audit_policy feature_flag dto client owner notes; do
    [[ "$exposure" == "default_off" ]] || continue
    [[ "$feature_flag" == api_surface.* ]] \
      || fail "default_off route $methods $path must use api_surface feature gate, got $feature_flag"

    sample_path="$(sample_inventory_path "$path")"
    IFS=',' read -r -a route_methods <<<"$methods"
    for method in "${route_methods[@]}"; do
      route_index=$((route_index + 1))
      request_id="runtime-default-off-$route_index"
      body="$RUNTIME_DIR/default_off_$route_index.body"
      status="$(curl_status "$body" \
        -X "$method" \
        -H "Authorization: Bearer $API_TOKEN" \
        -H "x-request-id: $request_id" \
        "$API_URL$sample_path")"
      [[ "$status" == "404" ]] \
        || fail "default-off route registered method=$method path=$sample_path class=$route_class feature=$feature_flag status=$status"
    done
  done <"$ROUTE_INVENTORY"

  (( route_index > 0 )) || fail "route inventory has no default_off api_surface routes"
  printf 'api security default-off route matrix ok routes=%s\n' "$route_index"
}

require_cmd cargo
require_cmd curl
require_cmd jq
require_cmd python3

API_PORT="${API_PORT:-$(find_port)}"
API_URL="http://$API_HOST:$API_PORT"
API_BIN="$ROOT/target/debug/crypto-arb-api"
AUDIT_LOG="$RUNTIME_DIR/security_audit.jsonl"

mkdir -p "$RUNTIME_DIR/data"
cat >"$RUNTIME_DIR/config.toml" <<EOF
[security]
allowed_origins = ["$API_ORIGIN"]
EOF

cargo build -p api

start_api

health_code="$(curl_status "$RUNTIME_DIR/public_health.body" "$API_URL/health")"
[[ "$health_code" == "200" ]] || fail "public /health status=$health_code"

missing_code="$(curl -sS -m 5 \
  -H 'x-request-id: runtime-security-rid' \
  -D "$RUNTIME_DIR/missing_auth.headers" \
  -o "$RUNTIME_DIR/missing_auth.body" \
  -w '%{http_code}' \
  "$API_URL/api/trading/status" 2>/dev/null || printf '000')"
[[ "$missing_code" == "401" ]] || fail "missing auth status=$missing_code"
jq -e '
  .error.code == "UNAUTHORIZED"
  and .error.requestId == "runtime-security-rid"
' "$RUNTIME_DIR/missing_auth.body" >/dev/null || fail "missing auth body is not typed UNAUTHORIZED"
[[ "$(header_value "$RUNTIME_DIR/missing_auth.headers" "x-request-id")" == "runtime-security-rid" ]] \
  || fail "missing auth did not echo x-request-id"

authed_code="$(curl -sS -m 5 \
  -H "Authorization: Bearer $API_TOKEN" \
  -o "$RUNTIME_DIR/authed_status.body" \
  -w '%{http_code}' \
  "$API_URL/api/trading/status" 2>/dev/null || printf '000')"
[[ "$authed_code" == "200" ]] || fail "authed trading status=$authed_code"

assert_default_off_surface_routes
assert_always_on_bearer_auth_matrix
assert_always_on_bearer_cors_matrix
assert_always_on_bearer_read_matrix

evil_code="$(curl -sS -m 5 \
  -X OPTIONS \
  -H "Origin: $EVIL_ORIGIN" \
  -H 'Access-Control-Request-Method: POST' \
  -H 'Access-Control-Request-Headers: content-type,idempotency-key' \
  -D "$RUNTIME_DIR/evil_preflight.headers" \
  -o "$RUNTIME_DIR/evil_preflight.body" \
  -w '%{http_code}' \
  "$API_URL/api/trading/orders" 2>/dev/null || printf '000')"
[[ "$evil_code" =~ ^(200|204)$ ]] || fail "evil preflight unexpected status=$evil_code"
[[ "$(header_value "$RUNTIME_DIR/evil_preflight.headers" "access-control-allow-origin")" != "$EVIL_ORIGIN" ]] \
  || fail "evil Origin was echoed by CORS"

allowed_code="$(curl -sS -m 5 \
  -X OPTIONS \
  -H "Origin: $API_ORIGIN" \
  -H 'Access-Control-Request-Method: POST' \
  -H 'Access-Control-Request-Headers: content-type,x-request-id,idempotency-key,x-idempotency-key' \
  -D "$RUNTIME_DIR/allowed_preflight.headers" \
  -o "$RUNTIME_DIR/allowed_preflight.body" \
  -w '%{http_code}' \
  "$API_URL/api/trading/orders" 2>/dev/null || printf '000')"
[[ "$allowed_code" =~ ^(200|204)$ ]] || fail "allowed preflight status=$allowed_code"
[[ "$(header_value "$RUNTIME_DIR/allowed_preflight.headers" "access-control-allow-origin")" == "$API_ORIGIN" ]] \
  || fail "allowed Origin was not echoed by CORS"
allowed_headers="$(header_value "$RUNTIME_DIR/allowed_preflight.headers" "access-control-allow-headers")"
[[ "$allowed_headers" == *"idempotency-key"* ]] || fail "idempotency-key not allowed by preflight"
[[ "$allowed_headers" == *"x-idempotency-key"* ]] || fail "x-idempotency-key not allowed by preflight"

extractor_code="$(curl -sS -m 5 \
  -X POST \
  -H "Authorization: Bearer $API_TOKEN" \
  -H 'Content-Type: application/json' \
  -H 'x-request-id: runtime-security-extractor-rid' \
  -D "$RUNTIME_DIR/extractor_422.headers" \
  -o "$RUNTIME_DIR/extractor_422.body" \
  -w '%{http_code}' \
  --data '{"active":true,"expectedActive":false,"expectedOpenOrderCount":0}' \
  "$API_URL/api/trading/kill-switch" 2>/dev/null || printf '000')"
[[ "$extractor_code" == "422" ]] || fail "extractor rejection status=$extractor_code"
jq -e '
  .error.code == "REQUEST_BODY_INVALID"
  and .error.status == 422
  and .error.requestId == "runtime-security-extractor-rid"
  and .error.details.extractor == "Json"
  and .error.details.kind == "data"
' "$RUNTIME_DIR/extractor_422.body" >/dev/null || fail "extractor rejection body is not typed REQUEST_BODY_INVALID"
[[ "$(header_value "$RUNTIME_DIR/extractor_422.headers" "x-request-id")" == "runtime-security-extractor-rid" ]] \
  || fail "extractor rejection did not echo x-request-id"

kill_code="$(curl -sS -m 10 \
  -X POST \
  -H "Authorization: Bearer $API_TOKEN" \
  -H 'Content-Type: application/json' \
  -H 'x-request-id: runtime-security-audit-rid' \
  -H 'idempotency-key: runtime-security-kill-idem' \
  -o "$RUNTIME_DIR/kill_switch.body" \
  -w '%{http_code}' \
  --data '{"active":false,"expectedActive":false,"expectedOpenOrderCount":0,"reason":"runtime.security.smoke"}' \
  "$API_URL/api/trading/kill-switch" 2>/dev/null || printf '000')"
[[ "$kill_code" == "200" ]] || fail "kill-switch status=$kill_code"

risk_code="$(curl -sS -m 10 \
  -X PATCH \
  -H "Authorization: Bearer $API_TOKEN" \
  -H 'Content-Type: application/json' \
  -H 'x-request-id: runtime-security-risk-rid' \
  -H 'idempotency-key: runtime-security-risk-idem' \
  -o "$RUNTIME_DIR/risk_config.body" \
  -w '%{http_code}' \
  --data '{"maxOrderNotional":12345,"maxOpenOrders":7,"allowedExchanges":["mock"],"allowedSymbols":["BTCUSDT"]}' \
  "$API_URL/api/trading/risk-config" 2>/dev/null || printf '000')"
[[ "$risk_code" == "200" ]] || fail "risk-config status=$risk_code"

adapter_code="$(curl -sS -m 10 \
  -X POST \
  -H "Authorization: Bearer $API_TOKEN" \
  -H 'Content-Type: application/json' \
  -H 'x-request-id: runtime-security-adapter-rid' \
  -o "$RUNTIME_DIR/adapter.body" \
  -w '%{http_code}' \
  --data '{"adapterId":"mock"}' \
  "$API_URL/api/trading/adapters/select" 2>/dev/null || printf '000')"
[[ "$adapter_code" == "200" ]] || fail "adapter select status=$adapter_code"

reconcile_code="$(curl -sS -m 10 \
  -X POST \
  -H "Authorization: Bearer $API_TOKEN" \
  -H 'x-request-id: runtime-security-reconcile-rid' \
  -o "$RUNTIME_DIR/reconcile.body" \
  -w '%{http_code}' \
  "$API_URL/api/trading/orders/reconcile" 2>/dev/null || printf '000')"
[[ "$reconcile_code" == "200" ]] || fail "order reconcile status=$reconcile_code"

fee_code="$(curl -sS -m 10 \
  -X POST \
  -H "Authorization: Bearer $API_TOKEN" \
  -H 'Content-Type: application/json' \
  -H 'x-request-id: runtime-security-fee-rid' \
  -o "$RUNTIME_DIR/fee.body" \
  -w '%{http_code}' \
  --data '{"venue":"runtime","symbol":"BTCUSDT","product":"perp","makerFeeBps":1,"takerFeeBps":2,"openFeeBps":1,"closeFeeBps":1,"source":"manual","fetchedAtMs":1,"validUntilMs":4102444800000}' \
  "$API_URL/api/trading/fee-snapshots" 2>/dev/null || printf '000')"
[[ "$fee_code" == "400" ]] || fail "fee snapshot denial status=$fee_code"

submit_payload='{"id":"runtime-security-order","clientOrderId":"runtime-security-order-idem","mode":"dry_run","source":"manual","exchange":"mock","symbol":"btcusdt","side":"buy","orderType":"limit","quantity":1,"price":10,"timeInForce":"gtc","leverage":1}'
submit_code="$(curl -sS -m 10 \
  -X POST \
  -H "Authorization: Bearer $API_TOKEN" \
  -H 'Content-Type: application/json' \
  -H 'x-request-id: runtime-security-submit-rid' \
  -o "$RUNTIME_DIR/submit.body" \
  -w '%{http_code}' \
  --data "$submit_payload" \
  "$API_URL/api/trading/orders" 2>/dev/null || printf '000')"
[[ "$submit_code" == "200" ]] || fail "paper order submit status=$submit_code"

cancel_code="$(curl -sS -m 10 \
  -X POST \
  -H "Authorization: Bearer $API_TOKEN" \
  -H 'x-request-id: runtime-security-cancel-rid' \
  -o "$RUNTIME_DIR/cancel.body" \
  -w '%{http_code}' \
  "$API_URL/api/trading/orders/runtime-security-missing/cancel" 2>/dev/null || printf '000')"
[[ "$cancel_code" == "404" ]] || fail "missing order cancel denial status=$cancel_code"

close_all_code="$(curl -sS -m 10 \
  -X POST \
  -H "Authorization: Bearer $API_TOKEN" \
  -H 'Content-Type: application/json' \
  -H 'x-request-id: runtime-security-close-all-rid' \
  -H 'idempotency-key: runtime-security-close-all-idem' \
  -o "$RUNTIME_DIR/close_all.body" \
  -w '%{http_code}' \
  --data '{"confirmationPhrase":"wrong","snapshotVersion":"runtime-security-empty","expectedLegCount":0,"reason":"runtime.security.close-all"}' \
  "$API_URL/api/trading/portfolio/close-all" 2>/dev/null || printf '000')"
[[ "$close_all_code" == "400" ]] || fail "close-all denial status=$close_all_code"

credential_code="$(curl -sS -m 10 \
  -X POST \
  -H "Authorization: Bearer $API_TOKEN" \
  -H 'Content-Type: application/json' \
  -H 'x-request-id: runtime-security-credential-rid' \
  -H 'idempotency-key: runtime-security-credential-idem' \
  -o "$RUNTIME_DIR/credential.body" \
  -w '%{http_code}' \
  --data "{\"venue\":\"runtime-invalid-venue\",\"fields\":[{\"key\":\"api_key\",\"value\":\"$CREDENTIAL_KEY\"},{\"key\":\"api_secret\",\"value\":\"$CREDENTIAL_SECRET\"}]}" \
  "$API_URL/api/exchanges/credentials" 2>/dev/null || printf '000')"
[[ "$credential_code" == "400" ]] || fail "credential denial status=$credential_code"
jq -e '
  .error.code == "CREDENTIAL_UNKNOWN_VENUE"
  and .error.status == 400
  and .error.requestId == "runtime-security-credential-rid"
' "$RUNTIME_DIR/credential.body" >/dev/null || fail "credential denial is not a typed correlated problem"

expected_audit_lines_before_replay=$((AUTH_DENIAL_EVENT_COUNT + 18))
for _ in {1..20}; do
  [[ "$(wc -l <"$AUDIT_LOG" 2>/dev/null || printf '0')" -ge "$expected_audit_lines_before_replay" ]] && break
  sleep 0.1
done
audit_lines_before_replay="$(awk 'END { print NR + 0 }' "$AUDIT_LOG" 2>/dev/null || printf '0')"
[[ "$audit_lines_before_replay" == "$expected_audit_lines_before_replay" ]] \
  || fail "expected $expected_audit_lines_before_replay audit events before replay, got $audit_lines_before_replay"

credential_replay_code="$(curl -sS -m 10 \
  -X POST \
  -H "Authorization: Bearer $API_TOKEN" \
  -H 'Content-Type: application/json' \
  -H 'x-request-id: runtime-security-credential-replay-rid' \
  -H 'idempotency-key: runtime-security-credential-idem' \
  -o "$RUNTIME_DIR/credential_replay.body" \
  -w '%{http_code}' \
  --data "{\"venue\":\"runtime-invalid-venue\",\"fields\":[{\"key\":\"api_key\",\"value\":\"$CREDENTIAL_KEY\"},{\"key\":\"api_secret\",\"value\":\"$CREDENTIAL_SECRET\"}]}" \
  "$API_URL/api/exchanges/credentials" 2>/dev/null || printf '000')"
[[ "$credential_replay_code" == "409" ]] || fail "credential replay status=$credential_replay_code"
jq -e '
  .error.code == "ACTION_RUN_REPLAY_FAILED"
  and .error.details.actionRunId != null
  and .error.details.idempotencyKey == "runtime-security-credential-idem"
' "$RUNTIME_DIR/credential_replay.body" >/dev/null || fail "credential replay lost its ActionRun correlation"

submit_replay_code="$(curl -sS -m 10 \
  -X POST \
  -H "Authorization: Bearer $API_TOKEN" \
  -H 'Content-Type: application/json' \
  -H 'x-request-id: runtime-security-submit-replay-rid' \
  -o "$RUNTIME_DIR/submit_replay.body" \
  -w '%{http_code}' \
  --data "$submit_payload" \
  "$API_URL/api/trading/orders" 2>/dev/null || printf '000')"
[[ "$submit_replay_code" == "200" ]] || fail "paper order replay status=$submit_replay_code"
jq -e --slurpfile first "$RUNTIME_DIR/submit.body" '
  .intent.id == $first[0].intent.id
  and .intent.clientOrderId == "runtime-security-order-idem"
' "$RUNTIME_DIR/submit_replay.body" >/dev/null || fail "paper order replay did not restore the first payload"

cancel_replay_code="$(curl -sS -m 10 \
  -X POST \
  -H "Authorization: Bearer $API_TOKEN" \
  -H 'x-request-id: runtime-security-cancel-replay-rid' \
  -o "$RUNTIME_DIR/cancel_replay.body" \
  -w '%{http_code}' \
  "$API_URL/api/trading/orders/runtime-security-missing/cancel" 2>/dev/null || printf '000')"
[[ "$cancel_replay_code" == "409" ]] || fail "missing order cancel replay status=$cancel_replay_code"
jq -e '.error.code == "ACTION_RUN_REPLAY_FAILED"' "$RUNTIME_DIR/cancel_replay.body" >/dev/null \
  || fail "missing order cancel replay lost the failed ActionRun outcome"

sleep 0.2
audit_lines_after_replay="$(awk 'END { print NR + 0 }' "$AUDIT_LOG" 2>/dev/null || printf '0')"
[[ "$audit_lines_after_replay" == "$audit_lines_before_replay" ]] \
  || fail "idempotent replay appended audit events before=$audit_lines_before_replay after=$audit_lines_after_replay"

action_runs_code="$(curl_status "$RUNTIME_DIR/action_runs.body" \
  -H "Authorization: Bearer $API_TOKEN" \
  "$API_URL/api/trading/action-runs")"
[[ "$action_runs_code" == "200" ]] || fail "action-run list status=$action_runs_code"
jq -e '
  .status == "ready"
  and .source == "action-run-registry"
  and (.data | type == "array")
  and .coverage.truncated == false
  and .coverage.expected == (.data | length)
  and .coverage.observed == (.data | length)
  and (.problems // [] | length) == 0
' "$RUNTIME_DIR/action_runs.body" >/dev/null \
  || fail "action-run list did not return a complete shared resource envelope"

[[ -s "$AUDIT_LOG" ]] || fail "audit log was not written"

kill_run_id="$(jq -er '.actionRunId' "$RUNTIME_DIR/kill_switch.body")" \
  || fail "kill-switch response lost actionRunId"
risk_run_id="$(jq -er '
  .data[]
  | select(
      .kind == "trading_risk_config_update"
      and .idempotencyKey == "runtime-security-risk-idem"
    )
  | .id
' "$RUNTIME_DIR/action_runs.body")" || fail "risk-config ActionRun is missing from the runtime ledger"
credential_run_id="$(jq -er '.error.details.actionRunId' "$RUNTIME_DIR/credential_replay.body")" \
  || fail "credential replay lost actionRunId"
adapter_run_id="$(jq -er '
  .data[] | select(.kind == "trading_adapter_select" and .requestId == "runtime-security-adapter-rid") | .id
' "$RUNTIME_DIR/action_runs.body")" || fail "adapter select ActionRun is missing from the runtime ledger"
reconcile_run_id="$(jq -er '
  .data[] | select(.kind == "trading_order_reconcile" and .requestId == "runtime-security-reconcile-rid") | .id
' "$RUNTIME_DIR/action_runs.body")" || fail "order reconcile ActionRun is missing from the runtime ledger"
fee_run_id="$(jq -er '
  .data[] | select(.kind == "trading_fee_snapshot_upsert" and .requestId == "runtime-security-fee-rid") | .id
' "$RUNTIME_DIR/action_runs.body")" || fail "fee snapshot ActionRun is missing from the runtime ledger"
submit_run_id="$(jq -er '
  .data[] | select(.kind == "trading_order_submit" and .requestId == "runtime-security-submit-rid") | .id
' "$RUNTIME_DIR/action_runs.body")" || fail "order submit ActionRun is missing from the runtime ledger"
cancel_run_id="$(jq -er '
  .data[] | select(.kind == "trading_order_cancel" and .requestId == "runtime-security-cancel-rid") | .id
' "$RUNTIME_DIR/action_runs.body")" || fail "order cancel ActionRun is missing from the runtime ledger"
close_all_run_id="$(jq -er '
  .data[] | select(.kind == "portfolio_close_all" and .requestId == "runtime-security-close-all-rid") | .id
' "$RUNTIME_DIR/action_runs.body")" || fail "close-all ActionRun is missing from the runtime ledger"

kill_detail_code="$(curl_status "$RUNTIME_DIR/kill_action_run.body" \
  -H "Authorization: Bearer $API_TOKEN" \
  "$API_URL/api/trading/action-runs/$kill_run_id")"
[[ "$kill_detail_code" == "200" ]] || fail "kill-switch ActionRun detail status=$kill_detail_code"
risk_detail_code="$(curl_status "$RUNTIME_DIR/risk_action_run.body" \
  -H "Authorization: Bearer $API_TOKEN" \
  "$API_URL/api/trading/action-runs/$risk_run_id")"
[[ "$risk_detail_code" == "200" ]] || fail "risk-config ActionRun detail status=$risk_detail_code"
credential_detail_code="$(curl_status "$RUNTIME_DIR/credential_action_run.body" \
  -H "Authorization: Bearer $API_TOKEN" \
  "$API_URL/api/trading/action-runs/$credential_run_id")"
[[ "$credential_detail_code" == "200" ]] \
  || fail "credential ActionRun detail status=$credential_detail_code"

for detail in \
  "adapter:$adapter_run_id" \
  "reconcile:$reconcile_run_id" \
  "fee:$fee_run_id" \
  "submit:$submit_run_id" \
  "cancel:$cancel_run_id" \
  "close_all:$close_all_run_id"; do
  detail_name="${detail%%:*}"
  detail_id="${detail#*:}"
  detail_code="$(curl_status "$RUNTIME_DIR/${detail_name}_action_run.body" \
    -H "Authorization: Bearer $API_TOKEN" \
    "$API_URL/api/trading/action-runs/$detail_id")"
  [[ "$detail_code" == "200" ]] || fail "$detail_name ActionRun detail status=$detail_code"
done

jq -e \
  --arg run_id "$kill_run_id" '
  .id == $run_id
  and .kind == "trading_kill_switch"
  and .status == "succeeded"
  and .requestId == "runtime-security-audit-rid"
  and .idempotencyKey == "runtime-security-kill-idem"
  and (.actor | test("^api-token:[0-9a-f]{16}$"))
' "$RUNTIME_DIR/kill_action_run.body" >/dev/null || fail "kill-switch ActionRun detail drifted"
jq -e \
  --arg run_id "$risk_run_id" '
  .id == $run_id
  and .kind == "trading_risk_config_update"
  and .status == "succeeded"
  and .requestId == "runtime-security-risk-rid"
  and .idempotencyKey == "runtime-security-risk-idem"
  and (.actor | test("^api-token:[0-9a-f]{16}$"))
' "$RUNTIME_DIR/risk_action_run.body" >/dev/null || fail "risk-config ActionRun detail drifted"
jq -e \
  --arg run_id "$credential_run_id" '
  .id == $run_id
  and .kind == "venue_credentials_update"
  and .status == "failed"
  and .requestId == "runtime-security-credential-rid"
  and .idempotencyKey == "runtime-security-credential-idem"
  and .problem.code == "CREDENTIAL_UNKNOWN_VENUE"
  and (.actor | test("^api-token:[0-9a-f]{16}$"))
' "$RUNTIME_DIR/credential_action_run.body" >/dev/null || fail "credential ActionRun detail drifted"
jq -e \
  --arg run_id "$adapter_run_id" '
  .id == $run_id and .kind == "trading_adapter_select" and .status == "succeeded"
  and .requestId == "runtime-security-adapter-rid" and .result.adapter == "mock"
  and (.actor | test("^api-token:[0-9a-f]{16}$"))
' "$RUNTIME_DIR/adapter_action_run.body" >/dev/null || fail "adapter ActionRun detail drifted"
jq -e \
  --arg run_id "$reconcile_run_id" '
  .id == $run_id and .kind == "trading_order_reconcile" and .status == "succeeded"
  and .requestId == "runtime-security-reconcile-rid" and (.result | type == "array")
  and (.actor | test("^api-token:[0-9a-f]{16}$"))
' "$RUNTIME_DIR/reconcile_action_run.body" >/dev/null || fail "reconcile ActionRun detail drifted"
jq -e \
  --arg run_id "$fee_run_id" '
  .id == $run_id and .kind == "trading_fee_snapshot_upsert" and .status == "failed"
  and .requestId == "runtime-security-fee-rid" and .problem.status == 400
  and (.actor | test("^api-token:[0-9a-f]{16}$"))
' "$RUNTIME_DIR/fee_action_run.body" >/dev/null || fail "fee ActionRun detail drifted"
jq -e \
  --arg run_id "$submit_run_id" '
  .id == $run_id and .kind == "trading_order_submit" and .status == "succeeded"
  and .requestId == "runtime-security-submit-rid"
  and .idempotencyKey == "runtime-security-order-idem"
  and .result.intent.clientOrderId == "runtime-security-order-idem"
  and (.actor | test("^api-token:[0-9a-f]{16}$"))
' "$RUNTIME_DIR/submit_action_run.body" >/dev/null || fail "submit ActionRun detail drifted"
jq -e \
  --arg run_id "$cancel_run_id" '
  .id == $run_id and .kind == "trading_order_cancel" and .status == "failed"
  and .requestId == "runtime-security-cancel-rid"
  and .idempotencyKey == "cancel:runtime-security-missing"
  and .problem.status == 404
  and (.actor | test("^api-token:[0-9a-f]{16}$"))
' "$RUNTIME_DIR/cancel_action_run.body" >/dev/null || fail "cancel ActionRun detail drifted"
jq -e \
  --arg run_id "$close_all_run_id" '
  .id == $run_id and .kind == "portfolio_close_all" and .status == "failed"
  and .requestId == "runtime-security-close-all-rid"
  and .idempotencyKey == "runtime-security-close-all-idem"
  and .problem.code == "BAD_REQUEST" and .problem.status == 400
  and (.actor | test("^api-token:[0-9a-f]{16}$"))
' "$RUNTIME_DIR/close_all_action_run.body" >/dev/null || fail "close-all ActionRun detail drifted"

jq -s -e \
  --argjson expected_audit_lines "$expected_audit_lines_before_replay" \
  --arg kill_run_id "$kill_run_id" \
  --arg risk_run_id "$risk_run_id" \
  --arg credential_run_id "$credential_run_id" \
  --arg adapter_run_id "$adapter_run_id" \
  --arg reconcile_run_id "$reconcile_run_id" \
  --arg fee_run_id "$fee_run_id" \
  --arg submit_run_id "$submit_run_id" \
  --arg cancel_run_id "$cancel_run_id" \
  --arg close_all_run_id "$close_all_run_id" '
  def pair($action; $resource; $request_id; $idempotency_key; $run_id; $terminal):
    [.[] | select(
      .action == $action
      and .resource == $resource
      and (.actor | test("^api-token:[0-9a-f]{16}$"))
      and .detail.requestId == $request_id
      and .detail.idempotencyKey == $idempotency_key
    )] as $events
    | ($events | length) == 2
      and ([$events[].outcome] | sort) == (["accepted", $terminal] | sort)
      and ([$events[].detail.actionRunId] | all(. == $run_id))
      and ([$events[].detail.actionRunId] | unique | length) == 1;
  def pair_without_idempotency($action; $resource; $request_id; $run_id; $terminal):
    [.[] | select(
      .action == $action
      and .resource == $resource
      and (.actor | test("^api-token:[0-9a-f]{16}$"))
      and .detail.requestId == $request_id
      and .detail.idempotencyKey == null
    )] as $events
    | ($events | length) == 2
      and ([$events[].outcome] | sort) == (["accepted", $terminal] | sort)
      and ([$events[].detail.actionRunId] | all(. == $run_id));
  length == $expected_audit_lines
  and pair("trading.kill_switch.set"; "kill-switch:off"; "runtime-security-audit-rid"; "runtime-security-kill-idem"; $kill_run_id; "success")
  and pair("trading.risk_config.update"; "risk-config"; "runtime-security-risk-rid"; "runtime-security-risk-idem"; $risk_run_id; "success")
  and pair("venue_credentials.update"; "runtime-invalid-venue"; "runtime-security-credential-rid"; "runtime-security-credential-idem"; $credential_run_id; "denied")
  and pair_without_idempotency("trading.adapter.select"; "mock"; "runtime-security-adapter-rid"; $adapter_run_id; "success")
  and pair_without_idempotency("trading.order.reconcile"; "open-orders"; "runtime-security-reconcile-rid"; $reconcile_run_id; "success")
  and pair_without_idempotency("trading.fee_snapshot.upsert"; "runtime:BTCUSDT:Perp"; "runtime-security-fee-rid"; $fee_run_id; "denied")
  and pair("trading.order.submit"; "runtime-security-order-idem"; "runtime-security-submit-rid"; "runtime-security-order-idem"; $submit_run_id; "success")
  and pair("trading.order.cancel"; "runtime-security-missing"; "runtime-security-cancel-rid"; "cancel:runtime-security-missing"; $cancel_run_id; "denied")
  and pair("portfolio.positions.close_all"; "all-positions"; "runtime-security-close-all-rid"; "runtime-security-close-all-idem"; $close_all_run_id; "denied")
' "$AUDIT_LOG" >/dev/null || fail "cross-route accepted/terminal audit pairs are missing or uncorrelated"

for secret in "$API_TOKEN" "$CREDENTIAL_KEY" "$CREDENTIAL_SECRET"; do
  rg -Fq -- "$secret" "$AUDIT_LOG" && fail "audit log leaked a credential sentinel"
done
jq -s -e '
  [.. | objects | keys[]]
  | all(test("^(api_?key|secret|passphrase|authorization|private_?key)$"; "i") | not)
' "$AUDIT_LOG" >/dev/null || fail "audit log contains a forbidden secret-bearing key"

audit_prefix_checksum="$(head -n "$audit_lines_before_replay" "$AUDIT_LOG" | cksum)"
stop_api
start_api

restart_code="$(curl -sS -m 10 \
  -X POST \
  -H "Authorization: Bearer $API_TOKEN" \
  -H 'Content-Type: application/json' \
  -H 'x-request-id: runtime-security-restart-rid' \
  -H 'idempotency-key: runtime-security-restart-idem' \
  -o "$RUNTIME_DIR/restart_kill_switch.body" \
  -w '%{http_code}' \
  --data '{"active":true,"expectedActive":false,"expectedOpenOrderCount":0,"reason":"runtime.security.restart.smoke"}' \
  "$API_URL/api/trading/kill-switch" 2>/dev/null || printf '000')"
[[ "$restart_code" == "200" ]] || fail "post-restart kill-switch status=$restart_code"
restart_run_id="$(jq -er '.actionRunId' "$RUNTIME_DIR/restart_kill_switch.body")" \
  || fail "post-restart kill-switch response lost actionRunId"

expected_audit_lines_after_restart=$((audit_lines_before_replay + 2))
for _ in {1..10}; do
  [[ "$(wc -l <"$AUDIT_LOG" 2>/dev/null || printf '0')" -ge "$expected_audit_lines_after_restart" ]] && break
  sleep 0.2
done
[[ "$(head -n "$audit_lines_before_replay" "$AUDIT_LOG" | cksum)" == "$audit_prefix_checksum" ]] \
  || fail "audit log prefix changed across process restart"
jq -s -e \
  --argjson expected_audit_lines "$expected_audit_lines_after_restart" \
  --arg restart_run_id "$restart_run_id" '
  def pair($action; $resource; $request_id; $idempotency_key; $run_id; $terminal):
    [.[] | select(
      .action == $action
      and .resource == $resource
      and .detail.requestId == $request_id
      and .detail.idempotencyKey == $idempotency_key
      and .detail.actionRunId == $run_id
    )] as $events
    | ($events | length) == 2
      and ([$events[].outcome] | sort) == (["accepted", $terminal] | sort);
  length == $expected_audit_lines
  and pair("trading.kill_switch.set"; "kill-switch:on"; "runtime-security-restart-rid"; "runtime-security-restart-idem"; $restart_run_id; "success")
' "$AUDIT_LOG" >/dev/null || fail "audit log did not append correlated evidence across restart"

for secret in "$API_TOKEN" "$CREDENTIAL_KEY" "$CREDENTIAL_SECRET"; do
  rg -Fq -- "$secret" "$AUDIT_LOG" && fail "restarted audit log leaked a credential sentinel"
done

SUCCESS=1
printf 'OK api security runtime smoke base=%s audit=%s\n' "$API_URL" "$AUDIT_LOG"
