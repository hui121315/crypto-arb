#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUDGET="$ROOT/scripts/check_wasm_budget.sh"
CI="$ROOT/.github/workflows/ci.yml"
RELEASE_QA="$ROOT/scripts/check_release_qa_contract.sh"
VERIFY="$ROOT/scripts/verify_implementation.sh"
FINISH="$ROOT/scripts/finish_batch.sh"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/crossline-wasm-contract.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT

fail() {
  printf 'wasm budget contract gate failed: %s\n' "$1" >&2
  exit 1
}

require_fixed() {
  local file="$1"
  local value="$2"
  local label="$3"
  rg -Fq -- "$value" "$file" || fail "$label"
}

expect_failure() {
  local label="$1"
  shift
  if "$@" >/dev/null 2>&1; then
    fail "$label unexpectedly passed"
  fi
}

require_fixed "$BUDGET" 'RAW_BUDGET_BYTES=' 'raw budget is missing'
require_fixed "$BUDGET" 'GZIP_BUDGET_BYTES=' 'gzip budget is missing'
require_fixed "$BUDGET" 'OZ_BUDGET_BYTES=' 'wasm-opt-Oz budget is missing'
require_fixed "$BUDGET" 'DIST_DIR="${WASM_DIST_DIR:-$ROOT/frontend/dist}"' \
  'isolated budget fixture input is missing'
require_fixed "$BUDGET" 'check_budget raw' 'raw stage is not checked'
require_fixed "$BUDGET" 'check_budget gzip' 'gzip stage is not checked'
require_fixed "$BUDGET" 'check_budget wasm-opt-Oz' 'wasm-opt-Oz stage is not checked'
require_fixed "$CI" 'WASM_REQUIRE_OZ=1 bash scripts/check_wasm_budget.sh' \
  'CI does not require the shared Oz gate'
require_fixed "$VERIFY" 'exec bash scripts/finish_batch.sh --release' \
  'local implementation verification does not delegate to the release finish'
require_fixed "$FINISH" 'WASM_REQUIRE_OZ=1' \
  'release finish does not require the shared Oz gate'
require_fixed "$FINISH" 'bash scripts/check_wasm_budget.sh' \
  'release finish does not run the shared Wasm budget gate'
require_fixed "$RELEASE_QA" 'WASM_REQUIRE_OZ=1 bash "$ROOT/scripts/check_wasm_budget.sh"' \
  'release QA does not require the shared Oz gate'

mkdir -p "$TMP/dist"
printf '\000asm\001\000\000\000' >"$TMP/dist/contract_bg.wasm"

FAKE_OPT="$TMP/wasm-opt"
cat >"$FAKE_OPT" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
if [[ "${1:-}" == "--help" ]]; then
  printf '%s\n' '--enable-bulk-memory --enable-bulk-memory-opt --enable-nontrapping-float-to-int --enable-sign-ext --converge'
  exit 0
fi
input=""
output=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    -o) output="$2"; shift 2 ;;
    -Oz|--*) shift ;;
    *) input="$1"; shift ;;
  esac
done
cp "$input" "$output"
EOF
chmod +x "$FAKE_OPT"

base_env=(
  WASM_DIST_DIR="$TMP/dist"
  WASM_OPT_BIN="$FAKE_OPT"
  WASM_REQUIRE_OZ=1
  WASM_SIZE_REPORT="$TMP/wasm-size.json"
)
env "${base_env[@]}" \
  WASM_RAW_BUDGET_BYTES=1024 WASM_GZIP_BUDGET_BYTES=1024 WASM_OZ_BUDGET_BYTES=1024 \
  bash "$BUDGET" >/dev/null
python3 - "$TMP/wasm-size.json" <<'PY'
import json
import sys
from pathlib import Path

report = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
assert report["schemaVersion"] == 1
assert report["generatedBy"] == "scripts/check_wasm_budget.sh"
assert report["gatePassed"] is True
assert report["summary"] == {
    "failed": 0,
    "measurements": 3,
    "passed": 3,
    "skipped": 0,
}
assert {row["stage"] for row in report["artifacts"]} == {
    "raw",
    "gzip",
    "wasm-opt-Oz",
}
sizes = {row["stage"]: row["sizeBytes"] for row in report["artifacts"]}
assert all(row["status"] == "ok" for row in report["artifacts"])
assert sizes["raw"] == 8 and sizes["wasm-opt-Oz"] == 8
assert 0 < sizes["gzip"] <= 1024
PY
expect_failure 'raw over-budget fixture' env "${base_env[@]}" \
  WASM_RAW_BUDGET_BYTES=1 WASM_GZIP_BUDGET_BYTES=1024 WASM_OZ_BUDGET_BYTES=1024 \
  bash "$BUDGET"
expect_failure 'gzip over-budget fixture' env "${base_env[@]}" \
  WASM_RAW_BUDGET_BYTES=1024 WASM_GZIP_BUDGET_BYTES=1 WASM_OZ_BUDGET_BYTES=1024 \
  bash "$BUDGET"
expect_failure 'Oz over-budget fixture' env "${base_env[@]}" \
  WASM_RAW_BUDGET_BYTES=1024 WASM_GZIP_BUDGET_BYTES=1024 WASM_OZ_BUDGET_BYTES=1 \
  bash "$BUDGET"
expect_failure 'required wasm-opt fixture' env \
  WASM_DIST_DIR="$TMP/dist" WASM_OPT_BIN="$TMP/missing-wasm-opt" WASM_REQUIRE_OZ=1 \
  WASM_RAW_BUDGET_BYTES=1024 WASM_GZIP_BUDGET_BYTES=1024 WASM_OZ_BUDGET_BYTES=1024 \
  bash "$BUDGET"

printf 'OK unified raw/gzip/wasm-opt-Oz budget contract\n'
