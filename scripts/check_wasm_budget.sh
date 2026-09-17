#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# The gzip budget is a reviewed regression ceiling; the <= 1.2 MB product
# target remains an optimization target. Raw/Oz budgets also catch optimizer
# drift. A same-toolchain rebuild of origin/codex/product-plan-execution on Rust
# 1.95, Trunk 0.21.14, and wasm-opt 130 measured 5,463,330 raw, 2,005,826 gzip,
# and 5,463,251 explicit Oz. The deterministic execution-artifact and confirmed
# Webhook workflow measured 5,563,262 raw, 2,047,205 gzip, and 5,563,183 Oz: a
# reviewed 1.83% raw / 2.06% gzip increase without a new dependency. These
# ceilings retain about 2% headroom for the next measured review.
RAW_BUDGET_BYTES="${WASM_RAW_BUDGET_BYTES:-5680000}"
GZIP_BUDGET_BYTES="${WASM_GZIP_BUDGET_BYTES:-${WASM_BUDGET_BYTES:-2090000}}"
OZ_BUDGET_BYTES="${WASM_OZ_BUDGET_BYTES:-5680000}"
REQUIRE_OZ="${WASM_REQUIRE_OZ:-0}"
WASM_OPT_BIN="${WASM_OPT_BIN:-wasm-opt}"
DIST_DIR="${WASM_DIST_DIR:-$ROOT/frontend/dist}"
TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/crossline-wasm.XXXXXX")"
REPORT_ROWS="$TMP_DIR/wasm-size-report.tsv"
REPORT="${WASM_SIZE_REPORT:-$ROOT/target/reports/wasm-size.json}"
trap 'rm -rf "$TMP_DIR"' EXIT

size_bytes() {
  wc -c < "$1" | tr -d ' '
}

gzip_size_bytes() {
  gzip -9 -c "$1" | wc -c | tr -d ' '
}

check_budget() {
  local label="$1"
  local size="$2"
  local budget="$3"
  local file="$4"
  if [ "$size" -gt "$budget" ]; then
    echo "FAIL ${label} ${file}: ${size} bytes > budget ${budget} bytes"
    return 1
  fi
  echo "OK ${label} ${file}: ${size} bytes <= ${budget} bytes"
}

append_summary_row() {
  local file="$1"
  local stage="$2"
  local size="$3"
  local budget="$4"
  local status="$5"
  [ -z "${GITHUB_STEP_SUMMARY:-}" ] && return 0
  printf '| `%s` | %s | %s | %s | %s |\n' "$file" "$stage" "$size" "$budget" "$status" >> "$GITHUB_STEP_SUMMARY"
}

append_report_row() {
  local file="$1"
  local stage="$2"
  local size="$3"
  local budget="$4"
  local status="$5"
  printf '%s\t%s\t%s\t%s\t%s\n' "$file" "$stage" "$size" "$budget" "$status" >>"$REPORT_ROWS"
}

render_report() {
  local fail="$1"
  mkdir -p "$(dirname "$REPORT")"
  python3 - "$REPORT_ROWS" "$REPORT" "$fail" <<'PY'
import csv
import json
import sys
from collections import Counter
from pathlib import Path

rows_path, report_path, fail = sys.argv[1:]
artifacts = []
with Path(rows_path).open(encoding="utf-8", newline="") as handle:
    for path, stage, size, budget, status in csv.reader(handle, delimiter="\t"):
        artifacts.append(
            {
                "path": path,
                "stage": stage,
                "sizeBytes": int(size) if size else None,
                "budgetBytes": int(budget),
                "status": status,
            }
        )
artifacts.sort(key=lambda row: (row["path"], row["stage"]))
statuses = Counter(row["status"] for row in artifacts)
report = {
    "schemaVersion": 1,
    "generatedBy": "scripts/check_wasm_budget.sh",
    "gatePassed": fail == "0",
    "summary": {
        "measurements": len(artifacts),
        "passed": statuses["ok"],
        "failed": statuses["fail"],
        "skipped": statuses["skip"],
    },
    "artifacts": artifacts,
}
Path(report_path).write_text(
    json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
    encoding="utf-8",
)
PY
  echo "REPORT wasm size: ${REPORT#$ROOT/}"
}

wasm_opt_supports_flag() {
  local flag="$1"
  case "$WASM_OPT_HELP" in
    *"$flag"*) return 0 ;;
    *) return 1 ;;
  esac
}

shopt -s nullglob
WASMS=("$DIST_DIR"/*_bg.wasm)
if [ "${#WASMS[@]}" -eq 0 ]; then
  echo "FAIL no ${DIST_DIR}/*_bg.wasm found; run trunk build --release=true first"
  exit 1
fi

if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  {
    echo "## Frontend WASM budget"
    echo ""
    echo "| File | Stage | Bytes | Budget | Status |"
    echo "|------|-------|-------|--------|--------|"
  } >> "$GITHUB_STEP_SUMMARY"
fi

FAIL=0
WASM_OPT_HELP=""
if command -v "$WASM_OPT_BIN" >/dev/null; then
  WASM_OPT_HELP="$("$WASM_OPT_BIN" --help 2>/dev/null || true)"
fi
for wasm in "${WASMS[@]}"; do
  rel="${wasm#$ROOT/}"

  raw_size="$(size_bytes "$wasm")"
  if check_budget raw "$raw_size" "$RAW_BUDGET_BYTES" "$rel"; then
    append_summary_row "$rel" raw "$raw_size" "$RAW_BUDGET_BYTES" OK
    append_report_row "$rel" raw "$raw_size" "$RAW_BUDGET_BYTES" ok
  else
    append_summary_row "$rel" raw "$raw_size" "$RAW_BUDGET_BYTES" FAIL
    append_report_row "$rel" raw "$raw_size" "$RAW_BUDGET_BYTES" fail
    FAIL=1
  fi

  gzip_size="$(gzip_size_bytes "$wasm")"
  if check_budget gzip "$gzip_size" "$GZIP_BUDGET_BYTES" "$rel"; then
    append_summary_row "$rel" gzip "$gzip_size" "$GZIP_BUDGET_BYTES" OK
    append_report_row "$rel" gzip "$gzip_size" "$GZIP_BUDGET_BYTES" ok
  else
    append_summary_row "$rel" gzip "$gzip_size" "$GZIP_BUDGET_BYTES" FAIL
    append_report_row "$rel" gzip "$gzip_size" "$GZIP_BUDGET_BYTES" fail
    FAIL=1
  fi

  if command -v "$WASM_OPT_BIN" >/dev/null; then
    oz_wasm="$TMP_DIR/$(basename "$wasm").oz"
    wasm_opt_args=()
    for flag in --enable-bulk-memory --enable-bulk-memory-opt --enable-nontrapping-float-to-int --enable-sign-ext --converge; do
      if wasm_opt_supports_flag "$flag"; then
        wasm_opt_args+=("$flag")
      fi
    done
    "$WASM_OPT_BIN" "${wasm_opt_args[@]}" -Oz "$wasm" -o "$oz_wasm"
    oz_size="$(size_bytes "$oz_wasm")"
    if check_budget wasm-opt-Oz "$oz_size" "$OZ_BUDGET_BYTES" "$rel"; then
      append_summary_row "$rel" wasm-opt-Oz "$oz_size" "$OZ_BUDGET_BYTES" OK
      append_report_row "$rel" wasm-opt-Oz "$oz_size" "$OZ_BUDGET_BYTES" ok
    else
      append_summary_row "$rel" wasm-opt-Oz "$oz_size" "$OZ_BUDGET_BYTES" FAIL
      append_report_row "$rel" wasm-opt-Oz "$oz_size" "$OZ_BUDGET_BYTES" fail
      FAIL=1
    fi
  elif [ "$REQUIRE_OZ" = "1" ]; then
    echo "FAIL wasm-opt is required but not installed"
    append_summary_row "$rel" wasm-opt-Oz missing "$OZ_BUDGET_BYTES" FAIL
    append_report_row "$rel" wasm-opt-Oz "" "$OZ_BUDGET_BYTES" fail
    FAIL=1
  else
    echo "SKIP wasm-opt-Oz ${rel}: wasm-opt not installed"
    append_summary_row "$rel" wasm-opt-Oz skipped "$OZ_BUDGET_BYTES" SKIP
    append_report_row "$rel" wasm-opt-Oz "" "$OZ_BUDGET_BYTES" skip
  fi
done

render_report "$FAIL"
exit "$FAIL"
