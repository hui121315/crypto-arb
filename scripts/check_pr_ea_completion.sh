#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
RUN_CONTRACT="$ROOT/shared-types/src/execution_run.rs"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

if [[ "$MODE" == "--self-test" ]]; then
  PR_EA_SKIP_TESTS=1 bash "$0" >/dev/null
  backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-ea.XXXXXX")"
  cp "$RUN_CONTRACT" "$backup"
  restore() {
    cp "$backup" "$RUN_CONTRACT"
    rm -f "$backup"
  }
  trap restore EXIT
  python3 - "$RUN_CONTRACT" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "pub const EXECUTION_RUN_TIMELINE_LIMIT: usize = 96;"
if marker not in source:
    raise SystemExit("PR-EA self-test setup failed: timeline limit missing")
path.write_text(source.replace(marker, "pub const DRIFTED_TIMELINE_LIMIT: usize = 96;", 1), encoding="utf-8")
PY
  if PR_EA_SKIP_TESTS=1 bash "$0" >/dev/null 2>&1; then
    printf 'PR-EA completion self-test failed: drifted run contract passed\n' >&2
    exit 1
  fi
  printf 'PR-EA completion self-test passed\n'
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
title = "PR-EA ExecutionRun Finality & ActionState Contract"
verify_anchor = "`bash scripts/check_pr_ea_completion.sh --self-test`"
evidence_contract = {
    "shared-run-evidence": "shared-types/src/execution_run.rs",
    "run-envelope-binding": "shared-types/src/hedge.rs",
    "orchestration-binding": "crates/api/src/services/execution_orchestrator/run_model.rs",
    "timeline-projector": "crates/api/src/services/execution_runs/timeline.rs",
    "order-ledger-projector": "crates/api/src/services/execution_runs/project.rs",
    "durable-run-ledger": "crates/api/src/services/execution_run_store.rs",
    "durable-replay-fixture": "crates/api/src/services/execution_run_store/tests.rs",
    "finality-problem-context": "crates/api/src/services/run_finality/problems.rs",
    "scoped-rest-replay": "frontend/src/panels/modules/execution/data/run.rs",
    "action-state": "frontend/src/panels/modules/execution/data/actions.rs",
    "timeline-ui": "frontend/src/panels/modules/execution/components/execution_status_bar.rs",
    "timeline-evidence-copy": "frontend/src/panels/modules/execution/components/execution_status_bar/format.rs",
    "timeline-responsive": "frontend/styles/src/skin/execution-controls-responsive.css",
    "product-browser": "test/e2e/pr_ea_execution_finality.spec.ts",
    "completion-governance": "scripts/check_pr_ea_completion.sh",
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-EA completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be completed with no local remainder")
if row.count(verify_anchor) != 1:
    fail("roadmap verification must name the destructive completion gate")
queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-EA\b", queue):
    fail("completed PR-EA remains in the local queue")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-EA"]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for kind, artifact in evidence_contract.items():
    if indexed[kind]["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"{kind} artifact drifted or is missing")

markers = {
    "shared-types/src/execution_run.rs": (
        "pub const EXECUTION_RUN_TIMELINE_LIMIT: usize = 96;",
        "pub struct ExecutionRunTimelineEvent",
        "pub finality_confidence: ExecutionFillConfidence",
        "pub struct ExecutionRunLegEvidence",
        "pub struct ExecutionRunEvidence",
    ),
    "shared-types/src/hedge.rs": (
        "pub identity: Option<VenueOrderIdentity>",
        "pub finality_source: Option<OrderUpdateSource>",
        "pub confirmed_filled_at_ms: Option<i64>",
        "pub evidence: ExecutionRunEvidence",
    ),
    "crates/api/src/services/execution_orchestrator/run_model.rs": (
        "execution_runs::initialize_evidence(",
        "common::request_id::current()",
        "execution_runs::record(state, run.clone())",
    ),
    "crates/api/src/services/execution_runs/timeline.rs": (
        "pub(crate) fn initialize_evidence(",
        "pub(crate) fn append_order_update_evidence(",
        "pub(crate) fn append_ledger_event_evidence(",
        "pub(crate) fn append_finality_problem_evidence(",
        "EXECUTION_RUN_TIMELINE_LIMIT",
    ),
    "crates/api/src/services/execution_runs/project.rs": (
        "append_order_update_evidence(run, record)",
        "append_ledger_event_evidence(&mut projected, event)",
        "project_ledger_event_update_durable",
    ),
    "crates/api/src/services/execution_run_store.rs": (
        "append_bare_jsonl(path, run)",
        "append_jsonl(path, run, true)",
        "file.sync_data()?",
        "ExecutionRunStoreReplay",
    ),
    "crates/api/src/services/run_finality/problems.rs": (
        "common::request_id::normalize(None)",
        "fn finality_remote_missing_problem(",
        "fn finality_refresh_failure_problem(",
        "codes::HEDGE_ORDER_FINALITY_FAILED",
    ),
    "frontend/src/panels/modules/execution/data/run.rs": (
        "execution_runs_for_context(",
        "apply_seed_result(run, seed_problem, &context, result)",
        "use_ws_channel_context_snapshot_fallback(",
    ),
    "frontend/src/panels/modules/execution/data/actions.rs": (
        "RwSignal<ActionState>",
        "store_execution_run_context(&run, &idempotency_key)",
        "refresh_nonce.update(",
    ),
    "frontend/src/panels/modules/execution/components/execution_status_bar.rs": (
        "<RunTimeline run=visible_run/>",
        "fn RunTimeline(",
        'class="execution-timeline"',
        "dropped_event_count",
    ),
    "frontend/src/panels/modules/execution/components/execution_status_bar/format.rs": (
        "instrument.native_symbol.as_str()",
        'format!("精度 {}", precision.join(" / "))',
        '"置信 {}",',
        'format!("request_id {request_id}")',
    ),
    "frontend/styles/src/skin/execution-controls-responsive.css": (
        ".execution-timeline-row",
        "grid-template-columns: 1fr",
    ),
}
for path, required in markers.items():
    source = (root / path).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"missing {path} marker: {marker}")

fixture_anchors = {
    "crates/api/src/services/execution_runs/timeline/tests.rs": (
        "timeline_is_bounded_and_reports_dropped_events",
        "private_ws_fill_promotes_leg_confidence_and_keeps_request_id",
    ),
    "crates/api/src/services/execution_run_store/tests.rs": (
        "durable_run_snapshot_replays_embedded_timeline",
    ),
    "frontend/src/panels/modules/execution/components/execution_status_bar/testing.rs": (
        "hedged_label_requires_both_legs_filled",
    ),
    "frontend/src/panels/modules/execution/components/execution_status_bar/testing/format_tests.rs": (
        "timeline_event_keeps_source_confidence_and_request_context",
        "leg_evidence_text_prefers_native_instrument_and_precision",
    ),
}
for path, anchors in fixture_anchors.items():
    source = (root / path).read_text(encoding="utf-8")
    for anchor in anchors:
        match = re.search(rf"#\[(?:tokio::)?test\]\s*(?:async\s+)?fn\s+{re.escape(anchor)}\b", source)
        if match is None:
            fail(f"non-skipping fixture anchor missing: {path}:{anchor}")
        prefix = source[max(0, match.start() - 160):match.start()]
        if "#[ignore" in prefix or "should_panic" in prefix:
            fail(f"fixture anchor is skipped or panic-expected: {path}:{anchor}")

browser = (root / evidence_contract["product-browser"]).read_text(encoding="utf-8")
if re.search(r"\btest\.(?:skip|fixme)\s*\(", browser):
    fail("browser fixture must not be skipped")
for marker in (
    "PR-EA renders ticket evidence and replayable finality timeline",
    "PR-EA restores the same run timeline from scoped REST after reload",
    'toContainText("第二腿已提交，等待成交确认")',
    'not.toContainText("双腿完成")',
    'toContainText("request_id req-pr-ea-submit")',
    'toContainText("精度 tick 0.01 / step 1 / contract 0.001")',
):
    if marker not in browser:
        fail(f"browser fixture missing marker: {marker}")

package = json.loads((root / "package.json").read_text(encoding="utf-8"))
scripts = package.get("scripts", {})
if scripts.get("test:e2e:pr-ea") != "playwright test test/e2e/pr_ea_execution_finality.spec.ts":
    fail("dedicated browser script is missing")
if scripts.get("test:e2e:product", "").count("pr_ea_execution_finality.spec.ts") != 1:
    fail("product suite wiring is missing or duplicated")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
for path in evidence_contract.values():
    if coverage.get(path, {}).get("coverage_status") != "exact":
        fail(f"exact coverage missing for {path}")

print(
    f"OK PR-EA contract ({len(evidence_contract)} evidence types; "
    "run-order-ledger-REST-browser closure)"
)
PY

bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"

if [[ "${PR_EA_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p shared-types execution_run --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api execution_runs::timeline --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api execution_run_store --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib execution_status_bar --no-fail-fast
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-ea -- --workers=1
fi

printf 'OK PR-EA execution run finality and action-state contract\n'
