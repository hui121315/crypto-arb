#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
SOURCE="$ROOT/crates/exchange/src/venue_capability.rs"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

if [[ "$MODE" == "--self-test" ]]; then
  if ! PR_ES_SKIP_TESTS=1 bash "$0" >/dev/null 2>&1; then
    printf 'PR-ES completion self-test setup failed: baseline gate does not pass\n' >&2
    exit 1
  fi
  backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-es.XXXXXX")"
  cp "$SOURCE" "$backup"
  restore() {
    cp "$backup" "$SOURCE"
    rm -f "$backup"
  }
  trap restore EXIT
  python3 - "$SOURCE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "ack_is_final: false,"
if marker not in source:
    raise SystemExit("PR-ES self-test setup failed: ACK finality marker missing")
path.write_text(source.replace(marker, "ack_is_final: true,", 1), encoding="utf-8")
PY
  if PR_ES_SKIP_TESTS=1 bash "$0" >/dev/null 2>&1; then
    printf 'PR-ES completion self-test failed: ACK-as-final regression passed\n' >&2
    exit 1
  fi
  printf 'PR-ES completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from __future__ import annotations

import csv
import re
import sys
from pathlib import Path


root = Path(sys.argv[1])
title = "PR-ES Venue Capability Matrix & Order Compiler Contract"
verify_anchor = "`bash scripts/check_pr_es_completion.sh --self-test`"
evidence_contract = {
    "shared-venue-capability-matrix": (
        "shared-types/src/venue_capabilities.rs",
        "VenueCapabilityMatrix",
    ),
    "eight-venue-compiler-registry": (
        "crates/exchange/src/venue_capability.rs",
        "hyperliquid_matrix_contract",
    ),
    "hedge-ticket-matrix-consumer": (
        "crates/api/src/services/hedge_preview/guards.rs",
        "exchange_capability_matrix",
    ),
    "exchange-scoped-order-read": (
        "crates/api/src/trading_service/live_adapters/route_tests/cases.rs",
        "live_router_generic_order_read_fails_without_scanning_routes",
    ),
    "settings-static-matrix": (
        "frontend/src/panels/modules/settings/tabs/adapters/venue_capabilities.rs",
        "ACK 非终态",
    ),
    "settings-browser-fixture": (
        "test/e2e/pr_es_venue_capability_matrix.spec.ts",
        "PR-ES Settings renders all venue compiler contracts without credential filtering",
    ),
    "completion-governance": (
        "scripts/check_pr_es_completion.sh",
        "bash scripts/check_pr_es_completion.sh --self-test",
    ),
}
coverage_paths = (
    "shared-types/src/venue_capabilities.rs",
    "shared-types/src/live_trading.rs",
    "shared-types/src/hedge.rs",
    "shared-types/src/lib.rs",
    "crates/exchange/src/venue_capability.rs",
    "crates/exchange/src/live.rs",
    "crates/exchange/src/lib.rs",
    "crates/api/src/services/hedge_preview/intent/compile.rs",
    "crates/api/src/services/hedge_preview/guards.rs",
    "crates/api/src/trading_service/adapters.rs",
    "crates/api/src/trading_service/live_adapters/router.rs",
    "crates/api/src/trading_service/live_adapters/routing.rs",
    "crates/api/src/trading_service/live_adapters/route_tests/cases.rs",
    "crates/api/src/trading_service/live_adapters/route_tests/cases/positions.rs",
    "frontend/src/panels/modules/settings/tabs/adapters.rs",
    "frontend/src/panels/modules/settings/tabs/adapters/venue_capabilities.rs",
    "test/e2e/pr_es_venue_capability_matrix.spec.ts",
    "scripts/check_pr_es_completion.sh",
    "scripts/verify_repo_gates.sh",
)


def fail(message: str) -> None:
    raise SystemExit(f"PR-ES completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be completed with no local remainder")
if row.count(verify_anchor) != 1:
    fail(f"roadmap verification must contain exact anchor {verify_anchor}")
queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-ES\b", queue):
    fail("completed PR-ES remains in the local queue")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-ES"]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for kind, (artifact, command_anchor) in evidence_contract.items():
    item = indexed[kind]
    if item["artifact"] != artifact or command_anchor not in item["command"]:
        fail(f"{kind} artifact/command drifted")
    if not (root / artifact).is_file():
        fail(f"missing evidence artifact {artifact}")

matrix = (root / "crates/exchange/src/venue_capability.rs").read_text(encoding="utf-8")
if matrix.count("venue_contract_test!(") != 8:
    fail("exactly eight venue matrix contract tests are required")
for marker in (
    "ack_is_final: false,",
    "ExchangeWsEvidenceScope::PrivateOrderStream",
    "ExchangeWsEvidenceScope::PrivateFillStream",
    "ExchangeWsEvidenceScope::OrderStatusRead",
    "client_order_id_policy(venue, \"capability-probe\")",
    "native_symbol_required: true",
    "native_sizing_required: true",
):
    if marker not in matrix:
        fail(f"matrix contract marker missing: {marker}")

router = (root / "crates/api/src/trading_service/live_adapters/router.rs").read_text(encoding="utf-8")
if "live router order read requires exchange-scoped get_exchange_order" not in router:
    fail("generic order read must fail closed instead of scanning routes")

tests = (root / "crates/api/src/trading_service/live_adapters/route_tests/cases.rs").read_text(encoding="utf-8")
for anchor in (
    "settings_capability_rows_cover_all_venues_without_credentials",
    "live_router_generic_order_read_fails_without_scanning_routes",
    "live_router_get_exchange_order_hits_only_target_route",
):
    if re.search(rf"#\[(?:tokio::)?test\]\s*async\s+fn\s+{anchor}\b", tests) is None \
            and re.search(rf"#\[test\]\s*fn\s+{anchor}\b", tests) is None:
        fail(f"missing non-skipping API test anchor: {anchor}")

browser = (root / "test/e2e/pr_es_venue_capability_matrix.spec.ts").read_text(encoding="utf-8")
if re.search(r"\btest\.(?:skip|fixme)\s*\(", browser):
    fail("browser fixture must not be skipped")
if "toHaveCount(8)" not in browser or "ACK 非终态" not in browser:
    fail("browser fixture must assert all venues and finality semantics")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
for path in coverage_paths:
    item = coverage.get(path)
    if item is None or item["coverage_status"] != "exact":
        fail(f"exact coverage missing for {path}")

print(
    f"OK PR-ES static contract ({len(evidence_contract)} evidence types; "
    f"8 venue contracts; {len(coverage_paths)} exact paths)"
)
PY

bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_evidence_index.sh"

if [[ "${PR_ES_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p exchange venue_capability --lib
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api live_router_generic_order_read_fails_without_scanning_routes
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api settings_capability_rows_cover_all_venues_without_credentials
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" venue_matrix_copy_keeps_non_native_market_and_finality_semantics_visible
  npm --prefix "$ROOT" run test:e2e:pr-es
fi

printf 'OK PR-ES venue capability matrix, scoped order read, Settings and browser contract\n'
