#!/usr/bin/env bash
set -euo pipefail

OUT_DIR="${OUT_DIR:-backups/postgres}"
POSTGRES_URL="${APP_STORAGE__POSTGRES_URL:-${DATABASE_URL:-postgres://app:app@127.0.0.1:5432/cryptoarb}}"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"

mkdir -p "$OUT_DIR"
pg_dump "$POSTGRES_URL" | gzip > "$OUT_DIR/cryptoarb-$STAMP.sql.gz"
echo "$OUT_DIR/cryptoarb-$STAMP.sql.gz"
