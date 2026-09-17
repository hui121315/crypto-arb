#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
CONSOLE="$ROOT/frontend/src/panels/modules/settings/tabs/venue_credentials.rs"
MAINTENANCE="$ROOT/frontend/src/panels/modules/settings/tabs/venue_credentials/maintenance.rs"
SETTINGS="$ROOT/frontend/src/panels/modules/settings/view.rs"
BROWSER="$ROOT/test/e2e/pr_du_settings_control_plane.spec.ts"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-DU completion gate failed: %s\n' "$1" >&2
  exit 1
}

if [[ "$MODE" == "--self-test" ]]; then
  console_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-du-console.XXXXXX")"
  maintenance_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-du-maintenance.XXXXXX")"
  settings_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-du-settings.XXXXXX")"
  browser_backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-du-browser.XXXXXX")"
  cp "$CONSOLE" "$console_backup"
  cp "$MAINTENANCE" "$maintenance_backup"
  cp "$SETTINGS" "$settings_backup"
  cp "$BROWSER" "$browser_backup"
  restore() {
    cp "$console_backup" "$CONSOLE"
    cp "$maintenance_backup" "$MAINTENANCE"
    cp "$settings_backup" "$SETTINGS"
    cp "$browser_backup" "$BROWSER"
    rm -f "$console_backup" "$maintenance_backup" "$settings_backup" "$browser_backup"
  }
  trap restore EXIT

  PR_DU_SKIP_TESTS=1 bash "$0" >/dev/null

  python3 - "$MAINTENANCE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = '"刷新当前证据"'
if source.count(marker) != 1:
    raise SystemExit("PR-DU self-test setup failed: refresh marker drifted")
path.write_text(source.replace(marker, '"刷新证据漂移"'), encoding="utf-8")
PY
  if PR_DU_SKIP_TESTS=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a disconnected selected-venue refresh control"
  fi
  cp "$maintenance_backup" "$MAINTENANCE"

  python3 - "$SETTINGS" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = 'SettingsTab::from_slug("readiness"), None'
if source.count(marker) != 1:
    raise SystemExit("PR-DU self-test setup failed: readiness guard drifted")
path.write_text(source.replace(marker, 'SettingsTab::from_slug("readiness"), Some(SettingsTab::Diagnostics)'), encoding="utf-8")
PY
  if PR_DU_SKIP_TESTS=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a legacy readiness slug"
  fi
  cp "$settings_backup" "$SETTINGS"

  python3 - "$BROWSER" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "expect(idempotencyKeys[1]).toBe(idempotencyKeys[0]);"
if source.count(marker) != 1:
    raise SystemExit("PR-DU self-test setup failed: retry assertion drifted")
path.write_text(source.replace(marker, "expect(idempotencyKeys[1]).not.toBe(idempotencyKeys[0]);"), encoding="utf-8")
PY
  if PR_DU_SKIP_TESTS=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted credential retry idempotency drift"
  fi

  printf 'PR-DU completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from __future__ import annotations

import csv
import json
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])
title = "PR-DU Settings Credential UX & Runtime Health Contract"
verify_anchor = "`bash scripts/check_pr_du_completion.sh --self-test`"
evidence_contract = {
    "credential-shared-contract": "shared-types/src/venues/credentials.rs",
    "credential-readiness-matrix": "shared-types/src/credential_matrix.rs",
    "scoped-validation": "crates/api/src/services/venue_credentials/validation/venues.rs",
    "atomic-secret-persistence": "crates/api/src/services/venue_credentials/dotenv.rs",
    "credential-action-run": "crates/api/src/routers/exchanges.rs",
    "runtime-health-projection": "crates/api/src/services/venue_operation_health/snapshot/part_01.rs",
    "settings-action-state": "frontend/src/panels/modules/settings/data/actions.rs",
    "settings-action-receipts": "frontend/src/panels/modules/settings/data/format.rs",
    "settings-credential-console": "frontend/src/panels/modules/settings/tabs/venue_credentials.rs",
    "settings-runtime-evidence": "frontend/src/panels/modules/settings/tabs/venue_credentials/panels/operation_health.rs",
    "settings-readonly-environment": "frontend/src/panels/modules/settings/tabs/adapters.rs",
    "legacy-readiness-boundary": "frontend/src/panels/modules/settings/view.rs",
    "credential-save-problem-browser": "test/e2e/data_pipeline.spec.ts",
    "product-browser": "test/e2e/pr_du_settings_control_plane.spec.ts",
    "credential-upstream-gate": "scripts/check_pr_dg_completion.sh",
    "runtime-upstream-gate": "scripts/check_pr_eg_completion.sh",
    "product-suite-contract": "package.json",
    "completion-governance": "scripts/check_pr_du_completion.sh",
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-DU completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be completed with no local remainder")
if row.count(verify_anchor) != 1:
    fail("roadmap verification must name the destructive completion gate")
queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-DU\b", queue):
    fail("completed PR-DU remains in the local queue")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-DU"]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for kind, artifact in evidence_contract.items():
    if indexed[kind]["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"{kind} artifact drifted or is missing")

markers = {
    "shared-types/src/venues/credentials.rs": (
        "pub struct VenueCredentialValidationEvidence",
        "pub request_id: Option<String>",
        "pub struct SecretStorageStatus",
        "pub action_run_id: Option<String>",
    ),
    "shared-types/src/credential_matrix.rs": (
        "pub fn live_required() -> [CredentialProbeLink; 5]",
        "fn is_live_trading_ready(&self) -> bool",
        "CredentialReadiness::Incomplete",
    ),
    "crates/api/src/services/venue_credentials/validation/venues.rs": (
        "mod cex;",
        "mod hyperliquid;",
    ),
    "crates/api/src/services/venue_credentials/validation/venues/cex.rs": (
        "validated_private_read_evidence(",
    ),
    "crates/api/src/services/venue_credentials/validation/venues/hyperliquid.rs": (
        "optional_safe_order_noop_probe",
        'values.optional("vault_address")',
    ),
    "crates/api/src/services/venue_credentials/dotenv.rs": (
        "write_dotenv_temp_then_rename",
        "file.sync_all()",
        "std::fs::rename(temp_path, target_path)",
    ),
    "crates/api/src/routers/exchanges.rs": (
        "ActionRunKind::VenueCredentialsUpdate",
        "update_credentials_replays_same_request_without_new_action_run",
        "response.request_id = run.request_id.clone()",
    ),
    "crates/api/src/services/venue_operation_health/snapshot/part_01.rs": (
        "validation_evidence_snapshot",
        "live_order_proof_health",
        "run_finality_health",
    ),
    "frontend/src/panels/modules/settings/data/actions.rs": (
        "ActionState::pending(\"正在校验凭证证据并保存字段\")",
        "should_reuse_credential_replay_key",
        "credential_save_replay_key",
    ),
    "frontend/src/panels/modules/settings/data/format.rs": (
        'message.push_str(" · Action ")',
        'message.push_str(" · Request ")',
        'message.push_str(" · Idempotency ")',
    ),
    "frontend/src/panels/modules/settings/tabs/venue_credentials.rs": (
        "credentials_refresh_nonce",
        "runtime_health_refresh_nonce",
        "account_state_refresh_nonce",
        "refresh_evidence_button(",
        "持久化位置以 Secret 存储状态为准",
    ),
    "frontend/src/panels/modules/settings/tabs/venue_credentials/maintenance.rs": (
        '"刷新当前证据"',
        "bump_refresh(nonce)",
        'message.set("已请求刷新当前交易所证据".into())',
    ),
    "frontend/src/panels/modules/settings/tabs/venue_credentials/panels/operation_health.rs": (
        '"下单/撤单权限"',
        '"写单运行态"',
        '"私有订单流"',
        '"订单终态"',
    ),
    "frontend/src/panels/modules/settings/tabs/adapters.rs": (
        '"执行环境诊断"',
        '"只读：凭证、风控与每张 HedgeTicket 的双腿预检共同决定是否可提交，不在此切换全局 adapter。"',
        'data-settings-table="execution-environment"',
    ),
    "frontend/src/panels/modules/settings/view.rs": (
        "SettingsTab::from_slug(\"readiness\"), None",
        "SettingsTab::Credentials",
        "SettingsTab::Diagnostics",
    ),
    "test/e2e/data_pipeline.spec.ts": (
        "settings credential save denial redacts secret and shows no success feedback",
        "request_id req-credential-save-denied",
        "settings risk kill switch surfaces extractor 422 typed problem",
    ),
}
for path, required in markers.items():
    source = (root / path).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"missing {path} marker: {marker}")

for legacy_path in (
    "frontend/src/panels/modules/settings/tabs/readiness.rs",
    "crates/api/src/services/live_readiness.rs",
):
    if (root / legacy_path).exists():
        fail(f"legacy readiness file returned: {legacy_path}")
app = (root / "crates/api/src/app.rs").read_text(encoding="utf-8")
if '.route("/api/trading/live-readiness"' in app:
    fail("legacy live-readiness route returned")

browser_path = evidence_contract["product-browser"]
browser = (root / browser_path).read_text(encoding="utf-8")
if re.search(r"\btest\.(?:skip|fixme)\s*\(", browser):
    fail("browser fixture must not be skipped")
browser_markers = (
    "PR-DU refreshes the selected venue credential and runtime evidence together",
    "PR-DU keeps request context and reuses idempotency after an in-flight save",
    "PR-DU keeps execution environment read-only and legacy readiness absent",
    "expect(idempotencyKeys[1]).toBe(idempotencyKeys[0]);",
    'getByRole("button", { name: "刷新当前证据" })',
)
for marker in browser_markers:
    if marker not in browser:
        fail(f"browser fixture missing marker: {marker}")
if browser.count('test("PR-DU ') != 3:
    fail("browser fixture must keep all three non-skipping scenarios")

package = json.loads((root / "package.json").read_text(encoding="utf-8"))
scripts = package.get("scripts", {})
dedicated = "playwright test test/e2e/pr_du_settings_control_plane.spec.ts"
if scripts.get("test:e2e:pr-du") != dedicated:
    fail("dedicated browser command is missing")
if scripts.get("test:e2e:product", "").count(browser_path) != 1:
    fail("product suite must include the PR-DU fixture exactly once")
release_fixture = json.loads(
    (root / "scripts/fixtures/release_qa_contract/package.json").read_text(encoding="utf-8")
)
if release_fixture.get("scripts", {}).get("test:e2e:product", "").count(browser_path) != 1:
    fail("release QA fixture must include the PR-DU fixture exactly once")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
for path in set(evidence_contract.values()) | set(markers):
    if path == "package.json":
        continue
    if coverage.get(path, {}).get("coverage_status") != "exact":
        fail(f"exact coverage missing for {path}")

print(
    f"OK PR-DU static contract ({len(evidence_contract)} evidence types; "
    "credential save, runtime refresh, ActionState and Paper/Live closure)"
)
PY

PR_DG_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dg_completion.sh"
PR_EG_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_eg_completion.sh"
PR_EI_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_ei_completion.sh"
bash "$ROOT/scripts/product_copy_gate.sh"
bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"
bash "$ROOT/scripts/check_release_qa_contract.sh"

if [[ "${PR_DU_SKIP_TESTS:-0}" != "1" ]]; then
  JOBS="${CARGO_BUILD_JOBS:-8}"
  CARGO_BUILD_JOBS="$JOBS" cargo test -p shared-types credential --lib --no-fail-fast
  CARGO_BUILD_JOBS="$JOBS" cargo test -p api --bin crypto-arb-api venue_credentials --no-fail-fast
  CARGO_BUILD_JOBS="$JOBS" cargo test -p api --bin crypto-arb-api venue_operation_health --no-fail-fast
  CARGO_BUILD_JOBS="$JOBS" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib settings --no-fail-fast
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-du
fi

printf 'OK PR-DU Settings credential UX and runtime health completion contract\n'
