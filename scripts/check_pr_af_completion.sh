#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
AUDIT="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
EVIDENCE="$ROOT/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
MODEL="$ROOT/frontend/src/panels/modules/opportunity_view_model/model.rs"
ASSEMBLY="$ROOT/frontend/src/panels/modules/opportunities/data/detail_assembly.rs"
INDEX_VIEW="$ROOT/frontend/src/panels/modules/index_composition/view.rs"
BROWSER="$ROOT/test/e2e/pr_af_opportunity_semantics.spec.ts"
REPO_GATE="$ROOT/scripts/verify_repo_gates.sh"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-AF completion gate failed: %s\n' "$1" >&2
  exit 1
}

run_static_gate() {
  PR_AF_SKIP_TESTS=1 PR_AF_SKIP_BROWSER=1 PR_AF_SKIP_UPSTREAM=1 \
    PR_AF_SKIP_BROWSER_LIST=1 bash "$0" >/dev/null
}

expect_rejected() {
  local message="$1"
  if run_static_gate 2>/dev/null; then
    fail "$message"
  fi
}

if [[ "$MODE" == "--self-test" ]]; then
  audit_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-af-audit.XXXXXX")"
  evidence_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-af-evidence.XXXXXX")"
  model_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-af-model.XXXXXX")"
  assembly_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-af-assembly.XXXXXX")"
  index_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-af-index.XXXXXX")"
  browser_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-af-browser.XXXXXX")"
  repo_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-af-repo.XXXXXX")"
  cp "$AUDIT" "$audit_backup"
  cp "$EVIDENCE" "$evidence_backup"
  cp "$MODEL" "$model_backup"
  cp "$ASSEMBLY" "$assembly_backup"
  cp "$INDEX_VIEW" "$index_backup"
  cp "$BROWSER" "$browser_backup"
  cp "$REPO_GATE" "$repo_backup"
  restore() {
    cp "$audit_backup" "$AUDIT"
    cp "$evidence_backup" "$EVIDENCE"
    cp "$model_backup" "$MODEL"
    cp "$assembly_backup" "$ASSEMBLY"
    cp "$index_backup" "$INDEX_VIEW"
    cp "$browser_backup" "$BROWSER"
    cp "$repo_backup" "$REPO_GATE"
    rm -f "$audit_backup" "$evidence_backup" "$model_backup" "$assembly_backup" \
      "$index_backup" "$browser_backup" "$repo_backup"
  }
  trap restore EXIT

  run_static_gate

  python3 - "$AUDIT" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "| `PR-AF Frontend Opportunity View Model Semantics` | ✅ 完成 |"
if source.count(marker) != 1:
    raise SystemExit("PR-AF self-test setup failed: completed row drifted")
path.write_text(source.replace(marker, "| `PR-AF Frontend Opportunity View Model Semantics` | 🟡 部分完成 |", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted a downgraded roadmap row"
  cp "$audit_backup" "$AUDIT"

  python3 - "$EVIDENCE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
lines = path.read_text(encoding="utf-8").splitlines()
for index, line in enumerate(lines):
    if line.startswith("PR-AF\t"):
        del lines[index]
        break
else:
    raise SystemExit("PR-AF self-test setup failed: evidence row missing")
path.write_text("\n".join(lines) + "\n", encoding="utf-8")
PY
  expect_rejected "self-test accepted incomplete evidence"
  cp "$evidence_backup" "$EVIDENCE"

  python3 - "$MODEL" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = ".filter(is_main_p0_row)"
if source.count(marker) != 1:
    raise SystemExit("PR-AF self-test setup failed: frontend boundary drifted")
path.write_text(source.replace(marker, ".filter(|_| true)", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted a disabled frontend MainP0 boundary"
  cp "$model_backup" "$MODEL"

  python3 - "$ASSEMBLY" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "    apply_authoritative_opportunity_semantics(&mut seed, &response.opportunity);"
if source.count(marker) != 1:
    raise SystemExit("PR-AF self-test setup failed: detail hydration drifted")
path.write_text(source.replace(marker, "    seed.funding_stats = Default::default();", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted detached authoritative detail semantics"
  cp "$assembly_backup" "$ASSEMBLY"

  python3 - "$INDEX_VIEW" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "const INDEX_COMPONENT_PREVIEW_COUNT: usize = 8;"
if source.count(marker) != 1:
    raise SystemExit("PR-AF self-test setup failed: index preview limit drifted")
path.write_text(source.replace(marker, "const INDEX_COMPONENT_PREVIEW_COUNT: usize = 99;", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted an index detail that no longer exercises expand"
  cp "$index_backup" "$INDEX_VIEW"

  python3 - "$BROWSER" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = 'test("PR-AF main opportunity detail consumes complete funding and index evidence"'
if source.count(marker) != 1:
    raise SystemExit("PR-AF self-test setup failed: browser anchor drifted")
path.write_text(source.replace(marker, 'test.skip("PR-AF main opportunity detail consumes complete funding and index evidence"', 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted skipped browser evidence"
  cp "$browser_backup" "$BROWSER"

  python3 - "$AUDIT" <<'PY'
from pathlib import Path
import re
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
updated, count = re.subn(r"(?m)^1\. \*\*PR-AG\b", "1. **PR-AF", source, count=1)
if count != 1:
    raise SystemExit("PR-AF self-test setup failed: successor queue head drifted")
path.write_text(updated, encoding="utf-8")
PY
  expect_rejected "self-test accepted PR-AF reinserted into the local queue"
  cp "$audit_backup" "$AUDIT"

  python3 - "$REPO_GATE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = 'PR_AF_SKIP_TESTS=1 PR_AF_SKIP_BROWSER=1 PR_AF_SKIP_UPSTREAM=1 PR_AF_SKIP_BROWSER_LIST=1 bash "$ROOT/scripts/check_pr_af_completion.sh"'
if source.count(marker) != 2:
    raise SystemExit("PR-AF self-test setup failed: repo wiring drifted")
path.write_text(source.replace(marker, "true # PR-AF gate removed", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted single-scope repository wiring"

  printf 'PR-AF completion self-test passed\n'
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
pr_id = "PR-AF"
title = "PR-AF Frontend Opportunity View Model Semantics"
verify_anchor = "`bash scripts/check_pr_af_completion.sh --self-test`"
evidence_contract = {
    "profitability-successor": "scripts/check_pr_dd_completion.sh",
    "funding-truth-successor": "shared-types/src/fees.rs",
    "instrument-coverage-successor": "shared-types/src/instrument_coverage.rs",
    "index-registry-successor": "scripts/check_pr_ec_completion.sh",
    "product-semantics-successor": "scripts/check_pr_bs_completion.sh",
    "funding-dto-projection": "frontend/src/panels/modules/funding_stats/from_dto.rs",
    "funding-evidence-ui": "frontend/src/panels/modules/funding_stats/component.rs",
    "authoritative-detail-assembly": "frontend/src/panels/modules/opportunities/data/detail_assembly.rs",
    "frontend-main-p0-boundary": "frontend/src/panels/modules/opportunity_view_model/model.rs",
    "stream-main-p0-boundary": "frontend/src/panels/modules/opportunities/data/stream_patch.rs",
    "backend-main-p0-boundary": "crates/api/src/services/opportunity_detail.rs",
    "index-detail-evidence": "frontend/src/panels/modules/index_composition/view.rs",
    "index-evidence-labels": "frontend/src/panels/modules/index_composition/labels.rs",
    "product-browser": "test/e2e/pr_af_opportunity_semantics.spec.ts",
    "completion-governance": "scripts/check_pr_af_completion.sh",
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-AF completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be complete with no local remainder")
if row.count(verify_anchor) != 1:
    fail("roadmap verification must name the destructive gate once")

queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-AF\b", queue):
    fail("completed PR-AF remains in the local queue")
successor_title = "PR-AG Shared Types Contract Stratification"
successor_row = next(
    (line for line in doc.splitlines() if line.startswith(f"| `{successor_title}`")),
    None,
)
if successor_row is None:
    fail("PR-AG successor roadmap row is missing")
successor_complete = "✅ 完成" in successor_row
successor_queued = re.search(r"(?m)^\d+\.\s+\*\*PR-AG\b", queue) is not None
successor_is_head = re.search(r"(?m)^1\.\s+\*\*PR-AG\b", queue) is not None
if successor_complete and successor_queued:
    fail("completed PR-AG successor remains in the local queue")
if not successor_complete and not successor_is_head:
    fail("unfinished PR-AG successor must be the local queue head")

for successor in ("PR-DD", "PR-FQ", "PR-FP", "PR-EC", "PR-BS"):
    successor_row = next(
        (line for line in doc.splitlines() if line.startswith(f"| `{successor} ")),
        None,
    )
    if successor_row is None or "✅ 完成" not in successor_row:
        fail(f"completed successor authority drifted: {successor}")

history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(encoding="utf-8")
if "## 2026-07-17 PR-AF Frontend Opportunity View Model Semantics Closure" not in history:
    fail("history closure appendix is missing")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == pr_id]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for evidence_type, artifact in evidence_contract.items():
    evidence = indexed[evidence_type]
    if evidence["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"evidence artifact drifted or is missing: {evidence_type}")
    if not evidence["command"].strip() or not evidence["notes"].strip():
        fail(f"evidence lacks command or notes: {evidence_type}")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
for artifact in evidence_contract.values():
    item = coverage.get(artifact)
    if item is None or item["coverage_status"] != "exact" or not item["evidence"].startswith("line:"):
        fail(f"exact coverage missing for {artifact}")

markers = {
    "frontend/src/panels/modules/funding_stats/from_dto.rs": (
        "pub(crate) fn from_dto(dto: &ArbitrageOpportunityDto)",
        "p95_diff_bps",
        "fn evidence_text(evidence: &FundingHistoryEvidence)",
    ),
    "frontend/src/panels/modules/funding_stats/component.rs": (
        'class="funding-cycle-distribution"',
        'class="funding-cycle-evidence"',
    ),
    "frontend/src/panels/modules/opportunities/data/detail_assembly.rs": (
        "apply_authoritative_opportunity_semantics(&mut seed, &response.opportunity);",
        "seed.funding_stats = FundingCycleStatsView::from_dto(opportunity);",
        "seed.index_composition =",
    ),
    "frontend/src/panels/modules/opportunity_view_model/model.rs": (
        ".filter(is_main_p0_row)",
        "pub(crate) fn rejected_main_p0_ids",
    ),
    "frontend/src/panels/modules/opportunities/data/stream_patch.rs": (
        "removed_ids.extend(rejected_main_p0_ids(&event.changed_rows));",
    ),
    "crates/api/src/services/opportunity_detail.rs": (
        "ensure_main_p0_opportunity(&opportunity)?;",
        '"strategyExposure": exposure',
        '"scope": "main_p0"',
    ),
    "frontend/src/panels/modules/index_composition/view.rs": (
        "const INDEX_COMPONENT_PREVIEW_COUNT: usize = 8;",
        "rows.split_off(rows.len().min(INDEX_COMPONENT_PREVIEW_COUNT))",
        '<details class="index-component-more">',
    ),
    "frontend/src/panels/modules/index_composition/labels.rs": (
        'parts.push(format!("官方来源 {url}"));',
        'parts.push(format!("schema {version}"));',
        'parts.push(format!("成分 {total} 项 · 首屏 {shown} 项"));',
    ),
}
for relative, required in markers.items():
    source = (root / relative).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"source marker missing: {relative}:{marker}")

browser_path = "test/e2e/pr_af_opportunity_semantics.spec.ts"
browser = (root / browser_path).read_text(encoding="utf-8")
browser_title = "PR-AF main opportunity detail consumes complete funding and index evidence"
if re.search(rf'test\.(?:skip|fixme)\(\s*["\']{re.escape(browser_title)}["\']', browser):
    fail("browser anchor is skipped")
if not re.search(rf'test\(\s*["\']{re.escape(browser_title)}["\']', browser):
    fail("browser anchor is missing")

for relative in ("package.json", "scripts/fixtures/release_qa_contract/package.json"):
    package = json.loads((root / relative).read_text(encoding="utf-8"))
    scripts = package.get("scripts", {})
    if scripts.get("test:e2e:pr-af") != f"playwright test {browser_path}":
        fail(f"dedicated browser command drifted in {relative}")
    if scripts.get("test:e2e:product", "").count(browser_path) != 1:
        fail(f"product browser suite must include PR-AF exactly once in {relative}")

repo_gate = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
if repo_gate.count("check_pr_af_completion.sh") != 2:
    fail("repo gate must execute PR-AF exactly once in docs and all scopes")

print(f"OK PR-AF static contract ({len(evidence_contract)} evidence types; 1 browser anchor)")
PY

if [[ "${PR_AF_SKIP_BROWSER_LIST:-0}" != "1" ]]; then
  npx playwright test "$BROWSER" --list >/dev/null
fi

if [[ "${PR_AF_SKIP_UPSTREAM:-0}" != "1" ]]; then
  PR_DD_SKIP_TESTS=1 PR_DD_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_dd_completion.sh"
  PR_EC_SKIP_TESTS=1 PR_EC_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_ec_completion.sh"
  PR_BS_SKIP_TESTS=1 PR_BS_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_bs_completion.sh"
  bash "$ROOT/scripts/check_product_audit_progress.sh"
  bash "$ROOT/scripts/check_product_audit_evidence.sh"
  bash "$ROOT/scripts/check_product_audit_evidence_index.sh"
fi

if [[ "${PR_AF_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api \
    non_p0_opportunity_is_blocked_from_main_detail --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" \
    main_ui_rejects_non_p0_rows_and_reports_patch_removals --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" \
    stream_patch_removes_row_that_leaves_main_p0_scope --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" \
    cycle_windows_drive_percentile_display --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" \
    snapshot_preserves_all_components_for_expand --no-fail-fast
fi

if [[ "${PR_AF_SKIP_BROWSER:-0}" != "1" ]]; then
  npm --prefix "$ROOT" run test:e2e:pr-af
fi

printf 'PR-AF completion gate passed\n'
