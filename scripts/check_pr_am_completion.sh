#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
AUDIT="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
EVIDENCE="$ROOT/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
ADAPTER="$ROOT/crates/exchange/src/adapter.rs"
WS_TESTS="$ROOT/crates/api/src/lifecycle/market_data_tests.rs"
REPO_GATE="$ROOT/scripts/verify_repo_gates.sh"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-AM completion gate failed: %s\n' "$1" >&2
  exit 1
}

run_static_gate() {
  PR_AM_SKIP_TESTS=1 PR_AM_SKIP_UPSTREAM=1 PR_AM_SKIP_BROWSER=1 bash "$0" >/dev/null
}

expect_rejected() {
  local message="$1"
  if run_static_gate 2>/dev/null; then
    fail "$message"
  fi
}

if [[ "$MODE" == "--self-test" ]]; then
  tmp="$(mktemp -d "${TMPDIR:-/tmp}/crossline-pr-am.XXXXXX")"
  files=("$AUDIT" "$EVIDENCE" "$ADAPTER" "$WS_TESTS" "$REPO_GATE")
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
old = "| `PR-AM MarketData Evidence Envelope & Cache Semantics` | ✅ 完成 |"
if text.count(old) != 1:
    raise SystemExit("PR-AM self-test setup failed: roadmap row drifted")
path.write_text(text.replace(old, "| `PR-AM MarketData Evidence Envelope & Cache Semantics` | 🟡 部分完成 |", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted a downgraded roadmap row"
  restore_file "$AUDIT"

  python3 - "$EVIDENCE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
rows = path.read_text(encoding="utf-8").splitlines()
for index, row in enumerate(rows):
    if row.startswith("PR-AM\tpublic-ws-outcome-contract\t"):
        del rows[index]
        break
else:
    raise SystemExit("PR-AM self-test setup failed: evidence row missing")
path.write_text("\n".join(rows) + "\n", encoding="utf-8")
PY
  expect_rejected "self-test accepted incomplete evidence"
  restore_file "$EVIDENCE"

  python3 - "$ADAPTER" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = "Self::Ready(_) => PublicWsSubscribeOutcome::Requested"
if text.count(old) != 1:
    raise SystemExit("PR-AM self-test setup failed: empty-ready guard drifted")
path.write_text(text.replace(old, "Self::Ready(_) => PublicWsSubscribeOutcome::Confirmed", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted an empty WS frame as subscribe success"
  restore_file "$ADAPTER"

  python3 - "$WS_TESTS" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = "#[tokio::test]\nasync fn empty_ws_ready_uses_rest_fallback_without_false_ingest_success()"
if text.count(old) != 1:
    raise SystemExit("PR-AM self-test setup failed: empty-ready fixture drifted")
path.write_text(text.replace(old, "#[tokio::test]\n#[ignore]\nasync fn empty_ws_ready_uses_rest_fallback_without_false_ingest_success()", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted a skipped empty-ready fixture"
  restore_file "$WS_TESTS"

  python3 - "$AUDIT" <<'PY'
from pathlib import Path
import re
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
head = re.search(r"(?m)^1\. \*\*(PR-[A-Z]+)\b", text)
if head is None:
    raise SystemExit("PR-AM self-test setup failed: successor queue head drifted")
updated = text[:head.start(1)] + "PR-AM" + text[head.end(1):]
path.write_text(updated, encoding="utf-8")
PY
  expect_rejected "self-test accepted PR-AM reinserted into the local queue"
  restore_file "$AUDIT"

  python3 - "$REPO_GATE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = 'PR_AM_SKIP_TESTS=1 PR_AM_SKIP_UPSTREAM=1 PR_AM_SKIP_BROWSER=1 bash "$ROOT/scripts/check_pr_am_completion.sh"'
if text.count(old) != 2:
    raise SystemExit("PR-AM self-test setup failed: repo wiring drifted")
path.write_text(text.replace(old, "true # PR-AM gate removed", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted single-scope repository wiring"

  printf 'PR-AM completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from pathlib import Path
import csv
import re
import sys

root = Path(sys.argv[1])
audit = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(encoding="utf-8")

def fail(message: str) -> None:
    raise SystemExit(f"PR-AM completion gate failed: {message}")

row = next((line for line in audit.splitlines() if line.startswith("| `PR-AM MarketData Evidence Envelope & Cache Semantics` |")), None)
if row is None or "| ✅ 完成 |" not in row or "剩余：无。" not in row:
    fail("roadmap row is not complete with no local remainder")
if "`bash scripts/check_pr_am_completion.sh --self-test`" not in row:
    fail("roadmap row lacks the canonical verification anchor")
if "> 本轮 PR-AM：" not in audit:
    fail("top progress summary lacks the PR-AM closure note")

queue = audit.split("### 🟡 6.5 下一步执行队列", 1)[-1]
numbered = re.findall(r"(?m)^\d+\. \*\*(PR-[A-Z]+)", queue)
if not numbered or "PR-AM" in numbered:
    fail(f"queue handoff drifted: {numbered[:5]}")
if "## 2026-07-17 PR-AM Market Data Evidence and Cache Semantics Closure" not in history:
    fail("history closure appendix is missing")

evidence_contract = {
    "public-ws-outcome-contract": "crates/exchange/src/adapter.rs",
    "public-ws-ingest-runtime": "crates/api/src/lifecycle/market_data/ws_touch/outcome.rs",
    "public-ws-empty-ready-fallback": "crates/api/src/lifecycle/market_data_tests.rs",
    "funding-row-evidence-envelope": "crates/api/src/services/market_data/envelope/tests/funding.rs",
    "spot-row-evidence-envelope": "crates/api/src/services/market_data/envelope/tests/funding.rs",
    "spot-row-evidence-producer": "crates/api/src/services/spot.rs",
    "fee-evidence-registry": "crates/arbitrage/src/algorithms/fee_evidence.rs",
    "fee-evidence-runtime-health": "crates/api/src/services/market_data/cache/helpers2.rs",
    "opportunity-fee-consumer": "crates/api/src/services/opportunity/row.rs",
    "execution-fee-consumer": "crates/api/src/services/hedge_ticket/cost.rs",
    "opportunity-market-evidence": "crates/arbitrage/src/algorithms/leg_market_evidence.rs",
    "execution-http-problem-consumer": "frontend/src/panels/modules/market_evidence.rs",
    "http-problem-browser": "test/e2e/pr_ev_exchange_problem.spec.ts",
    "cache-semantics-successor-gate": "scripts/check_pr_eu_completion.sh",
    "public-ws-successor-gate": "scripts/check_pr_ai_completion.sh",
    "fee-successor-gate": "scripts/check_pr_bv_completion.sh",
    "http-successor-gate": "scripts/check_pr_ev_completion.sh",
    "completion-governance": "scripts/check_pr_am_completion.sh",
}
with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-AM"]
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
for artifact in set(evidence_contract.values()):
    item = coverage.get(artifact)
    if item is None or item["coverage_status"] != "exact" or not item["evidence"].startswith("line:"):
        fail(f"exact coverage missing for {artifact}")

markers = {
    "crates/exchange/src/adapter.rs": (
        "pub enum PublicWsIngestOutcome",
        "pub fn ingest_outcome",
        "Self::Ready(_) => PublicWsSubscribeOutcome::Requested",
        "Self::Ready(_) | Self::Pending => PublicWsIngestOutcome::AwaitingFirstEvent",
        "public_ws_snapshot_exposes_explicit_subscribe_outcome",
    ),
    "crates/api/src/lifecycle/market_data/ws_touch.rs": (
        "let ingest_outcome = snapshot.ingest_outcome();",
        "PublicWsSnapshot::Ready(_) | PublicWsSnapshot::Pending",
        "ticker_fallback(runtime, venue, adapter, symbols).await",
        "funding_fallback(runtime, venue, adapter, symbols).await",
    ),
    "crates/api/src/lifecycle/market_data/ws_touch/outcome.rs": (
        "PublicWsIngestOutcome::Ingested => runtime.market_data.record_runtime_success(",
        "PublicWsIngestOutcome::AwaitingFirstEvent => runtime.market_data.record_runtime_pending(",
        "PublicWsIngestOutcome::Unsupported => runtime.market_data.record_runtime_unsupported(",
    ),
    "crates/api/src/services/market_data/envelope/tests/funding.rs": (
        "funding_rates_envelope_carries_row_evidence",
        "spot_ticks_envelope_carries_row_evidence",
    ),
    "crates/arbitrage/src/algorithms/fee_evidence.rs": (
        "STANDARD_FEE_VENUE_FAMILIES: [&str; 8]",
        "STANDARD_FEE_PRODUCTS: [FeeProduct; 2]",
    ),
    "crates/api/src/services/opportunity/row.rs": (
        "fee_evidence_complete",
        "fee_evidence_ids",
    ),
    "frontend/src/panels/modules/market_evidence.rs": (
        'push_problem_detail(&mut context, details, "operation")',
        'push_problem_detail(&mut context, details, "symbol")',
        'push_problem_detail(&mut context, details, "path")',
        "HTTP耗时 {latency_ms}ms",
    ),
}
for path, required in markers.items():
    source = (root / path).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"missing {path} marker: {marker}")

ws_tests = (root / "crates/api/src/lifecycle/market_data_tests.rs").read_text(encoding="utf-8")
if "#[ignore]\nasync fn empty_ws_ready_uses_rest_fallback_without_false_ingest_success" in ws_tests:
    fail("empty-ready runtime fixture is skipped")
repo_gate = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
wiring = 'PR_AM_SKIP_TESTS=1 PR_AM_SKIP_UPSTREAM=1 PR_AM_SKIP_BROWSER=1 bash "$ROOT/scripts/check_pr_am_completion.sh"'
if repo_gate.count(wiring) != 2:
    fail("completion gate must be wired into both repository scopes")

print(f"OK PR-AM contract ({len(evidence_contract)} evidence types; queue={numbered[0]})")
PY

bash "$ROOT/scripts/check_product_audit_progress.sh"
bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_evidence_index.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"

if [[ "${PR_AM_SKIP_UPSTREAM:-0}" != "1" ]]; then
  PR_AI_SKIP_TESTS=1 PR_AI_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_ai_completion.sh"
  PR_EU_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_eu_completion.sh"
  PR_BV_SKIP_TESTS=1 PR_BV_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_bv_completion.sh"
  PR_EV_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_ev_completion.sh"
fi

if [[ "${PR_AM_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p exchange public_ws_snapshot_exposes_explicit_subscribe_outcome --lib --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api ws_touch_stores_ticker_and_funding_rows_in_market_cache --bin crypto-arb-api --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api empty_ws_ready_uses_rest_fallback_without_false_ingest_success --bin crypto-arb-api --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api envelope_carries_row_evidence --bin crypto-arb-api --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p arbitrage standard_fee_registry --lib --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api fee_schedule_evidence_surfaces_per_venue_snapshot_status_rows --bin crypto-arb-api --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api list_row --bin crypto-arb-api --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api services::hedge_ticket::tests::cost_evidence --bin crypto-arb-api --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p arbitrage leg_market_evidence --lib --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" market_evidence --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" cost_copy --no-fail-fast
fi

if [[ "${PR_AM_SKIP_BROWSER:-0}" != "1" ]]; then
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-ev
fi

printf 'OK PR-AM market evidence envelope and cache semantics contract\n'
