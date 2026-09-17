#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
AUDIT="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
HISTORY="$ROOT/docs/audit_history/PRODUCT_AUDIT_HISTORY.md"
EVIDENCE="$ROOT/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
ALLOWLIST="$ROOT/scripts/module_size_debt_allowlist.tsv"
MODULE_GATE="$ROOT/scripts/check_module_size.sh"
MODULE_SELF_TEST="$ROOT/scripts/check_module_size_self_test.sh"
WASM_GATE="$ROOT/scripts/check_wasm_budget.sh"
WASM_CONTRACT="$ROOT/scripts/check_wasm_budget_contract.sh"
TRANSPORT_FACADE="$ROOT/crates/api/src/lifecycle/private_ws/transport.rs"
TRANSPORT_CONNECTION="$ROOT/crates/api/src/lifecycle/private_ws/transport/connection.rs"
REPO_GATE="$ROOT/scripts/verify_repo_gates.sh"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-AX completion gate failed: %s\n' "$1" >&2
  exit 1
}

run_static_gate() {
  PR_AX_STATIC_ONLY=1 bash "$0" >/dev/null
}

if [[ "$MODE" == "--self-test" ]]; then
  tmp="$(mktemp -d "${TMPDIR:-/tmp}/crossline-pr-ax.XXXXXX")"
  files=(
    "$AUDIT"
    "$EVIDENCE"
    "$MODULE_GATE"
    "$MODULE_SELF_TEST"
    "$WASM_GATE"
    "$WASM_CONTRACT"
    "$TRANSPORT_FACADE"
    "$TRANSPORT_CONNECTION"
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

  mutate_once() {
    local file="$1"
    local old="$2"
    local new="$3"
    python3 - "$file" "$old" "$new" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
old, new = sys.argv[2:]
text = path.read_text(encoding="utf-8")
if old not in text:
    raise SystemExit(f"PR-AX self-test setup failed: marker missing in {path}")
path.write_text(text.replace(old, new, 1), encoding="utf-8")
PY
  }

  assert_rejected() {
    local file="$1"
    local old="$2"
    local new="$3"
    local message="$4"
    mutate_once "$file" "$old" "$new"
    if run_static_gate 2>/dev/null; then
      fail "$message"
    fi
    restore_file "$file"
  }

  run_static_gate
  assert_rejected "$AUDIT" \
    '| `PR-AX Frontend Wasm & Module Debt Burn-down` | ✅ 完成 |' \
    '| `PR-AX Frontend Wasm & Module Debt Burn-down` | 🟡 部分完成 |' \
    'self-test accepted a downgraded roadmap row'
  assert_rejected "$EVIDENCE" \
    $'PR-AX\tmodule-size-budget\t' \
    $'PR-AX-MISSING\tmodule-size-budget\t' \
    'self-test accepted incomplete exact evidence'
  assert_rejected "$MODULE_GATE" \
    'REPORT="${MODULE_SIZE_REPORT:-$ROOT/target/reports/module-size.json}"' \
    'REPORT="$ROOT/target/reports/module-size-disabled.json"' \
    'self-test accepted a detached module-size JSON report'
  assert_rejected "$MODULE_SELF_TEST" \
    'report["generatedBy"] == "scripts/check_module_size.sh"' \
    'report["generatedBy"] == "disabled"' \
    'self-test accepted an unverified module-size report schema'
  assert_rejected "$WASM_GATE" \
    'REPORT="${WASM_SIZE_REPORT:-$ROOT/target/reports/wasm-size.json}"' \
    'REPORT="$ROOT/target/reports/wasm-size-disabled.json"' \
    'self-test accepted a detached Wasm JSON report'
  assert_rejected "$WASM_GATE" \
    'RAW_BUDGET_BYTES="${WASM_RAW_BUDGET_BYTES:-5100000}"' \
    'RAW_BUDGET_BYTES="${WASM_RAW_BUDGET_BYTES:-9999999}"' \
    'self-test accepted a relaxed default Wasm regression ceiling'
  assert_rejected "$WASM_CONTRACT" \
    'report["generatedBy"] == "scripts/check_wasm_budget.sh"' \
    'report["generatedBy"] == "disabled"' \
    'self-test accepted an unverified Wasm report schema'
  assert_rejected "$TRANSPORT_FACADE" \
    'mod connection;' \
    '// connection split disabled' \
    'self-test accepted a detached private WS transport split'
  current_lines="$(wc -l <"$TRANSPORT_CONNECTION" | tr -d ' ')"
  for index in $(seq 1 "$((301 - current_lines))"); do
    printf '// PR-AX oversize fixture %s\n' "$index" >>"$TRANSPORT_CONNECTION"
  done
  if run_static_gate 2>/dev/null; then
    fail 'self-test accepted a split module above 300 lines'
  fi
  restore_file "$TRANSPORT_CONNECTION"
  assert_rejected "$AUDIT" \
    '### 🟡 6.5 下一步执行队列' \
    $'### 🟡 6.5 下一步执行队列\n1. **PR-AX Frontend Wasm & Module Debt Burn-down** — stale queue fixture.' \
    'self-test accepted PR-AX reinserted into the local queue'
  assert_rejected "$REPO_GATE" \
    'PR_AX_STATIC_ONLY=1 bash "$ROOT/scripts/check_pr_ax_completion.sh"' \
    'true # PR-AX gate removed' \
    'self-test accepted single-scope repository wiring'

  printf 'PR-AX completion self-test passed\n'
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
    raise SystemExit(f"PR-AX completion gate failed: {message}")


title = "PR-AX Frontend Wasm & Module Debt Burn-down"
row = next((line for line in audit.splitlines() if line.startswith(f"| `{title}` |")), None)
if row is None or "| ✅ 完成 |" not in row or "剩余：无。" not in row:
    fail("roadmap row is not complete with no local remainder")
if "`bash scripts/check_pr_ax_completion.sh --self-test`" not in row:
    fail("roadmap row lacks the canonical destructive verification anchor")
if "> 本轮 PR-AX：" not in audit:
    fail("top progress summary lacks the PR-AX closure note")

queue = audit.split("### 🟡 6.5 下一步执行队列", 1)[-1]
numbered = re.findall(r"(?m)^\d+\. \*\*(PR-[A-Z]+)", queue)
successor_order = ("PR-AY", "PR-AZ", "PR-BB", "PR-BC")
incomplete_successors = []
for pr_id in successor_order:
    successor_row = next((line for line in audit.splitlines() if line.startswith(f"| `{pr_id} ")), None)
    if successor_row is None:
        fail(f"successor roadmap row is missing: {pr_id}")
    if "| ✅ 完成 |" not in successor_row:
        incomplete_successors.append(pr_id)
if (
    numbered[: len(incomplete_successors)] != incomplete_successors
    or len(numbered) != len(set(numbered))
    or "PR-AX" in numbered
):
    fail(f"queue handoff drifted: expected={incomplete_successors}, actual={numbered[:len(incomplete_successors)]}")
if "## 2026-07-19 PR-AX Wasm and Module Debt Burn-down Closure" not in history:
    fail("history closure appendix is missing")

evidence_contract = {
    "module-size-budget": "scripts/check_module_size.sh",
    "module-size-report-contract": "scripts/check_module_size_self_test.sh",
    "module-debt-zero-ledger": "scripts/module_size_debt_allowlist.tsv",
    "wasm-three-stage-budget": "scripts/check_wasm_budget.sh",
    "wasm-report-contract": "scripts/check_wasm_budget_contract.sh",
    "credential-validation-split": "crates/api/src/services/venue_credentials/validation/venues/cex.rs",
    "private-ws-health-state-split": "crates/api/src/services/private_ws_health/state.rs",
    "private-ws-transport-split": "crates/api/src/lifecycle/private_ws/transport/connection.rs",
    "private-ws-mapper-fixture-split": "crates/api/src/trading_service/private_ws_mapper/tests/cases_htx.rs",
    "hedge-compile-fixture-split": "crates/api/src/routers/arbitrage/hedge_tests/compile_venue_capabilities.rs",
    "frontend-capability-label-split": "frontend/src/panels/modules/execution/components/params_panel/capability/labels.rs",
    "repository-aggregate": "scripts/verify_repo_gates.sh",
    "completion-governance": "scripts/check_pr_ax_completion.sh",
}
with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [item for item in csv.DictReader(handle, delimiter="\t") if item["pr_id"] == "PR-AX"]
indexed = {item["evidence_type"]: item for item in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for evidence_type, artifact in evidence_contract.items():
    evidence = indexed[evidence_type]
    if evidence["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"evidence artifact drifted or is missing: {evidence_type}")
    if not evidence["command"].strip() or not evidence["notes"].strip():
        fail(f"evidence lacks command or notes: {evidence_type}")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {item["file"]: item for item in csv.DictReader(handle, delimiter="\t")}
for artifact in set(evidence_contract.values()):
    item = coverage.get(artifact)
    if item is None or item["coverage_status"] != "exact" or not item["evidence"].startswith("line:"):
        fail(f"exact coverage missing for {artifact}")

allowlist_rows = [
    line for line in (root / "scripts/module_size_debt_allowlist.tsv").read_text(encoding="utf-8").splitlines()
    if line.strip() and not line.lstrip().startswith("#")
]
if allowlist_rows:
    fail(f"module-size debt ledger is not empty: {allowlist_rows}")

expected_files = (
    "crates/api/src/services/venue_credentials/validation/venues/cex.rs",
    "crates/api/src/services/venue_credentials/validation/venues/hyperliquid.rs",
    "crates/api/src/services/private_ws_health.rs",
    "crates/api/src/services/private_ws_health/state.rs",
    "crates/api/src/lifecycle/private_ws/transport.rs",
    "crates/api/src/lifecycle/private_ws/transport/connection.rs",
    "crates/api/src/lifecycle/private_ws/transport/protocol.rs",
    "crates/api/src/lifecycle/private_ws/transport/subscriptions.rs",
    "crates/api/src/trading_service/private_ws_mapper/tests/cases_binance_okx.rs",
    "crates/api/src/trading_service/private_ws_mapper/tests/cases_gate_account.rs",
    "crates/api/src/trading_service/private_ws_mapper/tests/cases_htx.rs",
    "crates/api/src/trading_service/private_ws_mapper/tests/cases_kucoin.rs",
    "crates/api/src/trading_service/private_ws_mapper/tests/cases_hyperliquid_clearinghouse.rs",
    "crates/api/src/routers/arbitrage/hedge_tests/compile_venue_capabilities.rs",
    "crates/api/src/routers/arbitrage/hedge_tests/compile_venue_guards.rs",
    "crates/api/src/routers/arbitrage/hedge_tests/compile_venue_market.rs",
    "frontend/src/panels/modules/execution/components/params_panel/capability.rs",
    "frontend/src/panels/modules/execution/components/params_panel/capability/labels.rs",
)
for relative in expected_files:
    path = root / relative
    if not path.is_file():
        fail(f"split artifact is missing: {relative}")
    lines = len(path.read_text(encoding="utf-8").splitlines())
    if lines > 300:
        fail(f"split artifact exceeds 300 lines: {relative}={lines}")
for relative in (
    "crates/api/src/trading_service/private_ws_mapper/tests/cases_1.rs",
    "crates/api/src/trading_service/private_ws_mapper/tests/cases_2.rs",
    "crates/api/src/routers/arbitrage/hedge_tests/compile_venue.rs",
):
    if (root / relative).exists():
        fail(f"legacy oversized artifact returned: {relative}")

transport_facade = (root / "crates/api/src/lifecycle/private_ws/transport.rs").read_text(
    encoding="utf-8"
)
for marker in (
    "mod connection;",
    "mod protocol;",
    "mod subscriptions;",
    "pub(super) use connection::{",
    "pub(super) use protocol::{",
    "pub(super) use subscriptions::send_private_ws_subscriptions;",
):
    if marker not in transport_facade:
        fail(f"private WS transport facade lost split wiring: {marker}")

module_gate = (root / "scripts/check_module_size.sh").read_text(encoding="utf-8")
for marker in (
    'REPORT="${MODULE_SIZE_REPORT:-$ROOT/target/reports/module-size.json}"',
    '"schemaVersion": 1',
    '"inheritedDebt": statuses["inherited_debt"]',
    'render_report',
):
    if marker not in module_gate:
        fail(f"module-size report contract lost marker: {marker}")
module_self_test = (root / "scripts/check_module_size_self_test.sh").read_text(encoding="utf-8")
for marker in (
    'report["generatedBy"] == "scripts/check_module_size.sh"',
    'report["gatePassed"] is True',
    'paths == sorted(set(paths))',
):
    if marker not in module_self_test:
        fail(f"module-size report self-test lost marker: {marker}")
wasm_gate = (root / "scripts/check_wasm_budget.sh").read_text(encoding="utf-8")
for marker in (
    'REPORT="${WASM_SIZE_REPORT:-$ROOT/target/reports/wasm-size.json}"',
    'RAW_BUDGET_BYTES="${WASM_RAW_BUDGET_BYTES:-5100000}"',
    'GZIP_BUDGET_BYTES="${WASM_GZIP_BUDGET_BYTES:-${WASM_BUDGET_BYTES:-1850000}}"',
    'OZ_BUDGET_BYTES="${WASM_OZ_BUDGET_BYTES:-5100000}"',
    'append_report_row "$rel" raw',
    'append_report_row "$rel" gzip',
    'append_report_row "$rel" wasm-opt-Oz',
):
    if marker not in wasm_gate:
        fail(f"Wasm report contract lost marker: {marker}")
wasm_contract = (root / "scripts/check_wasm_budget_contract.sh").read_text(encoding="utf-8")
if (
    'report["generatedBy"] == "scripts/check_wasm_budget.sh"' not in wasm_contract
    or 'report["summary"] == {' not in wasm_contract
    or '"wasm-opt-Oz"' not in wasm_contract
):
    fail("Wasm JSON report schema is not fixture-verified")

repo_gate = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
wiring = 'PR_AX_STATIC_ONLY=1 bash "$ROOT/scripts/check_pr_ax_completion.sh"'
if repo_gate.count(wiring) != 2:
    fail("completion gate must be wired into both repository scopes")

print(f"OK PR-AX size contract ({len(evidence_contract)} evidence types; queue={numbered[0]})")
PY

if [[ "${PR_AX_STATIC_ONLY:-0}" == "1" ]]; then
  exit 0
fi

bash "$ROOT/scripts/check_product_audit_progress.sh"
bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_evidence_index.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"
bash "$MODULE_GATE"
bash "$MODULE_SELF_TEST"
bash "$WASM_CONTRACT"

if [[ "${PR_AX_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS=8 cargo test -p api --bin crypto-arb-api private_ws_mapper --no-fail-fast
  CARGO_BUILD_JOBS=8 cargo test -p api --bin crypto-arb-api hedge_tests --no-fail-fast
  CARGO_BUILD_JOBS=8 cargo test -p api --bin crypto-arb-api private_ws_health --no-fail-fast
  CARGO_BUILD_JOBS=8 cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib capability --no-fail-fast
fi

if [[ "${PR_AX_SKIP_RELEASE:-0}" != "1" ]]; then
  (cd "$ROOT/frontend" && env -u NO_COLOR trunk build --release=true)
  WASM_REQUIRE_OZ=1 bash "$WASM_GATE"
fi

printf 'OK PR-AX Wasm and module debt burn-down contract\n'
