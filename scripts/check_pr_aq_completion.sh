#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
AUDIT="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
EVIDENCE="$ROOT/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
SHARED="$ROOT/shared-types/src/orders.rs"
PARSER_TEST="$ROOT/crates/exchange/src/adapters/private_read_order_semantics_tests.rs"
BINANCE="$ROOT/crates/exchange/src/adapters/binance_private_data.rs"
PROJECTION="$ROOT/crates/api/src/services/account_open_orders/projection.rs"
BROWSER="$ROOT/test/e2e/pr_ed_account_state.spec.ts"
PACKAGE="$ROOT/package.json"
REPO_GATE="$ROOT/scripts/verify_repo_gates.sh"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-AQ completion gate failed: %s\n' "$1" >&2
  exit 1
}

run_static_gate() {
  PR_AQ_SKIP_TESTS=1 PR_AQ_SKIP_UPSTREAM=1 PR_AQ_SKIP_BROWSER=1 bash "$0" >/dev/null
}

expect_rejected() {
  local message="$1"
  if run_static_gate 2>/dev/null; then
    fail "$message"
  fi
}

if [[ "$MODE" == "--self-test" ]]; then
  tmp="$(mktemp -d "${TMPDIR:-/tmp}/crossline-pr-aq.XXXXXX")"
  files=("$AUDIT" "$EVIDENCE" "$SHARED" "$PARSER_TEST" "$BINANCE" "$PROJECTION" "$BROWSER" "$PACKAGE" "$REPO_GATE")
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
old = "| `PR-AQ Private Read Schema Fixtures & Account Mode Mapping` | ✅ 完成 |"
if text.count(old) != 1:
    raise SystemExit("PR-AQ self-test setup failed: roadmap row drifted")
path.write_text(text.replace(old, "| `PR-AQ Private Read Schema Fixtures & Account Mode Mapping` | 🟡 部分完成 |", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted a downgraded roadmap row"
  restore_file "$AUDIT"

  python3 - "$EVIDENCE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
rows = path.read_text(encoding="utf-8").splitlines()
for index, row in enumerate(rows):
    if row.startswith("PR-AQ\tprivate-order-schema-matrix\t"):
        del rows[index]
        break
else:
    raise SystemExit("PR-AQ self-test setup failed: evidence row missing")
path.write_text("\n".join(rows) + "\n", encoding="utf-8")
PY
  expect_rejected "self-test accepted incomplete evidence"
  restore_file "$EVIDENCE"

  python3 - "$SHARED" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = "    pub venue_time_in_force: Option<String>,\n"
if text.count(old) != 1:
    raise SystemExit("PR-AQ self-test setup failed: shared field drifted")
path.write_text(text.replace(old, "", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted a removed venue TIF contract"
  restore_file "$SHARED"

  python3 - "$BINANCE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = "        venue_time_in_force: Some(item.time_in_force.clone()),\n"
if text.count(old) != 1:
    raise SystemExit("PR-AQ self-test setup failed: Binance mapping drifted")
path.write_text(text.replace(old, "        venue_time_in_force: None,\n", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted a venue mapping that dropped TIF"
  restore_file "$BINANCE"

  python3 - "$PARSER_TEST" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = "#[test]\nfn official_private_read_schemas_preserve_order_condition_and_identity()"
if text.count(old) != 1:
    raise SystemExit("PR-AQ self-test setup failed: parser fixture test drifted")
path.write_text(text.replace(old, "#[test]\n#[ignore]\nfn official_private_read_schemas_preserve_order_condition_and_identity()", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted an ignored private-read fixture test"
  restore_file "$PARSER_TEST"

  python3 - "$PROJECTION" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = '                    "venueTimeInForce",\n'
if text.count(old) != 1:
    raise SystemExit("PR-AQ self-test setup failed: projection marker drifted")
path.write_text(text.replace(old, '                    "venueTifRemoved",\n', 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted hidden account-state TIF evidence"
  restore_file "$PROJECTION"

  python3 - "$BROWSER" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = 'test("PR-AQ private-read order semantics stay visible in account evidence"'
if text.count(old) != 1:
    raise SystemExit("PR-AQ self-test setup failed: browser marker drifted")
path.write_text(text.replace(old, 'test.skip("PR-AQ private-read order semantics stay visible in account evidence"', 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted a skipped account evidence fixture"
  restore_file "$BROWSER"

  python3 - "$AUDIT" <<'PY'
from pathlib import Path
import re
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
head = re.search(r"(?m)^1\. \*\*(PR-[A-Z]+)\b", text)
if head is None:
    raise SystemExit("PR-AQ self-test setup failed: successor queue head drifted")
path.write_text(text[:head.start(1)] + "PR-AQ" + text[head.end(1):], encoding="utf-8")
PY
  expect_rejected "self-test accepted PR-AQ reinserted into the local queue"
  restore_file "$AUDIT"

  python3 - "$REPO_GATE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = 'PR_AQ_SKIP_TESTS=1 PR_AQ_SKIP_UPSTREAM=1 PR_AQ_SKIP_BROWSER=1 bash "$ROOT/scripts/check_pr_aq_completion.sh"'
if text.count(old) != 2:
    raise SystemExit("PR-AQ self-test setup failed: repo wiring drifted")
path.write_text(text.replace(old, "true # PR-AQ gate removed", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted single-scope repository wiring"

  printf 'PR-AQ completion self-test passed\n'
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
    raise SystemExit(f"PR-AQ completion gate failed: {message}")


title = "PR-AQ Private Read Schema Fixtures & Account Mode Mapping"
row = next((line for line in audit.splitlines() if line.startswith(f"| `{title}` |")), None)
if row is None or "| ✅ 完成 |" not in row or "剩余：无。" not in row:
    fail("roadmap row is not complete with no local remainder")
if "`bash scripts/check_pr_aq_completion.sh --self-test`" not in row:
    fail("roadmap row lacks the canonical destructive verification anchor")
if "> 本轮 PR-AQ：" not in audit:
    fail("top progress summary lacks the PR-AQ closure note")

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
    or any(pr_id in numbered for pr_id in ("PR-AQ", "PR-AR", "PR-AS", "PR-AU", "PR-AV"))
):
    fail(f"queue handoff drifted: expected={incomplete_successors}, actual={numbered[:len(incomplete_successors)]}")
if "## 2026-07-18 PR-AQ Private Read Schema Closure" not in history:
    fail("history closure appendix is missing")

evidence_contract = {
    "successor-account-mode": "scripts/check_pr_cf_completion.sh",
    "successor-account-state": "scripts/check_pr_ed_completion.sh",
    "successor-portfolio-runtime": "scripts/check_pr_di_completion.sh",
    "successor-private-fixture": "scripts/check_pr_fc_completion.sh",
    "operation-evidence-matrix": "scripts/check_exchange_operation_evidence_matrix.sh",
    "order-info-native-condition": "shared-types/src/orders.rs",
    "private-order-schema-matrix": "crates/exchange/src/adapters/private_read_order_semantics_tests.rs",
    "binance-order-condition": "crates/exchange/src/adapters/binance_private_data.rs",
    "okx-order-condition": "crates/exchange/src/adapters/okx_private_data.rs",
    "bybit-order-condition": "crates/exchange/src/adapters/bybit_private_data.rs",
    "bitget-order-condition": "crates/exchange/src/adapters/bitget_uta_private_data/order.rs",
    "gate-order-condition": "crates/exchange/src/adapters/gate_private_data.rs",
    "htx-order-condition": "crates/exchange/src/adapters/htx_private_data.rs",
    "kucoin-order-condition": "crates/exchange/src/adapters/kucoin_private_data.rs",
    "hyperliquid-order-condition": "crates/exchange/src/adapters/hyperliquid_private_data.rs",
    "account-open-order-projection": "crates/api/src/services/account_open_orders/projection.rs",
    "account-open-order-fixture": "crates/api/src/services/account_open_orders/tests.rs",
    "account-state-browser": "test/e2e/pr_ed_account_state.spec.ts",
    "package-product-suite": "package.json",
    "release-qa-product-suite": "scripts/check_release_qa_contract.sh",
    "completion-governance": "scripts/check_pr_aq_completion.sh",
}
with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-AQ"]
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

markers = {
    "shared-types/src/orders.rs": ("pub venue_time_in_force: Option<String>",),
    "crates/exchange/src/adapters/binance_private_data.rs": ("venue_time_in_force: Some(item.time_in_force.clone())",),
    "crates/exchange/src/adapters/okx_private_data.rs": ("venue_time_in_force: Some(order.ord_type.clone())", "client_order_id_from_str(&order.cl_ord_id)"),
    "crates/exchange/src/adapters/bybit_private_data.rs": ("venue_time_in_force: Some(venue_time_in_force)",),
    "crates/exchange/src/adapters/bitget_uta_private_data/order.rs": ("venue_time_in_force: Some(order.time_in_force.clone())",),
    "crates/exchange/src/adapters/gate_private_data.rs": ("venue_time_in_force: Some(order.tif.clone())",),
    "crates/exchange/src/adapters/htx_private_data.rs": ("venue_time_in_force: Some(order.order_price_type.trim().to_owned())",),
    "crates/exchange/src/adapters/kucoin_private_data.rs": ("kucoin_time_in_force(order.time_in_force.as_deref()",),
    "crates/exchange/src/adapters/hyperliquid_private_data.rs": ("venue_time_in_force: order.tif.clone()",),
    "crates/api/src/services/account_open_orders/projection.rs": ('"clientOrderId"', '"venueTimeInForce"', '"reduceOnly"', "AccountFieldQualityStatus::Actual"),
}
for path, required in markers.items():
    source = (root / path).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"missing {path} marker: {marker}")

private_rest_paths = (
    "crates/exchange/src/adapters/binance_private_data.rs",
    "crates/exchange/src/adapters/okx_private_data.rs",
    "crates/exchange/src/adapters/bybit_private_data.rs",
    "crates/exchange/src/adapters/bitget_uta_private_data/order.rs",
    "crates/exchange/src/adapters/gate_private_data.rs",
    "crates/exchange/src/adapters/htx_private_data.rs",
    "crates/exchange/src/adapters/kucoin_private_data.rs",
    "crates/exchange/src/adapters/hyperliquid_private_data.rs",
)
default_pattern = re.compile(r"unwrap_or\((?:0|0\.0)\)|unwrap_or_default\(\)")
for path in private_rest_paths:
    if default_pattern.search((root / path).read_text(encoding="utf-8")):
        fail(f"private-read parser still silently defaults core evidence: {path}")

parser_test = (root / "crates/exchange/src/adapters/private_read_order_semantics_tests.rs").read_text(encoding="utf-8")
if re.search(r"#\[(?:ignore|should_panic|cfg[^\]]*ignore)", parser_test):
    fail("private-read schema matrix must not be ignored or panic-expected")
for venue in ("binance", "okx", "bybit", "bitget", "gate", "htx", "kucoin", "hyperliquid"):
    if f"fn assert_{venue}()" not in parser_test:
        fail(f"private-read schema matrix lacks {venue}")

browser = (root / "test/e2e/pr_ed_account_state.spec.ts").read_text(encoding="utf-8")
if re.search(r"\btest\.(?:skip|fixme)\s*\(", browser):
    fail("account-state browser fixture must not be skipped")
for marker in (
    "PR-AQ private-read order semantics stay visible in account evidence",
    "clientOrderId",
    "venueTimeInForce",
    "reduceOnly",
    "account_open_orders_runtime",
):
    if marker not in browser:
        fail(f"browser fixture missing private-read evidence: {marker}")

package = (root / "package.json").read_text(encoding="utf-8")
script = '"test:e2e:pr-aq": "playwright test test/e2e/pr_ed_account_state.spec.ts --grep \\"PR-AQ\\""'
if script not in package or package.count("test/e2e/pr_ed_account_state.spec.ts") != 3:
    fail("package PR-AQ script or product-suite wiring is missing")
release_qa = (root / "scripts/check_release_qa_contract.sh").read_text(encoding="utf-8")
release_fixture = (root / "scripts/fixtures/release_qa_contract/package.json").read_text(encoding="utf-8")
for source, label in ((release_qa, "release QA contract"), (release_fixture, "release QA fixture")):
    if source.count("test/e2e/pr_ed_account_state.spec.ts") != 1:
        fail(f"{label} does not lock the shared PR-ED/PR-AQ product fixture exactly once")
repo_gate = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
wiring = 'PR_AQ_SKIP_TESTS=1 PR_AQ_SKIP_UPSTREAM=1 PR_AQ_SKIP_BROWSER=1 bash "$ROOT/scripts/check_pr_aq_completion.sh"'
if repo_gate.count(wiring) != 2:
    fail("completion gate must be wired into both repository scopes")

print(f"OK PR-AQ contract ({len(evidence_contract)} evidence types; queue={numbered[0]})")
PY

bash "$ROOT/scripts/check_product_audit_progress.sh"
bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_evidence_index.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"
bash "$ROOT/scripts/check_release_qa_contract.sh"

if [[ "${PR_AQ_SKIP_UPSTREAM:-0}" != "1" ]]; then
  PR_CF_SKIP_TESTS=1 PR_CF_SKIP_UPSTREAM=1 PR_CF_SKIP_BROWSER_LIST=1 bash "$ROOT/scripts/check_pr_cf_completion.sh"
  PR_ED_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_ed_completion.sh"
  PR_DI_SKIP_TESTS=1 PR_DI_SKIP_UPSTREAM=1 PR_DI_SKIP_BROWSER_LIST=1 bash "$ROOT/scripts/check_pr_di_completion.sh"
  bash "$ROOT/scripts/check_pr_fc_completion.sh"
  bash "$ROOT/scripts/check_exchange_operation_evidence_matrix.sh"
fi

if [[ "${PR_AQ_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p exchange --lib private_read_order_semantics_tests --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api account_open_orders --bin crypto-arb-api --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p trading reconciler --lib --no-fail-fast
fi

if [[ "${PR_AQ_SKIP_BROWSER:-0}" != "1" ]]; then
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-aq
fi

printf 'OK PR-AQ private-read schema and account semantics contract\n'
