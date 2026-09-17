#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
SOURCE="$ROOT/shared-types/src/arbitrage.rs"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

if [[ "$MODE" == "--self-test" ]]; then
  PR_DF_SKIP_TESTS=1 bash "$0" >/dev/null
  backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-df.XXXXXX")"
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
marker = "pub struct HedgeConfirmContext {"
if marker not in source:
    raise SystemExit("PR-DF self-test setup failed: shared context marker missing")
path.write_text(source.replace(marker, "pub struct DriftedConfirmContext {", 1), encoding="utf-8")
PY
  if PR_DF_SKIP_TESTS=1 bash "$0" >/dev/null 2>&1; then
    printf 'PR-DF completion self-test failed: drifted shared context passed\n' >&2
    exit 1
  fi
  printf 'PR-DF completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from __future__ import annotations

import csv
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])
title = "PR-DF Execution Preview ActionState & ApiProblem Contract"
verify_anchor = "`bash scripts/check_pr_df_completion.sh --self-test`"
evidence_contract = {
    "shared-confirm-context": "shared-types/src/arbitrage.rs",
    "backend-context-finalizer": "crates/api/src/services/execution_orchestrator/context.rs",
    "router-error-context": "crates/api/src/services/hedge_confirm/confirm/context.rs",
    "router-action-correlation": "crates/api/src/services/hedge_confirm/confirm.rs",
    "partial-outcome-contract": "crates/api/src/services/execution_orchestrator/responses.rs",
    "frontend-confirm-action": "frontend/src/panels/modules/execution/data/actions.rs",
    "frontend-context-persistence": "frontend/src/panels/modules/execution/data/run/context.rs",
    "scoped-run-refresh": "frontend/src/panels/modules/execution/data/run.rs",
    "scoped-order-refresh": "frontend/src/panels/modules/execution/data/orders.rs",
    "structured-outcome-ui": "frontend/src/panels/modules/execution/data/outcome.rs",
    "scoped-refresh-browser": "test/e2e/pr_df_execution_action.spec.ts",
    "completion-governance": "scripts/check_pr_df_completion.sh",
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-DF completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be completed with no local remainder")
if row.count(verify_anchor) != 1:
    fail("roadmap verification must name the destructive completion gate")
queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-DF\b", queue):
    fail("completed PR-DF remains in the local queue")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-DF"]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for kind, artifact in evidence_contract.items():
    if indexed[kind]["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"{kind} artifact drifted or is missing")

markers = {
    "shared-types/src/arbitrage.rs": (
        "pub struct HedgeConfirmContext {",
        "pub context: HedgeConfirmContext,",
        "pub long_problem: Option<ApiProblem>,",
        "pub short_problem: Option<ApiProblem>,",
    ),
    "crates/api/src/services/execution_orchestrator/context.rs": (
        "pub(super) fn attach_response(",
        "assign_leg_problems(&response, &mut context);",
        'object.insert("confirmContext".into()',
    ),
    "crates/api/src/services/hedge_confirm/confirm/context.rs": (
        "confirm_request_context(",
        "confirm_preview_context(",
        "with_confirm_context(",
    ),
    "frontend/src/panels/modules/execution/data/actions.rs": (
        "resolved_confirm_context(&response, &request_context)",
        "let mut problem = error.problem;",
        "confirm_context_from_problem(&problem)",
        "attach_confirm_context_decode_problem(&mut problem, &decode_error);",
        "refresh_nonce.update(",
    ),
    "frontend/src/panels/modules/execution/data/run/context.rs": (
        "store_confirm_request_context(",
        "RUN_CONTEXT_IDEMPOTENCY_KEY",
    ),
    "frontend/src/panels/modules/execution/data/run.rs": (
        "refresh_nonce.get();",
        "execution_runs_for_context(",
    ),
    "frontend/src/panels/modules/execution/data/orders.rs": (
        "refresh_nonce.get();",
        "client.trading_orders()",
    ),
    "frontend/src/panels/modules/execution/data/outcome.rs": (
        "confirm_context_detail(",
        "confirm_outcome_detail(",
    ),
}
for path, required in markers.items():
    source = (root / path).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"missing {path} marker: {marker}")

browser = (root / evidence_contract["scoped-refresh-browser"]).read_text(encoding="utf-8")
if re.search(r"\btest\.(?:skip|fixme)\s*\(", browser):
    fail("browser fixture must not be skipped")
for marker in (
    "PR-DF confirm preserves scoped runtime context and refreshes execution feeds",
    'locator(".confirm-context-detail")',
    "requests.orders",
    "requests.runs",
    "expect(requests.preview).toBe(beforeConfirm.preview)",
):
    if marker not in browser:
        fail(f"browser fixture missing scoped refresh marker: {marker}")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
for path in evidence_contract.values():
    if coverage.get(path, {}).get("coverage_status") != "exact":
        fail(f"exact coverage missing for {path}")

print(f"OK PR-DF contract ({len(evidence_contract)} evidence types; scoped execution refresh)")
PY

bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"

if [[ "${PR_DF_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p shared-types --lib hedge_confirm --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api --bin crypto-arb-api second_leg_failure_keeps_identity_environment_and_both_venue_problems --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api --bin crypto-arb-api confirm_missing_preview_terminalizes_action_run_and_preserves_identity --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib resolved_context_uses_backend_run_and_preserves_preview_runtime_truth --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib failed_problem_decodes_shared_confirm_context --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib confirm_context_detail_keeps_runtime_identity_and_leg_problems --no-fail-fast
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-df
fi

printf 'OK PR-DF confirm context, structured outcome and scoped execution refresh contract\n'
