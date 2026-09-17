#!/usr/bin/env bash
set -euo pipefail

run_launchd_worker() {
  local python_bin="$1"
  shift
  "$python_bin" - "$@" <<'PY'
import os
import sys


cwd, pid_path, env_fifo, *command = sys.argv[1:]
if not command:
    raise SystemExit("launchd worker command is required")

with open(env_fifo, "rb") as env_stream:
    entries = env_stream.read().split(b"\0")
for entry in entries:
    key, separator, value = entry.partition(b"=")
    if separator and key:
        os.environb[key] = value

os.umask(0o077)
os.chdir(cwd)
with open(pid_path, "w", encoding="ascii") as pid_file:
    pid_file.write(f"{os.getpid()}\n")
os.execvpe(command[0], command, os.environ)
PY
}

if [[ "${1:-}" == "--launchd-worker" ]]; then
  shift
  run_launchd_worker "$@"
  exit 0
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUNTIME_DIR="${RUNTIME_DIR:-$ROOT/.crossline-runtime/dev}"
LOG_DIR="$RUNTIME_DIR/logs"
API_DATA_DIR_MARKER="$RUNTIME_DIR/api_data_dir"
if [[ -n "${APP_STORAGE__DATA_DIR:-}" ]]; then
  API_DATA_DIR="$APP_STORAGE__DATA_DIR"
elif [[ -s "$API_DATA_DIR_MARKER" ]]; then
  API_DATA_DIR="$(sed -n '1p' "$API_DATA_DIR_MARKER")"
  API_DATA_DIR="${API_DATA_DIR:-$RUNTIME_DIR/data}"
else
  API_DATA_DIR="$RUNTIME_DIR/data"
fi
API_HOST="${APP_HOST:-${API_HOST:-127.0.0.1}}"
API_PORT="${APP_PORT:-${API_PORT:-8000}}"
FRONTEND_HOST="${FRONTEND_HOST:-${TRUNK_SERVE_ADDRESS:-127.0.0.1}}"
FRONTEND_PORT="${FRONTEND_PORT:-${TRUNK_SERVE_PORT:-8080}}"
FRONTEND_RELEASE="${FRONTEND_RELEASE:-false}"
API_URL="${API_URL:-http://$API_HOST:$API_PORT}"
FRONTEND_URL="${FRONTEND_URL:-http://$FRONTEND_HOST:$FRONTEND_PORT}"
STARTUP_TIMEOUT_SECS="${STARTUP_TIMEOUT_SECS:-120}"
STARTED_API=0
STARTED_FRONTEND=0
SUCCESS=0
PROCESS_SUPERVISOR="${DEV_PROCESS_SUPERVISOR:-auto}"

mkdir -p "$LOG_DIR"
mkdir -p "$API_DATA_DIR"
API_DATA_DIR_MARKER_TMP="$API_DATA_DIR_MARKER.tmp.$$"
printf '%s\n' "$API_DATA_DIR" >"$API_DATA_DIR_MARKER_TMP"
chmod 600 "$API_DATA_DIR_MARKER_TMP"
mv -f "$API_DATA_DIR_MARKER_TMP" "$API_DATA_DIR_MARKER"

if [[ "${1:-}" == "--print-data-dir" ]]; then
  printf '%s\n' "$API_DATA_DIR"
  exit 0
fi

fail() {
  printf 'dev_up failed: %s\n' "$*" >&2
  exit 1
}

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "missing command: $1"
}

port_pids() {
  local port="$1"
  command -v lsof >/dev/null 2>&1 || return 0
  lsof -tiTCP:"$port" -sTCP:LISTEN 2>/dev/null | sort -u || true
}

ensure_port_free() {
  local label="$1"
  local port="$2"
  local pids
  pids="$(port_pids "$port")"
  [[ -z "$pids" ]] || fail "$label port $port already in use by pid(s): $pids; run scripts/dev_restart.sh"
}

pid_alive() {
  local pid="$1"
  [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null
}

read_pid_file() {
  local path="$1"
  if [[ -f "$path" ]]; then
    tr -dc '0-9' <"$path"
  fi
}

stop_started_pid() {
  local label="$1"
  local path="$2"
  local started="$3"
  local launchd_label="$4"
  local pid
  [[ "$started" == "1" ]] || return 0
  if [[ "$PROCESS_SUPERVISOR" == "launchctl" ]]; then
    launchctl remove "$launchd_label" >/dev/null 2>&1 || true
  fi
  pid="$(read_pid_file "$path")"
  if pid_alive "$pid"; then
    kill "$pid" 2>/dev/null || true
    printf '%s pid %s cleaned up\n' "$label" "$pid" >&2
  fi
  rm -f "$path"
}

spawn_with_double_fork() {
  local cwd="$1"
  local log="$2"
  local pid_file="$3"
  shift 3

  python3 - "$cwd" "$log" "$pid_file" "$@" <<'PY'
import os
import resource
import signal
import sys


cwd, log_path, pid_path, *command = sys.argv[1:]
if not command:
    raise SystemExit("detached process command is required")

first_child = os.fork()
if first_child:
    _, status = os.waitpid(first_child, 0)
    if not os.WIFEXITED(status) or os.WEXITSTATUS(status) != 0:
        raise SystemExit("failed to create detached process session")
    raise SystemExit(0)

try:
    os.setsid()
    second_child = os.fork()
except OSError as error:
    print(f"failed to detach process: {error}", file=sys.stderr)
    os._exit(1)

if second_child:
    os._exit(0)

os.umask(0o077)
os.chdir(cwd)
null_fd = os.open(os.devnull, os.O_RDONLY)
log_fd = os.open(log_path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
os.dup2(null_fd, 0)
os.dup2(log_fd, 1)
os.dup2(log_fd, 2)
if null_fd > 2:
    os.close(null_fd)
if log_fd > 2:
    os.close(log_fd)

with open(pid_path, "w", encoding="ascii") as pid_file:
    pid_file.write(f"{os.getpid()}\n")

for signal_number in (signal.SIGHUP, signal.SIGINT, signal.SIGTERM):
    signal.signal(signal_number, signal.SIG_DFL)

max_fd = resource.getrlimit(resource.RLIMIT_NOFILE)[0]
if max_fd == resource.RLIM_INFINITY:
    max_fd = 65_536
os.closerange(3, min(int(max_fd), 65_536))
os.execvpe(command[0], command, os.environ)
PY
}

spawn_with_launchctl() {
  local label="$1"
  local cwd="$2"
  local log="$3"
  local pid_file="$4"
  local launchd_label="$5"
  shift 5
  local env_fifo="$RUNTIME_DIR/.${label}.env.$$.fifo"

  rm -f "$env_fifo"
  mkfifo -m 600 "$env_fifo"
  launchctl remove "$launchd_label" >/dev/null 2>&1 || true
  : >"$log"
  chmod 600 "$log"
  if ! launchctl submit -l "$launchd_label" -o "$log" -e "$log" -- \
    /bin/bash "$ROOT/scripts/dev_up.sh" --launchd-worker \
    "$PYTHON3_BIN" "$cwd" "$pid_file" "$env_fifo" "$@"; then
    rm -f "$env_fifo"
    fail "$label launchctl submission failed"
  fi

  if ! "$PYTHON3_BIN" - "$env_fifo" <<'PY'
import errno
import os
import sys
import time


fifo_path = sys.argv[1]
deadline = time.monotonic() + 5
while True:
    try:
        fifo_fd = os.open(fifo_path, os.O_WRONLY | os.O_NONBLOCK)
        break
    except OSError as error:
        if error.errno != errno.ENXIO or time.monotonic() >= deadline:
            raise
        time.sleep(0.05)

with os.fdopen(fifo_fd, "wb", buffering=0) as env_stream:
    blocked_keys = {
        b"COMMAND_MODE",
        b"OLDPWD",
        b"PWD",
        b"SHLVL",
        b"XPC_FLAGS",
        b"XPC_SERVICE_NAME",
        b"_",
        b"__CFBundleIdentifier",
    }
    for key, value in os.environb.items():
        if key in blocked_keys or key.startswith(b"CODEX_"):
            continue
        env_stream.write(key + b"=" + value + b"\0")
PY
  then
    launchctl remove "$launchd_label" >/dev/null 2>&1 || true
    rm -f "$env_fifo"
    fail "$label launchctl worker did not accept its environment"
  fi
  rm -f "$env_fifo"
}

spawn_process() {
  local label="$1"
  local cwd="$2"
  local log="$3"
  local pid_file="$4"
  local launchd_label="$5"
  shift 5

  rm -f "$pid_file"
  if [[ "$PROCESS_SUPERVISOR" == "launchctl" ]]; then
    spawn_with_launchctl "$label" "$cwd" "$log" "$pid_file" "$launchd_label" "$@"
  else
    spawn_with_double_fork "$cwd" "$log" "$pid_file" "$@"
  fi

  local pid
  for _ in {1..50}; do
    pid="$(read_pid_file "$pid_file")"
    if pid_alive "$pid"; then
      return 0
    fi
    sleep 0.1
  done
  tail -80 "$log" >&2 2>/dev/null || true
  fail "$label process did not publish a live pid; log=$log"
}

cleanup_on_failure() {
  [[ "$SUCCESS" == "1" ]] && return 0
  stop_started_pid frontend "$FRONTEND_PID_FILE" "$STARTED_FRONTEND" "$FRONTEND_LAUNCHD_LABEL"
  stop_started_pid api "$API_PID_FILE" "$STARTED_API" "$API_LAUNCHD_LABEL"
}

resolve_process_supervisor() {
  case "$PROCESS_SUPERVISOR" in
    auto)
      if [[ "$(uname -s)" == "Darwin" \
        && ("${__CFBundleIdentifier:-}" == "com.openai.codex" || -n "${CODEX_SESSION_ID:-}") ]] \
        && command -v launchctl >/dev/null 2>&1; then
        PROCESS_SUPERVISOR="launchctl"
      else
        PROCESS_SUPERVISOR="detached"
      fi
      ;;
    detached)
      ;;
    launchctl)
      require_cmd launchctl
      ;;
    *)
      fail "unsupported DEV_PROCESS_SUPERVISOR=$PROCESS_SUPERVISOR (expected auto, detached, or launchctl)"
      ;;
  esac
}

wait_http() {
  local label="$1"
  local url="$2"
  local pid_file="$3"
  local body="$4"
  local log="$5"
  local needs_html="$6"
  local deadline=$((SECONDS + STARTUP_TIMEOUT_SECS))
  local code pid

  while (( SECONDS < deadline )); do
    pid="$(read_pid_file "$pid_file")"
    if ! pid_alive "$pid"; then
      tail -80 "$log" >&2 2>/dev/null || true
      fail "$label process exited early; log=$log"
    fi
    if ! code="$(curl -sS -L -m 3 -o "$body" -w '%{http_code}' "$url" 2>/dev/null)"; then
      code="000"
    fi
    if [[ "$code" == "200" ]]; then
      if [[ "$needs_html" != "1" ]] || grep -Eiq '<!doctype html|<html' "$body"; then
        printf '%s ready url=%s\n' "$label" "$url"
        return 0
      fi
    fi
    sleep 1
  done

  tail -80 "$log" >&2 2>/dev/null || true
  fail "$label did not become ready within ${STARTUP_TIMEOUT_SECS}s; url=$url log=$log"
}

require_cmd cargo
require_cmd trunk
require_cmd curl
require_cmd python3
require_cmd jq
resolve_process_supervisor
case "$FRONTEND_RELEASE" in
  true | false) ;;
  *) fail "unsupported FRONTEND_RELEASE=$FRONTEND_RELEASE (expected true or false)" ;;
esac
ensure_port_free api "$API_PORT"
ensure_port_free frontend "$FRONTEND_PORT"

API_LOG="$LOG_DIR/api.log"
FRONTEND_LOG="$LOG_DIR/frontend.log"
API_PID_FILE="$RUNTIME_DIR/api.pid"
FRONTEND_PID_FILE="$RUNTIME_DIR/frontend.pid"
API_BIN="$ROOT/target/debug/crypto-arb-api"
PYTHON3_BIN="$(command -v python3)"
API_LAUNCHD_LABEL="${DEV_API_LAUNCHD_LABEL:-com.crossline.omni.dev.api.$API_PORT}"
FRONTEND_LAUNCHD_LABEL="${DEV_FRONTEND_LAUNCHD_LABEL:-com.crossline.omni.dev.frontend.$FRONTEND_PORT}"
trap cleanup_on_failure EXIT

cargo build --locked -p api

spawn_process api "$ROOT" "$API_LOG" "$API_PID_FILE" "$API_LAUNCHD_LABEL" \
  env APP_HOST="$API_HOST" APP_PORT="$API_PORT" APP_STORAGE__DATA_DIR="$API_DATA_DIR" "$API_BIN"
STARTED_API=1

wait_http api "$API_URL/health/ready" "$API_PID_FILE" "$RUNTIME_DIR/api_ready.body" "$API_LOG" 0

if [[ "$FRONTEND_RELEASE" == "true" ]]; then
  spawn_process frontend "$ROOT/frontend" "$FRONTEND_LOG" "$FRONTEND_PID_FILE" "$FRONTEND_LAUNCHD_LABEL" \
    env -u NO_COLOR trunk serve --release=true --address "$FRONTEND_HOST" --port "$FRONTEND_PORT"
else
  spawn_process frontend "$ROOT/frontend" "$FRONTEND_LOG" "$FRONTEND_PID_FILE" "$FRONTEND_LAUNCHD_LABEL" \
    env -u NO_COLOR trunk serve --release=false --address "$FRONTEND_HOST" --port "$FRONTEND_PORT"
fi
STARTED_FRONTEND=1

wait_http frontend "$FRONTEND_URL/" "$FRONTEND_PID_FILE" "$RUNTIME_DIR/frontend.body" "$FRONTEND_LOG" 1

bash "$ROOT/scripts/verify_runtime.sh"
SUCCESS=1
trap - EXIT
printf 'dev runtime ready api=%s frontend=%s runtime_dir=%s supervisor=%s frontend_release=%s\n' \
  "$API_URL" "$FRONTEND_URL" "$RUNTIME_DIR" "$PROCESS_SUPERVISOR" "$FRONTEND_RELEASE"
