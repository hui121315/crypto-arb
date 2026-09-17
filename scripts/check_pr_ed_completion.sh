#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
NAV="$ROOT/crates/api/src/services/portfolio/risk.rs"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

if [[ "$MODE" == "--self-test" ]]; then
  PR_ED_SKIP_TESTS=1 bash "$0" >/dev/null
  backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-ed.XXXXXX")"
  cp "$NAV" "$backup"
  restore() {
    cp "$backup" "$NAV"
    rm -f "$backup"
  }
  trap restore EXIT
  python3 - "$NAV" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "status: AccountFieldQualityStatus::Actual,"
if marker not in source:
    raise SystemExit("PR-ED self-test setup failed: actual NAV evidence marker missing")
path.write_text(source.replace(marker, "status: AccountFieldQualityStatus::Missing,", 1), encoding="utf-8")
PY
  if PR_ED_SKIP_TESTS=1 bash "$0" >/dev/null 2>&1; then
    printf 'PR-ED completion self-test failed: false account NAV evidence passed\n' >&2
    exit 1
  fi
  printf 'PR-ED completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from __future__ import annotations

import csv
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])
title = "PR-ED AccountState Evidence & Unified Margin Contract"
verify_anchor = "`bash scripts/check_pr_ed_completion.sh --self-test`"
evidence_contract = {
    "shared-account-contract": "shared-types/src/orders.rs",
    "portfolio-nav-contract": "shared-types/src/portfolio/positions.rs",
    "hyperliquid-account-read": "crates/exchange/src/adapters/hyperliquid_account_read.rs",
    "hyperliquid-margin-parser": "crates/exchange/src/adapters/hyperliquid_private_data.rs",
    "account-state-projection": "crates/api/src/services/account_state.rs",
    "account-binding-scope": "crates/api/src/services/account_state/derive.rs",
    "account-field-quality": "crates/api/src/services/account_state/derive/account_summary.rs",
    "account-equity-nav": "crates/api/src/services/portfolio/risk.rs",
    "hedge-margin-facts": "crates/api/src/services/hedge_margin/venues.rs",
    "frontend-account-evidence": "frontend/src/panels/modules/settings/tabs/venue_credentials/panels/account_state/rows.rs",
    "account-state-browser": "test/e2e/pr_ed_account_state.spec.ts",
    "completion-governance": "scripts/check_pr_ed_completion.sh",
}
coverage_paths = tuple(evidence_contract.values())


def fail(message: str) -> None:
    raise SystemExit(f"PR-ED completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be completed with no local remainder")
if row.count(verify_anchor) != 1:
    fail("roadmap verification must name the destructive completion gate")
queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-ED\b", queue):
    fail("completed PR-ED remains in the local queue")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-ED"]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for kind, artifact in evidence_contract.items():
    if indexed[kind]["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"{kind} artifact drifted or is missing")

markers = {
    "shared-types/src/orders.rs": (
        "pub enum AccountEquityScope",
        "pub struct VenueAccountSummary",
        "pub account_bindings: Vec<AccountBindingEvidence>",
        "pub account_scope: Option<String>",
    ),
    "shared-types/src/portfolio/positions.rs": (
        "pub struct PortfolioNavEvidence",
        "pub nav_evidence: PortfolioNavEvidence",
    ),
    "crates/exchange/src/adapters/hyperliquid.rs": (
        "async fn get_account_read",
        "self.get_partial_account_read(currency).await",
    ),
    "crates/exchange/src/adapters/hyperliquid_account_read.rs": (
        "async fn get_partial_account_read",
        "parse_account_summary",
        "get_spot_balance(currency)",
        "tokio::join!",
    ),
    "crates/exchange/src/adapters/hyperliquid_private_data.rs": (
        "pub(super) fn parse_account_summary",
        "AccountEquityScope::Perpetuals",
        "withdrawable_balance_usd: Some(evidence.withdrawable)",
    ),
    "crates/api/src/services/account_state.rs": (
        "child_account_bindings(&balances, &positions, &open_orders)",
        "apply_account_scopes(&mut field_quality, &account_bindings)",
    ),
    "crates/api/src/services/account_state/derive.rs": (
        "mod account_summary;",
        "pub(super) fn apply_account_scopes",
        "for row in rows",
    ),
    "crates/api/src/services/account_state/derive/account_summary.rs": (
        "for row in &open_orders.rows",
        '"equityScope"',
        '"withdrawableBalance"',
        "AccountFieldQualityStatus::Missing",
    ),
    "crates/api/src/services/portfolio/risk.rs": (
        "pub(super) fn account_nav",
        "status: AccountFieldQualityStatus::Actual,",
        "status: AccountFieldQualityStatus::Missing,",
        'source: "account_state.account_summaries.total_equity_usd"',
        "NAV_SAMPLE_SOURCE_ACCOUNT_EQUITY_MISSING",
    ),
    "crates/api/src/services/hedge_margin/venues.rs": (
        "pub(super) fn account_margin_rows",
        "withdrawable_balance_usd",
        "total_available_balance_usd",
    ),
    "frontend/src/panels/modules/settings/tabs/venue_credentials/panels/account_state/rows.rs": (
        '"统一账户权益"',
        '"Available ${:.2} · Withdrawable {} · IM ${:.2} · MM ${:.2}"',
        'unwrap_or_else(|| "未知".to_owned())',
        "account_scope",
    ),
}
for path, required in markers.items():
    source = (root / path).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"missing {path} marker: {marker}")

browser = (root / "test/e2e/pr_ed_account_state.spec.ts").read_text(encoding="utf-8")
if re.search(r"\btest\.(?:skip|fixme)\s*\(", browser):
    fail("browser fixture must not be skipped")
for marker in ("navEvidence", "order-ed-1", "withdrawableBalance", "accountScope"):
    if marker not in browser:
        fail(f"browser fixture missing account evidence: {marker}")

package = (root / "package.json").read_text(encoding="utf-8")
if '"test:e2e:pr-ed"' not in package or "pr_ed_account_state.spec.ts" not in package:
    fail("package script or product suite wiring is missing")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
for path in coverage_paths:
    if coverage.get(path, {}).get("coverage_status") != "exact":
        fail(f"exact coverage missing for {path}")

print(f"OK PR-ED contract ({len(evidence_contract)} evidence types; {len(coverage_paths)} exact paths)")
PY

bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"

if [[ "${PR_ED_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p shared-types --lib orders::tests --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p exchange --lib hyperliquid_account_summary_preserves_perp_margin_and_withdrawable_facts --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api --bin crypto-arb-api account_state --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api --bin crypto-arb-api account_nav --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api --bin crypto-arb-api margin_rows_prefer_verified_account_summary_available_facts --no-fail-fast
  cargo test --manifest-path "$ROOT/frontend/Cargo.toml" account_state --no-fail-fast
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-ed
fi

printf 'OK PR-ED account state, unified margin and NAV evidence contract\n'
