#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
AUDIT="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
EVIDENCE="$ROOT/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
PREVIEW="$ROOT/crates/api/src/services/hedge_preview.rs"
READINESS="$ROOT/crates/api/src/services/hedge_preview/readiness.rs"
INTENT="$ROOT/crates/api/src/services/hedge_preview/intent.rs"
INTENT_TESTS="$ROOT/crates/api/src/services/hedge_preview/intent_tests.rs"
TICKET_SPEC="$ROOT/crates/api/src/services/hedge_ticket/spec.rs"
REPO_GATE="$ROOT/scripts/verify_repo_gates.sh"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-AK completion gate failed: %s\n' "$1" >&2
  exit 1
}

run_static_gate() {
  PR_AK_SKIP_TESTS=1 PR_AK_SKIP_UPSTREAM=1 PR_AK_SKIP_BROWSER=1 bash "$0" >/dev/null
}

expect_rejected() {
  local message="$1"
  if run_static_gate 2>/dev/null; then
    fail "$message"
  fi
}

if [[ "$MODE" == "--self-test" ]]; then
  tmp="$(mktemp -d "${TMPDIR:-/tmp}/crossline-pr-ak.XXXXXX")"
  files=(
    "$AUDIT"
    "$EVIDENCE"
    "$PREVIEW"
    "$READINESS"
    "$INTENT"
    "$INTENT_TESTS"
    "$TICKET_SPEC"
    "$REPO_GATE"
  )
  for file in "${files[@]}"; do
    cp "$file" "$tmp/$(basename "$file").$(printf '%s' "$file" | shasum | cut -c1-12)"
  done
  restore_file() {
    local file="$1"
    cp "$tmp/$(basename "$file").$(printf '%s' "$file" | shasum | cut -c1-12)" "$file"
  }
  restore_all() {
    for file in "${files[@]}"; do
      restore_file "$file"
    done
    rm -rf "$tmp"
  }
  trap restore_all EXIT

  run_static_gate

  python3 - "$AUDIT" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = "| `PR-AK Arbitrage Hedge Preview & Ticket Contract` | ✅ 完成 |"
if text.count(old) != 1:
    raise SystemExit("PR-AK self-test setup failed: roadmap row drifted")
path.write_text(text.replace(old, "| `PR-AK Arbitrage Hedge Preview & Ticket Contract` | 🟡 部分完成 |", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted a downgraded roadmap row"
  restore_file "$AUDIT"

  python3 - "$EVIDENCE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
rows = path.read_text(encoding="utf-8").splitlines()
for index, row in enumerate(rows):
    if row.startswith("PR-AK\tleg-market-contract\t"):
        del rows[index]
        break
else:
    raise SystemExit("PR-AK self-test setup failed: evidence row missing")
path.write_text("\n".join(rows) + "\n", encoding="utf-8")
PY
  expect_rejected "self-test accepted incomplete evidence"
  restore_file "$EVIDENCE"

  python3 - "$READINESS" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = "if !venue_names_equal(&evidence.venue, expected_venue)"
if text.count(old) != 1:
    raise SystemExit("PR-AK self-test setup failed: leg venue guard drifted")
path.write_text(text.replace(old, "if false", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted cross-wired leg market evidence"
  restore_file "$READINESS"

  python3 - "$INTENT" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = "        symbol: spec.symbol,"
if text.count(old) != 1:
    raise SystemExit("PR-AK self-test setup failed: intent symbol binding drifted")
path.write_text(text.replace(old, "        symbol: input.opp.symbol.clone(),", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted generic opportunity symbols in order intents"
  restore_file "$INTENT"

  python3 - "$PREVIEW" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = "ticket\n            .long_leg\n            .reference_price\n            .or(req.long_price)"
if text.count(old) != 1:
    raise SystemExit("PR-AK self-test setup failed: price authority drifted")
path.write_text(text.replace(old, "req.long_price.or(ticket.long_leg.reference_price)", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted client price precedence over ticket evidence"
  restore_file "$PREVIEW"

  python3 - "$INTENT_TESTS" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = "#[test]\nfn pr_ak_validate_preview_rejects_path_body_opportunity_mismatch()"
if text.count(old) != 1:
    raise SystemExit("PR-AK self-test setup failed: identity fixture drifted")
path.write_text(text.replace(old, "#[test]\n#[ignore]\nfn pr_ak_validate_preview_rejects_path_body_opportunity_mismatch()", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted a skipped request identity fixture"
  restore_file "$INTENT_TESTS"

  python3 - "$AUDIT" <<'PY'
from pathlib import Path
import re
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
updated, count = re.subn(r"(?m)^1\. \*\*PR-AM\b", "1. **PR-AK", text, count=1)
if count != 1:
    raise SystemExit("PR-AK self-test setup failed: successor queue head drifted")
path.write_text(updated, encoding="utf-8")
PY
  expect_rejected "self-test accepted PR-AK reinserted into the local queue"
  restore_file "$AUDIT"

  python3 - "$REPO_GATE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = 'PR_AK_SKIP_TESTS=1 PR_AK_SKIP_UPSTREAM=1 PR_AK_SKIP_BROWSER=1 bash "$ROOT/scripts/check_pr_ak_completion.sh"'
if text.count(old) != 2:
    raise SystemExit("PR-AK self-test setup failed: repo wiring drifted")
path.write_text(text.replace(old, "true # PR-AK gate removed", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted single-scope repository wiring"

  printf 'PR-AK completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from __future__ import annotations

import csv
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])
pr_id = "PR-AK"
title = "PR-AK Arbitrage Hedge Preview & Ticket Contract"
verify_anchor = "`bash scripts/check_pr_ak_completion.sh --self-test`"
evidence_contract = {
    "preview-request-identity": "crates/api/src/services/hedge_preview/intent.rs",
    "leg-market-contract": "crates/api/src/services/hedge_preview/readiness.rs",
    "ticket-leg-identity": "crates/api/src/services/hedge_ticket/spec.rs",
    "order-intent-leg-identity": "crates/api/src/services/hedge_preview/intent.rs",
    "ticket-reference-price": "crates/api/src/services/hedge_preview.rs",
    "request-identity-fixture": "crates/api/src/services/hedge_preview/intent_tests.rs",
    "leg-contract-fixture": "crates/api/src/routers/arbitrage/hedge_tests/pr_ak.rs",
    "ticket-symbol-fixture": "crates/api/src/services/hedge_ticket/tests/pr_ak.rs",
    "compile-symbol-fixture": "crates/api/src/routers/arbitrage/hedge_tests/pr_ak.rs",
    "price-authority-fixture": "crates/api/src/routers/arbitrage/hedge_tests/pr_ak.rs",
    "scoped-preflight-authority": "scripts/check_pr_dv_completion.sh",
    "scoped-margin-authority": "scripts/check_pr_cg_completion.sh",
    "instrument-sizing-authority": "scripts/check_pr_eb_completion.sh",
    "profitability-authority": "scripts/check_pr_dd_completion.sh",
    "run-evidence-authority": "scripts/check_pr_ea_completion.sh",
    "finality-authority": "scripts/check_pr_bw_completion.sh",
    "frontend-scoped-evidence": "frontend/src/panels/modules/execution/components/risk_preview/preflight.rs",
    "frontend-order-plan": "frontend/src/panels/modules/execution/components/risk_preview/evidence/order_plan.rs",
    "browser-scoped-preflight": "test/e2e/pr_dv_scoped_preflight.spec.ts",
    "browser-instrument-sizing": "test/e2e/pr_eb_instrument_sizing.spec.ts",
    "browser-finality": "test/e2e/pr_ea_execution_finality.spec.ts",
    "completion-governance": "scripts/check_pr_ak_completion.sh",
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-AK completion gate failed: {message}")


def source(relative: str) -> str:
    return (root / relative).read_text(encoding="utf-8")


def require_markers(relative: str, markers: tuple[str, ...]) -> str:
    text = source(relative)
    for marker in markers:
        if marker not in text:
            fail(f"{relative} marker missing: {marker}")
    return text


def require_non_skipping_test(relative: str, name: str) -> None:
    text = source(relative)
    match = re.search(
        rf"(?P<attrs>(?:\s*#\[[^\n]+\]\s*)+)(?:async\s+)?fn\s+{re.escape(name)}\b",
        text,
    )
    if match is None or "test" not in match.group("attrs"):
        fail(f"runnable test missing: {relative}::{name}")
    attrs = match.group("attrs")
    if any(marker in attrs for marker in ("ignore", "should_panic", "cfg(")):
        fail(f"evidence test is skippable: {relative}::{name}")


doc = source("docs/PRODUCT_FULL_AUDIT_REFINEMENT.md")
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be complete with no local remainder")
if row.count(verify_anchor) != 1:
    fail("roadmap verification must name the destructive gate once")

queue_start = doc.find("### 🟡 6.5")
if queue_start < 0:
    fail("local execution queue is missing")
queue = doc[queue_start:]
if re.search(r"(?m)^\d+\.\s+\*\*PR-AK\b", queue):
    fail("completed PR-AK remains in the local queue")
successor_row = next((line for line in doc.splitlines() if line.startswith("| `PR-AM ")), None)
if successor_row is None:
    fail("PR-AM successor roadmap row is missing")
if "✅ 完成" not in successor_row and re.search(r"(?m)^1\.\s+\*\*PR-AM\b", queue) is None:
    fail("unfinished PR-AM must be the local queue head")

for successor_id in ("PR-DV", "PR-CG", "PR-EB", "PR-DD", "PR-EA", "PR-BW"):
    successor_row = next(
        (line for line in doc.splitlines() if line.startswith(f"| `{successor_id} ")),
        None,
    )
    if successor_row is None or "✅ 完成" not in successor_row:
        fail(f"required successor authority is not complete: {successor_id}")

history = source("docs/audit_history/PRODUCT_AUDIT_HISTORY.md")
if "## 2026-07-17 PR-AK Arbitrage Hedge Preview and Ticket Contract Closure" not in history:
    fail("history closure appendix is missing")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [item for item in csv.DictReader(handle, delimiter="\t") if item["pr_id"] == pr_id]
indexed = {item["evidence_type"]: item for item in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for evidence_type, artifact in evidence_contract.items():
    item = indexed[evidence_type]
    if item["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"evidence artifact drifted or is missing: {evidence_type}")
    if not item["command"].strip() or not item["notes"].strip():
        fail(f"evidence lacks command or notes: {evidence_type}")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {item["file"]: item for item in csv.DictReader(handle, delimiter="\t")}
for artifact in set(evidence_contract.values()):
    item = coverage.get(artifact)
    if item is None or item["coverage_status"] != "exact" or not item["evidence"].startswith("line:"):
        fail(f"exact coverage missing for {artifact}")

require_markers(
    "crates/api/src/services/hedge_preview/intent.rs",
    ("codes::HEDGE_PREVIEW_OPPORTUNITY_MISMATCH", "pathOpportunityId", "symbol: spec.symbol", "evidence_bound_symbol"),
)
require_markers(
    "crates/api/src/services/hedge_preview/readiness.rs",
    (
        "leg_market_contract_blockers",
        "if !venue_names_equal(&evidence.venue, expected_venue)",
        "evidence.symbol.trim().is_empty()",
    ),
)
require_markers(
    "crates/api/src/services/hedge_preview.rs",
    (
        "ticket\n            .long_leg\n            .reference_price\n            .or(req.long_price)",
        "ticket\n            .short_leg\n            .reference_price\n            .or(req.short_price)",
    ),
)
require_markers(
    "crates/api/src/services/hedge_ticket/spec.rs",
    ("evidence_bound_identity", "evidence.symbol.trim().to_owned()", "price.is_finite() && *price > 0.0"),
)

for relative, test_name in (
    ("crates/api/src/services/hedge_preview/intent_tests.rs", "pr_ak_validate_preview_rejects_path_body_opportunity_mismatch"),
    ("crates/api/src/services/hedge_ticket/tests/pr_ak.rs", "pr_ak_leg_specs_bind_each_venue_market_symbol_and_price"),
    ("crates/api/src/routers/arbitrage/hedge_tests/pr_ak.rs", "pr_ak_build_leg_and_compile_plan_keep_distinct_market_evidence_symbols"),
    ("crates/api/src/routers/arbitrage/hedge_tests/pr_ak.rs", "pr_ak_preview_rejects_cross_wired_leg_market_evidence"),
    ("crates/api/src/routers/arbitrage/hedge_tests/pr_ak.rs", "pr_ak_preview_rejects_leg_market_evidence_without_symbol_or_price"),
    ("crates/api/src/routers/arbitrage/hedge_tests/pr_ak.rs", "pr_ak_preview_prices_prefer_ticket_evidence_over_client_values"),
):
    require_non_skipping_test(relative, test_name)

for relative in (
    "test/e2e/pr_dv_scoped_preflight.spec.ts",
    "test/e2e/pr_eb_instrument_sizing.spec.ts",
    "test/e2e/pr_ea_execution_finality.spec.ts",
):
    browser = source(relative)
    if "test(" not in browser or "test.skip" in browser or "test.fixme" in browser:
        fail(f"browser evidence is missing or skippable: {relative}")

repo_gate = source("scripts/verify_repo_gates.sh")
if repo_gate.count("check_pr_ak_completion.sh") != 2:
    fail("repo gate must execute PR-AK exactly once in docs and all scopes")

print(f"OK PR-AK static contract ({len(evidence_contract)} evidence types)")
PY

if [[ "${PR_AK_SKIP_UPSTREAM:-0}" != "1" ]]; then
  PR_DV_SKIP_TESTS=1 PR_DV_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_dv_completion.sh"
  PR_CG_SKIP_TESTS=1 PR_CG_SKIP_UPSTREAM=1 PR_CG_SKIP_BROWSER_LIST=1 bash "$ROOT/scripts/check_pr_cg_completion.sh"
  PR_EB_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_eb_completion.sh"
  PR_DD_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dd_completion.sh"
  PR_EA_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_ea_completion.sh"
  PR_BW_SKIP_TESTS=1 PR_BW_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_bw_completion.sh"
  bash "$ROOT/scripts/check_product_audit_progress.sh"
  bash "$ROOT/scripts/check_product_audit_evidence.sh"
  bash "$ROOT/scripts/check_product_audit_evidence_index.sh"
  bash "$ROOT/scripts/check_product_audit_coverage.sh"
fi

if [[ "${PR_AK_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test \
    -p api --bin crypto-arb-api pr_ak_ --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo check -p api
fi

if [[ "${PR_AK_SKIP_BROWSER:-0}" != "1" ]]; then
  CI=1 npm run test:e2e:pr-dv -- --workers=1
  CI=1 npm run test:e2e:pr-eb -- --workers=1
  CI=1 npm run test:e2e:pr-ea -- --workers=1
fi

printf 'PR-AK completion gate passed\n'
