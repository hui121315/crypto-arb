#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if [ "${1:-}" = "--root" ]; then
  if [ -z "${2:-}" ] || [ "$#" -ne 2 ]; then
    printf 'usage: %s [--root <repo-root>]\n' "$0" >&2
    exit 2
  fi
  ROOT="$(cd "$2" && pwd)"
elif [ "$#" -ne 0 ]; then
  printf 'usage: %s [--root <repo-root>]\n' "$0" >&2
  exit 2
fi

ALLOWLIST="$ROOT/scripts/crate_root_boundary_allowlist.tsv"
SEEN="$(mktemp "${TMPDIR:-/tmp}/crossline-crate-root-seen.XXXXXX")"
TMP="$(mktemp "${TMPDIR:-/tmp}/crossline-crate-root.XXXXXX")"
FAIL=0
TODAY="${CROSSLINE_TODAY:-$(date +%F)}"

cleanup() {
  rm -f "$SEEN" "$TMP"
}
trap cleanup EXIT

validate_allowlist() {
  awk -F '\t' -v today="$TODAY" '
    /^[[:space:]]*#/ || /^[[:space:]]*$/ { next }
    NF < 7 || $1 == "" || $2 !~ /^[0-9]+$/ || $3 !~ /^(private_mod|crate_attr_allow|crate_attr_unknown|restricted_export|unexpected_root_item)$/ || $4 == "" || $5 !~ /^PR-[A-Z0-9]+$/ || $6 !~ /^[0-9]{4}-[0-9]{2}-[0-9]{2}$/ {
      printf "FAIL malformed crate-root allowlist line %d: %s\n", NR, $0
      bad = 1
    }
    $1 ~ /^\// || $1 ~ /^\.\// || $1 ~ /(^|\/)\.\.($|\/)/ || $1 ~ /[*?\[]/ || $1 !~ /(^crates\/[^\/]+\/src\/lib\.rs$|^shared-types\/src\/lib\.rs$)/ {
      printf "FAIL invalid crate-root allowlist path line %d: %s\n", NR, $1
      bad = 1
    }
    $6 < today {
      printf "FAIL expired crate-root allowlist line %d: %s expired %s\n", NR, $1, $6
      bad = 1
    }
    seen[$1 "\t" $2 "\t" $3]++ {
      printf "FAIL duplicate crate-root allowlist entry line %d: %s:%s:%s\n", NR, $1, $2, $3
      bad = 1
    }
    END { exit bad ? 1 : 0 }
  ' "$ALLOWLIST" || FAIL=1
}

allowlist_entry() {
  local rel="$1"
  local line_no="$2"
  local kind="$3"
  awk -F '\t' -v rel="$rel" -v line_no="$line_no" -v kind="$kind" '
    /^[[:space:]]*#/ || /^[[:space:]]*$/ { next }
    $1 == rel && $2 == line_no && $3 == kind { print; found = 1; exit }
    END { exit found ? 0 : 1 }
  ' "$ALLOWLIST"
}

record_violation() {
  local file="$1"
  local line_no="$2"
  local kind="$3"
  local detail="$4"
  local rel entry owner ticket expires

  rel="${file#$ROOT/}"
  if entry="$(allowlist_entry "$rel" "$line_no" "$kind")"; then
    owner="$(printf '%s\n' "$entry" | cut -f4)"
    ticket="$(printf '%s\n' "$entry" | cut -f5)"
    expires="$(printf '%s\n' "$entry" | cut -f6)"
    printf '%s\t%s\t%s\n' "$rel" "$line_no" "$kind" >>"$SEEN"
    echo "DEBT crate-root boundary: ${rel}:${line_no} ${kind}; allowlist ${ticket}/${owner} expires ${expires}"
    return
  fi

  echo "FAIL crate-root boundary: ${rel}:${line_no} ${kind}: ${detail}"
  FAIL=1
}

valid_test_cfg_attr() {
  local block="$1"
  local compact
  compact="$(printf '%s' "$block" | tr -d '[:space:]')"
  [[ "$compact" =~ ^#!\[cfg_attr\(test,allow\(.*\)\)\]$ ]]
}

scan_file() {
  local file="$1"
  local line line_no cfg_block cfg_start in_cfg_attr in_pub_use

  line_no=0
  cfg_block=""
  cfg_start=0
  in_cfg_attr=0
  in_pub_use=0

  while IFS= read -r line || [ -n "$line" ]; do
    line_no=$((line_no + 1))

    if [ "$in_cfg_attr" -eq 1 ]; then
      cfg_block="${cfg_block}"$'\n'"${line}"
      if [[ "$line" =~ ^[[:space:]]*\)\][[:space:]]*$ ]]; then
        if ! valid_test_cfg_attr "$cfg_block"; then
          record_violation "$file" "$cfg_start" "crate_attr_unknown" "$cfg_block"
        fi
        in_cfg_attr=0
        cfg_block=""
      fi
      continue
    fi

    if [ "$in_pub_use" -eq 1 ]; then
      [[ "$line" == *";"* ]] && in_pub_use=0
      continue
    fi

    case "$line" in
      "" | "//! "* | "//!") continue ;;
    esac
    [[ "$line" =~ ^[[:space:]]*$ ]] && continue
    [[ "$line" =~ ^[[:space:]]*// ]] && continue

    if [[ "$line" =~ ^[[:space:]]*#!\[cfg_attr\( ]]; then
      cfg_block="$line"
      cfg_start="$line_no"
      if [[ "$line" =~ \][[:space:]]*$ ]]; then
        if ! valid_test_cfg_attr "$cfg_block"; then
          record_violation "$file" "$line_no" "crate_attr_unknown" "$line"
        fi
      else
        in_cfg_attr=1
      fi
      continue
    fi

    if [[ "$line" =~ ^[[:space:]]*#!\[allow\( ]]; then
      record_violation "$file" "$line_no" "crate_attr_allow" "$line"
      continue
    fi

    if [[ "$line" =~ ^[[:space:]]*#!\[ ]]; then
      record_violation "$file" "$line_no" "crate_attr_unknown" "$line"
      continue
    fi

    if [[ "$line" =~ ^[[:space:]]*pub[[:space:]]+mod[[:space:]]+[A-Za-z_][A-Za-z0-9_]*[[:space:]]*\;[[:space:]]*$ ]]; then
      continue
    fi

    if [[ "$line" =~ ^[[:space:]]*pub[[:space:]]+use[[:space:]]+ ]]; then
      [[ "$line" != *";"* ]] && in_pub_use=1
      continue
    fi

    if [[ "$line" =~ ^[[:space:]]*mod[[:space:]]+ ]]; then
      record_violation "$file" "$line_no" "private_mod" "$line"
    elif [[ "$line" =~ ^[[:space:]]*pub\((crate|super)\)[[:space:]]+(mod|use)[[:space:]]+ ]]; then
      record_violation "$file" "$line_no" "restricted_export" "$line"
    else
      record_violation "$file" "$line_no" "unexpected_root_item" "$line"
    fi
  done < "$file"

  if [ "$in_cfg_attr" -eq 1 ]; then
    record_violation "$file" "$cfg_start" "crate_attr_unknown" "unterminated cfg_attr block"
  fi
  if [ "$in_pub_use" -eq 1 ]; then
    record_violation "$file" "$line_no" "unexpected_root_item" "unterminated pub use block"
  fi
}

check_allowlist_freshness() {
  local rel line_no kind _owner ticket _expires _reason key file
  while IFS=$'\t' read -r rel line_no kind _owner ticket _expires _reason; do
    case "$rel" in
      "" | \#*) continue ;;
    esac
    file="$ROOT/$rel"
    key="${rel}"$'\t'"${line_no}"$'\t'"${kind}"
    if [ ! -f "$file" ]; then
      echo "FAIL stale crate-root allowlist: ${rel}:${line_no} ${kind} (${ticket}) file missing"
      FAIL=1
    elif ! grep -Fxq "$key" "$SEEN"; then
      echo "FAIL stale crate-root allowlist: ${rel}:${line_no} ${kind} (${ticket}) no longer matches a current violation"
      FAIL=1
    fi
  done < "$ALLOWLIST"
}

validate_allowlist

{
  find "$ROOT/crates" -mindepth 3 -maxdepth 3 -type f -path '*/src/lib.rs' -print
  printf '%s\n' "$ROOT/shared-types/src/lib.rs"
} | sort >"$TMP"

while IFS= read -r file; do
  scan_file "$file"
done < "$TMP"

check_allowlist_freshness

if [ "$FAIL" -ne 0 ]; then
  exit 1
fi

echo "OK crate root boundary gate"
