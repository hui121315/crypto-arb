#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUNTIME_DIR="$(mktemp -d "${TMPDIR:-/tmp}/crossline-runtime-required.XXXXXX")"
API_HOST="${RUNTIME_API_HOST:-127.0.0.1}"
API_PORT="${RUNTIME_API_PORT:-}"
API_BIN="${RUNTIME_API_BINARY:-$ROOT/target/debug/crypto-arb-api}"
API_LOG="$RUNTIME_DIR/api.log"
STARTUP_TIMEOUT_SECS="${RUNTIME_STARTUP_TIMEOUT_SECS:-90}"
API_PID=""
SUCCESS=0

fail() {
  printf 'runtime-required gate failed: %s\n' "$*" >&2
  if [[ -f "$API_LOG" ]]; then
    tail -120 "$API_LOG" >&2 || true
  fi
  exit 1
}

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "missing command: $1"
}

find_port() {
  python3 - <<'PY'
import socket
with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
    sock.bind(("127.0.0.1", 0))
    print(sock.getsockname()[1])
PY
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

cleanup() {
  stop_api
  if [[ "$SUCCESS" == "1" || "${KEEP_RUNTIME_DIR_ON_FAILURE:-0}" != "1" ]]; then
    rm -rf "$RUNTIME_DIR"
  else
    printf 'runtime-required artifacts kept at %s\n' "$RUNTIME_DIR" >&2
  fi
}
trap cleanup EXIT

wait_for_health() {
  local deadline=$((SECONDS + STARTUP_TIMEOUT_SECS))
  while (( SECONDS < deadline )); do
    if [[ -n "$API_PID" ]] && ! kill -0 "$API_PID" 2>/dev/null; then
      local status=0
      wait "$API_PID" 2>/dev/null || status="$?"
      API_PID=""
      fail "API exited before health check status=$status"
    fi
    if curl -fsS -m 2 "$API_URL/health" >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  fail "API did not become healthy within ${STARTUP_TIMEOUT_SECS}s base=$API_URL"
}

start_api() {
  mkdir -p "$RUNTIME_DIR/data"
  (
    cd "$RUNTIME_DIR"
    exec env \
      -u API_BEARER_TOKEN \
      -u APP_SECURITY__AUTH_TOKEN \
      APP_HOST="$API_HOST" \
      APP_PORT="$API_PORT" \
      APP_STORAGE__DATA_DIR="$RUNTIME_DIR/data" \
      RUST_LOG="${RUNTIME_RUST_LOG:-info}" \
      "$API_BIN"
  ) >"$API_LOG" 2>&1 &
  API_PID="$!"
  wait_for_health
}

run_contracts() {
  if [[ -n "${RUNTIME_API_BEARER_TOKEN:-}" ]]; then
    ALLOW_RUNTIME_SKIP=0 \
      API_URL="$API_URL" \
      API_BEARER_TOKEN="$RUNTIME_API_BEARER_TOKEN" \
      bash "$ROOT/scripts/verify_runtime_contracts.sh"
  else
    env -u API_BEARER_TOKEN \
      ALLOW_RUNTIME_SKIP=0 \
      API_URL="$API_URL" \
      bash "$ROOT/scripts/verify_runtime_contracts.sh"
  fi
}

require_cmd curl
require_cmd jq
require_cmd python3

if [[ -n "${RUNTIME_API_URL:-}" ]]; then
  API_URL="${RUNTIME_API_URL%/}"
else
  require_cmd cargo
  if [[ "${RUNTIME_BUILD_API:-1}" == "1" ]]; then
    cargo build --locked -p api
  fi
  [[ -x "$API_BIN" ]] || fail "API binary is missing or not executable: $API_BIN"
  API_PORT="${API_PORT:-$(find_port)}"
  API_URL="http://$API_HOST:$API_PORT"
  start_api
fi

run_contracts
SUCCESS=1
printf 'OK runtime-required contracts base=%s\n' "$API_URL"
