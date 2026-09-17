#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUDGET="$ROOT/scripts/api_binary_budget.tsv"
TODAY="${API_BINARY_BUDGET_TODAY:-$(date +%F)}"

size_bytes() {
  wc -c <"$1" | tr -dc '0-9'
}

validate_budget_file() {
  awk -F '\t' -v today="$TODAY" '
    BEGIN { ok = 1 }
    /^#/ || NF == 0 { next }
    NF < 6 {
      printf "FAIL malformed api binary budget line %d: expected 6 tab-separated fields\n", NR
      ok = 0
      next
    }
    $1 ~ /^\// || $1 ~ /^\.\// || $1 ~ /(^|\/)\.\.($|\/)/ || $1 !~ /^target\/release\/[A-Za-z0-9_-]+$/ {
      printf "FAIL invalid api binary budget path line %d: %s\n", NR, $1
      ok = 0
    }
    $2 !~ /^[0-9]+$/ || $2 <= 0 {
      printf "FAIL invalid api binary budget max_bytes line %d: %s\n", NR, $2
      ok = 0
    }
    $3 == "" || $4 == "" || $5 == "" || $6 == "" {
      printf "FAIL api binary budget line %d must include owner, ticket, expires and reason\n", NR
      ok = 0
    }
    $5 < today {
      printf "FAIL expired api binary budget line %d: %s expired %s\n", NR, $1, $5
      ok = 0
    }
    seen[$1]++ {
      printf "FAIL duplicate api binary budget path line %d: %s\n", NR, $1
      ok = 0
    }
    END { exit ok ? 0 : 1 }
  ' "$BUDGET"
}

check_binary_budget() {
  local fail=0
  while IFS=$'\t' read -r rel max_bytes _owner _ticket _expires _reason; do
    case "$rel" in
      ''|\#*) continue ;;
    esac
    local file="$ROOT/$rel"
    if [ ! -f "$file" ]; then
      printf 'FAIL api binary budget: missing %s; run cargo build --release -p api first\n' "$rel" >&2
      fail=1
      continue
    fi
    local actual
    actual="$(size_bytes "$file")"
    if [ "$actual" -gt "$max_bytes" ]; then
      printf 'FAIL api binary budget: %s is %s bytes > %s bytes\n' "$rel" "$actual" "$max_bytes" >&2
      fail=1
    else
      printf 'OK api binary budget: %s is %s bytes <= %s bytes\n' "$rel" "$actual" "$max_bytes"
    fi
  done <"$BUDGET"
  return "$fail"
}

validate_budget_file
check_binary_budget
