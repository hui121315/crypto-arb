#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
DOC="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
NAV_DTO="$ROOT/shared-types/src/portfolio/positions.rs"
PNL_SERVICE="$ROOT/crates/api/src/services/portfolio_pnl.rs"
PNL_EVIDENCE="$ROOT/crates/api/src/services/portfolio_pnl/evidence.rs"
NAV_BREAKDOWN="$ROOT/crates/api/src/services/portfolio/risk/nav_breakdown.rs"
FUNDING_RISK="$ROOT/crates/portfolio/src/risk.rs"
POSITION_PROJECTION="$ROOT/crates/api/src/services/account_positions/projection.rs"
POSITION_LIQUIDATION="$ROOT/crates/api/src/services/account_positions/projection/liquidation.rs"
POSITION_FANOUT="$ROOT/crates/api/src/services/account_positions/tests/error_paths.rs"
BROWSER="$ROOT/test/e2e/pr_cd_portfolio_evidence.spec.ts"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-CD completion gate failed: %s\n' "$1" >&2
  exit 1
}

if [[ "$MODE" == "--self-test" ]]; then
  temp="$(mktemp -d "${TMPDIR:-/tmp}/crossline-pr-cd.XXXXXX")"
  for file in "$DOC" "$NAV_DTO" "$PNL_SERVICE" "$PNL_EVIDENCE" "$NAV_BREAKDOWN" \
    "$FUNDING_RISK" "$POSITION_PROJECTION" "$POSITION_LIQUIDATION" \
    "$POSITION_FANOUT" "$BROWSER"; do
    cp "$file" "$temp/$(basename "$file").$(printf '%s' "$file" | shasum | cut -c1-8)"
  done
  restore_file() {
    local file="$1"
    cp "$temp/$(basename "$file").$(printf '%s' "$file" | shasum | cut -c1-8)" "$file"
  }
  restore() {
    for file in "$DOC" "$NAV_DTO" "$PNL_SERVICE" "$PNL_EVIDENCE" "$NAV_BREAKDOWN" \
      "$FUNDING_RISK" "$POSITION_PROJECTION" "$POSITION_LIQUIDATION" \
      "$POSITION_FANOUT" "$BROWSER"; do
      restore_file "$file"
    done
    rm -rf "$temp"
  }
  trap restore EXIT

  assert_rejected() {
    local label="$1"
    if PR_CD_SKIP_TESTS=1 PR_CD_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
      fail "self-test accepted $label"
    fi
  }

  PR_CD_SKIP_TESTS=1 PR_CD_SKIP_UPSTREAM=1 bash "$0" >/dev/null

  python3 - "$NAV_DTO" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
marker = "    pub breakdown: PortfolioNavBreakdown,\n"
if text.count(marker) != 1:
    raise SystemExit("PR-CD self-test setup failed: NAV breakdown marker drifted")
path.write_text(text.replace(marker, "", 1), encoding="utf-8")
PY
  assert_rejected "a missing shared NAV breakdown"
  restore_file "$NAV_DTO"

  python3 - "$PNL_EVIDENCE" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
marker = "review_domain::realized_pnl_field_quality(row)"
if text.count(marker) != 1:
    raise SystemExit("PR-CD self-test setup failed: PnL quality marker drifted")
path.write_text(text.replace(marker, "review_domain::RealizedPnlFieldQuality::default()", 1), encoding="utf-8")
PY
  assert_rejected "PnL quality detached from the review ledger"
  restore_file "$PNL_EVIDENCE"

  python3 - "$FUNDING_RISK" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
marker = "funding_payment_usd(row).max(0.0)"
if text.count(marker) != 1:
    raise SystemExit("PR-CD self-test setup failed: funding direction marker drifted")
path.write_text(text.replace(marker, "funding_payment_usd(row).abs()", 1), encoding="utf-8")
PY
  assert_rejected "funding receive legs counted as outflow"
  restore_file "$FUNDING_RISK"

  python3 - "$POSITION_LIQUIDATION" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
marker = '"position_liquidation_price_derived_distance"'
if text.count(marker) != 1:
    raise SystemExit("PR-CD self-test setup failed: liquidation estimate marker drifted")
path.write_text(text.replace(marker, '"position_liquidation_distance"', 1), encoding="utf-8")
PY
  assert_rejected "liquidation estimate provenance drift"
  restore_file "$POSITION_LIQUIDATION"

  python3 - "$POSITION_FANOUT" <<'PY'
from pathlib import Path
import re
import sys
path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
name = "partial_position_fanout_keeps_rows_and_surfaces_route_health"
match = re.search(rf"(?m)^(\s*)fn\s+{name}\s*\(", text)
if match is None:
    raise SystemExit("PR-CD self-test setup failed: fanout test drifted")
path.write_text(text[:match.start()] + match.group(1) + "#[ignore]\n" + text[match.start():], encoding="utf-8")
PY
  assert_rejected "an ignored partial-fanout fixture"
  restore_file "$POSITION_FANOUT"

  python3 - "$BROWSER" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
title = "PR-CD keeps NAV components PnL ledger quality and liquidation provenance visible"
marker = f'test("{title}"'
if text.count(marker) != 1:
    raise SystemExit("PR-CD self-test setup failed: browser anchor drifted")
path.write_text(text.replace(marker, f'test.skip("{title}"', 1), encoding="utf-8")
PY
  assert_rejected "a skipped product browser fixture"
  restore_file "$BROWSER"

  python3 - "$DOC" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
heading = "### 🟡 6.5 下一步执行队列"
stale = "\n\n1. **PR-CD Portfolio AccountState Evidence & Risk LoadState Contract** — stale"
if text.count(heading) != 1:
    raise SystemExit("PR-CD self-test setup failed: queue heading drifted")
path.write_text(text.replace(heading, heading + stale, 1), encoding="utf-8")
PY
  assert_rejected "completed PR-CD at the queue head"
  restore_file "$DOC"

  python3 - "$DOC" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
marker = "| `PR-FE CloseRun, Pairing Evidence & Emergency Action Contract` | ✅ 完成 |"
if text.count(marker) != 1:
    raise SystemExit("PR-CD self-test setup failed: pairing authority drifted")
path.write_text(text.replace(marker, marker.replace("✅ 完成", "🟡 部分完成"), 1), encoding="utf-8")
PY
  assert_rejected "an incomplete pairing authority"

  printf 'PR-CD completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from __future__ import annotations

import csv
import json
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])
pr_id = "PR-CD"
title = "PR-CD Portfolio AccountState Evidence & Risk LoadState Contract"
verify_anchor = "`bash scripts/check_pr_cd_completion.sh --self-test`"
evidence_contract = {
    "portfolio-summary-shared-contract": ("shared-types/src/portfolio/positions.rs", "cargo test -p shared-types portfolio"),
    "portfolio-wire-compat": ("shared-types/src/portfolio/tests/snapshot.rs", "portfolio_snapshot_serializes_operation_health"),
    "review-pnl-field-quality": ("crates/review/src/realized_pnl.rs", "realized_pnl_field_quality"),
    "portfolio-pnl-ledger-resolver": ("crates/api/src/services/portfolio_pnl.rs", "portfolio_pnl"),
    "pnl-ledger-tests": ("crates/api/src/services/portfolio_pnl/tests.rs", "empty_realized_window_is_an_actual_zero_with_source"),
    "nav-component-projection": ("crates/api/src/services/portfolio/risk/nav_breakdown.rs", "services::portfolio::tests"),
    "nav-component-tests": ("crates/api/src/services/portfolio/tests/cases_a/nav.rs", "nav_breakdown_keeps_wallet_position_cash_and_unrealized_quality"),
    "portfolio-envelope-degradation": ("crates/api/src/services/portfolio/snapshot.rs", "services::portfolio::tests"),
    "portfolio-lifecycle-stale": ("crates/api/src/lifecycle/portfolio.rs", "lifecycle::portfolio"),
    "position-fallback-quality": ("crates/api/src/services/account_positions/projection.rs", "account_positions"),
    "position-partial-fanout": ("crates/api/src/services/account_positions/tests/error_paths.rs", "partial_position_fanout_keeps_rows_and_surfaces_route_health"),
    "open-order-row-health": ("crates/api/src/services/account_open_orders/health.rs", "account_open_orders"),
    "open-order-partial-fanout": ("crates/api/src/services/account_open_orders/tests.rs", "partial_open_order_fanout_keeps_rows_and_surfaces_route_health"),
    "account-state-operation-health": ("crates/api/src/services/account_state/tests.rs", "account_state_status_degrades_on_private_order_stream_attention"),
    "funding-direction-authority": ("crates/portfolio/src/risk.rs", "funding_cluster_counts_short_paying_leg"),
    "pairing-authority": ("crates/portfolio/src/pairing.rs", "applies_execution_run_pair_evidence"),
    "frontend-load-state": ("frontend/src/panels/modules/positions/data/snapshot.rs", "degraded_snapshot_envelope_keeps_snapshot_as_stale"),
    "frontend-summary-evidence": ("frontend/src/panels/modules/positions/components/summary_cards.rs", "missing_nav_component_never_renders_as_zero"),
    "frontend-liquidation-evidence": ("frontend/src/panels/modules/positions/components/positions_table/quality.rs", "position_quality_keeps_actual_liquidation_source_but_not_other_actual_fields"),
    "frontend-position-tests": ("frontend/src/panels/modules/positions/components/positions_table/testing.rs", "position_quality_keeps_actual_liquidation_source_but_not_other_actual_fields"),
    "portfolio-product-browser": ("test/e2e/pr_cd_portfolio_evidence.spec.ts", "test:e2e:pr-cd"),
    "account-state-authority": ("scripts/check_pr_ed_completion.sh", "check_pr_ed_completion.sh"),
    "private-runtime-authority": ("scripts/check_pr_fa_completion.sh", "check_pr_fa_completion.sh"),
    "parser-risk-authority": ("scripts/check_pr_fc_completion.sh", "check_pr_fc_completion.sh"),
    "completion-governance": ("scripts/check_pr_cd_completion.sh", "check_pr_cd_completion.sh --self-test"),
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-CD completion gate failed: {message}")


def require_incomplete_queue_head(doc: str, queue: str) -> None:
    matched = re.search(r"(?m)^1\.\s+\*\*(PR-[A-Z]+)\b", queue)
    if not matched:
        fail("local queue must retain a PR roadmap item at its head")
    head = matched.group(1)
    row = next((line for line in doc.splitlines() if line.startswith(f"| `{head} ")), None)
    if row is None or "✅ 完成" in row:
        fail(f"local queue head must be an incomplete roadmap row: {head}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(
    encoding="utf-8"
)
fact_lines = history.split("## 附录 I：", 1)[0].splitlines() + doc.splitlines()
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be complete with no local remainder")
if row.count(verify_anchor) != 1:
    fail("roadmap verification must bind the destructive completion gate")

queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-CD\b", queue):
    fail("completed PR-CD remains in the local queue")
successor_title = "PR-CM Venue API Status Center & Runtime Probe Contract"
successor_row = next(
    (line for line in doc.splitlines() if line.startswith(f"| `{successor_title}`")),
    None,
)
if successor_row is None:
    fail("PR-CM successor roadmap row is missing")
successor_complete = "✅ 完成" in successor_row and "剩余：无" in successor_row
successor_is_head = bool(re.search(r"(?m)^1\.\s+\*\*PR-CM\b", queue))
next_title = "PR-CN Frontend Workstation State & Navigation Runtime Contract"
next_row = next(
    (line for line in doc.splitlines() if line.startswith(f"| `{next_title}`")),
    None,
)
if next_row is None:
    fail("PR-CN next-successor roadmap row is missing")
next_complete = "✅ 完成" in next_row and "剩余：无" in next_row
next_is_head = bool(re.search(r"(?m)^1\.\s+\*\*PR-CN\b", queue))
following_title = "PR-CQ Local Runtime & Operator QA Contract"
following_row = next(
    (line for line in doc.splitlines() if line.startswith(f"| `{following_title}`")),
    None,
)
if following_row is None:
    fail("PR-CQ following-successor roadmap row is missing")
following_complete = "✅ 完成" in following_row and "剩余：无" in following_row
following_is_head = bool(re.search(r"(?m)^1\.\s+\*\*PR-CQ\b", queue))
if successor_complete:
    if successor_is_head:
        fail("completed PR-CM successor remains at the local queue head")
    if next_complete:
        if next_is_head:
            fail("completed PR-CN next-successor remains at the local queue head")
        if following_complete:
            if following_is_head:
                fail("completed PR-CQ following-successor remains at the local queue head")
            require_incomplete_queue_head(doc, queue)
        elif not following_is_head:
            fail("PR-CQ must become the local queue head after PR-CN completion")
    elif not next_is_head:
        fail("PR-CN must become the local queue head after PR-CM completion")
elif not successor_is_head:
    fail("PR-CM must remain the local queue head until its completion contract closes")
queue_items = re.findall(r"(?m)^\d+\.\s+\*\*", queue)
if not queue_items:
    fail("local queue plus external pool must not be empty")

delegated = [
    line
    for line in fact_lines
    if line.startswith("|") and "PR-CD" in line and "✅ 完成" in " ".join(line.split("|")[1:3])
]
if len(delegated) < 14:
    fail(f"expected at least 14 PR-CD roadmap/finding rows, found {len(delegated)}")
for line in delegated:
    cells = [cell.strip() for cell in line.strip().strip("|").split("|")]
    status = " ".join(cells[:2])
    if "✅ 完成" not in status or any(marker in status for marker in ("🟡", "⏳", "❌")):
        fail(f"unfinished audit row still delegates work to PR-CD: {cells[0]}")

for successor in ("PR-BN", "PR-BY", "PR-CH", "PR-CK", "PR-DS", "PR-DZ", "PR-ED", "PR-EX", "PR-FA", "PR-FB", "PR-FC", "PR-FE"):
    successor_row = next((line for line in doc.splitlines() if line.startswith(f"| `{successor} ")), None)
    if successor_row is None or "✅ 完成" not in successor_row:
        fail(f"required successor authority is not complete: {successor}")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [entry for entry in csv.DictReader(handle, delimiter="\t") if entry["pr_id"] == pr_id]
indexed = {entry["evidence_type"]: entry for entry in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for evidence_type, (artifact, command_anchor) in evidence_contract.items():
    evidence = indexed[evidence_type]
    if evidence["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"{evidence_type} artifact drifted or is missing")
    if command_anchor not in evidence["command"] or not evidence["notes"].strip():
        fail(f"{evidence_type} command or notes drifted")

markers = {
    "shared-types/src/portfolio/positions.rs": (
        "pub breakdown: PortfolioNavBreakdown", "pub struct PortfolioNavBreakdown",
        "pub value_usd: Option<f64>",
        "pub struct PortfolioPnlEvidence", "pub missing_fields: Vec<ReviewPnlField>",
    ),
    "crates/api/src/services/portfolio/risk/nav_breakdown.rs": (
        "fn nav_breakdown(", "account_state.positions.margin_plus_unrealized_pnl",
        "wallet_equity_minus_position_equity", "AccountFieldQualityStatus::Estimated",
    ),
    "crates/api/src/services/portfolio_pnl.rs": ("trading_sql_realized_window+execution_ledger+close_runs",),
    "crates/api/src/services/portfolio_pnl/evidence.rs": (
        "review_domain::realized_pnl_field_quality(row)", "close_run_count:", "unwind_run_count:",
    ),
    "crates/api/src/services/portfolio/snapshot.rs": (
        "summary.pnl_breakdown.evidence.quality == shared_types::ExecutionLedgerQuality::Missing",
    ),
    "crates/api/src/services/account_positions/projection.rs": (
        "position_entry_price_fallback", "position_notional_over_leverage_estimate",
    ),
    "crates/api/src/services/account_positions/projection/liquidation.rs": (
        "gate_rest_liq_price_derived_distance", "position_liquidation_price_derived_distance",
    ),
    "crates/portfolio/src/risk.rs": ("funding_payment_usd(row).max(0.0)",),
    "frontend/src/panels/modules/positions/components/summary_cards.rs": (
        'Card label="当日已实现 PnL"', 'class="nav-breakdown"',
        "evidence.missing_fields", "evidence.unwind_run_count",
    ),
    "frontend/src/panels/modules/positions/components/positions_table/quality.rs": (
        '"交易所强平价"', '"交易所距离"', '"估算距离"', '"强平距离不可用"',
    ),
}
for relative_path, required in markers.items():
    source = (root / relative_path).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"missing {relative_path} marker: {marker}")

test_anchors = (
    ("shared-types/src/portfolio/tests/snapshot.rs", "portfolio_snapshot_defaults_operation_health_for_legacy_payloads"),
    ("shared-types/src/portfolio/tests/snapshot.rs", "portfolio_snapshot_serializes_operation_health"),
    ("crates/api/src/services/account_positions/tests/quality.rs", "position_field_quality_marks_price_and_margin_fallbacks_as_estimated"),
    ("crates/api/src/services/account_positions/tests/quality.rs", "liquidation_distance_quality_distinguishes_exchange_estimate_and_unavailable"),
    ("crates/api/src/services/account_positions/tests/error_paths.rs", "partial_position_fanout_keeps_rows_and_surfaces_route_health"),
    ("crates/api/src/services/account_positions/tests/error_paths.rs", "partial_position_envelope_keeps_rows_and_binds_each_venue"),
    ("crates/api/src/services/account_open_orders/tests.rs", "open_order_row_health_keeps_source_freshness_and_retry_context"),
    ("crates/api/src/services/account_open_orders/tests.rs", "partial_open_order_fanout_keeps_rows_and_surfaces_route_health"),
    ("crates/api/src/services/account_state/tests.rs", "account_state_status_degrades_on_private_order_stream_attention"),
    ("crates/api/src/lifecycle/portfolio.rs", "stale_degraded_snapshot_keeps_last_business_values_and_problem_source"),
    ("crates/api/src/lifecycle/portfolio.rs", "cold_start_failure_publishes_error_envelope_without_fake_snapshot"),
    ("crates/api/src/services/portfolio_pnl/tests.rs", "today_pnl_uses_ledger_fill_snapshots_only"),
    ("crates/api/src/services/portfolio_pnl/tests.rs", "sql_realized_window_close_run_costs_reduce_today_and_history_pnl"),
    ("crates/api/src/services/portfolio_pnl/tests.rs", "empty_realized_window_is_an_actual_zero_with_source"),
    ("crates/api/src/services/portfolio/tests/cases_a/nav.rs", "nav_breakdown_keeps_wallet_position_cash_and_unrealized_quality"),
    ("crates/api/src/services/portfolio/tests/cases_a/nav.rs", "nav_breakdown_uses_only_valuation_quality_for_position_equity"),
    ("crates/portfolio/src/risk/tests.rs", "funding_cluster_excludes_long_receiving_leg"),
    ("crates/portfolio/src/risk/tests.rs", "funding_cluster_excludes_short_receiving_leg"),
    ("crates/portfolio/src/risk/tests.rs", "funding_cluster_counts_short_paying_leg"),
    ("crates/portfolio/src/pairing.rs", "applies_execution_run_pair_evidence"),
    ("crates/portfolio/src/pairing.rs", "does_not_apply_one_sided_evidence"),
    ("frontend/src/panels/modules/positions/data/tests/snapshot.rs", "degraded_snapshot_envelope_keeps_snapshot_as_stale"),
    ("frontend/src/panels/modules/positions/data/tests/snapshot.rs", "degraded_raw_ws_snapshot_keeps_snapshot_as_stale"),
    ("frontend/src/panels/modules/positions/components/summary_cards.rs", "missing_nav_component_never_renders_as_zero"),
    ("frontend/src/panels/modules/positions/components/summary_cards.rs", "pnl_label_keeps_missing_fields_and_ledger_source"),
    ("frontend/src/panels/modules/positions/components/positions_table/testing.rs", "position_quality_keeps_actual_liquidation_source_but_not_other_actual_fields"),
)
for relative_path, name in test_anchors:
    source = (root / relative_path).read_text(encoding="utf-8")
    pattern = re.compile(rf"(?m)((?:^\s*#\[[^\n]+\]\s*\n)+)\s*(?:async\s+)?fn\s+{re.escape(name)}\s*\(")
    match = pattern.search(source)
    if match is None or not re.search(r"#\[(?:tokio::)?test", match.group(1)):
        fail(f"missing runnable test anchor: {relative_path}::{name}")
    if re.search(r"\b(?:ignore|should_panic)\b", match.group(1)):
        fail(f"test anchor is ignored or should_panic: {relative_path}::{name}")

browser_path = "test/e2e/pr_cd_portfolio_evidence.spec.ts"
browser_title = "PR-CD keeps NAV components PnL ledger quality and liquidation provenance visible"
browser = (root / browser_path).read_text(encoding="utf-8")
escaped = re.escape(browser_title)
if re.search(rf"(?m)^\s*test\.(?:skip|fixme)\(\s*['\"]{escaped}['\"]", browser):
    fail("PR-CD browser anchor must not be skipped")
if not re.search(rf"(?m)^\s*test\(\s*['\"]{escaped}['\"]", browser):
    fail("missing non-skipping PR-CD browser anchor")

package = json.loads((root / "package.json").read_text(encoding="utf-8"))
scripts = package.get("scripts", {})
if scripts.get("test:e2e:pr-cd") != "playwright test test/e2e/pr_cd_portfolio_evidence.spec.ts":
    fail("PR-CD package browser command drifted")
if scripts.get("test:e2e:product", "").count(browser_path) != 1:
    fail("full product browser suite must include PR-CD exactly once")

verify_source = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
if verify_source.count("check_pr_cd_completion.sh") != 2:
    fail("repo docs/full gate wiring drifted")
for predecessor, required in {
    "scripts/check_pr_cb_completion.sh": ('successor_title = "PR-CD Portfolio AccountState Evidence & Risk LoadState Contract"', "next_is_head"),
    "scripts/check_pr_ca_completion.sh": ('following_title = "PR-CD Portfolio AccountState Evidence & Risk LoadState Contract"', "following_is_head"),
    "scripts/check_pr_bz_completion.sh": ('following_title = "PR-CD Portfolio AccountState Evidence & Risk LoadState Contract"', "final_is_head"),
}.items():
    source = (root / predecessor).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"predecessor handoff is missing {predecessor}: {marker}")

if "## 2026-07-16 PR-CD Portfolio AccountState Evidence and Risk LoadState Closure" not in history:
    fail("history closure appendix is missing")

closure_paths = {artifact for artifact, _ in evidence_contract.values()} | {
    path for path, _ in test_anchors
} | {
    "frontend/styles/src/skin/positions-summary.css",
    "scripts/check_pr_bz_completion.sh", "scripts/check_pr_ca_completion.sh",
    "scripts/check_pr_cb_completion.sh", "scripts/verify_repo_gates.sh",
}
with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {entry["file"]: entry for entry in csv.DictReader(handle, delimiter="\t")}
for path in sorted(closure_paths):
    if coverage.get(path, {}).get("coverage_status") != "exact":
        fail(f"exact coverage missing for {path}")

print(
    f"OK PR-CD static contract ({len(evidence_contract)} evidence rows; "
    f"{len(test_anchors)} runnable anchors; 1 browser anchor; {len(closure_paths)} exact paths)"
)
PY

bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"

if [[ "${PR_CD_SKIP_UPSTREAM:-0}" != "1" ]]; then
  PR_DZ_SKIP_TESTS=1 PR_DZ_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_dz_completion.sh"
  PR_ED_SKIP_TESTS=1 PR_ED_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_ed_completion.sh"
  PR_FA_SKIP_TESTS=1 PR_FA_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_fa_completion.sh"
  PR_FB_SKIP_TESTS=1 PR_FB_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_fb_completion.sh"
  PR_FC_SKIP_TESTS=1 PR_FC_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_fc_completion.sh"
  PR_BN_SKIP_TESTS=1 PR_BN_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_bn_completion.sh"
  PR_BY_SKIP_TESTS=1 PR_BY_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_by_completion.sh"
  PR_CK_SKIP_TESTS=1 PR_CK_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_ck_completion.sh"
  bash "$ROOT/scripts/check_pr_fe_completion.sh"
fi

if [[ "${PR_CD_SKIP_TESTS:-0}" != "1" ]]; then
  jobs="${CARGO_BUILD_JOBS:-10}"
  CARGO_BUILD_JOBS="$jobs" cargo test -p shared-types portfolio --lib --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p review realized_pnl --lib --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p portfolio --lib --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api account_positions --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api account_open_orders --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api account_state --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api portfolio_pnl --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api services::portfolio::tests --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api lifecycle::portfolio --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib positions --no-fail-fast
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-cd -- --workers=1
fi

printf 'PR-CD Portfolio AccountState evidence and Risk LoadState completion passed\n'
