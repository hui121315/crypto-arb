#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
POSTGRES_ROOT="$(mktemp -d /tmp/crossline-pr-dm-postgres.XXXXXX)"
DATA_DIR="$POSTGRES_ROOT/data"
SOCKET_DIR="$POSTGRES_ROOT/socket"
LOG_FILE="$POSTGRES_ROOT/postgres.log"
DB_USER="$(id -un)"
DB_NAME="crossline_pr_dm"
PORT="$((55432 + ($$ % 1000)))"

for command in initdb pg_ctl createdb; do
  if ! command -v "$command" >/dev/null 2>&1; then
    printf 'PR-DM PostgreSQL gate requires %s on PATH\n' "$command" >&2
    exit 1
  fi
done

while lsof -nP -iTCP:"$PORT" -sTCP:LISTEN >/dev/null 2>&1; do
  PORT=$((PORT + 1))
done

cleanup() {
  status=$?
  if [[ -f "$DATA_DIR/postmaster.pid" ]]; then
    pg_ctl -D "$DATA_DIR" -m fast -w stop >/dev/null 2>&1 || true
  fi
  if [[ $status -ne 0 && -f "$LOG_FILE" ]]; then
    printf '%s\n' '--- PR-DM PostgreSQL log ---' >&2
    tail -n 120 "$LOG_FILE" >&2 || true
  fi
  rm -rf "$POSTGRES_ROOT"
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT TERM

mkdir -p "$SOCKET_DIR"
initdb \
  -D "$DATA_DIR" \
  -U "$DB_USER" \
  --no-locale \
  --encoding=UTF8 \
  --auth-local=trust \
  --auth-host=trust \
  >/dev/null

pg_ctl \
  -D "$DATA_DIR" \
  -l "$LOG_FILE" \
  -o "-F -h 127.0.0.1 -p $PORT -k $SOCKET_DIR" \
  -w start \
  >/dev/null

createdb -h 127.0.0.1 -p "$PORT" -U "$DB_USER" "$DB_NAME"

export CROSSLINE_TEST_POSTGRES_URL="postgresql://$DB_USER@127.0.0.1:$PORT/$DB_NAME"
CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test \
  --manifest-path "$ROOT/Cargo.toml" \
  -p trading \
  --test sql_ledger_postgres \
  -- \
  --ignored \
  --test-threads=1

CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test \
  --manifest-path "$ROOT/Cargo.toml" \
  -p api \
  --bin crypto-arb-api \
  run_cost_facts_rebuild_execution_and_close_reconciliation \
  -- \
  --ignored \
  --test-threads=1

CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test \
  --manifest-path "$ROOT/Cargo.toml" \
  -p api \
  --bin crypto-arb-api \
  startup_projection_worker_catches_pending_fill_once \
  -- \
  --ignored \
  --test-threads=1

printf 'OK PR-DM disposable PostgreSQL ledger contract\n'
