#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
AUDIT="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
HISTORY="$ROOT/docs/audit_history/PRODUCT_AUDIT_HISTORY.md"
EVIDENCE="$ROOT/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
SHARED="$ROOT/shared-types/src/venues/credentials.rs"
SHARED_TEST="$ROOT/shared-types/src/venues/credentials/tests.rs"
STORAGE_HEALTH="$ROOT/crates/api/src/services/venue_credentials/storage/health.rs"
STORAGE_TEST="$ROOT/crates/api/src/services/venue_credentials/storage/tests.rs"
CEX_MATRIX="$ROOT/crates/api/src/services/venue_credentials/validation/order_permission_matrix_tests.rs"
HL_PROBES="$ROOT/crates/api/src/services/venue_credentials/validation/hyperliquid_probes.rs"
FORMAT="$ROOT/frontend/src/panels/modules/settings/tabs/venue_credentials/format/status.rs"
PANELS="$ROOT/frontend/src/panels/modules/settings/tabs/venue_credentials/panels.rs"
FRONTEND_TEST="$ROOT/frontend/src/panels/modules/settings/tabs/venue_credentials/tests_credentials/secret_storage.rs"
BROWSER="$ROOT/test/e2e/pr_as_credential_health.spec.ts"
PACKAGE="$ROOT/package.json"
RELEASE_QA="$ROOT/scripts/check_release_qa_contract.sh"
RELEASE_FIXTURE="$ROOT/scripts/fixtures/release_qa_contract/package.json"
REPO_GATE="$ROOT/scripts/verify_repo_gates.sh"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-AS completion gate failed: %s\n' "$1" >&2
  exit 1
}

run_static_gate() {
  PR_AS_STATIC_ONLY=1 PR_AS_SKIP_TESTS=1 PR_AS_SKIP_UPSTREAM=1 PR_AS_SKIP_BROWSER=1 bash "$0" >/dev/null
}

expect_rejected() {
  local message="$1"
  if run_static_gate 2>/dev/null; then
    fail "$message"
  fi
}

if [[ "$MODE" == "--self-test" ]]; then
  tmp="$(mktemp -d "${TMPDIR:-/tmp}/crossline-pr-as.XXXXXX")"
  files=(
    "$AUDIT" "$EVIDENCE" "$SHARED" "$STORAGE_HEALTH" "$CEX_MATRIX"
    "$FORMAT" "$BROWSER" "$RELEASE_QA" "$REPO_GATE"
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
old = "| `PR-AS Credential Validation Scope & Secret Persistence` | ✅ 完成 |"
if text.count(old) != 1:
    raise SystemExit("PR-AS self-test setup failed: roadmap row drifted")
path.write_text(text.replace(old, "| `PR-AS Credential Validation Scope & Secret Persistence` | 🟡 部分完成 |", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted a downgraded roadmap row"
  restore_file "$AUDIT"

  python3 - "$EVIDENCE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
rows = path.read_text(encoding="utf-8").splitlines()
for index, row in enumerate(rows):
    if row.startswith("PR-AS\ttyped-secret-storage-health\t"):
        del rows[index]
        break
else:
    raise SystemExit("PR-AS self-test setup failed: evidence row missing")
path.write_text("\n".join(rows) + "\n", encoding="utf-8")
PY
  expect_rejected "self-test accepted incomplete exact evidence"
  restore_file "$EVIDENCE"

  python3 - "$SHARED" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = "    Unavailable,\n"
if text.count(old) != 1:
    raise SystemExit("PR-AS self-test setup failed: health enum drifted")
path.write_text(text.replace(old, "    Ready,\n", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted secret storage without unavailable state"
  restore_file "$SHARED"

  python3 - "$STORAGE_HEALTH" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = "        status = status.with_backend_error(error.value().clone());\n"
if text.count(old) != 1:
    raise SystemExit("PR-AS self-test setup failed: backend health projection drifted")
path.write_text(text.replace(old, "        status.warning = Some(error.value().clone());\n", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted backend errors without typed unavailable health"
  restore_file "$STORAGE_HEALTH"

  python3 - "$FORMAT" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = '        SecretStorageHealth::Unavailable => "status-pill blocked",\n'
if text.count(old) != 1:
    raise SystemExit("PR-AS self-test setup failed: unavailable class drifted")
path.write_text(text.replace(old, '        SecretStorageHealth::Unavailable => "status-pill pending",\n', 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted unavailable secret storage as pending"
  restore_file "$FORMAT"

  python3 - "$CEX_MATRIX" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = "#[test]\nfn save_time_order_permission_probe_matrix_matches_validator_wiring()"
if text.count(old) != 1:
    raise SystemExit("PR-AS self-test setup failed: CEX matrix drifted")
path.write_text(text.replace(old, "#[test]\n#[ignore]\nfn save_time_order_permission_probe_matrix_matches_validator_wiring()", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted an ignored eight-venue permission matrix"
  restore_file "$CEX_MATRIX"

  python3 - "$BROWSER" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = 'test("PR-AS keeps secret backend health separate from save-time and live runtime evidence"'
if text.count(old) != 1:
    raise SystemExit("PR-AS self-test setup failed: browser marker drifted")
path.write_text(text.replace(old, 'test.skip("PR-AS keeps secret backend health separate from save-time and live runtime evidence"', 1), encoding="utf-8")
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
    raise SystemExit("PR-AS self-test setup failed: queue head drifted")
path.write_text(text[:head.start(1)] + "PR-AS" + text[head.end(1):], encoding="utf-8")
PY
  expect_rejected "self-test accepted PR-AS reinserted into the local queue"
  restore_file "$AUDIT"

  python3 - "$RELEASE_QA" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = " test/e2e/pr_as_credential_health.spec.ts"
if text.count(old) != 1:
    raise SystemExit("PR-AS self-test setup failed: release QA wiring drifted")
path.write_text(text.replace(old, "", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted release QA without the PR-AS browser"
  restore_file "$RELEASE_QA"

  python3 - "$REPO_GATE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = 'PR_AS_SKIP_TESTS=1 PR_AS_SKIP_UPSTREAM=1 PR_AS_SKIP_BROWSER=1 bash "$ROOT/scripts/check_pr_as_completion.sh"'
if text.count(old) != 2:
    raise SystemExit("PR-AS self-test setup failed: repository wiring drifted")
path.write_text(text.replace(old, "true # PR-AS gate removed", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted single-scope repository wiring"

  printf 'PR-AS completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from pathlib import Path
import csv
import json
import re
import sys

root = Path(sys.argv[1])
audit = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(encoding="utf-8")


def fail(message: str) -> None:
    raise SystemExit(f"PR-AS completion gate failed: {message}")


title = "PR-AS Credential Validation Scope & Secret Persistence"
row = next((line for line in audit.splitlines() if line.startswith(f"| `{title}` |")), None)
if row is None or "| ✅ 完成 |" not in row or "剩余：无。" not in row:
    fail("roadmap row is not complete with no local remainder")
if "`bash scripts/check_pr_as_completion.sh --self-test`" not in row:
    fail("roadmap row lacks the canonical destructive verification anchor")
if "> 本轮 PR-AS：" not in audit:
    fail("top progress summary lacks the PR-AS closure note")

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
    or any(pr_id in numbered for pr_id in ("PR-AS", "PR-AU", "PR-AV"))
):
    fail(f"queue handoff drifted: expected={incomplete_successors}, actual={numbered[:len(incomplete_successors)]}")
if "## 2026-07-19 PR-AS Credential Scope and Secret Health Closure" not in history:
    fail("history closure appendix is missing")

evidence_contract = {
    "successor-credential-contract": "scripts/check_pr_dg_completion.sh",
    "successor-settings-control-plane": "scripts/check_pr_du_completion.sh",
    "successor-operation-health": "scripts/check_pr_eg_completion.sh",
    "successor-operator-secret-boundary": "scripts/check_pr_cq_completion.sh",
    "successor-hyperliquid-account": "scripts/check_pr_ce_completion.sh",
    "successor-three-layer-readiness": "scripts/check_pr_cb_completion.sh",
    "successor-credential-validation": "scripts/check_pr_ff_completion.sh",
    "successor-runtime-boundary": "scripts/check_pr_cm_completion.sh",
    "successor-scoped-preflight": "scripts/check_pr_cg_completion.sh",
    "successor-secret-governance": "scripts/check_pr_ac_completion.sh",
    "successor-load-state-copy": "scripts/check_pr_bs_completion.sh",
    "successor-secret-health": "scripts/check_pr_bp_completion.sh",
    "credential-shared-contract": "shared-types/src/venues/credentials.rs",
    "typed-secret-storage-health": "shared-types/src/venues/credentials/tests.rs",
    "secret-backend-health-projection": "crates/api/src/services/venue_credentials/storage/health.rs",
    "secret-backend-health-fixture": "crates/api/src/services/venue_credentials/storage/tests.rs",
    "eight-venue-permission-matrix": "crates/api/src/services/venue_credentials/validation/order_permission_matrix_tests.rs",
    "hyperliquid-account-relation": "crates/api/src/services/venue_credentials/validation/hyperliquid_probes.rs",
    "settings-health-format": "frontend/src/panels/modules/settings/tabs/venue_credentials/format/status.rs",
    "settings-secret-panel": "frontend/src/panels/modules/settings/tabs/venue_credentials/panels.rs",
    "settings-health-fixture": "frontend/src/panels/modules/settings/tabs/venue_credentials/tests_credentials/secret_storage.rs",
    "product-browser": "test/e2e/pr_as_credential_health.spec.ts",
    "package-product-suite": "package.json",
    "release-qa-product-suite": "scripts/check_release_qa_contract.sh",
    "completion-governance": "scripts/check_pr_as_completion.sh",
}
with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [item for item in csv.DictReader(handle, delimiter="\t") if item["pr_id"] == "PR-AS"]
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
    if artifact == "package.json":
        continue
    item = coverage.get(artifact)
    if item is None or item["coverage_status"] != "exact" or not item["evidence"].startswith("line:"):
        fail(f"exact coverage missing for {artifact}")

shared = (root / "shared-types/src/venues/credentials.rs").read_text(encoding="utf-8")
for marker in (
    "pub enum SecretStorageHealth",
    "pub health: SecretStorageHealth",
    "pub last_error: Option<String>",
    "pub fn with_backend_error",
):
    if marker not in shared:
        fail(f"typed secret storage contract missing: {marker}")
health_body = shared.split("pub enum SecretStorageHealth {", 1)[1].split("}", 1)[0]
for variant in ("Ready", "Degraded", "Unavailable", "Unknown"):
    if not re.search(rf"(?m)^\s+{variant},$", health_body):
        fail(f"typed secret storage health variant missing: {variant}")

storage_health = (root / "crates/api/src/services/venue_credentials/storage/health.rs").read_text(encoding="utf-8")
if "status.with_backend_error(error.value().clone())" not in storage_health:
    fail("backend read failure does not project typed unavailable health")
storage_test = (root / "crates/api/src/services/venue_credentials/storage/tests.rs").read_text(encoding="utf-8")
for marker in ("secret_backend_read_failure_is_visible_in_storage_status", "SecretStorageHealth::Unavailable", "last_error"):
    if marker not in storage_test:
        fail(f"backend health fixture missing: {marker}")

matrix = (root / "crates/api/src/services/venue_credentials/validation/order_permission_matrix_tests.rs").read_text(encoding="utf-8")
if re.search(r"#\[(?:ignore|should_panic)", matrix):
    fail("eight-venue permission matrix must not be ignored or panic-expected")
for venue in ("binance", "okx", "bybit", "bitget", "gate", "htx", "kucoin", "hyperliquid"):
    if f'venue: "{venue}"' not in matrix:
        fail(f"permission matrix lacks {venue}")

hyperliquid = (root / "crates/api/src/services/venue_credentials/validation/hyperliquid_probes.rs").read_text(encoding="utf-8")
for marker in ("account_signer_vault_relation", "userRole(account)+userRole(signer)+vaultDetails", "clearinghouseState(account)+spotClearinghouseState(account)"):
    if marker not in hyperliquid:
        fail(f"Hyperliquid account/signer/vault evidence missing: {marker}")

status_format = (root / "frontend/src/panels/modules/settings/tabs/venue_credentials/format/status.rs").read_text(encoding="utf-8")
for marker in ('SecretStorageHealth::Ready => "status-pill ready"', 'SecretStorageHealth::Unavailable => "status-pill blocked"'):
    if marker not in status_format:
        fail(f"Settings secret health mapping missing: {marker}")
panels = (root / "frontend/src/panels/modules/settings/tabs/venue_credentials/panels.rs").read_text(encoding="utf-8")
for marker in ("data-secret-storage-health=health", '"后端错误："', "status.last_error"):
    if marker not in panels:
        fail(f"Settings secret health panel missing: {marker}")

for path, marker in (
    ("shared-types/src/venues/credentials/tests.rs", "legacy_secret_storage_status_defaults_to_unknown_health"),
    ("frontend/src/panels/modules/settings/tabs/venue_credentials/tests_credentials/secret_storage.rs", "only_ready_secret_storage_is_green"),
):
    source = (root / path).read_text(encoding="utf-8")
    if marker not in source or re.search(r"#\[(?:ignore|should_panic)", source):
        fail(f"non-skipping PR-AS fixture missing: {path}")

browser_path = "test/e2e/pr_as_credential_health.spec.ts"
browser = (root / browser_path).read_text(encoding="utf-8")
if re.search(r"\btest\.(?:skip|fixme)\s*\(", browser):
    fail("PR-AS product fixture must not be skipped")
for marker in (
    "PR-AS keeps secret backend health separate from save-time and live runtime evidence",
    "PR-AS blocks unavailable secret storage without erasing credential evidence",
    'data-secret-storage-health="可用"',
    'data-secret-storage-health="不可用"',
):
    if marker not in browser:
        fail(f"PR-AS browser fixture missing: {marker}")

package = json.loads((root / "package.json").read_text(encoding="utf-8"))
focused = "playwright test test/e2e/pr_as_credential_health.spec.ts"
if package.get("scripts", {}).get("test:e2e:pr-as") != focused:
    fail("package PR-AS focused command is missing")
package_text = (root / "package.json").read_text(encoding="utf-8")
if package_text.count(browser_path) != 2:
    fail("PR-AS browser must be wired once as a focused command and once in the product suite")
for path in ("scripts/check_release_qa_contract.sh", "scripts/fixtures/release_qa_contract/package.json"):
    if (root / path).read_text(encoding="utf-8").count(browser_path) != 1:
        fail(f"release QA command lock drifted: {path}")

repo_gate = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
wiring = 'PR_AS_SKIP_TESTS=1 PR_AS_SKIP_UPSTREAM=1 PR_AS_SKIP_BROWSER=1 bash "$ROOT/scripts/check_pr_as_completion.sh"'
if repo_gate.count(wiring) != 2:
    fail("completion gate must be wired into both repository scopes")
if repo_gate.count(browser_path) != 3:
    fail("repository gate must lock the focused command, product suite and browser list")

print(f"OK PR-AS contract ({len(evidence_contract)} evidence types; queue={numbered[0]})")
PY

if [[ "${PR_AS_STATIC_ONLY:-0}" == "1" ]]; then
  exit 0
fi

bash "$ROOT/scripts/check_product_audit_progress.sh"
bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_evidence_index.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"
bash "$ROOT/scripts/check_release_qa_contract.sh"

if [[ "${PR_AS_SKIP_UPSTREAM:-0}" != "1" ]]; then
  PR_DG_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dg_completion.sh"
  PR_DU_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_du_completion.sh"
  PR_EG_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_eg_completion.sh"
  PR_CE_SKIP_TESTS=1 PR_CE_SKIP_UPSTREAM=1 PR_CE_SKIP_BROWSER_LIST=1 bash "$ROOT/scripts/check_pr_ce_completion.sh"
  PR_CB_SKIP_TESTS=1 PR_CB_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_cb_completion.sh"
  PR_CM_SKIP_TESTS=1 PR_CM_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_cm_completion.sh"
  PR_CG_SKIP_TESTS=1 PR_CG_SKIP_UPSTREAM=1 PR_CG_SKIP_BROWSER_LIST=1 bash "$ROOT/scripts/check_pr_cg_completion.sh"
  PR_AC_SKIP_TESTS=1 PR_AC_SKIP_UPSTREAM=1 PR_AC_SKIP_BROWSER_LIST=1 bash "$ROOT/scripts/check_pr_ac_completion.sh"
  PR_BS_SKIP_TESTS=1 PR_BS_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_bs_completion.sh"
  PR_BP_SKIP_TESTS=1 PR_BP_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_bp_completion.sh"
fi

if [[ "${PR_AS_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p shared-types venues::credentials::tests --lib --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api --bin crypto-arb-api services::venue_credentials --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p exchange --test hyperliquid_test agent_approval_validation --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p exchange --test hyperliquid_test credential_relation --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib venue_credentials --no-fail-fast
fi

if [[ "${PR_AS_SKIP_BROWSER:-0}" != "1" ]]; then
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-as
fi

printf 'OK PR-AS credential scope and secret health contract\n'
