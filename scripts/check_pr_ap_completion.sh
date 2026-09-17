#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
AUDIT="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
EVIDENCE="$ROOT/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
EXECUTION="$ROOT/crates/trading/src/execution.rs"
COLLATERAL="$ROOT/crates/api/src/services/hedge_margin/collateral.rs"
MARGIN_EVIDENCE="$ROOT/crates/api/src/services/hedge_margin/evidence.rs"
FRONTEND_EVIDENCE="$ROOT/frontend/src/panels/modules/execution/components/risk_preview/evidence.rs"
BROWSER="$ROOT/test/e2e/pr_ap_account_margin_evidence.spec.ts"
REPO_GATE="$ROOT/scripts/verify_repo_gates.sh"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-AP completion gate failed: %s\n' "$1" >&2
  exit 1
}

run_static_gate() {
  PR_AP_SKIP_TESTS=1 PR_AP_SKIP_UPSTREAM=1 PR_AP_SKIP_BROWSER=1 bash "$0" >/dev/null
}

expect_rejected() {
  local message="$1"
  if run_static_gate 2>/dev/null; then
    fail "$message"
  fi
}

if [[ "$MODE" == "--self-test" ]]; then
  tmp="$(mktemp -d "${TMPDIR:-/tmp}/crossline-pr-ap.XXXXXX")"
  files=("$AUDIT" "$EVIDENCE" "$EXECUTION" "$COLLATERAL" "$MARGIN_EVIDENCE" "$FRONTEND_EVIDENCE" "$BROWSER" "$REPO_GATE")
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
old = "| `PR-AP AccountStateSnapshot & Leg-Scoped Margin Evidence` | ✅ 完成 |"
if text.count(old) != 1:
    raise SystemExit("PR-AP self-test setup failed: roadmap row drifted")
path.write_text(text.replace(old, "| `PR-AP AccountStateSnapshot & Leg-Scoped Margin Evidence` | 🟡 部分完成 |", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted a downgraded roadmap row"
  restore_file "$AUDIT"

  python3 - "$EVIDENCE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
rows = path.read_text(encoding="utf-8").splitlines()
for index, row in enumerate(rows):
    if row.startswith("PR-AP\tapi-unrelated-collateral-fixture\t"):
        del rows[index]
        break
else:
    raise SystemExit("PR-AP self-test setup failed: evidence row missing")
path.write_text("\n".join(rows) + "\n", encoding="utf-8")
PY
  expect_rejected "self-test accepted incomplete evidence"
  restore_file "$EVIDENCE"

  python3 - "$EXECUTION" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = "    (None, 0.0)\n}\n\npub fn selected_margin_currency_for_intent"
if text.count(old) != 1:
    raise SystemExit("PR-AP self-test setup failed: margin fallback marker drifted")
new = "    (None, fallback_available_margin(balances, &intent.exchange))\n}\n\npub fn selected_margin_currency_for_intent"
path.write_text(text.replace(old, new, 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted arbitrary-asset margin fallback"
  restore_file "$EXECUTION"

  python3 - "$MARGIN_EVIDENCE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = "    pub(super) collateral_rows: Vec<VenueBalanceInfo>,"
if text.count(old) != 1:
    raise SystemExit("PR-AP self-test setup failed: collateral evidence marker drifted")
path.write_text(text.replace(old, "", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted dropped raw collateral evidence"
  restore_file "$MARGIN_EVIDENCE"

  python3 - "$FRONTEND_EVIDENCE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = '"{} {} {} · {}",'
if text.count(old) != 1:
    raise SystemExit("PR-AP self-test setup failed: field source formatter drifted")
path.write_text(text.replace(old, '"{} {} {}",', 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted hidden account field source"
  restore_file "$FRONTEND_EVIDENCE"

  python3 - "$BROWSER" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = 'test("PR-AP execution exposes scoped margin currency, account source, and mode evidence"'
if text.count(old) != 1:
    raise SystemExit("PR-AP self-test setup failed: browser marker drifted")
path.write_text(text.replace(old, 'test.skip("PR-AP execution exposes scoped margin currency, account source, and mode evidence"', 1), encoding="utf-8")
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
    raise SystemExit("PR-AP self-test setup failed: successor queue head drifted")
path.write_text(text[:head.start(1)] + "PR-AP" + text[head.end(1):], encoding="utf-8")
PY
  expect_rejected "self-test accepted PR-AP reinserted into the local queue"
  restore_file "$AUDIT"

  python3 - "$REPO_GATE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = 'PR_AP_SKIP_TESTS=1 PR_AP_SKIP_UPSTREAM=1 PR_AP_SKIP_BROWSER=1 bash "$ROOT/scripts/check_pr_ap_completion.sh"'
if text.count(old) != 2:
    raise SystemExit("PR-AP self-test setup failed: repo wiring drifted")
path.write_text(text.replace(old, "true # PR-AP gate removed", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted single-scope repository wiring"

  printf 'PR-AP completion self-test passed\n'
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
    raise SystemExit(f"PR-AP completion gate failed: {message}")


title = "PR-AP AccountStateSnapshot & Leg-Scoped Margin Evidence"
row = next((line for line in audit.splitlines() if line.startswith(f"| `{title}` |")), None)
if row is None or "| ✅ 完成 |" not in row or "剩余：无。" not in row:
    fail("roadmap row is not complete with no local remainder")
if "`bash scripts/check_pr_ap_completion.sh --self-test`" not in row:
    fail("roadmap row lacks the canonical destructive verification anchor")
if "> 本轮 PR-AP：" not in audit:
    fail("top progress summary lacks the PR-AP closure note")

queue = audit.split("### 🟡 6.5 下一步执行队列", 1)[-1]
numbered = re.findall(r"(?m)^\d+\. \*\*(PR-[A-Z]+)", queue)
if not numbered or "PR-AP" in numbered:
    fail(f"queue handoff drifted: {numbered[:5]}")
if "## 2026-07-18 PR-AP Scoped Account Margin Evidence Closure" not in history:
    fail("history closure appendix is missing")

evidence_contract = {
    "successor-account-state": "scripts/check_pr_ed_completion.sh",
    "successor-portfolio": "scripts/check_pr_cd_completion.sh",
    "successor-scoped-preflight": "scripts/check_pr_dv_completion.sh",
    "successor-cex-account-mode": "scripts/check_pr_cf_completion.sh",
    "successor-portfolio-runtime": "scripts/check_pr_di_completion.sh",
    "successor-private-fixture": "scripts/check_pr_fc_completion.sh",
    "margin-currency-selection": "crates/trading/src/execution.rs",
    "collateral-projection": "crates/api/src/services/hedge_margin/collateral.rs",
    "margin-evidence-source": "crates/api/src/services/hedge_margin/evidence.rs",
    "margin-row-health": "crates/api/src/services/hedge_margin/health.rs",
    "margin-field-quality": "crates/api/src/services/hedge_margin/problems.rs",
    "account-margin-summary": "crates/api/src/services/hedge_margin/venues.rs",
    "api-summary-fixture": "crates/api/src/services/hedge_margin/tests/account_summary.rs",
    "api-unrelated-collateral-fixture": "crates/api/src/services/hedge_margin/tests/collateral.rs",
    "api-error-envelope-fixture": "crates/api/src/services/hedge_margin/tests/evidence.rs",
    "frontend-source-consumer": "frontend/src/panels/modules/execution/components/risk_preview/evidence.rs",
    "frontend-health-formatter": "frontend/src/panels/modules/execution/components/risk_preview/format.rs",
    "frontend-unit-fixture": "frontend/src/panels/modules/execution/components/risk_preview/tests/cases.rs",
    "account-margin-browser": "test/e2e/pr_ap_account_margin_evidence.spec.ts",
    "operation-evidence-matrix": "scripts/check_exchange_operation_evidence_matrix.sh",
    "package-product-suite": "package.json",
    "release-qa-product-suite": "scripts/check_release_qa_contract.sh",
    "completion-governance": "scripts/check_pr_ap_completion.sh",
}
with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-AP"]
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
    "crates/trading/src/execution.rs": (
        "pub fn selected_margin_currency_for_intent(",
        "(None, 0.0)",
        "hedge_margin_check_rejects_unrelated_asset_as_collateral",
    ),
    "crates/api/src/services/hedge_margin/collateral.rs": (
        "margin_account_state_rows",
        "margin_collateral_row_health",
        "margin_collateral_field_quality",
        '"collateral_currency"',
        '"account_equity_source"',
        '"margin_currency"',
    ),
    "crates/api/src/services/hedge_margin/evidence.rs": (
        "pub(super) collateral_rows: Vec<VenueBalanceInfo>",
        "scoped_collateral_rows(collateral_rows, venues)",
    ),
    "frontend/src/panels/modules/execution/components/risk_preview/evidence.rs": (
        '"{} {} {} · {}"',
        "row.source",
    ),
    "frontend/src/panels/modules/execution/components/risk_preview/format.rs": (
        ".take(8)",
    ),
}
for path, required in markers.items():
    source = (root / path).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"missing {path} marker: {marker}")

execution = (root / "crates/trading/src/execution.rs").read_text(encoding="utf-8")
if "fallback_available_margin" in execution:
    fail("arbitrary-asset margin fallback returned")

browser = (root / "test/e2e/pr_ap_account_margin_evidence.spec.ts").read_text(encoding="utf-8")
if re.search(r"\btest\.(?:skip|fixme)\s*\(", browser):
    fail("account margin browser fixture must not be skipped")
for marker in (
    "bybit USDT bybit.v5.wallet_balance:coin",
    "account_equity_source OK",
    "okx.v5.account_balance:multi_currency_margin",
    "bybit_position_mode:hedge·UNIFIED",
    "okx_position_mode:long_short_mode·multi_currency_margin",
):
    if marker not in browser:
        fail(f"browser fixture missing account evidence: {marker}")

package = (root / "package.json").read_text(encoding="utf-8")
if '"test:e2e:pr-ap"' not in package or package.count("test/e2e/pr_ap_account_margin_evidence.spec.ts") != 2:
    fail("package script or product suite wiring is missing")
release_qa = (root / "scripts/check_release_qa_contract.sh").read_text(encoding="utf-8")
release_fixture = (root / "scripts/fixtures/release_qa_contract/package.json").read_text(encoding="utf-8")
for source, label in ((release_qa, "release QA contract"), (release_fixture, "release QA fixture")):
    if source.count("test/e2e/pr_ap_account_margin_evidence.spec.ts") != 1:
        fail(f"{label} does not lock the PR-AP product fixture exactly once")
repo_gate = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
wiring = 'PR_AP_SKIP_TESTS=1 PR_AP_SKIP_UPSTREAM=1 PR_AP_SKIP_BROWSER=1 bash "$ROOT/scripts/check_pr_ap_completion.sh"'
if repo_gate.count(wiring) != 2:
    fail("completion gate must be wired into both repository scopes")

print(f"OK PR-AP contract ({len(evidence_contract)} evidence types; queue={numbered[0]})")
PY

bash "$ROOT/scripts/check_product_audit_progress.sh"
bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_evidence_index.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"
bash "$ROOT/scripts/check_release_qa_contract.sh"

if [[ "${PR_AP_SKIP_UPSTREAM:-0}" != "1" ]]; then
  PR_ED_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_ed_completion.sh"
  PR_CD_SKIP_TESTS=1 PR_CD_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_cd_completion.sh"
  PR_DV_SKIP_TESTS=1 PR_DV_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_dv_completion.sh"
  PR_CF_SKIP_TESTS=1 PR_CF_SKIP_UPSTREAM=1 PR_CF_SKIP_BROWSER_LIST=1 bash "$ROOT/scripts/check_pr_cf_completion.sh"
  PR_DI_SKIP_TESTS=1 PR_DI_SKIP_UPSTREAM=1 PR_DI_SKIP_BROWSER_LIST=1 bash "$ROOT/scripts/check_pr_di_completion.sh"
  bash "$ROOT/scripts/check_pr_fc_completion.sh"
  bash "$ROOT/scripts/check_exchange_operation_evidence_matrix.sh"
fi

if [[ "${PR_AP_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p trading hedge_margin_check --lib --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api hedge_margin --bin crypto-arb-api --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" preflight_summary_counts_blocked_outcomes --no-fail-fast
fi

if [[ "${PR_AP_SKIP_BROWSER:-0}" != "1" ]]; then
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-ap
fi

printf 'OK PR-AP scoped account margin evidence contract\n'
