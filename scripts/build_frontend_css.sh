#!/usr/bin/env bash
set -euo pipefail

export PATH="/usr/local/bin:/opt/homebrew/bin:/usr/bin:/bin:${HOME:-}/.nvm/versions/node/v24.14.1/bin:${PATH:-}"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MANIFEST="$ROOT/frontend/styles/src/manifest.txt"
OUT="$ROOT/frontend/styles/.generated/input.css"
TMP="$(mktemp)"
trap 'rm -f "$TMP"' EXIT

mkdir -p "$(dirname "$OUT")"
while IFS= read -r rel || [ -n "$rel" ]; do
  case "$rel" in
    ''|\#*) continue ;;
  esac
  cat "$ROOT/frontend/styles/src/$rel"
done < "$MANIFEST" > "$TMP"

if [ -f "$OUT" ] && cmp -s "$TMP" "$OUT"; then
  echo "$OUT"
  exit 0
fi

mv "$TMP" "$OUT"
trap - EXIT

echo "$OUT"
