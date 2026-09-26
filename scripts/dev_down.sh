#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUNTIME_DIR="${RUNTIME_DIR:-$ROOT/.crossline-runtime/dev}"
API_PORT="${APP_PORT:-${API_PORT:-8000}}"
FRONTEND_PORT="${FRONTEND_PORT:-${TRUNK_SERVE_PORT:-8080}}"
DEV_RELEASE_PORTS="${DEV_RELEASE_PORTS:-0}"
API_LAUNCHD_LABEL="${DEV_API_LAUNCHD_LABEL:-com.crossline.omni.dev.api.$API_PORT}"
FRONTEND_LAUNCHD_LABEL="${DEV_FRONTEND_LAUNCHD_LABEL:-com.crossline.omni.dev.frontend.$FRONTEND_PORT}"

mkdir -p "$RUNTIME_DIR/logs"

read_pid() {
  local path="$1"
  if [[ -f "$path" ]]; then
    tr -dc '0-9' <"$path"
  fi
}

pid_alive() {
  local pid="$1"
  [[ -n "$pid" ]] || return 1
  if kill -0 "$pid" 2>/dev/null; then
    return 0
  fi
  command -v lsof >/dev/null 2>&1 || return 1
  lsof -a -p "$pid" -d cwd -Fp 2>/dev/null | grep -qx "p$pid"
}

wait_exit() {
  local pid="$1"
  for _ in {1..20}; do
    pid_alive "$pid" || return 0
    sleep 0.1
  done
  return 1
}

terminate_pid() {
  local pid="$1"
  kill "$pid" 2>/dev/null || true
  wait_exit "$pid" && return 0
  kill -9 "$pid" 2>/dev/null || true
  wait_exit "$pid"
}

stop_pid_file() {
  local label="$1"
  local path="$2"
  local pid
  pid="$(read_pid "$path")"
  if [[ -z "$pid" ]]; then
    rm -f "$path"
    return 0
  fi
  if ! pid_alive "$pid"; then
    rm -f "$path"
    printf '%s pid %s already stopped\n' "$label" "$pid"
    return 0
  fi
  if ! terminate_pid "$pid"; then
    printf '%s pid %s could not be stopped; pid file retained\n' "$label" "$pid" >&2
    return 1
  fi
  rm -f "$path"
  printf '%s pid %s stopped\n' "$label" "$pid"
}

remove_launchd_job() {
  local label="$1"
  command -v launchctl >/dev/null 2>&1 || return 0
  launchctl remove "$label" >/dev/null 2>&1 || true
}

release_port() {
  local label="$1"
  local port="$2"
  local pids pid

  command -v lsof >/dev/null 2>&1 || return 0
  pids="$(lsof -tiTCP:"$port" -sTCP:LISTEN 2>/dev/null | sort -u || true)"
  [[ -n "$pids" ]] || return 0
  for pid in $pids; do
    [[ "$pid" != "$$" ]] || continue
    if ! project_owned_listener "$label" "$pid"; then
      printf '%s port %s pid %s left untouched (not a CROSSLINE dev process)\n' "$label" "$port" "$pid"
      continue
    fi
    if ! terminate_pid "$pid"; then
      printf '%s port %s pid %s could not be released\n' "$label" "$port" "$pid" >&2
      return 1
    fi
    printf '%s port %s pid %s released\n' "$label" "$port" "$pid"
  done
}

project_owned_listener() {
  local label="$1"
  local pid="$2"
  local command cwd executable
  command="$(ps -p "$pid" -o command= 2>/dev/null || true)"
  cwd="$(lsof -a -p "$pid" -d cwd -Fn 2>/dev/null | sed -n 's/^n//p' | head -1)"
  executable="$(lsof -a -p "$pid" -d txt -Fn 2>/dev/null | sed -n 's/^n//p' | head -1)"
  case "$label" in
    api)
      [[ "$command" == *"$ROOT/target/debug/crypto-arb-api"* || "$executable" == "$ROOT/target/debug/crypto-arb-api" ]]
      ;;
    frontend)
      [[ ("$command" == *"trunk serve"* || "$executable" == */trunk) && "$cwd" == "$ROOT/frontend" ]]
      ;;
    *)
      return 1
      ;;
  esac
}

remove_launchd_job "$API_LAUNCHD_LABEL"
remove_launchd_job "$FRONTEND_LAUNCHD_LABEL"
stop_pid_file api "$RUNTIME_DIR/api.pid"
stop_pid_file frontend "$RUNTIME_DIR/frontend.pid"

if [[ "$DEV_RELEASE_PORTS" == "1" ]]; then
  release_port api "$API_PORT"
  release_port frontend "$FRONTEND_PORT"
fi

bash "$ROOT/scripts/verify_runtime.sh" --optional >/dev/null || true
printf 'dev runtime stopped runtime_dir=%s\n' "$RUNTIME_DIR"
bash "$ROOT/scripts/cache_hygiene.sh" --auto-clean || printf 'cache maintenance deferred\n' >&2
