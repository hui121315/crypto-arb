#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUNTIME_DIR="${RUNTIME_DIR:-$ROOT/.crossline-runtime/dev}"
LOG_DIR="$RUNTIME_DIR/logs"
PROBE_DIR="$RUNTIME_DIR/probes/$$"
API_HOST="${APP_HOST:-${API_HOST:-127.0.0.1}}"
API_PORT="${APP_PORT:-${API_PORT:-8000}}"
FRONTEND_HOST="${FRONTEND_HOST:-${TRUNK_SERVE_ADDRESS:-127.0.0.1}}"
FRONTEND_PORT="${FRONTEND_PORT:-${TRUNK_SERVE_PORT:-8080}}"
API_URL="${API_URL:-http://$API_HOST:$API_PORT}"
FRONTEND_URL="${FRONTEND_URL:-http://$FRONTEND_HOST:$FRONTEND_PORT}"
HTTP_TIMEOUT_SECS="${HTTP_TIMEOUT_SECS:-5}"
SNAPSHOT_PATH="${SNAPSHOT_PATH:-$RUNTIME_DIR/runtime_snapshot.json}"
OPTIONAL=0
EXTERNAL=0
ALLOW_DEGRADED_RUNTIME="${ALLOW_DEGRADED_RUNTIME:-0}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --optional)
      OPTIONAL=1
      shift
      ;;
    --external)
      EXTERNAL=1
      shift
      ;;
    *)
      printf 'verify_runtime: unknown argument: %s\n' "$1" >&2
      exit 2
      ;;
  esac
done

mkdir -p "$LOG_DIR" "$PROBE_DIR"
trap 'rm -rf "$PROBE_DIR"' EXIT

read_pid() {
  local path="$1"
  if [[ -f "$path" ]]; then
    tr -dc '0-9' <"$path"
  fi
}

pid_alive() {
  local pid="$1"
  [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null
}

run_probe() {
  local prefix="$1"
  local url="$2"
  local body="$3"
  local err="$4"
  local meta code time_total error
  local curl_args=(-sS -L -m "$HTTP_TIMEOUT_SECS" -o "$body" -w '%{http_code} %{time_total}')

  rm -f "$body" "$err"
  if [[ -n "${API_BEARER_TOKEN:-}" && "$url" == "$API_URL"* ]]; then
    curl_args+=(-H "Authorization: Bearer $API_BEARER_TOKEN")
  fi

  if meta="$(curl "${curl_args[@]}" "$url" 2>"$err")"; then
    code="${meta%% *}"
    time_total="${meta#* }"
    error=""
  else
    code="000"
    time_total="0"
    error="$(tr '\n' ' ' <"$err" | sed 's/[[:space:]]*$//')"
  fi

  printf -v "${prefix}_CODE" '%s' "$code"
  printf -v "${prefix}_TIME" '%s' "$time_total"
  printf -v "${prefix}_ERROR" '%s' "$error"
}

API_PID="$(read_pid "$RUNTIME_DIR/api.pid")"
FRONTEND_PID="$(read_pid "$RUNTIME_DIR/frontend.pid")"
API_PID_ALIVE=0
FRONTEND_PID_ALIVE=0
pid_alive "$API_PID" && API_PID_ALIVE=1
pid_alive "$FRONTEND_PID" && FRONTEND_PID_ALIVE=1

API_HEALTH_BODY="$PROBE_DIR/api_health.body"
API_HEALTH_ERR="$PROBE_DIR/api_health.err"
FRONTEND_BODY="$PROBE_DIR/frontend.body"
FRONTEND_ERR="$PROBE_DIR/frontend.err"
SYSTEM_BODY="$PROBE_DIR/system_health.body"
SYSTEM_ERR="$PROBE_DIR/system_health.err"

run_probe API_HEALTH "$API_URL/health" "$API_HEALTH_BODY" "$API_HEALTH_ERR"
run_probe FRONTEND "$FRONTEND_URL/" "$FRONTEND_BODY" "$FRONTEND_ERR"

SYSTEM_CODE="000"
SYSTEM_TIME="0"
SYSTEM_ERROR="api health unreachable"
SYSTEM_WS_CHANNELS="null"
SYSTEM_API_HEALTHY=""
SYSTEM_API_TOTAL=""
SYSTEM_DEGRADED=""
SYSTEM_PROBLEMS="[]"

if [[ "$API_HEALTH_CODE" == "200" ]]; then
  run_probe SYSTEM "$API_URL/api/system/health" "$SYSTEM_BODY" "$SYSTEM_ERR"
  if [[ "$SYSTEM_CODE" == "200" ]]; then
    SYSTEM_WS_CHANNELS="$(jq -c '.ws.channels // null' "$SYSTEM_BODY" 2>/dev/null || printf 'null')"
    SYSTEM_API_HEALTHY="$(jq -r '.api.healthy // empty' "$SYSTEM_BODY" 2>/dev/null || true)"
    SYSTEM_API_TOTAL="$(jq -r '.api.total // empty' "$SYSTEM_BODY" 2>/dev/null || true)"
    SYSTEM_DEGRADED="$(jq -r '.degraded // empty' "$SYSTEM_BODY" 2>/dev/null || true)"
    SYSTEM_PROBLEMS="$(jq -c '.problems // []' "$SYSTEM_BODY" 2>/dev/null || printf '[]')"
  fi
fi

FRONTEND_HTML=0
if [[ "$FRONTEND_CODE" == "200" ]] && grep -Eiq '<!doctype html|<html' "$FRONTEND_BODY" 2>/dev/null; then
  FRONTEND_HTML=1
fi

SNAPSHOT_STATUS="ok"
SNAPSHOT_LAST_ERROR=""
if [[ "$OPTIONAL" != "1" && "$EXTERNAL" != "1" && "$API_PID_ALIVE" != "1" ]]; then
  SNAPSHOT_STATUS="unready"
  SNAPSHOT_LAST_ERROR="api pid from $RUNTIME_DIR/api.pid is not alive; use --external to verify an externally managed API"
elif [[ "$OPTIONAL" != "1" && "$EXTERNAL" != "1" && "$FRONTEND_PID_ALIVE" != "1" ]]; then
  SNAPSHOT_STATUS="unready"
  SNAPSHOT_LAST_ERROR="frontend pid from $RUNTIME_DIR/frontend.pid is not alive; use --external to verify an externally managed frontend"
elif [[ "$API_HEALTH_CODE" != "200" ]]; then
  SNAPSHOT_STATUS="unready"
  SNAPSHOT_LAST_ERROR="api health unreachable: ${API_HEALTH_ERROR:-status $API_HEALTH_CODE}"
elif [[ "$FRONTEND_CODE" != "200" || "$FRONTEND_HTML" != "1" ]]; then
  SNAPSHOT_STATUS="unready"
  SNAPSHOT_LAST_ERROR="frontend html unreachable: ${FRONTEND_ERROR:-status $FRONTEND_CODE}"
elif [[ "$SYSTEM_CODE" == "200" && "$SYSTEM_DEGRADED" == "true" ]]; then
  SNAPSHOT_STATUS="degraded"
  SNAPSHOT_LAST_ERROR="$(jq -r '.problems[0].message // .problems[0].reason // empty' "$SYSTEM_BODY" 2>/dev/null || true)"
elif [[ "$SYSTEM_CODE" != "200" ]]; then
  SNAPSHOT_STATUS="degraded"
  SNAPSHOT_LAST_ERROR="system health unavailable: ${SYSTEM_ERROR:-status $SYSTEM_CODE}"
fi

if [[ "$SNAPSHOT_STATUS" == "unready" && "$OPTIONAL" == "1" ]]; then
  SNAPSHOT_STATUS="skipped"
fi

export SNAPSHOT_PATH SNAPSHOT_STATUS SNAPSHOT_LAST_ERROR OPTIONAL EXTERNAL ALLOW_DEGRADED_RUNTIME
export API_URL FRONTEND_URL API_HOST API_PORT FRONTEND_HOST FRONTEND_PORT
export API_PID FRONTEND_PID API_PID_ALIVE FRONTEND_PID_ALIVE
export API_HEALTH_CODE API_HEALTH_TIME API_HEALTH_ERROR API_HEALTH_BODY
export FRONTEND_CODE FRONTEND_TIME FRONTEND_ERROR FRONTEND_HTML FRONTEND_BODY
export SYSTEM_CODE SYSTEM_TIME SYSTEM_ERROR SYSTEM_WS_CHANNELS SYSTEM_API_HEALTHY SYSTEM_API_TOTAL SYSTEM_DEGRADED SYSTEM_PROBLEMS

python3 - <<'PY'
import json
import os
import time


def env(name, default=""):
    return os.environ.get(name, default)


def as_int(raw):
    try:
        return int(raw)
    except (TypeError, ValueError):
        return None


def as_bool(raw):
    return raw in {"1", "true", "True", "yes"}


def as_json(raw, default):
    try:
        return json.loads(raw)
    except (TypeError, json.JSONDecodeError):
        return default


def preview(path, limit=320):
    try:
        with open(path, "r", encoding="utf-8", errors="replace") as handle:
            return handle.read(limit)
    except OSError:
        return ""


snapshot = {
    "generatedAtMs": int(time.time() * 1000),
    "status": env("SNAPSHOT_STATUS"),
    "optional": as_bool(env("OPTIONAL")),
    "external": as_bool(env("EXTERNAL")),
    "allowDegraded": as_bool(env("ALLOW_DEGRADED_RUNTIME")),
    "lastError": env("SNAPSHOT_LAST_ERROR") or None,
    "ports": {
        "api": as_int(env("API_PORT")),
        "frontend": as_int(env("FRONTEND_PORT")),
    },
    "pids": {
        "api": as_int(env("API_PID")),
        "apiAlive": as_bool(env("API_PID_ALIVE")),
        "frontend": as_int(env("FRONTEND_PID")),
        "frontendAlive": as_bool(env("FRONTEND_PID_ALIVE")),
    },
    "api": {
        "baseUrl": env("API_URL"),
        "health": {
            "code": as_int(env("API_HEALTH_CODE")),
            "timeTotalSec": env("API_HEALTH_TIME"),
            "error": env("API_HEALTH_ERROR") or None,
            "bodyPreview": preview(env("API_HEALTH_BODY")),
        },
        "systemHealth": {
            "code": as_int(env("SYSTEM_CODE")),
            "timeTotalSec": env("SYSTEM_TIME"),
            "error": env("SYSTEM_ERROR") or None,
            "apiHealthy": as_int(env("SYSTEM_API_HEALTHY")),
            "apiTotal": as_int(env("SYSTEM_API_TOTAL")),
            "degraded": env("SYSTEM_DEGRADED") == "true",
            "problems": as_json(env("SYSTEM_PROBLEMS"), []),
        },
    },
    "ws": {
        "channels": as_json(env("SYSTEM_WS_CHANNELS"), None),
    },
    "frontend": {
        "url": env("FRONTEND_URL"),
        "html": as_bool(env("FRONTEND_HTML")),
        "health": {
            "code": as_int(env("FRONTEND_CODE")),
            "timeTotalSec": env("FRONTEND_TIME"),
            "error": env("FRONTEND_ERROR") or None,
            "bodyPreview": preview(env("FRONTEND_BODY")),
        },
    },
    "artifacts": {
        "runtimeDir": os.path.dirname(env("SNAPSHOT_PATH")),
        "apiLog": os.path.join(os.path.dirname(env("SNAPSHOT_PATH")), "logs", "api.log"),
        "frontendLog": os.path.join(os.path.dirname(env("SNAPSHOT_PATH")), "logs", "frontend.log"),
    },
}

with open(env("SNAPSHOT_PATH"), "w", encoding="utf-8") as handle:
    json.dump(snapshot, handle, ensure_ascii=False, indent=2, sort_keys=True)
    handle.write("\n")
PY

printf 'runtime snapshot written %s status=%s api=%s frontend=%s\n' \
  "$SNAPSHOT_PATH" "$SNAPSHOT_STATUS" "$API_HEALTH_CODE" "$FRONTEND_CODE"

if [[ "$SNAPSHOT_STATUS" == "unready" ]]; then
  printf 'runtime unready: %s\n' "$SNAPSHOT_LAST_ERROR" >&2
  exit 1
fi

if [[ "$SNAPSHOT_STATUS" == "degraded" && "$OPTIONAL" != "1" && "$ALLOW_DEGRADED_RUNTIME" != "1" ]]; then
  printf 'runtime degraded: %s\n' "$SNAPSHOT_LAST_ERROR" >&2
  exit 1
fi
