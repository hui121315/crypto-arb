#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
AUDIT="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
EVIDENCE="$ROOT/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
SHARED="$ROOT/shared-types/src/venues/credentials/permissions.rs"
CEX="$ROOT/crates/api/src/services/venue_credentials/validation/venues/cex.rs"
PERMISSION_UI="$ROOT/frontend/src/panels/modules/settings/tabs/venue_credentials/validation/permissions.rs"
TICKET_UI="$ROOT/frontend/src/panels/modules/settings/tabs/diagnostics/ticket_health.rs"
BROWSER="$ROOT/test/e2e/pr_bb_settings_runtime.spec.ts"
PACKAGE="$ROOT/package.json"
RELEASE="$ROOT/scripts/check_release_qa_contract.sh"
REPO_GATE="$ROOT/scripts/verify_repo_gates.sh"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-BB completion gate failed: %s\n' "$1" >&2
  exit 1
}

run_static_gate() {
  PR_BB_STATIC_ONLY=1 bash "$0" >/dev/null
}

if [[ "$MODE" == "--self-test" ]]; then
  tmp="$(mktemp -d "${TMPDIR:-/tmp}/crossline-pr-bb.XXXXXX")"
  files=("$AUDIT" "$EVIDENCE" "$SHARED" "$CEX" "$PERMISSION_UI" "$TICKET_UI" "$BROWSER" "$PACKAGE" "$RELEASE" "$REPO_GATE")
  for file in "${files[@]}"; do
    cp "$file" "$tmp/$(basename "$file").$(printf '%s' "$file" | shasum | cut -c1-12)"
  done
  restore_file() {
    local file="$1"
    cp "$tmp/$(basename "$file").$(printf '%s' "$file" | shasum | cut -c1-12)" "$file"
  }
  restore_all() {
    for file in "${files[@]}"; do restore_file "$file"; done
    rm -rf "$tmp"
  }
  trap restore_all EXIT

  mutate_once() {
    local file="$1" old="$2" new="$3"
    python3 - "$file" "$old" "$new" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
old, new = sys.argv[2:]
text = path.read_text(encoding="utf-8")
if old not in text:
    raise SystemExit(f"PR-BB self-test setup failed: marker missing in {path}")
path.write_text(text.replace(old, new, 1), encoding="utf-8")
PY
  }
  assert_rejected() {
    local file="$1" old="$2" new="$3" message="$4"
    mutate_once "$file" "$old" "$new"
    if run_static_gate 2>/dev/null; then fail "$message"; fi
    restore_file "$file"
  }

  run_static_gate
  assert_rejected "$AUDIT" \
    '| `PR-BB Settings Runtime Diagnostics & Credential Health UI` | ✅ 完成 |' \
    '| `PR-BB Settings Runtime Diagnostics & Credential Health UI` | 🟡 部分完成 |' \
    'self-test accepted a downgraded roadmap row'
  assert_rejected "$EVIDENCE" $'PR-BB\tcredential-permission-contract\t' \
    $'PR-BB-MISSING\tcredential-permission-contract\t' \
    'self-test accepted missing exact evidence'
  assert_rejected "$SHARED" 'pub enum VenueCredentialPermissionStatus {' \
    'enum HiddenVenueCredentialPermissionStatus {' \
    'self-test accepted a hidden typed permission status'
  assert_rejected "$CEX" '&[VenueCredentialPermission::CancelOrder],' \
    '&[VenueCredentialPermission::PlaceOrder],' \
    'self-test accepted a cancel-only venue mapped as place-only'
  assert_rejected "$PERMISSION_UI" '<th>"request_id"</th>' \
    '<th>"request"</th>' \
    'self-test accepted credentials UI without request_id'
  assert_rejected "$TICKET_UI" 'HedgeTicket 双腿预检是最终提交权威' \
    '设置页矩阵直接授予提交权限' \
    'self-test accepted a second submission authority'
  assert_rejected "$BROWSER" 'test("PR-BB renders explicit open, place, and cancel credential permission evidence"' \
    'test.skip("PR-BB renders explicit open, place, and cancel credential permission evidence"' \
    'self-test accepted a skipped browser fixture'
  assert_rejected "$PACKAGE" '"test:e2e:pr-bb": "playwright test test/e2e/pr_bb_settings_runtime.spec.ts"' \
    '"test:e2e:pr-bb": "true"' \
    'self-test accepted a skipped product command'
  assert_rejected "$AUDIT" '1. **PR-BC Review Fact Source & PnL Semantics**' \
    '1. **PR-BB Settings Runtime Diagnostics & Credential Health UI**' \
    'self-test accepted PR-BB reinserted at queue head'
  assert_rejected "$REPO_GATE" 'PR_BB_STATIC_ONLY=1 bash "$ROOT/scripts/check_pr_bb_completion.sh"' \
    'true # PR-BB gate removed' \
    'self-test accepted single-scope repository wiring'

  printf 'PR-BB completion self-test passed\n'
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
    raise SystemExit(f"PR-BB completion gate failed: {message}")


title = "PR-BB Settings Runtime Diagnostics & Credential Health UI"
row = next((line for line in audit.splitlines() if line.startswith(f"| `{title}` |")), None)
if row is None or "| ✅ 完成 |" not in row or "剩余：无。" not in row:
    fail("roadmap row is not complete with no local remainder")
if "`bash scripts/check_pr_bb_completion.sh --self-test`" not in row:
    fail("roadmap row lacks the destructive verification anchor")
if "> 本轮 PR-BB：" not in audit:
    fail("top progress summary lacks the PR-BB closure note")

queue = audit.split("### 🟡 6.5 下一步执行队列", 1)[-1]
numbered = re.findall(r"(?m)^\d+\. \*\*(PR-[A-Z]+)", queue)
roadmap_status = {}
for candidate in ("PR-BC Review Fact Source & PnL Semantics", "PR-BD Frontend ActionState & High-Risk Button Boundary"):
    candidate_row = next((line for line in audit.splitlines() if line.startswith(f"| `{candidate}` |")), "")
    roadmap_status[candidate.split()[0]] = "| ✅ 完成 |" in candidate_row
expected_local = [pr_id for pr_id in ("PR-BC", "PR-BD") if not roadmap_status[pr_id]]
if numbered[:len(expected_local)] != expected_local or any(
    pr_id in numbered for pr_id, complete in roadmap_status.items() if complete
) or "PR-BB" in numbered or len(numbered) != len(set(numbered)):
    fail(f"queue handoff drifted: {numbered}")
if "## 2026-07-19 PR-BB Settings Permission and Ticket Health Closure" not in history:
    fail("history closure appendix is missing")

contract = {
    "predecessor-credential-health": "scripts/check_pr_as_completion.sh",
    "predecessor-runtime-health": "scripts/check_pr_eg_completion.sh",
    "predecessor-workspace-runtime": "scripts/check_pr_cn_completion.sh",
    "credential-permission-contract": "shared-types/src/venues/credentials/permissions.rs",
    "credential-permission-tests": "shared-types/src/venues/credentials/tests.rs",
    "credential-permission-projection": "crates/api/src/services/venue_credentials/validation/probes.rs",
    "credential-probe-classification-tests": "crates/api/src/services/venue_credentials/validation/probes/tests.rs",
    "cex-permission-scope-matrix": "crates/api/src/services/venue_credentials/validation/venues/cex.rs",
    "hyperliquid-permission-scope": "crates/api/src/services/venue_credentials/validation/venues/hyperliquid.rs",
    "permission-scope-fixture": "crates/api/src/services/venue_credentials/validation/order_permission_matrix_tests.rs",
    "permission-endpoint-spec-fixture": "crates/api/src/services/venue_credentials/validation/order_permission_matrix_tests/endpoints.rs",
    "settings-permission-matrix": "frontend/src/panels/modules/settings/tabs/venue_credentials/validation/permissions.rs",
    "execution-selection-context": "frontend/src/panels/modules/execution/selection.rs",
    "settings-runtime-wiring": "frontend/src/panels/workstation.rs",
    "ticket-venue-health": "frontend/src/panels/modules/settings/tabs/diagnostics/ticket_health.rs",
    "product-browser": "test/e2e/pr_bb_settings_runtime.spec.ts",
    "package-product-suite": "package.json",
    "release-qa-product-suite": "scripts/check_release_qa_contract.sh",
    "release-qa-fixture": "scripts/fixtures/release_qa_contract/package.json",
    "repository-aggregate": "scripts/verify_repo_gates.sh",
    "completion-governance": "scripts/check_pr_bb_completion.sh",
}
with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [item for item in csv.DictReader(handle, delimiter="\t") if item["pr_id"] == "PR-BB"]
indexed = {item["evidence_type"]: item for item in rows}
if len(indexed) != len(rows) or set(indexed) != set(contract):
    fail(f"evidence type drift: expected={sorted(contract)}, actual={sorted(indexed)}")
for evidence_type, artifact in contract.items():
    item = indexed[evidence_type]
    if item["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"evidence artifact drifted or is missing: {evidence_type}")
    if not item["command"].strip() or not item["notes"].strip():
        fail(f"evidence lacks command or notes: {evidence_type}")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {item["file"]: item for item in csv.DictReader(handle, delimiter="\t")}
for artifact in set(contract.values()):
    if artifact.endswith(".json"):
        continue
    item = coverage.get(artifact)
    if item is None or item["coverage_status"] != "exact" or not item["evidence"].startswith("line:"):
        fail(f"exact coverage missing for {artifact}")

markers = {
    "shared-types/src/venues/credentials/permissions.rs": (
        "pub enum VenueCredentialPermission {", "pub enum VenueCredentialPermissionStatus {",
        "pub request_id: Option<String>", "pub error: Option<String>", "with_order_permission_scopes",
    ),
    "crates/api/src/services/venue_credentials/validation/probes.rs": (
        "evidence_with_order_permission_scopes", "permission_evidence: Vec::new()",
    ),
    "crates/api/src/services/venue_credentials/validation/probes/tests.rs": (
        "optional_probe_auth_failure_is_failed_not_unknown", "optional_probe_transient_errors_stay_unknown",
    ),
    "crates/api/src/services/venue_credentials/validation/order_permission_matrix_tests.rs": (
        "save_time_permission_matrix_distinguishes_open_place_and_cancel", "expected_permissions",
    ),
    "crates/api/src/services/venue_credentials/validation/order_permission_matrix_tests/endpoints.rs": (
        "save_time_order_permission_probe_sources_have_endpoint_specs", 'path: "/exchange"',
    ),
    "frontend/src/panels/modules/settings/tabs/venue_credentials/validation/permissions.rs": (
        '"validated"', '"probe_kind"', '"permission_scope"', '"checked_at"', '"request_id"', '"error / message"',
    ),
    "frontend/src/panels/modules/execution/selection.rs": ("pub(in crate::panels) fn venue_pair",),
    "frontend/src/panels/workstation.rs": ("settings_module(runtime.execution_runtime)",),
    "frontend/src/panels/modules/settings/tabs/diagnostics/ticket_health.rs": (
        "ticket_venue_health_panel", "selected_venue_health", "HedgeTicket 双腿预检是最终提交权威",
    ),
}
for relative, required in markers.items():
    text = (root / relative).read_text(encoding="utf-8")
    for marker in required:
        if marker not in text:
            fail(f"runtime contract lost marker in {relative}: {marker}")

cex = (root / "crates/api/src/services/venue_credentials/validation/venues/cex.rs").read_text(encoding="utf-8")
expected_scopes = {
    "validate_binance": {"PlaceOrder", "CancelOrder"},
    "validate_okx": {"PlaceOrder"},
    "validate_bybit": {"PlaceOrder"},
    "validate_bitget": {"CancelOrder"},
    "validate_gate": {"CancelOrder"},
    "validate_htx": {"PlaceOrder"},
    "validate_kucoin": {"PlaceOrder", "CancelOrder"},
}
for name, expected in expected_scopes.items():
    match = re.search(rf"async fn {name}\b(?P<body>.*?)(?=\npub\(in super::super\) async fn|\Z)", cex, re.S)
    if match is None:
        fail(f"venue validator missing: {name}")
    actual = set(re.findall(r"VenueCredentialPermission::(PlaceOrder|CancelOrder)", match.group("body")))
    if actual != expected:
        fail(f"venue permission scope drift for {name}: expected={sorted(expected)}, actual={sorted(actual)}")
hyperliquid = (root / "crates/api/src/services/venue_credentials/validation/venues/hyperliquid.rs").read_text(encoding="utf-8")
hyperliquid_scopes = set(re.findall(r"VenueCredentialPermission::(PlaceOrder|CancelOrder)", hyperliquid))
if hyperliquid_scopes != {"PlaceOrder", "CancelOrder"}:
    fail(f"Hyperliquid permission scope drift: {sorted(hyperliquid_scopes)}")

browser = (root / "test/e2e/pr_bb_settings_runtime.spec.ts").read_text(encoding="utf-8")
if re.search(r"\btest\.(?:skip|fixme)\(", browser) or browser.count('test("PR-BB') != 2:
    fail("browser contract is skipped or no longer has both product fixtures")
for marker in ("req-pr-bb-place", "credential-permissions", "ticket-venue-health", "HedgeTicket 双腿预检是最终提交权威"):
    if marker not in browser:
        fail(f"browser evidence marker missing: {marker}")

package = json.loads((root / "package.json").read_text(encoding="utf-8"))
dedicated = "playwright test test/e2e/pr_bb_settings_runtime.spec.ts"
if package["scripts"].get("test:e2e:pr-bb") != dedicated:
    fail("dedicated PR-BB browser command drifted")
if package["scripts"].get("test:e2e:product", "").count("test/e2e/pr_bb_settings_runtime.spec.ts") != 1:
    fail("product browser suite must include PR-BB exactly once")
release = (root / "scripts/check_release_qa_contract.sh").read_text(encoding="utf-8")
if '.scripts["test:e2e:pr-bb"]' not in release or "test/e2e/pr_bb_settings_runtime.spec.ts" not in release:
    fail("release QA no longer locks PR-BB browser evidence")
repo = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
if repo.count('PR_BB_STATIC_ONLY=1 bash "$ROOT/scripts/check_pr_bb_completion.sh"') != 2:
    fail("completion gate must be wired into both repository scopes")
if repo.count('npx playwright test "$ROOT/test/e2e/pr_bb_settings_runtime.spec.ts" --list') != 1:
    fail("repository browser fixture listing is missing")

print(f"OK PR-BB settings permission and ticket health contract ({len(contract)} evidence types; queue={numbered[0]})")
PY

if [[ "${PR_BB_STATIC_ONLY:-0}" == "1" ]]; then exit 0; fi

bash "$ROOT/scripts/check_product_audit_progress.sh"
bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_evidence_index.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"
bash "$ROOT/scripts/check_release_qa_contract.sh" --self-test
PR_AS_STATIC_ONLY=1 bash "$ROOT/scripts/check_pr_as_completion.sh"
PR_EG_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_eg_completion.sh"
PR_CN_SKIP_TESTS=1 PR_CN_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_cn_completion.sh"

if [[ "${PR_BB_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS=8 cargo test -p shared-types venues::credentials::tests --lib --no-fail-fast
  CARGO_BUILD_JOBS=8 cargo test -p api order_permission_matrix_tests --no-fail-fast
  CARGO_BUILD_JOBS=8 cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib --no-fail-fast
fi
if [[ "${PR_BB_SKIP_BROWSER:-0}" != "1" ]]; then
  CI=1 npm run test:e2e:pr-bb -- --workers=1
fi

printf 'OK PR-BB settings permission and ticket health contract\n'
