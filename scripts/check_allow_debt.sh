#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ALLOWLIST="$ROOT/scripts/allow_debt_allowlist.tsv"
SEEN="$(mktemp "${TMPDIR:-/tmp}/crossline-allow-debt-seen.XXXXXX")"
TMP="$(mktemp "${TMPDIR:-/tmp}/crossline-allow-debt.XXXXXX")"
FAIL=0
TODAY="${CROSSLINE_TODAY:-$(date +%F)}"
BASELINE_COMMIT=""

cleanup() {
  rm -f "$SEEN" "$TMP"
}
trap cleanup EXIT

validate_allowlist() {
  awk -F '\t' -v today="$TODAY" '
    /^[[:space:]]*#/ || /^[[:space:]]*$/ { next }
    NF < 9 || $1 == "" || $2 !~ /^[0-9]+$/ || $3 !~ /^(outer_allow|inner_allow|cfg_attr_allow)$/ || $4 !~ /^(production|test|cfg-test|example)$/ || $5 == "" || $6 == "" || $7 !~ /^PR-[A-Z0-9]+$/ || $8 !~ /^[0-9]{4}-[0-9]{2}-[0-9]{2}$/ {
      printf "FAIL malformed allow-debt allowlist line %d: %s\n", NR, $0
      bad = 1
    }
    $1 ~ /^\// || $1 ~ /^\.\// || $1 ~ /(^|\/)\.\.($|\/)/ || $1 ~ /[*?\[]/ || $1 !~ /\.rs$/ {
      printf "FAIL invalid allow-debt allowlist path line %d: %s\n", NR, $1
      bad = 1
    }
    $4 == "production" && $8 < today {
      printf "FAIL expired allow-debt allowlist line %d: %s expired %s\n", NR, $1, $8
      bad = 1
    }
    seen[$1 "\t" $2 "\t" $3 "\t" $4 "\t" $5]++ {
      printf "FAIL duplicate allow-debt allowlist entry line %d: %s:%s:%s:%s:%s\n", NR, $1, $2, $3, $4, $5
      bad = 1
    }
    END { exit bad ? 1 : 0 }
  ' "$ALLOWLIST" || FAIL=1
}

load_merge_base() {
  local base_ref
  base_ref="${ALLOW_DEBT_BASE_REF:-origin/HEAD}"

  if ! git -C "$ROOT" rev-parse --verify --quiet "${base_ref}^{commit}" >/dev/null; then
    if [ -n "${ALLOW_DEBT_BASE_REF:-}" ]; then
      echo "FAIL allow-debt baseline ref does not resolve: ${base_ref}"
      return 1
    fi
    base_ref="HEAD"
  fi
  if ! BASELINE_COMMIT="$(git -C "$ROOT" merge-base HEAD "$base_ref")"; then
    echo "FAIL cannot resolve allow-debt merge base for ${base_ref}"
    return 1
  fi
  echo "BASELINE allow debt: ${base_ref} merge-base ${BASELINE_COMMIT}"
}

allowlist_entry() {
  local rel="$1"
  local line_no="$2"
  local kind="$3"
  local scope="$4"
  local lints="$5"
  awk -F '\t' -v rel="$rel" -v line_no="$line_no" -v kind="$kind" -v scope="$scope" -v lints="$lints" '
    /^[[:space:]]*#/ || /^[[:space:]]*$/ { next }
    $1 == rel && $2 == line_no && $3 == kind && $4 == scope && $5 == lints { print; found = 1; exit }
    END { exit found ? 0 : 1 }
  ' "$ALLOWLIST"
}

record_allow() {
  local file="$1"
  local line_no="$2"
  local kind="$3"
  local scope="$4"
  local lints="$5"
  local rel entry owner ticket expires key

  rel="${file#$ROOT/}"
  key="${rel}"$'\t'"${line_no}"$'\t'"${kind}"$'\t'"${scope}"$'\t'"${lints}"

  if [ "$scope" != "production" ]; then
    echo "INFO allow debt (${scope}): ${rel}:${line_no} ${kind} ${lints}"
    return
  fi

  if entry="$(allowlist_entry "$rel" "$line_no" "$kind" "$scope" "$lints")"; then
    owner="$(printf '%s\n' "$entry" | cut -f6)"
    ticket="$(printf '%s\n' "$entry" | cut -f7)"
    expires="$(printf '%s\n' "$entry" | cut -f8)"
    printf '%s\n' "$key" >>"$SEEN"
    echo "DEBT allow: ${rel}:${line_no} ${kind} ${lints}; allowlist ${ticket}/${owner} expires ${expires}"
    return
  fi

  if baseline_allow_matches "$rel" "$line_no" "$kind" "$scope" "$lints"; then
    echo "INHERITED allow: ${rel}:${line_no} ${kind} ${lints}; exact merge-base match"
    return
  fi

  echo "FAIL production allow without owner/expiry: ${rel}:${line_no} ${kind} ${lints}"
  FAIL=1
}

normalize_lints() {
  local block="$1"
  printf '%s' "$block" \
    | tr -d '[:space:]' \
    | sed -E 's/.*allow\(([^)]*)\).*/\1/'
}

allow_kind() {
  local block="$1"
  if [[ "$block" =~ ^[[:space:]]*#!\[cfg_attr ]]; then
    echo "cfg_attr_allow"
  elif [[ "$block" =~ ^[[:space:]]*#\[cfg_attr ]]; then
    echo "cfg_attr_allow"
  elif [[ "$block" =~ ^[[:space:]]*#!\[allow ]]; then
    echo "inner_allow"
  else
    echo "outer_allow"
  fi
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

is_cfg_test_allow() {
  local block="$1"
  local compact
  compact="$(printf '%s' "$block" | tr -d '[:space:]')"
  [[ "$compact" =~ ^#\!?\[cfg_attr\(test,allow\(.*\)\)\]$ ]]
}

attr_block_at() {
  local file="$1"
  local line_no="$2"
  local end
  end=$((line_no + 12))
  sed -n "${line_no},${end}p" "$file" | awk '{ print } /\]/ { exit }'
}

attr_block_at_ref() {
  local object="$1"
  local line_no="$2"
  local end
  end=$((line_no + 12))
  git -C "$ROOT" show "$object" | sed -n "${line_no},${end}p" | awk '{ print } /\]/ { exit }'
}

# Bare legacy allows are accepted only when their exact source position and attribute
# still match the merge base. Moving or changing one requires explicit ownership.
baseline_allow_matches() {
  local rel="$1"
  local line_no="$2"
  local kind="$3"
  local scope="$4"
  local lints="$5"
  local object block baseline_kind baseline_lints

  [ -n "$BASELINE_COMMIT" ] || return 1
  object="${BASELINE_COMMIT}:${rel}"
  git -C "$ROOT" cat-file -e "$object" 2>/dev/null || return 1
  block="$(attr_block_at_ref "$object" "$line_no")" || return 1
  [[ "$(printf '%s' "$block" | tr -d '[:space:]')" == *"allow("* ]] || return 1
  is_cfg_test_allow "$block" && return 1
  [ "$(path_scope "$rel")" = "$scope" ] || return 1

  baseline_kind="$(allow_kind "$block")"
  baseline_lints="$(normalize_lints "$block")"
  [ "$baseline_kind" = "$kind" ] && [ "$baseline_lints" = "$lints" ]
}

near_test_context() {
  local file="$1"
  local line_no="$2"
  local prev_line short_start mod_start
  prev_line=$((line_no > 1 ? line_no - 1 : 1))
  short_start=$((line_no > 6 ? line_no - 6 : 1))
  mod_start=$((line_no > 220 ? line_no - 220 : 1))

  if sed -n "${prev_line},${prev_line}p" "$file" | rg -q '#\[cfg\(test\)\]'; then
    return 0
  fi

  if sed -n "${short_start},$((line_no - 1))p" "$file" | rg -q '#\[test\]|#\[tokio::test\]'; then
    return 0
  fi

  sed -n "${mod_start},${line_no}p" "$file" \
    | rg -q '^[[:space:]]*mod tests[[:space:]]*\{'
}

scan_attr_match() {
  local file="$1"
  local line_no="$2"
  local rel scope block compact kind lints

  rel="${file#$ROOT/}"
  scope="$(path_scope "$rel")"
  block="$(attr_block_at "$file" "$line_no")"
  compact="$(printf '%s' "$block" | tr -d '[:space:]')"
  [[ "$compact" == *"allow("* ]] || return 0

  kind="$(allow_kind "$block")"
  lints="$(normalize_lints "$block")"

  if is_cfg_test_allow "$block"; then
    record_allow "$file" "$line_no" "$kind" "cfg-test" "$lints"
  elif [ "$scope" != "production" ]; then
    record_allow "$file" "$line_no" "$kind" "$scope" "$lints"
  elif near_test_context "$file" "$line_no"; then
    record_allow "$file" "$line_no" "$kind" "test" "$lints"
  else
    record_allow "$file" "$line_no" "$kind" "production" "$lints"
  fi
}

scan_allow_attrs() {
  local match file line_no _text

  rg -n --type rust '^\s*#(!)?\[(allow|cfg_attr)\b' \
    "$ROOT/crates" \
    "$ROOT/frontend/src" \
    "$ROOT/shared-types" \
    | sort >"$TMP"

  while IFS=: read -r file line_no _text; do
    if [ -n "$file" ] && [ -n "$line_no" ]; then
      scan_attr_match "$file" "$line_no"
    fi
  done < "$TMP"
}

check_allowlist_freshness() {
  local rel line_no kind scope lints _owner ticket _expires _reason key file
  while IFS=$'\t' read -r rel line_no kind scope lints _owner ticket _expires _reason; do
    case "$rel" in
      "" | \#*) continue ;;
    esac
    [ "$scope" = "production" ] || continue
    file="$ROOT/$rel"
    key="${rel}"$'\t'"${line_no}"$'\t'"${kind}"$'\t'"${scope}"$'\t'"${lints}"
    if [ ! -f "$file" ]; then
      echo "FAIL stale allow-debt allowlist: ${rel}:${line_no} ${kind} (${ticket}) file missing"
      FAIL=1
    elif ! grep -Fxq "$key" "$SEEN"; then
      echo "FAIL stale allow-debt allowlist: ${rel}:${line_no} ${kind} ${lints} (${ticket}) no longer matches a current production allow"
      FAIL=1
    fi
  done < "$ALLOWLIST"
}

validate_allowlist
if ! load_merge_base; then
  exit 1
fi
scan_allow_attrs
check_allowlist_freshness

if [ "$FAIL" -ne 0 ]; then
  exit 1
fi

echo "OK production allow debt gate"
