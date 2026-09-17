#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="$ROOT/frontend/styles/src"
MANIFEST="$SRC/manifest.txt"
OUT="$ROOT/frontend/styles/.generated/input.css"
TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/crossline_css_gate.XXXXXX")"
trap 'rm -rf "$TMP_DIR"' EXIT

MANIFEST_ENTRIES="$TMP_DIR/manifest_entries.txt"
MANIFEST_SORTED="$TMP_DIR/manifest_sorted.txt"
FOUND_SORTED="$TMP_DIR/found_sorted.txt"
EXPECTED="$TMP_DIR/expected.css"

check_skin_css() {
  local path="$1"
  local rel="$2"

  awk -v rel="$rel" '
    function count_char(s, ch, n, i) {
      n = 0
      for (i = 1; i <= length(s); i++) {
        if (substr(s, i, 1) == ch) {
          n++
        }
      }
      return n
    }
    /^[[:space:]]*$/ && first == "" { next }
    first == "" {
      first = $0
      if ($0 !~ /^[[:space:]]*@layer[[:space:]]+components[[:space:]]*\{[[:space:]]*$/) {
        printf "FAIL skin CSS must start with @layer components {: %s\n", rel
        bad = 1
      }
    }
    {
      if ($0 ~ /^[[:space:]]*@layer[[:space:]]+components[[:space:]]*\{[[:space:]]*$/) {
        layers++
      }
      level += count_char($0, "{") - count_char($0, "}")
      if (level < 0) {
        printf "FAIL skin CSS closes before opening: %s:%d\n", rel, FNR
        bad = 1
      }
      if ($0 !~ /^[[:space:]]*$/) {
        last = $0
      }
    }
    END {
      if (first == "") {
        printf "FAIL skin CSS is empty: %s\n", rel
        bad = 1
      }
      if (layers != 1) {
        printf "FAIL skin CSS must contain exactly one @layer components block: %s layers=%d\n", rel, layers
        bad = 1
      }
      if (level != 0) {
        printf "FAIL skin CSS braces are unbalanced: %s level=%d\n", rel, level
        bad = 1
      }
      if (last !~ /^[[:space:]]*\}[[:space:]]*$/) {
        printf "FAIL skin CSS must end with }: %s\n", rel
        bad = 1
      }
      exit bad ? 1 : 0
    }
  ' "$path"
}

awk '
  /^[[:space:]]*#/ || /^[[:space:]]*$/ { next }
  {
    gsub(/^[[:space:]]+|[[:space:]]+$/, "", $0)
    print
  }
' "$MANIFEST" >"$MANIFEST_ENTRIES"

awk '
  $0 ~ /^\// || $0 ~ /^\.\// || $0 ~ /(^|\/)\.\.($|\/)/ || $0 ~ /[*?\[]/ || $0 !~ /\.css$/ {
    printf "FAIL invalid CSS manifest path: %s\n", $0
    bad = 1
  }
  seen[$0]++ {
    printf "FAIL duplicate CSS manifest path: %s\n", $0
    bad = 1
  }
  END { exit bad ? 1 : 0 }
' "$MANIFEST_ENTRIES"

while IFS= read -r rel || [ -n "$rel" ]; do
  if [ ! -f "$SRC/$rel" ]; then
    echo "FAIL CSS manifest references missing file: $rel" >&2
    exit 1
  fi
done <"$MANIFEST_ENTRIES"

while IFS= read -r rel || [ -n "$rel" ]; do
  case "$rel" in
    skin/*.css) check_skin_css "$SRC/$rel" "$rel" ;;
  esac
done <"$MANIFEST_ENTRIES"

find "$SRC" -type f -name '*.css' -print \
  | sed "s#^$SRC/##" \
  | sort >"$FOUND_SORTED"
sort "$MANIFEST_ENTRIES" >"$MANIFEST_SORTED"

if ! diff -u "$FOUND_SORTED" "$MANIFEST_SORTED"; then
  echo "FAIL CSS manifest must list every source CSS file exactly once" >&2
  exit 1
fi

while IFS= read -r rel || [ -n "$rel" ]; do
  cat "$SRC/$rel"
done <"$MANIFEST_ENTRIES" >"$EXPECTED"

if ! cmp -s "$EXPECTED" "$OUT"; then
  echo "FAIL generated CSS is stale: run scripts/build_frontend_css.sh" >&2
  exit 1
fi

echo "OK frontend CSS manifest and generated input"
