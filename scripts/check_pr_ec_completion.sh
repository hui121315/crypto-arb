#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
COVERAGE_CONTRACT="$ROOT/shared-types/src/instrument_coverage.rs"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

if [[ "$MODE" == "--self-test" ]]; then
  PR_EC_SKIP_TESTS=1 bash "$0" >/dev/null
  backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-ec.XXXXXX")"
  cp "$COVERAGE_CONTRACT" "$backup"
  restore() {
    cp "$backup" "$COVERAGE_CONTRACT"
    rm -f "$backup"
  }
  trap restore EXIT
  python3 - "$COVERAGE_CONTRACT" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
anchor = "            && self.execution_ready\n"
if anchor not in source:
    raise SystemExit("self-test mutation anchor missing")
path.write_text(source.replace(anchor, "            && true\n", 1), encoding="utf-8")
PY
  if PR_EC_SKIP_TESTS=1 bash "$0" >/dev/null 2>&1; then
    printf 'PR-EC self-test failed: destructive mutation was not detected\n' >&2
    exit 1
  fi
  printf 'OK PR-EC destructive completion gate self-test\n'
  exit 0
fi

cd "$ROOT"

python3 - "$ROOT" <<'PY'
import csv
import json
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])

def fail(message: str) -> None:
    raise SystemExit(f"PR-EC completion gate failed: {message}")

audit = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
row = next((line for line in audit.splitlines() if line.startswith("| `PR-EC ")), "")
if "| ✅ 完成 |" not in row:
    fail("audit row is not complete")
if "`bash scripts/check_pr_ec_completion.sh --self-test`" not in row:
    fail("audit row lacks destructive verification anchor")
queue_match = re.search(
    r"(?ms)^###?\s+(?:[✅🟡⏳➖❌]\s+)?6\.5\b.*?"
    r"(?=^###?\s+(?:[✅🟡⏳➖❌]\s+)?6\.6\b|\Z)",
    audit,
)
if queue_match is None:
    fail("section 6.5 queue is missing")
if re.search(r"(?m)^\d+\. \*\*PR-EC\b", queue_match.group(0)):
    fail("completed PR-EC remains in the executable queue")

required_evidence = {
    "shared-contract": "shared-types/src/instrument_registry.rs",
    "venue-schema-matrix": "crates/exchange/src/venue_spec.rs",
    "registry-runtime": "crates/api/src/services/instrument_registry.rs",
    "scanner-sizing-gate": "crates/api/src/services/instrument_registry/coverage.rs",
    "operation-evidence": "crates/api/src/services/venue_operation_health/snapshot/part_18.rs",
    "adapter-fixtures": "crates/exchange/src/adapters/binance_exchange_info_tests.rs",
    "product-browser": "test/e2e/instrument-coverage.spec.ts",
    "completion-governance": "scripts/check_pr_ec_completion.sh",
}
with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    evidence_rows = list(csv.DictReader(handle, delimiter="\t"))
actual_evidence = {
    row["evidence_type"]: row["artifact"]
    for row in evidence_rows
    if row.get("pr_id") == "PR-EC"
}
for evidence_type, artifact in required_evidence.items():
    if actual_evidence.get(evidence_type) != artifact:
        fail(f"missing evidence {evidence_type} -> {artifact}")

source_anchors = (
    ("shared-types/src/instruments.rs", "pub type InstrumentMetadataEnvelope = crate::instrument_registry::InstrumentSpec;"),
    ("shared-types/src/instrument_registry.rs", "pub fn is_hedge_constructible_at"),
    ("shared-types/src/instrument_coverage.rs", "&& self.execution_ready"),
    ("crates/api/src/services/instrument_registry.rs", "fn metadata_evidence_matches"),
    ("crates/api/src/services/instrument_registry.rs", "fn instrument_metadata_evidence_rows"),
    ("crates/api/src/services/instrument_registry.rs", "probe_allows_execution"),
    ("crates/api/src/services/instrument_registry.rs", "fn source_matches_endpoint"),
    ("crates/api/src/services/instrument_registry/tests.rs", "metadata_registry_accepts_all_venue_family_evidence"),
    ("crates/api/src/services/instrument_registry/coverage.rs", "规格就绪 {executable_count}/{venue_count}"),
    ("crates/api/src/services/instrument_registry/coverage/runtime_health.rs", "execution_ready_rows"),
    ("crates/api/src/services/venue_operation_health/snapshot/part_18.rs", "instrument_metadata_endpoint_missing=true"),
)
for relative, anchor in source_anchors:
    if anchor not in (root / relative).read_text(encoding="utf-8"):
        fail(f"missing source anchor {relative}: {anchor}")

schema_matrix = {
    "crates/exchange/src/adapters/binance_exchange_info.rs": "binance-usdm-futures-exchange-information-usdt-usdc-2026-07-11",
    "crates/exchange/src/adapters/okx_instruments.rs": "okx-v5-public-get-instruments-2026-06-03",
    "crates/exchange/src/adapters/bybit_instruments.rs": "bybit-v5-get-instruments-info-2026-07-13",
    "crates/exchange/src/adapters/bitget_instruments.rs": "bitget-uta-get-instruments-2026-06-03",
    "crates/exchange/src/adapters/gate_contracts.rs": "gate-apiv4-list-futures-contracts-2026-06-03",
    "crates/exchange/src/adapters/htx_instruments.rs": "htx-usdt-swap-query-swap-info-2026-06-03",
    "crates/exchange/src/adapters/kucoin_instruments.rs": "kucoin-futures-native-contract-matrix-2026-07-11",
    "crates/exchange/src/adapters/hyperliquid_instruments.rs": "hyperliquid-perpetuals-meta-and-asset-ctxs-2026-06-03",
}
for relative, schema in schema_matrix.items():
    source = (root / relative).read_text(encoding="utf-8")
    if schema not in source or "schema_version: None" in source:
        fail(f"venue schema is missing or optional in {relative}")

fixture_anchors = {
    "crates/exchange/src/adapters/binance_exchange_info_tests.rs": "registry_projection_uses_compiled_usdt_and_usdc_specs",
    "crates/exchange/src/adapters/okx_instruments_tests.rs": "into_venue_instrument_maps_min_qty_from_min_sz",
    "crates/exchange/src/adapters/bybit_instruments_tests.rs": "bybit_identity_matrix_preserves_usdt_usdc_and_rwa_contracts",
    "crates/exchange/src/adapters/bitget_instruments_tests.rs": "official_identity_matrix_covers_usdt_usdc_coin_and_reality_boundaries",
    "crates/exchange/src/adapters/gate_contracts_tests.rs": "venue_instrument_uses_the_same_verified_native_identity",
    "crates/exchange/src/adapters/htx_instruments_tests.rs": "into_venue_instrument_maps_quanto_and_min_qty",
    "crates/exchange/src/adapters/kucoin_instruments_tests.rs": "official_matrix_maps_usdt_usdc_and_verified_equity_contracts",
    "crates/exchange/src/adapters/hyperliquid_instruments_tests.rs": "into_venue_instrument_derives_ticks_from_sz_decimals",
}
for relative, anchor in fixture_anchors.items():
    source = (root / relative).read_text(encoding="utf-8")
    match = re.search(rf"#\[test\]\s*fn\s+{re.escape(anchor)}\b", source)
    if match is None:
        fail(f"non-skipping fixture anchor missing: {relative}:{anchor}")
    prefix = source[max(0, match.start() - 160):match.start()]
    if "#[ignore" in prefix or "should_panic" in prefix:
        fail(f"fixture anchor is skipped or panic-expected: {relative}:{anchor}")

browser = (root / "test/e2e/instrument-coverage.spec.ts").read_text(encoding="utf-8")
if re.search(r"\btest\.(?:skip|fixme)\s*\(", browser):
    fail("browser fixture must not be skipped")
for marker in (
    "PR-EC keeps listed-but-incomplete specs observed-only across all registry venues",
    "PR-EC retains the build action only for two execution-ready specs",
    "PR-EC Settings exposes exact instrument schema evidence and refresh failure",
    "MU 规格就绪 1/13",
    "execution_ready_rows=0",
    "INSTRUMENT_COVERAGE_REFRESH_FAILED",
):
    if marker not in browser:
        fail(f"browser fixture missing marker: {marker}")

package = json.loads((root / "package.json").read_text(encoding="utf-8"))
scripts = package.get("scripts", {})
if scripts.get("test:e2e:pr-ec") != "playwright test test/e2e/instrument-coverage.spec.ts":
    fail("dedicated browser script is missing")
if scripts.get("test:e2e:product", "").count("instrument-coverage.spec.ts") != 1:
    fail("product suite wiring is missing or duplicated")

required_coverage = set(required_evidence.values()) | set(schema_matrix) | set(fixture_anchors) | {
    "shared-types/src/instruments.rs",
    "shared-types/src/instrument_coverage.rs",
    "crates/api/src/services/instrument_registry/coverage/runtime_health.rs",
    "crates/api/src/services/instrument_registry/tests.rs",
}
with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
for relative in sorted(required_coverage):
    if coverage.get(relative, {}).get("coverage_status") != "exact":
        fail(f"exact coverage missing for {relative}")

print(
    "OK PR-EC contract "
    f"({len(schema_matrix)} venue schemas; {len(fixture_anchors)} non-skipping fixtures; "
    "scanner+sizing+Settings closure)"
)
PY

bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"

if [[ "${PR_EC_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p shared-types instrument --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p exchange --lib instrument --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p exchange --lib registry_projection_uses_compiled_usdt_and_usdc_specs --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p exchange --lib rest_registry::tests --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api instrument_registry --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api snapshot_exposes_instrument_and_opportunity_generation_evidence --no-fail-fast
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-ec -- --workers=1
fi

printf 'OK PR-EC instrument metadata refresh and listing coverage contract\n'
