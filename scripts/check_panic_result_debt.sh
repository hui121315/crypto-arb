#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ALLOWLIST="$ROOT/scripts/panic_result_debt_allowlist.tsv"
SEEN="$(mktemp "${TMPDIR:-/tmp}/crossline-panic-result-seen.XXXXXX")"
TMP="$(mktemp "${TMPDIR:-/tmp}/crossline-panic-result.XXXXXX")"
FAIL=0
TODAY="${CROSSLINE_TODAY:-$(date +%F)}"

cleanup() {
  rm -f "$SEEN" "$TMP"
}
trap cleanup EXIT

validate_allowlist() {
  awk -F '\t' -v today="$TODAY" '
    /^[[:space:]]*#/ || /^[[:space:]]*$/ { next }
    NF < 8 || $1 == "" || $2 !~ /^[0-9]+$/ || $3 !~ /^(panic_macro|todo_macro|unimplemented_macro|dbg_macro|unwrap_call|expect_call)$/ || $4 !~ /^(production|test|example)$/ || $5 == "" || $6 !~ /^PR-[A-Z0-9]+$/ || $7 !~ /^[0-9]{4}-[0-9]{2}-[0-9]{2}$/ {
      printf "FAIL malformed panic-result allowlist line %d: %s\n", NR, $0
      bad = 1
    }
    $1 ~ /^\// || $1 ~ /^\.\// || $1 ~ /(^|\/)\.\.($|\/)/ || $1 ~ /[*?\[]/ || $1 !~ /\.rs$/ {
      printf "FAIL invalid panic-result allowlist path line %d: %s\n", NR, $1
      bad = 1
    }
    $4 == "production" && $7 < today {
      printf "FAIL expired panic-result allowlist line %d: %s expired %s\n", NR, $1, $7
      bad = 1
    }
    seen[$1 "\t" $2 "\t" $3 "\t" $4]++ {
      printf "FAIL duplicate panic-result allowlist entry line %d: %s:%s:%s:%s\n", NR, $1, $2, $3, $4
      bad = 1
    }
    END { exit bad ? 1 : 0 }
  ' "$ALLOWLIST" || FAIL=1
}

path_scope() {
  local rel="$1"
  local base="${rel##*/}"
  case "$rel" in
    */examples/*) echo "example"; return ;;
    */tests/*) echo "test"; return ;;
  esac
  case "$base" in
    tests.rs | *_test.rs | *_tests.rs) echo "test"; return ;;
  esac
  echo "production"
}

inside_cfg_test_module() {
  local file="$1"
  local line_no="$2"
  awk -v target="$line_no" '
    function brace_delta(text, opens, closes) {
      opens = gsub(/\{/, "{", text)
      closes = gsub(/\}/, "}", text)
      return opens - closes
    }
    NR >= target { exit }
    {
      if ($0 ~ /^[[:space:]]*#\[cfg\(test\)\]/) {
        pending_cfg_test = 1
      }
      next_depth = depth + brace_delta($0)
      if (pending_cfg_test && $0 ~ /^[[:space:]]*mod[[:space:]]+[A-Za-z_][A-Za-z0-9_]*[[:space:]]*\{/) {
        test_depth = next_depth
        pending_cfg_test = 0
      } else if ($0 !~ /^[[:space:]]*#/) {
        pending_cfg_test = 0
      }
      depth = next_depth
      if (test_depth > 0 && depth < test_depth) {
        test_depth = 0
      }
    }
    END { exit test_depth > 0 ? 0 : 1 }
  ' "$file"
}

near_test_function() {
  local file="$1"
  local line_no="$2"
  local start
  start=$((line_no > 8 ? line_no - 8 : 1))
  sed -n "${start},$((line_no - 1))p" "$file" \
    | rg -q '#\[test\]|#\[tokio::test\]|#\[cfg\(test\)\]'
}

line_scope() {
  local file="$1"
  local line_no="$2"
  local rel scope

  rel="${file#$ROOT/}"
  scope="$(path_scope "$rel")"
  if [ "$scope" != "production" ]; then
    echo "$scope"
  elif inside_cfg_test_module "$file" "$line_no" || near_test_function "$file" "$line_no"; then
    echo "test"
  else
    echo "production"
  fi
}

line_kind() {
  local text="$1"
  if [[ "$text" == *"panic!"* ]]; then
    echo "panic_macro"
  elif [[ "$text" == *"todo!"* ]]; then
    echo "todo_macro"
  elif [[ "$text" == *"unimplemented!"* ]]; then
    echo "unimplemented_macro"
  elif [[ "$text" == *"dbg!"* ]]; then
    echo "dbg_macro"
  elif [[ "$text" == *".unwrap()"* ]]; then
    echo "unwrap_call"
  else
    echo "expect_call"
  fi
}

allowlist_entry() {
  local rel="$1"
  local line_no="$2"
  local kind="$3"
  local scope="$4"
  awk -F '\t' -v rel="$rel" -v line_no="$line_no" -v kind="$kind" -v scope="$scope" '
    /^[[:space:]]*#/ || /^[[:space:]]*$/ { next }
    $1 == rel && $2 == line_no && $3 == kind && $4 == scope { print; found = 1; exit }
    END { exit found ? 0 : 1 }
  ' "$ALLOWLIST"
}

record_hit() {
  local file="$1"
  local line_no="$2"
  local kind="$3"
  local scope="$4"
  local rel entry owner ticket expires key

  rel="${file#$ROOT/}"
  key="${rel}"$'\t'"${line_no}"$'\t'"${kind}"$'\t'"${scope}"

  if [ "$scope" != "production" ]; then
    if [ "${CROSSLINE_VERBOSE_DEBT:-0}" = "1" ]; then
      echo "INFO panic-result debt (${scope}): ${rel}:${line_no} ${kind}"
    fi
    return
  fi

  if entry="$(allowlist_entry "$rel" "$line_no" "$kind" "$scope")"; then
    owner="$(printf '%s\n' "$entry" | cut -f5)"
    ticket="$(printf '%s\n' "$entry" | cut -f6)"
    expires="$(printf '%s\n' "$entry" | cut -f7)"
    printf '%s\n' "$key" >>"$SEEN"
    echo "DEBT panic-result: ${rel}:${line_no} ${kind}; allowlist ${ticket}/${owner} expires ${expires}"
    return
  fi

  echo "FAIL production panic/result without owner/expiry: ${rel}:${line_no} ${kind}"
  FAIL=1
}

scan_hits() {
  local file line_no text kind scope

  rg -n --type rust '\b(todo!|unimplemented!|panic!|dbg!)|\.unwrap\(\)|\.expect\(' \
    "$ROOT/crates" \
    "$ROOT/frontend/src" \
    "$ROOT/shared-types" \
    | sort >"$TMP"

  while IFS=: read -r file line_no text; do
    [ -n "$file" ] && [ -n "$line_no" ] || continue
    kind="$(line_kind "$text")"
    scope="$(line_scope "$file" "$line_no")"
    record_hit "$file" "$line_no" "$kind" "$scope"
  done <"$TMP"
}

check_allowlist_freshness() {
  local rel line_no kind scope _owner ticket _expires _reason key file
  while IFS=$'\t' read -r rel line_no kind scope _owner ticket _expires _reason; do
    case "$rel" in
      "" | \#*) continue ;;
    esac
    [ "$scope" = "production" ] || continue
    file="$ROOT/$rel"
    key="${rel}"$'\t'"${line_no}"$'\t'"${kind}"$'\t'"${scope}"
    if [ ! -f "$file" ]; then
      echo "FAIL stale panic-result allowlist: ${rel}:${line_no} ${kind} (${ticket}) file missing"
      FAIL=1
    elif ! grep -Fxq "$key" "$SEEN"; then
      echo "FAIL stale panic-result allowlist: ${rel}:${line_no} ${kind} (${ticket}) no longer matches a current production hit"
      FAIL=1
    fi
  done <"$ALLOWLIST"
}

validate_allowlist
scan_hits
check_allowlist_freshness

if [ "$FAIL" -ne 0 ]; then
  exit 1
fi

echo "OK production panic/result debt gate"
