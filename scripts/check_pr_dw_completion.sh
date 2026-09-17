#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
QUALITY_SERVICE="$ROOT/crates/api/src/services/venue_quality.rs"
QUALITY_AGGREGATE="$ROOT/crates/api/src/services/venue_quality/aggregate.rs"
BROWSER="$ROOT/test/e2e/pr_dw_review_runtime.spec.ts"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-DW completion gate failed: %s\n' "$1" >&2
  exit 1
}

if [[ "$MODE" == "--self-test" ]]; then
  service_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-dw-service.XXXXXX")"
  aggregate_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-dw-aggregate.XXXXXX")"
  browser_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-dw-browser.XXXXXX")"
  cp "$QUALITY_SERVICE" "$service_backup"
  cp "$QUALITY_AGGREGATE" "$aggregate_backup"
  cp "$BROWSER" "$browser_backup"
  restore() {
    cp "$service_backup" "$QUALITY_SERVICE"
    cp "$aggregate_backup" "$QUALITY_AGGREGATE"
    cp "$browser_backup" "$BROWSER"
    rm -f "$service_backup" "$aggregate_backup" "$browser_backup"
  }
  trap restore EXIT

  PR_DW_SKIP_TESTS=1 PR_DW_SKIP_UPSTREAM=1 bash "$0" >/dev/null

  python3 - "$QUALITY_SERVICE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = ".with_request_id(common::request_id::current())"
if source.count(marker) != 1:
    raise SystemExit("PR-DW self-test setup failed: quality request marker drifted")
path.write_text(source.replace(marker, "", 1), encoding="utf-8")
PY
  if PR_DW_SKIP_TESTS=1 PR_DW_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a quality envelope without body request correlation"
  fi
  cp "$service_backup" "$QUALITY_SERVICE"

  python3 - "$QUALITY_AGGREGATE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "use exchange::HttpQualityWindowSnapshot;"
if source.count(marker) != 1:
    raise SystemExit("PR-DW self-test setup failed: bounded HTTP window marker drifted")
path.write_text(
    source.replace(marker, "use exchange::HttpOutcomeMetricSnapshot;", 1),
    encoding="utf-8",
)
PY
  if PR_DW_SKIP_TESTS=1 PR_DW_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted cumulative HTTP outcomes as the quality window"
  fi
  cp "$aggregate_backup" "$QUALITY_AGGREGATE"

  python3 - "$BROWSER" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = 'await expect(okx).not.toContainText("30ms");'
if source.count(marker) != 1:
    raise SystemExit("PR-DW self-test setup failed: venue isolation assertion drifted")
path.write_text(
    source.replace(marker, 'await expect(okx).toContainText("30ms");', 1),
    encoding="utf-8",
)
PY
  if PR_DW_SKIP_TESTS=1 PR_DW_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted cross-venue latency leakage"
  fi

  printf 'PR-DW completion self-test passed\n'
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
title = "PR-DW Review Runtime Evidence & VenueQuality LoadState Contract"
verify_anchor = "`bash scripts/check_pr_dw_completion.sh --self-test`"
evidence_contract = {
    "shared-review-envelope": "shared-types/src/review.rs",
    "shared-quality-window-contract": "shared-types/src/venues/quality.rs",
    "bounded-http-quality-window": "crates/exchange/src/http_metrics.rs",
    "bounded-realtime-quality-tracker": "crates/realtime/src/quality.rs",
    "funding-latency-boundary": "crates/api/src/lifecycle/funding.rs",
    "quality-rebuild-lifecycle": "crates/api/src/lifecycle/venue_quality.rs",
    "per-operation-quality-aggregation": "crates/api/src/services/venue_quality/aggregate.rs",
    "quality-request-correlation": "crates/api/src/services/venue_quality.rs",
    "review-request-correlation": "crates/api/src/routers/review.rs",
    "durable-review-reader": "crates/api/src/services/review/ledger.rs",
    "review-storage-health": "crates/api/src/services/review/storage_health.rs",
    "review-cursor-budget": "crates/api/src/services/review/paging.rs",
    "funding-ingest-evidence": "crates/api/src/services/review/tests/funding_ingest.rs",
    "close-unwind-evidence": "crates/api/src/services/review/close_run_linkage.rs",
    "frontend-quality-retry": "frontend/src/panels/modules/review/data/retry.rs",
    "frontend-review-quality-meta": "frontend/src/panels/modules/review/view/derive.rs",
    "frontend-operation-evidence": "frontend/src/panels/modules/review/components/venue_quality_metrics.rs",
    "frontend-quality-table": "frontend/src/panels/modules/review/components/venue_quality_panel.rs",
    "product-browser": "test/e2e/pr_dw_review_runtime.spec.ts",
    "product-suite-contract": "package.json",
    "completion-governance": "scripts/check_pr_dw_completion.sh",
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-DW completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be completed with no local remainder")
if row.count(verify_anchor) != 1:
    fail("roadmap verification must name the destructive completion gate")
queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-DW\b", queue):
    fail("completed PR-DW remains in the local queue")

history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(
    encoding="utf-8"
)
if "## 2026-07-14 PR-DW Review Runtime Evidence and VenueQuality Closure" not in history:
    fail("history closure appendix is missing")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(
    encoding="utf-8", newline=""
) as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-DW"]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for kind, artifact in evidence_contract.items():
    if indexed[kind]["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"{kind} artifact drifted or is missing")

markers = {
    "shared-types/src/review.rs": (
        "pub request_id: Option<String>",
        "pub storage_health: Option<VenueOperationHealth>",
        "pub funding_payment_ingest: Option<FundingPaymentIngestReport>",
        "pub fn with_request_id",
    ),
    "shared-types/src/venues/quality.rs": (
        "VENUE_QUALITY_WINDOW_MAX_SAMPLES: u32 = 600",
        "pub sample_window: VenueQualitySampleWindow",
        "pub operation_health: Vec<VenueOperationHealth>",
        "pub retry_after_ms: Option<u64>",
        "pub request_id: Option<String>",
    ),
    "crates/exchange/src/http_metrics.rs": (
        "pub struct HttpQualityWindowSnapshot",
        "VENUE_QUALITY_WINDOW_MAX_SAMPLES as usize",
        "pub fn http_quality_window_snapshot",
        "quality_window_is_venue_scoped_and_bounded",
    ),
    "crates/realtime/src/quality.rs": (
        "fill_success: VecDeque<bool>",
        "slippage_bps: VecDeque<f64>",
        "fill_and_slippage_windows_are_bounded",
    ),
    "crates/api/src/lifecycle/venue_quality.rs": (
        "Duration::from_secs(5)",
        "MissedTickBehavior::Skip",
        "quality.rebuild_snapshot()",
    ),
    "crates/api/src/services/venue_quality/aggregate.rs": (
        "use exchange::HttpQualityWindowSnapshot;",
        "group_operations",
        "apply_operation_context",
        "VENUE_QUALITY_OPERATION_DEGRADED",
    ),
    "crates/api/src/services/venue_quality.rs": (
        "venue_operation_health::snapshot(state)",
        "exchange::http_quality_window_snapshot()",
        ".with_request_id(common::request_id::current())",
    ),
    "frontend/src/panels/modules/review/data/retry.rs": (
        "self.retry_after_ms",
        "review_retry_deadline_for_result",
    ),
    "frontend/src/panels/modules/review/view/derive.rs": (
        'parts.push(format!("request_id {request_id}"))',
        'format!("{} operation", envelope.operation_count)',
        'parts.push(format!("retry {retry_after_ms}ms"))',
    ),
    "frontend/src/panels/modules/review/components/venue_quality_metrics.rs": (
        "operation_class",
        "operation_value",
        "operation_title",
    ),
    "frontend/src/panels/modules/review/components/venue_quality_panel.rs": (
        '<th>"运行证据"</th>',
        "title=operation_detail",
    ),
}
for path, required in markers.items():
    source = (root / path).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"missing {path} marker: {marker}")

funding = (root / "crates/api/src/lifecycle/funding.rs").read_text(encoding="utf-8")
if "record_venue_quality" in funding or "quality.record_api_probe" in funding:
    fail("funding whole-scan latency still pollutes venue quality")
lifecycle = (root / "crates/api/src/lifecycle.rs").read_text(encoding="utf-8")
if "mod venue_quality;" not in lifecycle or "venue_quality::spawn_updater" not in lifecycle:
    fail("venue quality lifecycle updater is not registered")
review_router = (root / "crates/api/src/routers/review.rs").read_text(encoding="utf-8")
if review_router.count(".with_request_id(common::request_id::current())") != 3:
    fail("all three review envelopes must carry the body request id")

browser_path = evidence_contract["product-browser"]
browser = (root / browser_path).read_text(encoding="utf-8")
if re.search(r"\btest\.(?:skip|fixme)\s*\(", browser):
    fail("browser fixture must not be skipped")
browser_markers = (
    "PR-DW keeps durable review, funding, close, unwind, storage, and body request evidence visible",
    "PR-DW renders bounded per-operation quality without cross-venue latency leakage and honors retry",
    "req-review-body-pr-dw",
    "req-quality-body-pr-dw",
    "HTTP_RATE_LIMITED",
    'await expect(okx).not.toContainText("30ms");',
    "expect(qualityRequests).toBe(beforeRetryWindow);",
)
for marker in browser_markers:
    if marker not in browser:
        fail(f"browser fixture missing marker: {marker}")
if browser.count('test("PR-DW ') != 2:
    fail("browser fixture must keep both non-skipping scenarios")

package = json.loads((root / "package.json").read_text(encoding="utf-8"))
scripts = package.get("scripts", {})
dedicated = "playwright test test/e2e/pr_dw_review_runtime.spec.ts"
if scripts.get("test:e2e:pr-dw") != dedicated:
    fail("dedicated browser command is missing")
if scripts.get("test:e2e:product", "").count(browser_path) != 1:
    fail("product suite must include the PR-DW fixture exactly once")
release_fixture = json.loads(
    (root / "scripts/fixtures/release_qa_contract/package.json").read_text(encoding="utf-8")
)
if release_fixture.get("scripts", {}).get("test:e2e:product", "").count(browser_path) != 1:
    fail("release QA fixture must include the PR-DW fixture exactly once")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(
    encoding="utf-8", newline=""
) as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
for path in evidence_contract.values():
    if path == "package.json":
        continue
    if coverage.get(path, {}).get("coverage_status") != "exact":
        fail(f"exact coverage missing for {path}")

print(
    f"OK PR-DW static contract ({len(evidence_contract)} evidence types; "
    "durable review, bounded operation quality, request correlation and retry closure)"
)
PY

if [[ "${PR_DW_SKIP_UPSTREAM:-0}" != "1" ]]; then
  PR_DH_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dh_completion.sh"
  PR_DT_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dt_completion.sh"
  PR_DZ_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dz_completion.sh"
  PR_EG_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_eg_completion.sh"
  bash "$ROOT/scripts/check_product_audit_evidence.sh"
  bash "$ROOT/scripts/check_product_audit_coverage.sh"
  bash "$ROOT/scripts/check_release_qa_contract.sh"
fi

if [[ "${PR_DW_SKIP_TESTS:-0}" != "1" ]]; then
  JOBS="${CARGO_BUILD_JOBS:-10}"
  CARGO_BUILD_JOBS="$JOBS" cargo test -p shared-types review --lib --no-fail-fast
  CARGO_BUILD_JOBS="$JOBS" cargo test -p shared-types venues::tests --lib --no-fail-fast
  CARGO_BUILD_JOBS="$JOBS" cargo test -p exchange http_metrics::tests --lib --no-fail-fast
  CARGO_BUILD_JOBS="$JOBS" cargo test -p realtime quality::tests --lib --no-fail-fast
  CARGO_BUILD_JOBS="$JOBS" cargo test -p api --bin crypto-arb-api services::venue_quality --no-fail-fast
  CARGO_BUILD_JOBS="$JOBS" cargo test -p api --bin crypto-arb-api services::review --no-fail-fast
  CARGO_BUILD_JOBS="$JOBS" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib review --no-fail-fast
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-dw -- --workers=1
fi

printf 'OK PR-DW review runtime evidence and VenueQuality completion contract\n'
