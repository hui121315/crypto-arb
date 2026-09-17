#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
AUDIT="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
HISTORY="$ROOT/docs/audit_history/PRODUCT_AUDIT_HISTORY.md"
EVIDENCE="$ROOT/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
TYPES="$ROOT/crates/api/src/trading_service/private_ws_events/types.rs"
MAPPER_DIRTY="$ROOT/crates/api/src/trading_service/private_ws_mapper/account_dirty.rs"
MAPPER_MATRIX="$ROOT/crates/api/src/trading_service/private_ws_mapper/tests/account_dirty_matrix.rs"
SERVICE_CACHE="$ROOT/crates/api/src/trading_service/cache.rs"
POSITION_CACHE="$ROOT/crates/api/src/trading_service/venue_position_cache.rs"
BALANCE_CACHE="$ROOT/crates/api/src/trading_service/venue_balance_cache.rs"
APPLY="$ROOT/crates/api/src/trading_service/private_ws_events/apply.rs"
APPLY_BINANCE="$ROOT/crates/api/src/trading_service/private_ws_events/apply/binance.rs"
FUNDING="$ROOT/crates/api/src/trading_service/private_ws_events/funding.rs"
HEALTH_COUNTS="$ROOT/crates/api/src/services/private_ws_health/counts.rs"
PROJECTION="$ROOT/crates/api/src/services/venue_operation_health/snapshot/part_05.rs"
HEALTH_TEST="$ROOT/crates/api/src/services/private_ws_health/tests/account_dirty.rs"
CACHE_TEST="$ROOT/crates/api/src/trading_service/private_ws_events/tests/cache_patch.rs"
BROWSER="$ROOT/test/e2e/pr_ar_private_ws_health.spec.ts"
PACKAGE="$ROOT/package.json"
RELEASE_QA="$ROOT/scripts/check_release_qa_contract.sh"
RELEASE_FIXTURE="$ROOT/scripts/fixtures/release_qa_contract/package.json"
REPO_GATE="$ROOT/scripts/verify_repo_gates.sh"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-AR completion gate failed: %s\n' "$1" >&2
  exit 1
}

run_static_gate() {
  PR_AR_STATIC_ONLY=1 PR_AR_SKIP_TESTS=1 PR_AR_SKIP_UPSTREAM=1 PR_AR_SKIP_BROWSER=1 bash "$0" >/dev/null
}

expect_rejected() {
  local message="$1"
  if run_static_gate 2>/dev/null; then
    fail "$message"
  fi
}

if [[ "$MODE" == "--self-test" ]]; then
  tmp="$(mktemp -d "${TMPDIR:-/tmp}/crossline-pr-ar.XXXXXX")"
  files=(
    "$AUDIT" "$EVIDENCE" "$TYPES" "$MAPPER_MATRIX" "$SERVICE_CACHE"
    "$POSITION_CACHE" "$HEALTH_COUNTS" "$PROJECTION" "$BROWSER"
    "$RELEASE_QA" "$REPO_GATE"
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
old = "| `PR-AR Private WS Runtime Health & Event Evidence` | ✅ 完成 |"
if text.count(old) != 1:
    raise SystemExit("PR-AR self-test setup failed: roadmap row drifted")
path.write_text(text.replace(old, "| `PR-AR Private WS Runtime Health & Event Evidence` | 🟡 部分完成 |", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted a downgraded roadmap row"
  restore_file "$AUDIT"

  python3 - "$EVIDENCE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
rows = path.read_text(encoding="utf-8").splitlines()
for index, row in enumerate(rows):
    if row.startswith("PR-AR\ttyped-dirty-contract\t"):
        del rows[index]
        break
else:
    raise SystemExit("PR-AR self-test setup failed: evidence row missing")
path.write_text("\n".join(rows) + "\n", encoding="utf-8")
PY
  expect_rejected "self-test accepted incomplete exact evidence"
  restore_file "$EVIDENCE"

  python3 - "$TYPES" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = "    AccountDirty(PrivateAccountDirty),\n"
if text.count(old) != 1:
    raise SystemExit("PR-AR self-test setup failed: typed event drifted")
path.write_text(text.replace(old, "    AccountDirty,\n", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted a unit AccountDirty event"
  restore_file "$TYPES"

  python3 - "$SERVICE_CACHE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = "            self.position_cache.invalidate(&venue);\n"
if text.count(old) != 1:
    raise SystemExit("PR-AR self-test setup failed: scoped invalidation drifted")
path.write_text(text.replace(old, "            self.clear_account_cache();\n", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted global cache clearing for a scoped dirty event"
  restore_file "$SERVICE_CACHE"

  python3 - "$POSITION_CACHE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = "            entry.invalidated = true;\n"
if text.count(old) != 1:
    raise SystemExit("PR-AR self-test setup failed: stale marker drifted")
path.write_text(text.replace(old, "            entry.invalidated = false;\n", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted an ineffective stale marker"
  restore_file "$POSITION_CACHE"

  python3 - "$HEALTH_COUNTS" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = "                .with_account_dirty(dirty.clone()),\n"
if text.count(old) != 1:
    raise SystemExit("PR-AR self-test setup failed: health context drifted")
path.write_text(text.replace(old, "                .with_rows(0),\n", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted health that dropped typed dirty context"
  restore_file "$HEALTH_COUNTS"

  python3 - "$MAPPER_MATRIX" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = "#[test]\nfn eight_venue_account_dirty_matrix_preserves_scope_and_reason()"
if text.count(old) != 1:
    raise SystemExit("PR-AR self-test setup failed: mapper matrix drifted")
path.write_text(text.replace(old, "#[test]\n#[ignore]\nfn eight_venue_account_dirty_matrix_preserves_scope_and_reason()", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted an ignored eight-venue mapper matrix"
  restore_file "$MAPPER_MATRIX"

  python3 - "$BROWSER" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = 'test("PR-AR surfaces scoped AccountDirty and bounded refetch evidence in Settings"'
if text.count(old) != 1:
    raise SystemExit("PR-AR self-test setup failed: browser marker drifted")
path.write_text(text.replace(old, 'test.skip("PR-AR surfaces scoped AccountDirty and bounded refetch evidence in Settings"', 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted a skipped product fixture"
  restore_file "$BROWSER"

  python3 - "$AUDIT" <<'PY'
from pathlib import Path
import re
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
head = re.search(r"(?m)^1\. \*\*(PR-[A-Z]+)\b", text)
if head is None:
    raise SystemExit("PR-AR self-test setup failed: queue head drifted")
path.write_text(text[:head.start(1)] + "PR-AR" + text[head.end(1):], encoding="utf-8")
PY
  expect_rejected "self-test accepted PR-AR reinserted into the local queue"
  restore_file "$AUDIT"

  python3 - "$RELEASE_QA" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = " test/e2e/pr_ar_private_ws_health.spec.ts"
if text.count(old) != 1:
    raise SystemExit("PR-AR self-test setup failed: release QA wiring drifted")
path.write_text(text.replace(old, "", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted release QA without the PR-AR browser"
  restore_file "$RELEASE_QA"

  python3 - "$REPO_GATE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = 'PR_AR_SKIP_TESTS=1 PR_AR_SKIP_UPSTREAM=1 PR_AR_SKIP_BROWSER=1 bash "$ROOT/scripts/check_pr_ar_completion.sh"'
if text.count(old) != 2:
    raise SystemExit("PR-AR self-test setup failed: repository wiring drifted")
path.write_text(text.replace(old, "true # PR-AR gate removed", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted single-scope repository wiring"

  printf 'PR-AR completion self-test passed\n'
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
    raise SystemExit(f"PR-AR completion gate failed: {message}")


title = "PR-AR Private WS Runtime Health & Event Evidence"
row = next((line for line in audit.splitlines() if line.startswith(f"| `{title}` |")), None)
if row is None or "| ✅ 完成 |" not in row or "剩余：无。" not in row:
    fail("roadmap row is not complete with no local remainder")
if "`bash scripts/check_pr_ar_completion.sh --self-test`" not in row:
    fail("roadmap row lacks the canonical destructive verification anchor")
if "> 本轮 PR-AR：" not in audit:
    fail("top progress summary lacks the PR-AR closure note")

queue = audit.split("### 🟡 6.5 下一步执行队列", 1)[-1]
numbered = re.findall(r"(?m)^\d+\. \*\*(PR-[A-Z]+)", queue)
successor_order = ("PR-AW", "PR-AX", "PR-AY", "PR-AZ", "PR-BB", "PR-BC")
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
    or any(pr_id in numbered for pr_id in ("PR-AR", "PR-AS", "PR-AU", "PR-AV"))
):
    fail(f"queue handoff drifted: expected={incomplete_successors}, actual={numbered[:len(incomplete_successors)]}")
if "## 2026-07-18 PR-AR Private WS Runtime and Scoped Refetch Closure" not in history:
    fail("history closure appendix is missing")

evidence_contract = {
    "successor-private-runtime": "scripts/check_pr_ds_completion.sh",
    "successor-operation-health": "scripts/check_pr_eg_completion.sh",
    "successor-account-state": "scripts/check_pr_di_completion.sh",
    "successor-execution-finality": "scripts/check_pr_ea_completion.sh",
    "successor-durable-ledger": "scripts/check_pr_fz_completion.sh",
    "successor-hyperliquid-ledger": "scripts/check_pr_da_completion.sh",
    "venue-okx-private-ws": "scripts/check_pr_ek_completion.sh",
    "venue-binance-private-ws": "scripts/check_pr_el_completion.sh",
    "venue-bybit-private-ws": "scripts/check_pr_em_completion.sh",
    "venue-bitget-private-ws": "scripts/check_pr_en_completion.sh",
    "venue-kucoin-private-ws": "scripts/check_pr_eo_completion.sh",
    "venue-htx-private-ws": "scripts/check_pr_ep_completion.sh",
    "venue-hyperliquid-private-ws": "scripts/check_pr_eq_completion.sh",
    "venue-gate-private-ws": "scripts/check_pr_er_completion.sh",
    "typed-dirty-contract": "crates/api/src/trading_service/private_ws_events/types.rs",
    "dirty-mapper-boundary": "crates/api/src/trading_service/private_ws_mapper/account_dirty.rs",
    "eight-venue-dirty-matrix": "crates/api/src/trading_service/private_ws_mapper/tests/account_dirty_matrix.rs",
    "scoped-dirty-dispatch": "crates/api/src/trading_service/cache.rs",
    "scoped-position-stale": "crates/api/src/trading_service/venue_position_cache.rs",
    "scoped-balance-stale": "crates/api/src/trading_service/venue_balance_cache.rs",
    "private-ws-event-apply": "crates/api/src/trading_service/private_ws_events/apply.rs",
    "private-ws-health": "crates/api/src/services/private_ws_health/counts.rs",
    "operation-health-projection": "crates/api/src/services/venue_operation_health/snapshot/part_05.rs",
    "health-recovery-fixture": "crates/api/src/services/private_ws_health/tests/account_dirty.rs",
    "scoped-cache-fixture": "crates/api/src/trading_service/private_ws_events/tests/cache_patch.rs",
    "product-browser": "test/e2e/pr_ar_private_ws_health.spec.ts",
    "package-product-suite": "package.json",
    "release-qa-product-suite": "scripts/check_release_qa_contract.sh",
    "completion-governance": "scripts/check_pr_ar_completion.sh",
}
with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-AR"]
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
    if artifact == "package.json":
        continue
    item = coverage.get(artifact)
    if item is None or item["coverage_status"] != "exact" or not item["evidence"].startswith("line:"):
        fail(f"exact coverage missing for {artifact}")

types = (root / "crates/api/src/trading_service/private_ws_events/types.rs").read_text(encoding="utf-8")
for marker in (
    "AccountDirty(PrivateAccountDirty)",
    "pub(crate) enum PrivateAccountScope",
    "pub(crate) venue: String",
    "pub(crate) scope: PrivateAccountScope",
    "pub(crate) reason: String",
    "account_cache_dirty: Option<PrivateAccountDirty>",
):
    if marker not in types:
        fail(f"typed dirty contract missing: {marker}")
if re.search(r"\bAccountDirty\s*,", types):
    fail("unit AccountDirty event regressed")

service_cache = (root / "crates/api/src/trading_service/cache.rs").read_text(encoding="utf-8")
for marker in (
    "fn mark_private_account_dirty",
    "self.position_cache.invalidate(&venue)",
    "self.balance_cache.invalidate(&venue)",
    "self.account_summaries.remove(&venue)",
):
    if marker not in service_cache:
        fail(f"scoped cache dispatch missing: {marker}")

apply_markers = {
    "crates/api/src/trading_service/private_ws_events/apply.rs": "account_cache_dirty: Some(dirty)",
    "crates/api/src/trading_service/private_ws_events/apply/binance.rs": "account_cache_dirty,",
    "crates/api/src/trading_service/private_ws_events/funding.rs": "account_cache_dirty: Some(dirty)",
}
for path, dirty_marker in apply_markers.items():
    source = (root / path).read_text(encoding="utf-8")
    if "clear_account_cache()" in source:
        fail(f"private WS apply still clears every venue cache: {path}")
    if dirty_marker not in source:
        fail(f"private WS apply lacks typed dirty outcome: {path}")

for path in (
    "crates/api/src/trading_service/venue_position_cache.rs",
    "crates/api/src/trading_service/venue_balance_cache.rs",
):
    source = (root / path).read_text(encoding="utf-8")
    for marker in ("invalidated: bool", "entry.invalidated = true", "allow_invalidated"):
        if marker not in source:
            fail(f"bounded stale cache marker missing in {path}: {marker}")

matrix = (root / "crates/api/src/trading_service/private_ws_mapper/tests/account_dirty_matrix.rs").read_text(encoding="utf-8")
if re.search(r"#\[(?:ignore|should_panic)", matrix):
    fail("eight-venue dirty matrix must not be ignored or panic-expected")
for marker in (
    "eight_venue_account_dirty_matrix_preserves_scope_and_reason",
    "map_binance_event",
    "map_okx_event",
    "map_bybit_event",
    "map_bitget_event",
    "map_gate_event",
    "map_htx_event",
    "map_kucoin_event",
    "map_hyperliquid_event",
):
    if marker not in matrix:
        fail(f"eight-venue dirty matrix missing: {marker}")

health = (root / "crates/api/src/services/private_ws_health/counts.rs").read_text(encoding="utf-8")
for marker in ("record_account_dirty", ".with_requested(1)", ".with_rows(0)", ".with_account_dirty(dirty.clone())"):
    if marker not in health:
        fail(f"private WS health dirty marker missing: {marker}")
projection = (root / "crates/api/src/services/venue_operation_health/snapshot/part_05.rs").read_text(encoding="utf-8")
for marker in ('"accountDirty"', '"bounded_rest_on_next_read"', "account_dirty_venue=", "account_dirty_scope="):
    if marker not in projection:
        fail(f"operation health dirty evidence missing: {marker}")

for path, marker in (
    ("crates/api/src/services/private_ws_health/tests/account_dirty.rs", "account_dirty_is_warn_with_scoped_refetch_context"),
    ("crates/api/src/trading_service/private_ws_events/tests/cache_patch.rs", "account_dirty_marks_only_scoped_venue_stale"),
    ("crates/api/src/services/venue_operation_health/snapshot/tests/part_17.rs", "private_ws_account_dirty_projects_typed_refetch_problem_and_evidence"),
):
    source = (root / path).read_text(encoding="utf-8")
    if marker not in source or re.search(r"#\[(?:ignore|should_panic)", source):
        fail(f"non-skipping PR-AR fixture missing: {path}")

browser_path = "test/e2e/pr_ar_private_ws_health.spec.ts"
browser = (root / browser_path).read_text(encoding="utf-8")
if re.search(r"\btest\.(?:skip|fixme)\s*\(", browser):
    fail("PR-AR product fixture must not be skipped")
for marker in ("PR-AR surfaces scoped AccountDirty", "account_dirty_scope=positions", "bounded_rest_on_next_read"):
    if marker not in browser:
        fail(f"PR-AR browser fixture missing: {marker}")

package = (root / "package.json").read_text(encoding="utf-8")
if '"test:e2e:pr-ar": "playwright test test/e2e/pr_ar_private_ws_health.spec.ts"' not in package:
    fail("package PR-AR command is missing")
if package.count(browser_path) != 2:
    fail("PR-AR browser must be wired once as a focused command and once in the product suite")
for path in ("scripts/check_release_qa_contract.sh", "scripts/fixtures/release_qa_contract/package.json"):
    if (root / path).read_text(encoding="utf-8").count(browser_path) != 1:
        fail(f"release QA command lock drifted: {path}")

repo_gate = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
wiring = 'PR_AR_SKIP_TESTS=1 PR_AR_SKIP_UPSTREAM=1 PR_AR_SKIP_BROWSER=1 bash "$ROOT/scripts/check_pr_ar_completion.sh"'
if repo_gate.count(wiring) != 2:
    fail("completion gate must be wired into both repository scopes")
if repo_gate.count(browser_path) != 3:
    fail("repository gate must lock the focused command, product suite and browser list")

print(f"OK PR-AR contract ({len(evidence_contract)} evidence types; queue={numbered[0]})")
PY

if [[ "${PR_AR_STATIC_ONLY:-0}" == "1" ]]; then
  exit 0
fi

bash "$ROOT/scripts/check_product_audit_progress.sh"
bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_evidence_index.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"
bash "$ROOT/scripts/check_release_qa_contract.sh"

if [[ "${PR_AR_SKIP_UPSTREAM:-0}" != "1" ]]; then
  PR_DS_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_ds_completion.sh"
  PR_EG_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_eg_completion.sh"
  PR_EA_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_ea_completion.sh"
  PR_DI_SKIP_TESTS=1 PR_DI_SKIP_UPSTREAM=1 PR_DI_SKIP_BROWSER_LIST=1 bash "$ROOT/scripts/check_pr_di_completion.sh"
  PR_DA_SKIP_TESTS=1 PR_DA_SKIP_UPSTREAM=1 PR_DA_SKIP_BROWSER_LIST=1 bash "$ROOT/scripts/check_pr_da_completion.sh"
fi

if [[ "${PR_AR_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api --bin crypto-arb-api account_dirty --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api --bin crypto-arb-api invalidate_marks_only_target_venue_stale_and_retains_bounded_rows --no-fail-fast
fi

if [[ "${PR_AR_SKIP_BROWSER:-0}" != "1" ]]; then
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-ar
fi

printf 'OK PR-AR private WS runtime health and scoped refetch contract\n'
