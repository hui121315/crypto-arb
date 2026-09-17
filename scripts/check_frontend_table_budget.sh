#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/crossline-table-budget.XXXXXX")"
trap 'rm -rf "$TMP_DIR"' EXIT

OFFENDERS="$TMP_DIR/offenders"
SCAN_PATHS=("$@")
if [ "${#SCAN_PATHS[@]}" -eq 0 ]; then
  SCAN_PATHS=("$ROOT/frontend/src/panels")
fi

TABLE_FILES=()
while IFS= read -r file; do
  TABLE_FILES+=("$file")
done < <(rg -l '<table' "${SCAN_PATHS[@]}" --glob '*.rs' | sort || true)
if [ "${#TABLE_FILES[@]}" -eq 0 ]; then
  printf 'OK frontend table budget gate\n'
  exit 0
fi

awk '
  function reset() {
    in_table = 0
    start_line = 0
    table_file = ""
    block = ""
    after = 0
    bounded = 0
  }
  function has_rows(value) {
    return value ~ /(collect_view\(|for_each|<For|For<)/
  }
  function has_budget(value) {
    return value ~ /(PageControls|ServerPageControls|ListPageControls|page_slice|table_runtime|use_table_runtime|virtual|\.take\(|table-budget:bounded-small|table-budget:server-page|table-budget:row-cap|table-budget:table-runtime|data-table-budget=.*bounded-small|data-table-budget=.*server-page|data-table-budget=.*row-cap|data-table-budget=.*table-runtime)/
  }
  function record_if_needed() {
    if (start_line == 0) {
      return
    }
    if (has_rows(block) && !bounded) {
      print table_file ":" start_line
    }
  }
  FNR == 1 {
    record_if_needed()
    reset()
  }
  {
    if ($0 ~ /<table/) {
      record_if_needed()
      reset()
      in_table = 1
      start_line = FNR
      table_file = FILENAME
      block = $0
      bounded = has_budget(block)
      after = 0
      next
    }
    if (in_table) {
      block = block "\n" $0
      if (has_budget($0)) {
        bounded = 1
      }
      if ($0 ~ /<\/table>/) {
        after = 8
      } else if (after > 0) {
        after--
        if (has_budget($0)) {
          bounded = 1
        }
        if (after == 0) {
          record_if_needed()
          reset()
        }
      }
    }
  }
  END {
    record_if_needed()
  }
' "${TABLE_FILES[@]}" >"$OFFENDERS"

if [ -s "$OFFENDERS" ]; then
  printf 'frontend table budget gate failed: each table renderer must declare pagination, truncation, virtualization, or bounded-small evidence\n' >&2
  cat "$OFFENDERS" >&2
  exit 1
fi

printf 'OK frontend table budget gate\n'
