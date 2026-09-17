#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
FIXTURE="$ROOT/crates/arbitrage/fixtures/fee_schedule_registry_v2.json"
FEE_REGISTRY="$ROOT/crates/arbitrage/src/algorithms/fee_evidence.rs"
ENGINE="$ROOT/crates/arbitrage/src/engine_v3.rs"
SCORE_EXPLAIN="$ROOT/frontend/src/panels/modules/score_explain.rs"
AUDIT_DOC="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-BV completion gate failed: %s\n' "$1" >&2
  exit 1
}

if [[ "$MODE" == "--self-test" ]]; then
  fixture_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-bv-fixture.XXXXXX")"
  fee_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-bv-fee.XXXXXX")"
  engine_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-bv-engine.XXXXXX")"
  score_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-bv-score.XXXXXX")"
  audit_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-bv-audit.XXXXXX")"
  cp "$FIXTURE" "$fixture_backup"
  cp "$FEE_REGISTRY" "$fee_backup"
  cp "$ENGINE" "$engine_backup"
  cp "$SCORE_EXPLAIN" "$score_backup"
  cp "$AUDIT_DOC" "$audit_backup"
  restore() {
    cp "$fixture_backup" "$FIXTURE"
    cp "$fee_backup" "$FEE_REGISTRY"
    cp "$engine_backup" "$ENGINE"
    cp "$score_backup" "$SCORE_EXPLAIN"
    cp "$audit_backup" "$AUDIT_DOC"
    rm -f "$fixture_backup" "$fee_backup" "$engine_backup" "$score_backup" "$audit_backup"
  }
  trap restore EXIT

  PR_BV_SKIP_TESTS=1 PR_BV_SKIP_UPSTREAM=1 bash "$0" >/dev/null

  python3 - "$FIXTURE" <<'PY'
from pathlib import Path
import json
import sys

path = Path(sys.argv[1])
payload = json.loads(path.read_text(encoding="utf-8"))
payload["venues"][0]["schedules"] = [
    row for row in payload["venues"][0]["schedules"] if row["product"] != "spot"
]
path.write_text(json.dumps(payload, ensure_ascii=True, indent=2) + "\n", encoding="utf-8")
PY
  if PR_BV_SKIP_TESTS=1 PR_BV_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a venue without both required products"
  fi
  cp "$fixture_backup" "$FIXTURE"

  python3 - "$ENGINE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "let resolution = resolve_ranking(RankingInputs {"
if source.count(marker) != 1:
    raise SystemExit("PR-BV self-test setup failed: ranking resolver marker drifted")
path.write_text(source.replace(marker, "let resolution = legacy_depth_ranking(RankingInputs {", 1), encoding="utf-8")
PY
  if PR_BV_SKIP_TESTS=1 PR_BV_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted depth priority detached from the full resolver"
  fi
  cp "$engine_backup" "$ENGINE"

  python3 - "$SCORE_EXPLAIN" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "let label = fee_evidence_label(ids.len(), complete);"
if source.count(marker) != 1:
    raise SystemExit("PR-BV self-test setup failed: shared fee-copy marker drifted")
path.write_text(source.replace(marker, 'let label = format!("费率证据 {}/2", ids.len());', 1), encoding="utf-8")
PY
  if PR_BV_SKIP_TESTS=1 PR_BV_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted score copy detached from the shared fee policy"
  fi
  cp "$score_backup" "$SCORE_EXPLAIN"

  python3 - "$FEE_REGISTRY" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "    fn standard_fee_registry_covers_every_scanner_venue_product() {"
if source.count(marker) != 1:
    raise SystemExit("PR-BV self-test setup failed: fixture test marker drifted")
path.write_text(source.replace(marker, "    #[ignore]\n" + marker, 1), encoding="utf-8")
PY
  if PR_BV_SKIP_TESTS=1 PR_BV_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a skipped venue-product fixture test"
  fi
  cp "$fee_backup" "$FEE_REGISTRY"

  python3 - "$AUDIT_DOC" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
updated, replacements = __import__("re").subn(
    r"(?m)^1\. \*\*PR-[A-Z]+",
    "1. **PR-BW",
    source,
    count=1,
)
if replacements != 1:
    raise SystemExit("PR-BV self-test setup failed: current queue head is missing")
path.write_text(updated, encoding="utf-8")
PY
  if PR_BV_SKIP_TESTS=1 PR_BV_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a completed PR-BW successor in the local queue"
  fi

  printf 'PR-BV completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from __future__ import annotations

import csv
import json
import re
import sys
from pathlib import Path
from urllib.parse import urlparse

root = Path(sys.argv[1])
pr_id = "PR-BV"
title = "PR-BV Ranking Contract & Score Explanation"
verify_anchor = "`bash scripts/check_pr_bv_completion.sh --self-test`"
evidence_contract = {
    "frontend-score-explain": "frontend/src/panels/modules/futures/mapper/conversion.rs",
    "frontend-kpi-score-explain": "frontend/src/panels/modules/futures/view/sections.rs",
    "verified-ranking-fee-evidence": "shared-types/src/arbitrage.rs",
    "fee-registry-contract": "crates/arbitrage/src/algorithms/fee_evidence.rs",
    "fee-registry-fixture": "crates/arbitrage/fixtures/fee_schedule_registry_v2.json",
    "fee-registry-route": "crates/api/src/app.rs",
    "fee-registry-runtime-health": "crates/api/src/services/market_data/cache/helpers2.rs",
    "full-ranking-resolver": "crates/arbitrage/src/calculator.rs",
    "depth-ranking-resolver": "crates/arbitrage/src/engine_v3.rs",
    "nonmutating-ranking-history": "crates/arbitrage/src/algorithms/funding_history.rs",
    "cross-module-cost-copy": "frontend/src/panels/modules/cost_copy.rs",
    "predecessor-fee-gate": "scripts/check_pr_ee_completion.sh",
    "completion-governance": "scripts/check_pr_bv_completion.sh",
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-BV completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(
    encoding="utf-8"
)
fact_lines = history.split("## 附录 I：", 1)[0].splitlines() + doc.splitlines()
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be complete with no local remainder")
if row.count(verify_anchor) != 1:
    fail("roadmap verification must name the destructive gate once")

queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-BV\b", queue):
    fail("completed PR-BV remains in the local queue")
successor_title = "PR-BW ExecutionRun Finality & ActionState Snapshot"
successor_row = next(
    (line for line in doc.splitlines() if line.startswith(f"| `{successor_title}`")),
    None,
)
if successor_row is None:
    fail("PR-BW successor roadmap row is missing")
successor_complete = "✅ 完成" in successor_row
successor_queued = re.search(r"(?m)^\d+\.\s+\*\*PR-BW\b", queue) is not None
successor_is_head = re.search(r"(?m)^1\.\s+\*\*PR-BW\b", queue) is not None
if successor_complete and successor_queued:
    fail("completed PR-BW successor remains in the local queue")
if not successor_complete and not successor_is_head:
    fail("unfinished PR-BW successor must remain the local queue head")

for finding_title in (
    "盈利性评分解释已前端可见，深度预算已按完整 verified resolver 排序",
    "TradeFeeSnapshot 官方 evidence 契约覆盖标准产品",
    "OfficialSchedule 具备完整标准产品证据",
    "深度预取排序与最终产品评分共用 resolver",
):
    finding = next((line for line in reversed(fact_lines) if finding_title in line), None)
    if finding is None or "✅ 完成" not in finding:
        fail(f"ranking or fee finding remains incomplete: {finding_title}")

audit_237 = next((line for line in reversed(fact_lines) if "`AUD-237`" in line), None)
if audit_237 is None or "✅ 完成" not in audit_237 or "PR-BV" in audit_237:
    fail("AUD-237 successor closure drifted or still delegates work to PR-BV")

if "## 2026-07-15 PR-BV Ranking Contract and Score Explanation Closure" not in history:
    fail("history closure appendix is missing")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == pr_id]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for evidence_type, artifact in evidence_contract.items():
    evidence = indexed[evidence_type]
    if evidence["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"{evidence_type} artifact drifted or is missing")
    if not evidence["command"].strip() or not evidence["notes"].strip():
        fail(f"{evidence_type} lacks command or notes")

fixture = json.loads(
    (root / "crates/arbitrage/fixtures/fee_schedule_registry_v2.json").read_text(encoding="utf-8")
)
if fixture.get("schema") != {
    "version": "fee_schedule_registry_v2",
    "fingerprint": "5b455df819b75439",
}:
    fail("fee registry schema fingerprint drifted")
expected_hosts = {
    "binance": {"developers.binance.com"},
    "bitget": {"www.bitget.com"},
    "bybit": {"www.bybit.com", "bybit-exchange.github.io"},
    "gate": {"www.gate.com", "www.gate.io"},
    "htx": {"www.htx.com", "huobiapi.github.io"},
    "hyperliquid": {"hyperliquid.gitbook.io"},
    "kucoin": {"www.kucoin.com"},
    "okx": {"www.okx.com"},
}
venues = fixture.get("venues", [])
if {venue.get("venue") for venue in venues} != set(expected_hosts) or len(venues) != 8:
    fail("fee registry must contain exactly the eight supported venue families")
row_count = 0
for venue in venues:
    venue_id = venue["venue"]
    schedules = venue.get("schedules", [])
    row_count += len(schedules)
    if len(schedules) != 2 or {row.get("product") for row in schedules} != {"spot", "perp"}:
        fail(f"{venue_id} must contain exactly one spot and one perp schedule")
    for schedule in schedules:
        product = schedule["product"]
        evidence = schedule.get("evidence", {})
        version = evidence.get("scheduleVersion", "")
        expected_id = f"standard-fee:{venue_id}:{product}:{version}"
        if schedule.get("fixtureId") != expected_id:
            fail(f"{venue_id} {product} fixture identity drifted")
        if schedule.get("snapshotTtlMs") != 86_400_000:
            fail(f"{venue_id} {product} snapshot TTL drifted")
        if not evidence.get("tier") or not evidence.get("scope") or not evidence.get("checkedAtMs"):
            fail(f"{venue_id} {product} lacks tier, contract scope, or checked time")
        source_url = evidence.get("sourceUrl", "")
        parsed = urlparse(source_url)
        if parsed.scheme != "https" or parsed.hostname not in expected_hosts[venue_id]:
            fail(f"{venue_id} {product} source is not an approved official provider host")
if row_count != 16:
    fail("fee registry must expose 16 venue-product schedules")

markers = {
    "crates/arbitrage/src/algorithms/fee_evidence.rs": (
        "pub const STANDARD_FEE_PRODUCTS: [FeeProduct; 2]",
        "official_source_host_matches",
        "standard_fee_registry_fails_closed_when_product_coverage_is_removed",
        "standard_fee_registry_fails_closed_on_tier_or_contract_scope_drift",
        "standard_fee_registry_rejects_non_provider_source_hosts",
    ),
    "crates/arbitrage/src/calculator.rs": (
        'RANKING_RESOLVER_SOURCE: &str = "arbitrage/calculator:verified-ranking-v1"',
        "pub(crate) struct RankingInputs",
        "pub(crate) fn resolve_ranking(inputs: RankingInputs<'_>)",
    ),
    "crates/arbitrage/src/engine_v3.rs": (
        "let resolution = resolve_ranking(RankingInputs {",
        "history.preview_with_candidate(&ranked_row)",
        "depth_priority_uses_full_verified_ranking_resolver",
    ),
    "crates/arbitrage/src/algorithms/funding_history.rs": (
        "pub fn preview_with_candidate",
        "preview_matches_next_snapshot_without_recording_twice",
    ),
    "crates/api/src/services/market_data/cache/helpers2.rs": (
        "fee_evidence::STANDARD_FEE_PRODUCTS",
        "if received == fee_evidence::STANDARD_FEE_PRODUCTS.len() as u64",
    ),
    "frontend/src/panels/modules/cost_copy.rs": (
        "pub(crate) fn fee_evidence_label",
        "pub(crate) const fn one_cycle_verdict",
        "shared_cost_copy_fails_closed_and_keeps_one_cycle_verdicts_stable",
    ),
    "frontend/src/panels/modules/score_explain.rs": (
        "let label = fee_evidence_label(ids.len(), complete);",
    ),
    "frontend/src/panels/modules/score_explain/tests.rs": (
        "score_explanation_fails_closed_for_partial_fee_evidence",
    ),
    "frontend/src/panels/modules/opportunity_view_model/labels.rs": ("fee_evidence_label(",),
    "frontend/src/panels/modules/futures/data/model.rs": ("fee_evidence_label(",),
    "frontend/src/panels/modules/execution/components/risk_preview/cost.rs": (
        "fee_evidence_label(",
        "one_cycle_verdict(",
    ),
    "scripts/check_pr_ee_completion.sh": (
        "pr_bv_complete",
        "expected_schedule_count = 16 if pr_bv_complete else 10",
    ),
}
for path, required in markers.items():
    source = (root / path).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"missing {path} marker: {marker}")

fee_source = (root / "crates/arbitrage/src/algorithms/fee_evidence.rs").read_text(encoding="utf-8")
if "#[ignore]" in fee_source:
    fail("fee registry fixture tests must not be ignored")
engine_source = (root / "crates/arbitrage/src/engine_v3.rs").read_text(encoding="utf-8")
for retired in ("depth_profit_quality", "depth_history_quality", "depth_one_cycle_net_bps"):
    if retired in engine_source:
        fail(f"retired lightweight ranking helper returned: {retired}")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
for artifact in evidence_contract.values():
    if Path(artifact).suffix == ".json":
        continue
    if coverage.get(artifact, {}).get("coverage_status") != "exact":
        fail(f"exact coverage missing for {artifact}")

print(
    f"OK PR-BV static contract ({len(evidence_contract)} evidence rows; "
    "8 venues, 16 fee fixtures, one verified ranking resolver and shared cost copy)"
)
PY

bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"

if [[ "${PR_BV_SKIP_UPSTREAM:-0}" != "1" ]]; then
  PR_BS_SKIP_TESTS=1 PR_BS_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_bs_completion.sh"
fi

if [[ "${PR_BV_SKIP_TESTS:-0}" != "1" ]]; then
  jobs="${CARGO_BUILD_JOBS:-10}"
  CARGO_BUILD_JOBS="$jobs" cargo test -p arbitrage standard_fee_registry --lib --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p arbitrage depth_ --lib --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p arbitrage preview_matches_next_snapshot_without_recording_twice --lib --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api fee_schedule --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib cost_copy --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib cost_profile --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib score_explain --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib risk_preview --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib opportunity_view_model --no-fail-fast
fi

printf 'PR-BV Ranking Contract & Score Explanation completion gate passed\n'
