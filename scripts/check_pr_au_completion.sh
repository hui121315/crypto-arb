#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
AUDIT="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
HISTORY="$ROOT/docs/audit_history/PRODUCT_AUDIT_HISTORY.md"
EVIDENCE="$ROOT/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
SHARED="$ROOT/shared-types/src/system.rs"
SERVICE="$ROOT/crates/api/src/services/system_health.rs"
ROUTER="$ROOT/crates/api/src/routers/system.rs"
API_BASE="$ROOT/frontend/src/panels/modules/settings/data/api_base.rs"
API_BASE_TEST="$ROOT/frontend/src/panels/modules/settings/data/tests/api_base.rs"
WS_RUNTIME="$ROOT/frontend/src/api/ws_runtime.rs"
SETTINGS_WS="$ROOT/frontend/src/panels/modules/settings/tabs/diagnostics/ws_rtt.rs"
SETTINGS_WS_TEST="$ROOT/frontend/src/panels/modules/settings/tabs/diagnostics/tests/ws_rtt.rs"
EXECUTION_WS="$ROOT/frontend/src/panels/modules/execution/components/execution_status_bar/state.rs"
EXECUTION_WS_TEST="$ROOT/frontend/src/panels/modules/execution/components/execution_status_bar/testing.rs"
BROWSER="$ROOT/test/e2e/pr_au_transport_health.spec.ts"
RELEASE_QA="$ROOT/scripts/check_release_qa_contract.sh"
REPO_GATE="$ROOT/scripts/verify_repo_gates.sh"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-AU completion gate failed: %s\n' "$1" >&2
  exit 1
}

run_static_gate() {
  PR_AU_STATIC_ONLY=1 PR_AU_SKIP_TESTS=1 PR_AU_SKIP_UPSTREAM=1 PR_AU_SKIP_BROWSER=1 bash "$0" >/dev/null
}

expect_rejected() {
  local message="$1"
  if run_static_gate 2>/dev/null; then
    fail "$message"
  fi
}

if [[ "$MODE" == "--self-test" ]]; then
  tmp="$(mktemp -d "${TMPDIR:-/tmp}/crossline-pr-au.XXXXXX")"
  files=(
    "$AUDIT" "$EVIDENCE" "$SHARED" "$SERVICE" "$API_BASE" "$EXECUTION_WS"
    "$BROWSER" "$RELEASE_QA" "$REPO_GATE"
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
old = "| `PR-AU Frontend Transport Auth & WS Multiplexer` | ✅ 完成 |"
if text.count(old) != 1:
    raise SystemExit("PR-AU self-test setup failed: roadmap row drifted")
path.write_text(text.replace(old, "| `PR-AU Frontend Transport Auth & WS Multiplexer` | 🟡 部分完成 |", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted a downgraded roadmap row"
  restore_file "$AUDIT"

  python3 - "$EVIDENCE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
rows = path.read_text(encoding="utf-8").splitlines()
for index, row in enumerate(rows):
    if row.startswith("PR-AU\tshared-api-version-contract\t"):
        del rows[index]
        break
else:
    raise SystemExit("PR-AU self-test setup failed: evidence row missing")
path.write_text("\n".join(rows) + "\n", encoding="utf-8")
PY
  expect_rejected "self-test accepted incomplete exact evidence"
  restore_file "$EVIDENCE"

  python3 - "$SHARED" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = "    pub api_version: String,\n"
if text.count(old) != 1:
    raise SystemExit("PR-AU self-test setup failed: apiVersion contract drifted")
path.write_text(text.replace(old, "    pub server_version: String,\n", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted a renamed API version field"
  restore_file "$SHARED"

  python3 - "$API_BASE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = "    require_api_version(&health)?;\n"
if text.count(old) != 1:
    raise SystemExit("PR-AU self-test setup failed: version validation drifted")
path.write_text(text.replace(old, "    // version validation removed\n", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted API Base validation without version proof"
  restore_file "$API_BASE"

  python3 - "$API_BASE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = "    let api_auth_token = use_context::<crate::state::AppContext>().map(|ctx| ctx.api_auth_token);\n"
if text.count(old) != 1:
    raise SystemExit("PR-AU self-test setup failed: captured token signal drifted")
path.write_text(text.replace(old, "    let api_auth_token = None;\n", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted connection validation without the captured token signal"
  restore_file "$API_BASE"

  python3 - "$EXECUTION_WS" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = '        "{} 通道 · {status} · {}",\n'
if text.count(old) != 1:
    raise SystemExit("PR-AU self-test setup failed: execution channel copy drifted")
path.write_text(text.replace(old, '        "{status} · {}",\n', 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted execution health without its channel identity"
  restore_file "$EXECUTION_WS"

  python3 - "$BROWSER" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = 'test("PR-AU validates API version and surfaces per-channel Settings health"'
if text.count(old) != 1:
    raise SystemExit("PR-AU self-test setup failed: browser marker drifted")
path.write_text(text.replace(old, 'test.skip("PR-AU validates API version and surfaces per-channel Settings health"', 1), encoding="utf-8")
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
    raise SystemExit("PR-AU self-test setup failed: queue head drifted")
path.write_text(text[:head.start(1)] + "PR-AU" + text[head.end(1):], encoding="utf-8")
PY
  expect_rejected "self-test accepted PR-AU reinserted into the local queue"
  restore_file "$AUDIT"

  python3 - "$RELEASE_QA" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = " test/e2e/pr_au_transport_health.spec.ts"
if text.count(old) != 1:
    raise SystemExit("PR-AU self-test setup failed: release QA wiring drifted")
path.write_text(text.replace(old, "", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted release QA without the PR-AU browser"
  restore_file "$RELEASE_QA"

  python3 - "$REPO_GATE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
old = 'PR_AU_SKIP_TESTS=1 PR_AU_SKIP_UPSTREAM=1 PR_AU_SKIP_BROWSER=1 bash "$ROOT/scripts/check_pr_au_completion.sh"'
if text.count(old) != 2:
    raise SystemExit("PR-AU self-test setup failed: repository wiring drifted")
path.write_text(text.replace(old, "true # PR-AU gate removed", 1), encoding="utf-8")
PY
  expect_rejected "self-test accepted single-scope repository wiring"

  printf 'PR-AU completion self-test passed\n'
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
    raise SystemExit(f"PR-AU completion gate failed: {message}")


title = "PR-AU Frontend Transport Auth & WS Multiplexer"
row = next((line for line in audit.splitlines() if line.startswith(f"| `{title}` |")), None)
if row is None or "| ✅ 完成 |" not in row or "剩余：无。" not in row:
    fail("roadmap row is not complete with no local remainder")
if "`bash scripts/check_pr_au_completion.sh --self-test`" not in row:
    fail("roadmap row lacks the canonical destructive verification anchor")
if "> 本轮 PR-AU：" not in audit:
    fail("top progress summary lacks the PR-AU closure note")

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
    or any(pr_id in numbered for pr_id in ("PR-AU", "PR-AV"))
):
    fail(f"queue handoff drifted: expected={incomplete_successors}, actual={numbered[:len(incomplete_successors)]}")
if "## 2026-07-19 PR-AU Transport Auth and Channel Health Closure" not in history:
    fail("history closure appendix is missing")

evidence_contract = {
    "successor-rest-transport": "scripts/check_pr_dl_completion.sh",
    "successor-ws-runtime": "scripts/check_pr_ds_completion.sh",
    "successor-settings-control-plane": "scripts/check_pr_du_completion.sh",
    "shared-api-version-contract": "shared-types/src/system.rs",
    "backend-api-version-projection": "crates/api/src/services/system_health.rs",
    "backend-api-version-route-test": "crates/api/src/routers/system.rs",
    "api-base-validation": "frontend/src/panels/modules/settings/data/api_base.rs",
    "api-base-validation-fixtures": "frontend/src/panels/modules/settings/data/tests/api_base.rs",
    "ws-multiplexer-runtime": "frontend/src/api/ws_runtime.rs",
    "settings-channel-health": "frontend/src/panels/modules/settings/tabs/diagnostics/ws_rtt.rs",
    "settings-channel-health-fixtures": "frontend/src/panels/modules/settings/tabs/diagnostics/tests/ws_rtt.rs",
    "execution-channel-health": "frontend/src/panels/modules/execution/components/execution_status_bar/state.rs",
    "execution-channel-health-fixtures": "frontend/src/panels/modules/execution/components/execution_status_bar/testing.rs",
    "product-browser": "test/e2e/pr_au_transport_health.spec.ts",
    "package-product-suite": "package.json",
    "release-qa-product-suite": "scripts/check_release_qa_contract.sh",
    "completion-governance": "scripts/check_pr_au_completion.sh",
}
with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [item for item in csv.DictReader(handle, delimiter="\t") if item["pr_id"] == "PR-AU"]
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

markers = {
    "shared-types/src/system.rs": (
        "pub api_version: String",
        "legacy_system_health_without_api_version_decodes_as_unverified",
    ),
    "crates/api/src/services/system_health.rs": (
        'api_version: env!("CARGO_PKG_VERSION").to_owned()',
    ),
    "crates/api/src/routers/system.rs": (
        "system_health_route_returns_shared_resource_envelope",
        "health.api_version.as_str()",
    ),
    "frontend/src/panels/modules/settings/data/api_base.rs": (
        "let api_auth_token = use_context::<crate::state::AppContext>()",
        ".map(|signal| signal.get_untracked())",
        "require_api_version(&health)?;",
        '"API_VERSION_MISSING"',
        'validation_request_with_timeout(client.ws_ticket(), "/api/auth/ws-ticket")',
        "/api/system/health version {}",
    ),
    "frontend/src/panels/modules/settings/data/tests/api_base.rs": (
        "api_base_validate_rejects_health_without_api_version",
        "api_base_validate_success_message_reports_ws_ticket_without_secret",
    ),
    "frontend/src/api/ws_runtime.rs": (
        "struct WsRuntimeInner",
        "channel_states: RefCell<BTreeMap<String, WsChannelState>>",
        "fn sync_subscriber_state",
        "runtime_existing_channel_inherits_full_state_without_resubscribe",
    ),
    "frontend/src/panels/modules/settings/tabs/diagnostics/ws_rtt.rs": (
        "app_ws_broadcast_rows(&snapshot.rows)",
        "AppWS channels {} · lag {} · 丢帧 {} · 近期异常 {}",
        'format!("{} · {}", kind.label_zh(), row.operation)',
    ),
    "frontend/src/panels/modules/settings/tabs/diagnostics/tests/ws_rtt.rs": (
        "app_ws_rows_and_scope_summary_keep_lag_counts_visible",
    ),
    "frontend/src/panels/modules/execution/components/execution_status_bar/state.rs": (
        '"{} 通道 · {status} · {}"',
        "state.channel",
        "WS最后帧",
    ),
    "frontend/src/panels/modules/execution/components/execution_status_bar/testing.rs": (
        "channel_problem_keeps_request_and_retry_context",
        "channel_meta_surfaces_subscription_and_last_frame",
        "execution 通道",
    ),
}
for relative, required in markers.items():
    source = (root / relative).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"{relative} lost marker: {marker}")

browser_path = "test/e2e/pr_au_transport_health.spec.ts"
browser = (root / browser_path).read_text(encoding="utf-8")
if re.search(r"\btest\.(?:skip|fixme)\s*\(", browser):
    fail("PR-AU product fixture must not be skipped")
if len(re.findall(r"(?m)^test\(", browser)) != 2:
    fail("PR-AU browser fixture must expose exactly two runnable scenarios")
for marker in (
    "PR-AU validates API version and surfaces per-channel Settings health",
    "PR-AU execution page exposes the multiplexed execution channel ACK",
    "app_ws_broadcast:execution",
    "WS已订阅，等待首帧",
    'runtime.socketCount()).toBe(1)',
):
    if marker not in browser:
        fail(f"PR-AU browser fixture missing: {marker}")

package = json.loads((root / "package.json").read_text(encoding="utf-8"))
focused = "playwright test test/e2e/pr_au_transport_health.spec.ts"
if package.get("scripts", {}).get("test:e2e:pr-au") != focused:
    fail("package PR-AU focused command is missing")
package_text = (root / "package.json").read_text(encoding="utf-8")
if package_text.count(browser_path) != 2:
    fail("PR-AU browser must be wired once as a focused command and once in the product suite")
for path in ("scripts/check_release_qa_contract.sh", "scripts/fixtures/release_qa_contract/package.json"):
    if (root / path).read_text(encoding="utf-8").count(browser_path) != 1:
        fail(f"release QA command lock drifted: {path}")

repo_gate = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
wiring = 'PR_AU_SKIP_TESTS=1 PR_AU_SKIP_UPSTREAM=1 PR_AU_SKIP_BROWSER=1 bash "$ROOT/scripts/check_pr_au_completion.sh"'
if repo_gate.count(wiring) != 2:
    fail("completion gate must be wired into both repository scopes")
focused_entry = '["test:e2e:pr-au", "playwright test test/e2e/pr_au_transport_health.spec.ts"]'
browser_list = 'npx playwright test "$ROOT/test/e2e/pr_au_transport_health.spec.ts" --list'
if repo_gate.count(focused_entry) != 1 or repo_gate.count(browser_list) != 1:
    fail("repository gate must lock the focused command and browser list exactly once")

print(f"OK PR-AU contract ({len(evidence_contract)} evidence types; queue={numbered[0]})")
PY

if [[ "${PR_AU_STATIC_ONLY:-0}" == "1" ]]; then
  exit 0
fi

bash "$ROOT/scripts/check_product_audit_progress.sh"
bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_evidence_index.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"
bash "$ROOT/scripts/check_release_qa_contract.sh"

if [[ "${PR_AU_SKIP_UPSTREAM:-0}" != "1" ]]; then
  PR_DL_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dl_completion.sh"
  PR_DS_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_ds_completion.sh"
  PR_DU_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_du_completion.sh"
fi

if [[ "${PR_AU_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p shared-types system_health --lib --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api --bin crypto-arb-api system_health_route_returns_shared_resource_envelope --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib api_base_validate --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib execution_status_bar --no-fail-fast
fi

if [[ "${PR_AU_SKIP_BROWSER:-0}" != "1" ]]; then
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-au
fi

printf 'OK PR-AU frontend transport auth and channel health contract\n'
