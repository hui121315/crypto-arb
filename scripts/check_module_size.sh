#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ALLOWLIST="$ROOT/scripts/module_size_debt_allowlist.tsv"
SEEN_DEBT_FILE="$(mktemp "${TMPDIR:-/tmp}/crossline_module_debt_seen.XXXXXX")"
MATCH_FILE="$(mktemp "${TMPDIR:-/tmp}/crossline_module_size_match.XXXXXX")"
BASELINE_FILE="$(mktemp "${TMPDIR:-/tmp}/crossline_module_size_baseline.XXXXXX")"
REPORT_ROWS="$(mktemp "${TMPDIR:-/tmp}/crossline_module_size_report.XXXXXX")"
REPORT="${MODULE_SIZE_REPORT:-$ROOT/target/reports/module-size.json}"
TODAY="${MODULE_SIZE_TODAY:-$(date +%F)}"
BASELINE_COMMIT=""
FAIL=0

cleanup() {
  rm -f "$SEEN_DEBT_FILE" "$MATCH_FILE" "$BASELINE_FILE" "$REPORT_ROWS"
}
trap cleanup EXIT

validate_debt_allowlist() {
  awk -F '\t' -v today="$TODAY" '
    /^[[:space:]]*#/ || /^[[:space:]]*$/ { next }
    NF != 7 || $1 !~ /^[0-9]+$/ || $1 <= 300 || $2 == "" || $3 !~ /^PR-[A-Z0-9]+$/ || $3 == "PR-FV" || $4 !~ /^[a-z][a-z0-9-]*$/ || $5 !~ /^[a-z][a-z0-9-]*$/ {
      printf "FAIL malformed module-size debt allowlist line %d: %s\n", NR, $0
      bad = 1
    }
    $2 ~ /^\// || $2 ~ /^\.\// || $2 ~ /(^|\/)\.\.($|\/)/ || $2 ~ /[*?\[]/ || $2 !~ /\.(rs|css)$/ {
      printf "FAIL invalid module-size debt allowlist path line %d: %s\n", NR, $2
      bad = 1
    }
    $6 == $2 || $6 ~ /^\// || $6 ~ /^\.\// || $6 ~ /(^|\/)\.\.($|\/)/ || $6 ~ /[*?\[]/ || $6 !~ /\.(rs|css)$/ {
      printf "FAIL invalid module-size split target line %d: %s\n", NR, $6
      bad = 1
    }
    $7 !~ /^[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]$/ || substr($7, 6, 2) + 0 < 1 || substr($7, 6, 2) + 0 > 12 || substr($7, 9, 2) + 0 < 1 || substr($7, 9, 2) + 0 > 31 || $7 < today {
      printf "FAIL expired or invalid module-size review date line %d: %s (today %s)\n", NR, $7, today
      bad = 1
    }
    seen[$2]++ {
      printf "FAIL duplicate module-size debt allowlist path line %d: %s\n", NR, $2
      bad = 1
    }
    END { exit bad ? 1 : 0 }
  ' "$ALLOWLIST" || FAIL=1
}

check_allowlist_monotonic() {
  local current="$1"
  local baseline="$2"

  awk -F '\t' '
    FNR == NR {
      if ($0 !~ /^[[:space:]]*#/ && $0 !~ /^[[:space:]]*$/) {
        baseline_cap[$2] = $1
      }
      next
    }
    /^[[:space:]]*#/ || /^[[:space:]]*$/ { next }
    !($2 in baseline_cap) {
      printf "FAIL new module-size debt entry is not allowed: %s (%s lines)\n", $2, $1
      bad = 1
      next
    }
    $1 > baseline_cap[$2] {
      printf "FAIL module-size debt cap increase: %s %s -> %s lines\n", $2, baseline_cap[$2], $1
      bad = 1
    }
    END { exit bad ? 1 : 0 }
  ' "$baseline" "$current"
}

is_unchanged_inherited_debt() {
  local current="$1"
  local baseline="$2"
  local max="$3"

  [ "$current" -gt "$max" ] && [ "$baseline" -gt "$max" ] && [ "$current" -le "$baseline" ]
}

load_merge_base_allowlist() {
  local base_ref object
  base_ref="${MODULE_SIZE_BASE_REF:-origin/HEAD}"

  if ! git -C "$ROOT" rev-parse --verify --quiet "$base_ref^{commit}" >/dev/null; then
    if [ -n "${MODULE_SIZE_BASE_REF:-}" ]; then
      echo "FAIL module-size baseline ref does not resolve: ${base_ref}"
      return 1
    fi
    base_ref="HEAD"
  fi
  if ! BASELINE_COMMIT="$(git -C "$ROOT" merge-base HEAD "$base_ref")"; then
    echo "FAIL cannot resolve module-size merge base for ${base_ref}"
    return 1
  fi

  object="${BASELINE_COMMIT}:scripts/module_size_debt_allowlist.tsv"
  if ! git -C "$ROOT" cat-file -e "$object" 2>/dev/null; then
    echo "FAIL module-size allowlist is missing at merge base ${BASELINE_COMMIT} (${base_ref})"
    return 1
  fi
  if ! git -C "$ROOT" show "$object" >"$BASELINE_FILE"; then
    echo "FAIL cannot read module-size allowlist at merge base ${BASELINE_COMMIT} (${base_ref})"
    return 1
  fi
  echo "BASELINE module-size debt: ${base_ref} merge-base ${BASELINE_COMMIT}"
}

baseline_line_count() {
  local rel="$1"
  local object

  [ -n "$BASELINE_COMMIT" ] || return 1
  object="${BASELINE_COMMIT}:${rel}"
  git -C "$ROOT" cat-file -e "$object" 2>/dev/null || return 1
  git -C "$ROOT" show "$object" | wc -l | tr -d ' '
}

if [ "${1:-}" = "--check-monotonic" ]; then
  if [ "$#" -ne 3 ]; then
    echo "usage: $0 --check-monotonic CURRENT_ALLOWLIST BASELINE_ALLOWLIST" >&2
    exit 2
  fi
  check_allowlist_monotonic "$2" "$3"
  exit $?
fi

if [ "${1:-}" = "--check-inherited-debt" ]; then
  if [ "$#" -ne 4 ]; then
    echo "usage: $0 --check-inherited-debt CURRENT_LINES BASELINE_LINES MAX_LINES" >&2
    exit 2
  fi
  for value in "$2" "$3" "$4"; do
    case "$value" in
      ''|*[!0-9]*)
        echo "FAIL inherited module-size debt arguments must be non-negative integers" >&2
        exit 2
        ;;
    esac
  done
  if is_unchanged_inherited_debt "$2" "$3" "$4"; then
    exit 0
  fi
  echo "FAIL inherited module-size debt is not unchanged" >&2
  exit 1
fi

debt_entry() {
  local rel="$1"
  awk -F '\t' -v rel="$rel" '
    /^[[:space:]]*#/ || /^[[:space:]]*$/ { next }
    $2 == rel { print; found = 1; exit }
    END { exit found ? 0 : 1 }
  ' "$ALLOWLIST"
}

record_line_budget() {
  local label="$1"
  local rel="$2"
  local count="$3"
  local max="$4"
  local kind="$5"
  local entry cap ticket owner category split_target review_by baseline

  if entry="$(debt_entry "$rel")"; then
    cap="$(printf '%s\n' "$entry" | cut -f1)"
    ticket="$(printf '%s\n' "$entry" | cut -f3)"
    owner="$(printf '%s\n' "$entry" | cut -f4)"
    category="$(printf '%s\n' "$entry" | cut -f5)"
    split_target="$(printf '%s\n' "$entry" | cut -f6)"
    review_by="$(printf '%s\n' "$entry" | cut -f7)"
    printf '%s\n' "$rel" >>"$SEEN_DEBT_FILE"
    if [ "$count" -eq "$cap" ]; then
      echo "DEBT ${label}: ${rel} = ${count} lines > ${max}; ${ticket}/${owner} ${category} -> ${split_target}, review ${review_by}"
      printf '%s\t%s\t%s\t%s\t%s\t%s\n' \
        "$kind" "$rel" "$count" "$max" debt "$ticket" >>"$REPORT_ROWS"
      return 0
    fi
    echo "FAIL ${label}: ${rel} = ${count} lines > ${max}; allowlist cap is ${cap}, update the ticketed cap when debt changes"
    printf '%s\t%s\t%s\t%s\t%s\t%s\n' \
      "$kind" "$rel" "$count" "$max" invalid_debt "$ticket" >>"$REPORT_ROWS"
    FAIL=1
    return 0
  fi

  if baseline="$(baseline_line_count "$rel")" && \
    is_unchanged_inherited_debt "$count" "$baseline" "$max"; then
    echo "INHERITED DEBT ${label}: ${rel} = ${count} lines > ${max}; baseline ${baseline} lines, no growth"
    printf '%s\t%s\t%s\t%s\t%s\t%s\n' \
      "$kind" "$rel" "$count" "$max" inherited_debt "" >>"$REPORT_ROWS"
    return 0
  fi

  echo "FAIL ${label}: ${rel} = ${count} lines > ${max}"
  printf '%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$kind" "$rel" "$count" "$max" over_budget "" >>"$REPORT_ROWS"
  FAIL=1
}

check_debt_allowlist_freshness() {
  local cap rel ticket owner category split_target review_by count file
  while IFS=$'\t' read -r cap rel ticket owner category split_target review_by; do
    case "$cap" in
      ''|\#*) continue ;;
    esac
    file="$ROOT/$rel"
    if [ ! -f "$file" ]; then
      echo "FAIL stale module-size debt allowlist: ${rel} (${ticket}/${owner}) does not exist"
      FAIL=1
      continue
    fi
    count="$(wc -l < "$file" | tr -d ' ')"
    if [ "$count" -le 300 ]; then
      echo "FAIL stale module-size debt allowlist: ${rel} = ${count} lines <= 300; remove the debt entry"
      FAIL=1
    elif [ "$count" -ne "$cap" ]; then
      echo "FAIL stale module-size debt allowlist: ${rel} = ${count} lines, cap ${cap}; update cap and ticket evidence"
      FAIL=1
    fi
    if ! grep -Fxq "$rel" "$SEEN_DEBT_FILE"; then
      echo "FAIL stale module-size debt allowlist: ${rel} (${ticket}/${owner}) is outside the scanned product roots"
      FAIL=1
    fi
  done < "$ALLOWLIST"
}

check_lines() {
  local max="$1"
  local label="$2"
  shift 2
  while IFS= read -r file; do
    local rel count
    rel="${file#$ROOT/}"
    count="$(wc -l < "$file" | tr -d ' ')"
    if [ "$count" -gt "$max" ]; then
      record_line_budget "$label" "$rel" "$count" "$max" rust
    else
      printf 'rust\t%s\t%s\t%s\tok\t\n' "$rel" "$count" "$max" >>"$REPORT_ROWS"
    fi
  done < <(find "$@" -type f -name '*.rs')
}

check_css_lines() {
  local max="$1"
  local label="$2"
  shift 2
  while IFS= read -r file; do
    local rel count
    rel="${file#$ROOT/}"
    count="$(wc -l < "$file" | tr -d ' ')"
    if [ "$count" -gt "$max" ]; then
      record_line_budget "$label" "$rel" "$count" "$max" css
    else
      printf 'css\t%s\t%s\t%s\tok\t\n' "$rel" "$count" "$max" >>"$REPORT_ROWS"
    fi
  done < <(find "$@" -type f -name '*.css')
}

check_no_match() {
  local pattern="$1"
  local label="$2"
  shift 2
  if rg -n "$pattern" "$@" >"$MATCH_FILE"; then
    echo "FAIL ${label}"
    cat "$MATCH_FILE"
    FAIL=1
  fi
}

check_no_css_hex_outside_tokens() {
  local css_root="$1"
  if rg -n '#[0-9A-Fa-f]{3,6}' "$css_root" --glob '!**/tokens.css' >"$MATCH_FILE"; then
    echo "FAIL frontend colors must come from tokens.css"
    cat "$MATCH_FILE"
    FAIL=1
  fi
}

render_report() {
  mkdir -p "$(dirname "$REPORT")"
  python3 - "$REPORT_ROWS" "$REPORT" "$BASELINE_COMMIT" "$FAIL" <<'PY'
import csv
import json
import sys
from collections import Counter
from pathlib import Path

rows_path, report_path, baseline_commit, fail = sys.argv[1:]
modules = []
with Path(rows_path).open(encoding="utf-8", newline="") as handle:
    for kind, path, lines, maximum, status, ticket in csv.reader(handle, delimiter="\t"):
        modules.append(
            {
                "kind": kind,
                "path": path,
                "lines": int(lines),
                "maxLines": int(maximum),
                "status": status,
                "ticket": ticket or None,
            }
        )
modules.sort(key=lambda row: row["path"])
statuses = Counter(row["status"] for row in modules)
report = {
    "schemaVersion": 1,
    "generatedBy": "scripts/check_module_size.sh",
    "baselineCommit": baseline_commit or None,
    "gatePassed": fail == "0",
    "summary": {
        "scanned": len(modules),
        "withinBudget": statuses["ok"],
        "allowlistedDebt": statuses["debt"],
        "inheritedDebt": statuses["inherited_debt"],
        "failures": statuses["invalid_debt"] + statuses["over_budget"],
    },
    "modules": modules,
}
Path(report_path).write_text(
    json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
    encoding="utf-8",
)
PY
  echo "REPORT module size: ${REPORT#$ROOT/}"
}

PRODUCT_ROOTS=(
  "$ROOT/crates/portfolio"
  "$ROOT/crates/review"
  "$ROOT/crates/api/src/services"
  "$ROOT/crates/api/src/lifecycle.rs"
  "$ROOT/crates/api/src/lifecycle"
  "$ROOT/crates/api/src/trading_service.rs"
  "$ROOT/crates/api/src/trading_service"
  "$ROOT/crates/api/src/routers/arbitrage.rs"
  "$ROOT/crates/api/src/routers/arbitrage"
  "$ROOT/crates/api/src/routers/trading.rs"
  "$ROOT/crates/api/src/routers/trading"
  "$ROOT/crates/options/src/strategy.rs"
  "$ROOT/crates/options/src/strategy"
  "$ROOT/crates/options/src/strategies.rs"
  "$ROOT/crates/options/src/strategies"
  "$ROOT/crates/realtime/src/history.rs"
  "$ROOT/crates/realtime/src/history"
  "$ROOT/crates/simulation/src/portfolio.rs"
  "$ROOT/crates/simulation/src/portfolio"
  "$ROOT/crates/trading/src/journal.rs"
  "$ROOT/crates/trading/src/journal"
  "$ROOT/crates/trading/src/risk.rs"
  "$ROOT/crates/trading/src/risk"
  "$ROOT/frontend/src/api/rest.rs"
  "$ROOT/frontend/src/api/rest"
  "$ROOT/frontend/src/i18n.rs"
  "$ROOT/frontend/src/panels"
  "$ROOT/frontend/src/state/context.rs"
  "$ROOT/shared-types/src/portfolio.rs"
  "$ROOT/shared-types/src/review.rs"
  "$ROOT/shared-types/src/strategy.rs"
  "$ROOT/shared-types/src/system.rs"
  "$ROOT/shared-types/src/venues.rs"
)

validate_debt_allowlist
if load_merge_base_allowlist; then
  check_allowlist_monotonic "$ALLOWLIST" "$BASELINE_FILE" || FAIL=1
else
  FAIL=1
fi
check_lines 300 "product Rust module" "${PRODUCT_ROOTS[@]}"
check_css_lines 300 "frontend CSS module" "$ROOT/frontend/styles/src"
check_debt_allowlist_freshness
check_no_match 'd3|echarts|chart\.js|lightweight-charts|highcharts|lucide|tabler' \
  "frontend must not add heavy chart/icon libraries" "$ROOT/frontend/Cargo.toml"
check_no_match '@import|fonts\.googleapis|fonts\.gstatic|fontawesome' \
  "frontend must not depend on remote font/icon assets" "$ROOT/frontend/styles" "$ROOT/frontend/src"
check_no_css_hex_outside_tokens "$ROOT/frontend/styles/src"
check_no_match 'CEX x DEX|dYdX|rwa_basket|链上信号|链上池子|\b(DEX|dex)\b' \
  "DEX/onchain copy must stay inside canonical onchain surfaces" "$ROOT/frontend/src" \
  --glob '!**/panels/modules/onchain/**' \
  --glob '!**/panels/shared/onchain_provider_credentials/**'

render_report
exit "$FAIL"
