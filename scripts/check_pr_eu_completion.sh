#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
INDEX="$ROOT/crates/api/src/services/opportunity_index.rs"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

if [[ "$MODE" == "--self-test" ]]; then
  PR_EU_SKIP_TESTS=1 bash "$0" >/dev/null
  backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-eu.XXXXXX")"
  cp "$INDEX" "$backup"
  restore() {
    cp "$backup" "$INDEX"
    rm -f "$backup"
  }
  trap restore EXIT
  python3 - "$INDEX" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "current: ArcSwap<OpportunityIndexSnapshot>"
if marker not in source:
    raise SystemExit("PR-EU self-test setup failed: atomic snapshot marker missing")
path.write_text(source.replace(marker, "current: RwLock<OpportunityIndexSnapshot>", 1), encoding="utf-8")
PY
  if PR_EU_SKIP_TESTS=1 bash "$0" >/dev/null 2>&1; then
    printf 'PR-EU completion self-test failed: non-atomic index passed\n' >&2
    exit 1
  fi
  printf 'PR-EU completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from __future__ import annotations

import csv
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])
title = "PR-EU Backend Runtime, MarketDataCache & Operation Health Contract"
verify_anchor = "`bash scripts/check_pr_eu_completion.sh --self-test`"
evidence_contract = {
    "typed-metadata-refresh-retry": "crates/api/src/lifecycle/market_data.rs",
    "public-ws-phase-fallback": "crates/api/src/lifecycle/market_data_tests.rs",
    "instrument-registry-operation-health": "crates/api/src/services/venue_operation_health/snapshot/part_18.rs",
    "atomic-opportunity-index": "crates/api/src/services/opportunity_index.rs",
    "stale-preview-fail-closed": "crates/api/src/services/hedge_preview.rs",
    "frontend-snapshot-propagation": "frontend/src/panels/modules/execution/data/preview_tests.rs",
    "history-fallback-visibility": "frontend/src/panels/modules/settings/tabs/diagnostics/tests/search.rs",
    "snapshot-generation-browser": "test/e2e/pr_eu_runtime_snapshot.spec.ts",
    "completion-governance": "scripts/check_pr_eu_completion.sh",
}
coverage_paths = (
    "crates/exchange/src/adapter.rs",
    "crates/exchange/src/aggregator.rs",
    "crates/api/src/lifecycle/market_data.rs",
    "crates/api/src/lifecycle/market_data/metadata.rs",
    "crates/api/src/lifecycle/market_data/ws_touch.rs",
    "crates/api/src/lifecycle/market_data/ws_touch/fallback.rs",
    "crates/api/src/lifecycle/market_data_tests.rs",
    "crates/api/src/lifecycle/market_data_tests/support.rs",
    "crates/api/src/services/instrument_registry.rs",
    "crates/api/src/services/instrument_registry/coverage/runtime_health.rs",
    "crates/api/src/services/opportunity_index.rs",
    "crates/api/src/services/hedge_preview.rs",
    "crates/api/src/services/venue_operation_health/snapshot/part_18.rs",
    "crates/api/src/state.rs",
    "shared-types/src/arbitrage.rs",
    "shared-types/src/venues/operation.rs",
    "frontend/src/panels/modules/execution/selection.rs",
    "frontend/src/panels/modules/execution/data/preview/response.rs",
    "frontend/src/panels/modules/settings/tabs/diagnostics/tests/search.rs",
    "test/e2e/pr_eu_runtime_snapshot.spec.ts",
    "scripts/check_pr_eu_completion.sh",
)


def fail(message: str) -> None:
    raise SystemExit(f"PR-EU completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be completed with no local remainder")
if row.count(verify_anchor) != 1:
    fail("roadmap verification must name the destructive completion gate")
queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-EU\b", queue):
    fail("completed PR-EU remains in the local queue")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-EU"]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for kind, artifact in evidence_contract.items():
    if indexed[kind]["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"{kind} artifact drifted or is missing")

markers = {
    "crates/api/src/lifecycle/market_data.rs": ("METADATA_REFRESH_EVERY_TICKS", "refresh_metadata"),
    "crates/exchange/src/adapter.rs": ("MetadataRefreshOutcome", "PublicWsSnapshot"),
    "crates/api/src/services/opportunity_index.rs": (
        "current: ArcSwap<OpportunityIndexSnapshot>",
        "OpportunitySnapshotMismatch",
    ),
    "crates/api/src/services/venue_operation_health/snapshot/part_18.rs": (
        "instrument_registry_rows",
        "opportunity_snapshot_row",
    ),
    "frontend/src/panels/modules/execution/data/preview/response.rs": ("opportunity_snapshot_id",),
    "frontend/src/panels/modules/settings/tabs/diagnostics/tests/search.rs": (
        "history_memory_fallback_is_visible_and_searchable",
    ),
}
for path, required in markers.items():
    source = (root / path).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"missing {path} marker: {marker}")

browser = (root / "test/e2e/pr_eu_runtime_snapshot.spec.ts").read_text(encoding="utf-8")
if re.search(r"\btest\.(?:skip|fixme)\s*\(", browser):
    fail("browser fixture must not be skipped")
if "opportunitySnapshotId: \"e2e-snapshot-1\"" not in browser:
    fail("browser fixture must assert the selected snapshot generation")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
for path in coverage_paths:
    if coverage.get(path, {}).get("coverage_status") != "exact":
        fail(f"exact coverage missing for {path}")

print(f"OK PR-EU contract ({len(evidence_contract)} evidence types; {len(coverage_paths)} exact paths)")
PY

bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"

if [[ "${PR_EU_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api market_data --bin crypto-arb-api --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api opportunity_index --bin crypto-arb-api --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api stale_list_snapshot --bin crypto-arb-api --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" snapshot_generation --no-fail-fast
  npm --prefix "$ROOT" run test:e2e:pr-eu
fi

printf 'OK PR-EU runtime health, atomic opportunity generation and fallback visibility contract\n'
