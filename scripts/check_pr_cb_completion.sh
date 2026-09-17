#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
DOC="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
MATRIX="$ROOT/shared-types/src/credential_matrix.rs"
RUNTIME="$ROOT/shared-types/src/venues/runtime_health.rs"
SPECS="$ROOT/crates/api/src/services/venue_credentials/specs.rs"
SETTINGS_ACTIONS="$ROOT/frontend/src/panels/modules/settings/data/actions.rs"
BROWSER="$ROOT/test/e2e/pr_du_settings_control_plane.spec.ts"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

fail() {
  printf 'PR-CB completion gate failed: %s\n' "$1" >&2
  exit 1
}

if [[ "$MODE" == "--self-test" ]]; then
  temp="$(mktemp -d "${TMPDIR:-/tmp}/crossline-pr-cb.XXXXXX")"
  cp "$DOC" "$temp/audit.md"
  cp "$MATRIX" "$temp/credential_matrix.rs"
  cp "$RUNTIME" "$temp/runtime_health.rs"
  cp "$SPECS" "$temp/specs.rs"
  cp "$SETTINGS_ACTIONS" "$temp/actions.rs"
  cp "$BROWSER" "$temp/browser.spec.ts"
  restore() {
    cp "$temp/audit.md" "$DOC"
    cp "$temp/credential_matrix.rs" "$MATRIX"
    cp "$temp/runtime_health.rs" "$RUNTIME"
    cp "$temp/specs.rs" "$SPECS"
    cp "$temp/actions.rs" "$SETTINGS_ACTIONS"
    cp "$temp/browser.spec.ts" "$BROWSER"
    rm -rf "$temp"
  }
  trap restore EXIT

  PR_CB_SKIP_TESTS=1 PR_CB_SKIP_UPSTREAM=1 bash "$0" >/dev/null

  python3 - "$MATRIX" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "pub fn live_required() -> [CredentialProbeLink; 5]"
if source.count(marker) != 1:
    raise SystemExit("PR-CB self-test setup failed: five-link marker drifted")
path.write_text(source.replace(marker, "pub fn live_required() -> [CredentialProbeLink; 4]", 1), encoding="utf-8")
PY
  if PR_CB_SKIP_TESTS=1 PR_CB_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a reduced credential validation matrix"
  fi
  cp "$temp/credential_matrix.rs" "$MATRIX"

  python3 - "$RUNTIME" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "    PrivateWs,\n"
if source.count(marker) != 1:
    raise SystemExit("PR-CB self-test setup failed: private WS runtime slot drifted")
path.write_text(source.replace(marker, "", 1), encoding="utf-8")
PY
  if PR_CB_SKIP_TESTS=1 PR_CB_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a missing private WS runtime slot"
  fi
  cp "$temp/runtime_health.rs" "$RUNTIME"

  python3 - "$SPECS" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = '"vault_address"'
if source.count(marker) != 1:
    raise SystemExit("PR-CB self-test setup failed: dynamic vault field drifted")
path.write_text(source.replace(marker, '"vault_scope_drift"', 1), encoding="utf-8")
PY
  if PR_CB_SKIP_TESTS=1 PR_CB_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted dynamic credential-field drift"
  fi
  cp "$temp/specs.rs" "$SPECS"

  python3 - "$SETTINGS_ACTIONS" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = 'ActionState::pending("正在校验凭证证据并保存字段")'
if source.count(marker) != 1:
    raise SystemExit("PR-CB self-test setup failed: credential pending state drifted")
path.write_text(source.replace(marker, 'ActionState::idle()', 1), encoding="utf-8")
PY
  if PR_CB_SKIP_TESTS=1 PR_CB_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted credential mutation outside ActionState"
  fi
  cp "$temp/actions.rs" "$SETTINGS_ACTIONS"

  python3 - "$BROWSER" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
title = "PR-DU keeps execution environment read-only and legacy readiness absent"
marker = f'test("{title}"'
if source.count(marker) != 1:
    raise SystemExit("PR-CB self-test setup failed: browser authority drifted")
path.write_text(source.replace(marker, f'test.skip("{title}"', 1), encoding="utf-8")
PY
  if PR_CB_SKIP_TESTS=1 PR_CB_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted a skipped Settings product fixture"
  fi
  cp "$temp/browser.spec.ts" "$BROWSER"

  python3 - "$DOC" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
heading = "### 🟡 6.5 下一步执行队列"
if source.count(heading) != 1:
    raise SystemExit("PR-CB self-test setup failed: queue heading drifted")
stale = "\n\n1. **PR-CB Settings Credential Health & ActionState Contract** — stale completed row"
path.write_text(source.replace(heading, heading + stale, 1), encoding="utf-8")
PY
  if PR_CB_SKIP_TESTS=1 PR_CB_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted completed PR-CB in the local queue"
  fi
  cp "$temp/audit.md" "$DOC"

  python3 - "$DOC" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "| `PR-DV HedgeTicket Scoped Preflight & Confirm Contract` | ✅ 完成 |"
if source.count(marker) != 1:
    raise SystemExit("PR-CB self-test setup failed: PR-DV authority drifted")
path.write_text(source.replace(marker, marker.replace("✅ 完成", "🟡 部分完成"), 1), encoding="utf-8")
PY
  if PR_CB_SKIP_TESTS=1 PR_CB_SKIP_UPSTREAM=1 bash "$0" >/dev/null 2>&1; then
    fail "self-test accepted an incomplete scoped-preflight authority"
  fi

  printf 'PR-CB completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from __future__ import annotations

import csv
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])
pr_id = "PR-CB"
title = "PR-CB Settings Credential Health & ActionState Contract"
verify_anchor = "`bash scripts/check_pr_cb_completion.sh --self-test`"
evidence_contract = {
    "credential-shared-contract": (
        "shared-types/src/venues/credentials.rs",
        "cargo test -p shared-types credential",
    ),
    "credential-readiness-matrix": (
        "shared-types/src/credential_matrix.rs",
        "cargo test -p shared-types credential",
    ),
    "dynamic-field-spec": (
        "crates/api/src/services/venue_credentials/specs.rs",
        "cargo test -p api --bin crypto-arb-api venue_credentials",
    ),
    "atomic-secret-persistence": (
        "crates/api/src/services/venue_credentials/dotenv.rs",
        "cargo test -p api --bin crypto-arb-api venue_credentials",
    ),
    "keychain-secret-backend": (
        "crates/api/src/services/venue_credentials/keychain.rs",
        "cargo test -p api --bin crypto-arb-api venue_credentials",
    ),
    "secret-maintenance-lifecycle": (
        "crates/api/src/services/venue_credentials/maintenance.rs",
        "cargo test -p api --bin crypto-arb-api venue_credentials",
    ),
    "credential-validation-matrix": (
        "crates/api/src/services/venue_credentials/validation/venues.rs",
        "cargo test -p api --bin crypto-arb-api venue_credentials",
    ),
    "hyperliquid-relation-evidence": (
        "crates/api/src/services/venue_credentials/validation/tests.rs",
        "hyperliquid_relation_probe_accepts_approved_agent_for_led_vault",
    ),
    "credential-action-run": (
        "crates/api/src/routers/exchanges.rs",
        "cargo test -p api --bin crypto-arb-api venue_credentials",
    ),
    "runtime-health-shared-contract": (
        "shared-types/src/venues/runtime_health.rs",
        "cargo test -p shared-types runtime_health",
    ),
    "runtime-health-projection": (
        "crates/api/src/services/venue_operation_health/snapshot/part_01.rs",
        "cargo test -p api --bin crypto-arb-api venue_operation_health",
    ),
    "settings-load-state": (
        "frontend/src/panels/modules/settings/data/resources.rs",
        "cargo test --manifest-path frontend/Cargo.toml --lib settings",
    ),
    "settings-action-state": (
        "frontend/src/panels/modules/settings/data/actions.rs",
        "cargo test --manifest-path frontend/Cargo.toml --lib settings",
    ),
    "settings-maintenance-action": (
        "frontend/src/panels/modules/settings/data/credential_maintenance.rs",
        "cargo test --manifest-path frontend/Cargo.toml --lib credential_maintenance",
    ),
    "settings-dynamic-console": (
        "frontend/src/panels/modules/settings/tabs/venue_credentials.rs",
        "cargo test --manifest-path frontend/Cargo.toml --lib venue_credentials",
    ),
    "settings-validation-panel": (
        "frontend/src/panels/modules/settings/tabs/venue_credentials/validation.rs",
        "validation_summary_is_fail_closed_for_missing_required_links",
    ),
    "settings-runtime-diagnostics": (
        "frontend/src/panels/modules/settings/tabs/diagnostics/runtime_matrix.rs",
        "cargo test --manifest-path frontend/Cargo.toml --lib runtime_health",
    ),
    "settings-readonly-environment": (
        "frontend/src/panels/modules/settings/tabs/adapters.rs",
        "live_adapter_copy_separates_configured_fields_from_readiness",
    ),
    "ticket-scoped-preflight": (
        "crates/api/src/services/hedge_preflight/live_health/credentials/part_01.rs",
        "cargo test -p api --bin crypto-arb-api services::hedge_preflight::tests",
    ),
    "settings-product-browser": (
        "test/e2e/pr_du_settings_control_plane.spec.ts",
        "npm run test:e2e:pr-du",
    ),
    "runtime-product-browser": (
        "test/e2e/pr_bx_runtime.spec.ts",
        "npm run test:e2e:pr-bx",
    ),
    "scoped-preflight-browser": (
        "test/e2e/pr_dv_scoped_preflight.spec.ts",
        "npm run test:e2e:pr-dv",
    ),
    "action-evidence-authority": (
        "scripts/check_pr_ck_completion.sh",
        "check_pr_ck_completion.sh --self-test",
    ),
    "completion-governance": (
        "scripts/check_pr_cb_completion.sh",
        "check_pr_cb_completion.sh --self-test",
    ),
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-CB completion gate failed: {message}")


def require_incomplete_queue_head(doc: str, queue: str) -> None:
    matched = re.search(r"(?m)^1\.\s+\*\*(PR-[A-Z]+)\b", queue)
    if not matched:
        fail("local queue must retain a PR roadmap item at its head")
    head = matched.group(1)
    row = next((line for line in doc.splitlines() if line.startswith(f"| `{head} ")), None)
    if row is None or "✅ 完成" in row:
        fail(f"local queue head must be an incomplete roadmap row: {head}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(
    encoding="utf-8"
)
fact_lines = history.split("## 附录 I：", 1)[0].splitlines() + doc.splitlines()
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be complete with no local remainder")
if row.count(verify_anchor) != 1:
    fail("roadmap verification must bind the destructive completion gate")

queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-CB\b", queue):
    fail("completed PR-CB remains in the local queue")
successor_title = "PR-CD Portfolio AccountState Evidence & Risk LoadState Contract"
successor_row = next(
    (line for line in doc.splitlines() if line.startswith(f"| `{successor_title}`")),
    None,
)
if successor_row is None:
    fail("PR-CD successor roadmap row is missing")
successor_complete = "✅ 完成" in successor_row and "剩余：无。" in successor_row
successor_is_head = bool(re.search(r"(?m)^1\.\s+\*\*PR-CD\b", queue))
next_title = "PR-CM Venue API Status Center & Runtime Probe Contract"
next_row = next(
    (line for line in doc.splitlines() if line.startswith(f"| `{next_title}`")),
    None,
)
if next_row is None:
    fail("PR-CM next-successor roadmap row is missing")
next_complete = "✅ 完成" in next_row and "剩余：无" in next_row
next_is_head = bool(re.search(r"(?m)^1\.\s+\*\*PR-CM\b", queue))
following_title = "PR-CN Frontend Workstation State & Navigation Runtime Contract"
following_row = next(
    (line for line in doc.splitlines() if line.startswith(f"| `{following_title}`")),
    None,
)
if following_row is None:
    fail("PR-CN following-successor roadmap row is missing")
following_complete = "✅ 完成" in following_row and "剩余：无" in following_row
following_is_head = bool(re.search(r"(?m)^1\.\s+\*\*PR-CN\b", queue))
final_title = "PR-CQ Local Runtime & Operator QA Contract"
final_row = next(
    (line for line in doc.splitlines() if line.startswith(f"| `{final_title}`")),
    None,
)
if final_row is None:
    fail("PR-CQ final-successor roadmap row is missing")
final_complete = "✅ 完成" in final_row and "剩余：无" in final_row
final_is_head = bool(re.search(r"(?m)^1\.\s+\*\*PR-CQ\b", queue))
if successor_complete:
    if successor_is_head:
        fail("completed PR-CD successor remains at the local queue head")
    if next_complete:
        if next_is_head:
            fail("completed PR-CM next-successor remains at the local queue head")
        if following_complete:
            if following_is_head:
                fail("completed PR-CN following-successor remains at the local queue head")
            if final_complete:
                if final_is_head:
                    fail("completed PR-CQ final-successor remains at the local queue head")
                require_incomplete_queue_head(doc, queue)
            elif not final_is_head:
                fail("PR-CQ must become the local queue head after PR-CN completion")
        elif not following_is_head:
            fail("PR-CN must become the local queue head after PR-CM completion")
    elif not next_is_head:
        fail("PR-CM must become the local queue head after PR-CD completion")
elif not successor_is_head:
    fail("PR-CD must remain the local queue head until its completion contract closes")
queue_items = re.findall(r"(?m)^\d+\.\s+\*\*", queue)
if not queue_items:
    fail("local queue plus external pool must not be empty")

delegated_rows = [
    line
    for line in fact_lines
    if line.startswith("|") and "PR-CB" in line and "✅ 完成" in " ".join(line.split("|")[1:3])
]
if len(delegated_rows) < 10:
    fail(f"expected at least ten PR-CB audit rows, found {len(delegated_rows)}")
for delegated in delegated_rows:
    cells = [cell.strip() for cell in delegated.strip().strip("|").split("|")]
    status = " ".join(cells[:2])
    if "✅ 完成" not in status or any(marker in status for marker in ("🟡", "⏳", "❌")):
        fail(f"unfinished audit row still delegates work to PR-CB: {cells[0]}")

for successor in (
    "PR-BS",
    "PR-BX",
    "PR-BY",
    "PR-CK",
    "PR-CL",
    "PR-DG",
    "PR-DU",
    "PR-DV",
    "PR-EG",
    "PR-EI",
    "PR-EQ",
):
    successor_row = next(
        (line for line in doc.splitlines() if line.startswith(f"| `{successor} ")),
        None,
    )
    if successor_row is None or "✅ 完成" not in successor_row:
        fail(f"required successor authority is not complete: {successor}")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [entry for entry in csv.DictReader(handle, delimiter="\t") if entry["pr_id"] == pr_id]
indexed = {entry["evidence_type"]: entry for entry in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for evidence_type, (artifact, command_anchor) in evidence_contract.items():
    evidence = indexed[evidence_type]
    if evidence["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"{evidence_type} artifact drifted or is missing")
    if command_anchor not in evidence["command"] or not evidence["notes"].strip():
        fail(f"{evidence_type} command or notes drifted")

markers = {
    "shared-types/src/venues/credentials.rs": (
        "pub struct VenueCredentialValidationEvidence",
        "pub request_id: Option<String>",
        "pub struct SecretStorageStatus",
        "pub struct VenueCredentialMaintenanceResponse",
    ),
    "shared-types/src/credential_matrix.rs": (
        "pub fn live_required() -> [CredentialProbeLink; 5]",
        "CredentialReadiness::Blocked",
        "CredentialReadiness::Incomplete",
        "私有 WS 运行态就绪不在保存期探针矩阵内判定",
    ),
    "shared-types/src/venues/runtime_health.rs": (
        "pub enum VenueRuntimeOperation",
        "PrivateWs,",
        "OrderStream,",
        "Finality,",
        "pub currently_usable: bool",
    ),
    "crates/api/src/services/venue_credentials/specs.rs": (
        '"live_passphrase"',
        '"vault_address"',
        '"HYPERLIQUID_VAULT_ADDRESS"',
        "optional_field(",
    ),
    "crates/api/src/services/venue_credentials/dotenv.rs": (
        "write_dotenv_temp_then_rename",
        "file.sync_all()",
        "std::fs::rename(temp_path, target_path)",
    ),
    "crates/api/src/services/venue_credentials/keychain.rs": (
        "set_generic_password",
        "delete_generic_password",
        "SecretStorageStatus::keychain(SERVICE)",
    ),
    "crates/api/src/services/venue_credentials/maintenance.rs": (
        "pub(crate) async fn clear(",
        "pub(crate) async fn migrate(",
        "refresh_credential_fingerprint",
        "validation_evidence_store().remove",
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
        "optional_hyperliquid_account_relation_probe",
    ),
    "crates/api/src/services/venue_operation_health/snapshot/part_01.rs": (
        "validation_evidence_snapshot",
        "live_order_proof_health",
        "run_finality_health",
    ),
    "frontend/src/panels/modules/settings/data/resources.rs": (
        "type SettingsResource<T> = RwSignal<LoadState<T>>",
        "state.apply_result(result.map_err(|error| error.problem))",
    ),
    "frontend/src/panels/modules/settings/data/actions.rs": (
        'ActionState::pending("正在校验凭证证据并保存字段")',
        "should_reuse_credential_replay_key",
        "MutationRequestContext::with_idempotency_key(",
    ),
    "frontend/src/panels/modules/settings/data/credential_maintenance.rs": (
        "VenueCredentialMaintenance::Clear",
        "VenueCredentialMaintenance::Migrate",
        "ActionState::pending(",
        "credential_maintenance_replay_key",
    ),
    "frontend/src/panels/modules/settings/tabs/venue_credentials.rs": (
        "credential_inputs(",
        "credential_maintenance_controls(",
        "refresh_evidence_button(",
        "runtime_health_refresh_nonce",
    ),
    "frontend/src/panels/modules/settings/tabs/venue_credentials/maintenance.rs": (
        '"刷新当前证据"',
        "bump_refresh(nonce)",
        "credential_maintenance_controls(",
    ),
    "frontend/src/panels/modules/settings/tabs/venue_credentials/validation.rs": (
        '"保存期验证"',
        '"；字段已填写不代表私有读、下单权限或账户模式已验证。"',
        'CredentialReadiness::LiveReady => "权限验证完整"',
    ),
    "frontend/src/panels/modules/settings/tabs/diagnostics/runtime_matrix.rs": (
        'data-settings-table="venue-runtime-health"',
        '"交易运行状态中心"',
        "operation.problem.as_ref()",
    ),
    "frontend/src/panels/modules/settings/tabs/adapters.rs": (
        '"执行环境诊断"',
        '"只读：凭证、风控与每张 HedgeTicket 的双腿预检共同决定是否可提交，不在此切换全局 adapter。"',
        '"可选路由；下单仍需票据级权限与运行态证据"',
    ),
    "crates/api/src/services/hedge_preflight/live_health/credentials/part_01.rs": (
        "HedgePreflightOperation::PrivateRead",
        "HedgePreflightOperation::PrivateWs",
        "HedgePreflightOperation::OrderFinality",
        "live_operation_request_id(plans, rows)",
    ),
}
for relative_path, required in markers.items():
    source = (root / relative_path).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"missing {relative_path} marker: {marker}")

spec_source = (root / "crates/api/src/services/venue_credentials/specs.rs").read_text(encoding="utf-8")
if spec_source.count("VenueId::") != 8:
    fail("dynamic credential registry must retain exactly eight venue specs")

test_anchors = (
    ("shared-types/src/credential_matrix.rs", "all_required_ok_is_live_ready"),
    ("shared-types/src/credential_matrix.rs", "missing_link_is_not_ready_and_incomplete"),
    ("shared-types/src/venues/tests_runtime_health.rs", "runtime_health_exposes_every_pr_bx_operation_slot"),
    ("shared-types/src/venues/tests_runtime_health.rs", "snapshot_groups_normalized_venues_into_all_runtime_slots"),
    ("crates/api/src/services/venue_credentials/validation/order_permission_readiness_tests.rs", "safe_order_permission_probes_do_not_grant_live_readiness"),
    ("crates/api/src/services/venue_credentials/validation/tests.rs", "hyperliquid_relation_probe_accepts_approved_agent_for_led_vault"),
    ("crates/api/src/services/venue_credentials/storage/tests.rs", "clear_masks_runtime_value"),
    ("crates/api/src/services/venue_credentials/tests.rs", "dotenv_path_removal_is_atomic_and_leaves_other_credentials_intact"),
    ("crates/api/src/services/hedge_preflight/tests/cases_live_b.rs", "live_operation_health_guard_scopes_all_evidence_to_ticket_venues"),
    ("frontend/src/panels/modules/settings/data/tests.rs", "credential_replay_key_reuses_only_same_fingerprint"),
    ("frontend/src/panels/modules/settings/data/tests/credential_maintenance.rs", "credential_maintenance_replay_key_reuses_only_the_same_operation"),
    ("frontend/src/panels/modules/settings/tabs/venue_credentials/validation.rs", "validation_summary_is_fail_closed_for_missing_required_links"),
    ("frontend/src/panels/modules/settings/tabs/venue_credentials/tests_runtime/health.rs", "current_availability_requires_every_runtime_link_to_be_ok"),
    ("frontend/src/panels/modules/settings/tabs/adapters.rs", "live_adapter_copy_separates_configured_fields_from_readiness"),
)
skip_pattern = re.compile(r"#\s*\[\s*(?:ignore|should_panic)|(?:test|describe)\.skip|\.skip\(|FIXME", re.I)
for relative_path, anchor in test_anchors:
    source = (root / relative_path).read_text(encoding="utf-8")
    if anchor not in source:
        fail(f"missing runnable test anchor {anchor} in {relative_path}")
    if skip_pattern.search(source):
        fail(f"skip marker found in PR-CB test artifact {relative_path}")

browser_anchors = {
    "test/e2e/pr_dg_settings_credentials.spec.ts": (
        "PR-DG persists the dynamic Hyperliquid vault field and keeps readiness fail-closed",
        "PR-DG credential cold error exposes typed request and retry context",
    ),
    "test/e2e/pr_du_settings_control_plane.spec.ts": (
        "PR-DU refreshes the selected venue credential and runtime evidence together",
        "PR-DU keeps request context and reuses idempotency after an in-flight save",
        "PR-DU keeps execution environment read-only and legacy readiness absent",
    ),
    "test/e2e/pr_bx_runtime.spec.ts": (
        "PR-BX Settings separates configuration, capability, and current usability",
    ),
    "test/e2e/pr_dv_scoped_preflight.spec.ts": (
        "PR-DV renders ticket-scoped capability, account, finality, and request evidence",
        "PR-DV keeps confirm-time scoped preflight failure and correlation context visible",
    ),
    "test/e2e/pr_eq_hyperliquid_contract.spec.ts": (
        "PR-EQ Settings keeps builder-scoped Hyperliquid endpoint and wallet/vault evidence non-live",
    ),
    "test/e2e/pr_ck_action_evidence.spec.ts": (
        "PR-CK restores structured ActionRun evidence after settings unmount",
    ),
}
for relative_path, titles in browser_anchors.items():
    source = (root / relative_path).read_text(encoding="utf-8")
    if skip_pattern.search(source):
        fail(f"skip marker found in PR-CB browser artifact {relative_path}")
    for test_title in titles:
        escaped = re.escape(test_title)
        if not re.search(rf"(?m)^\s*test\(\s*['\"]{escaped}['\"]", source):
            fail(f"missing non-skipping browser anchor: {test_title}")

for legacy_path in (
    "frontend/src/panels/modules/settings/tabs/readiness.rs",
    "crates/api/src/services/live_readiness.rs",
):
    if (root / legacy_path).exists():
        fail(f"legacy readiness file returned: {legacy_path}")
app = (root / "crates/api/src/app.rs").read_text(encoding="utf-8")
if '.route("/api/trading/live-readiness"' in app:
    fail("legacy live-readiness route returned")

verify_source = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
if verify_source.count("check_pr_cb_completion.sh") != 2:
    fail("repo docs/full gate wiring drifted")
predecessor = (root / "scripts/check_pr_ca_completion.sh").read_text(encoding="utf-8")
for marker in (
    'successor_title = "PR-CB Settings Credential Health & ActionState Contract"',
    "successor_complete",
    "next_is_head",
):
    if marker not in predecessor:
        fail(f"PR-CA successor-aware handoff is missing: {marker}")

if "## 2026-07-15 PR-CB Settings Credential Health and ActionState Closure" not in history:
    fail("history closure appendix is missing")

closure_paths = {
    artifact for artifact, _ in evidence_contract.values()
} | {
    "scripts/check_pr_ca_completion.sh",
    "scripts/verify_repo_gates.sh",
    *markers,
    *browser_anchors,
    *(path for path, _ in test_anchors),
}
with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {entry["file"]: entry for entry in csv.DictReader(handle, delimiter="\t")}
for path in sorted(closure_paths):
    if coverage.get(path, {}).get("coverage_status") != "exact":
        fail(f"exact coverage missing for {path}")

print(
    f"OK PR-CB static contract ({len(evidence_contract)} evidence types; "
    f"{len(test_anchors)} runnable anchors; {sum(map(len, browser_anchors.values()))} browser anchors; "
    f"{len(closure_paths)} exact paths)"
)
PY

if [[ "${PR_CB_SKIP_UPSTREAM:-0}" != "1" ]]; then
  PR_DG_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dg_completion.sh"
  PR_DU_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_du_completion.sh"
  bash "$ROOT/scripts/check_pr_bx_completion.sh"
  PR_DV_SKIP_TESTS=1 PR_DV_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_dv_completion.sh"
  PR_CK_SKIP_TESTS=1 PR_CK_SKIP_UPSTREAM=1 bash "$ROOT/scripts/check_pr_ck_completion.sh"
  bash "$ROOT/scripts/check_pr_eq_completion.sh"
  PR_EG_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_eg_completion.sh"
  PR_EI_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_ei_completion.sh"
  bash "$ROOT/scripts/product_copy_gate.sh"
  bash "$ROOT/scripts/check_product_audit_evidence.sh"
  bash "$ROOT/scripts/check_product_audit_coverage.sh"
  bash "$ROOT/scripts/check_release_qa_contract.sh"
fi

if [[ "${PR_CB_SKIP_TESTS:-0}" != "1" ]]; then
  JOBS="${CARGO_BUILD_JOBS:-10}"
  CARGO_BUILD_JOBS="$JOBS" cargo test -p shared-types credential --lib --no-fail-fast
  CARGO_BUILD_JOBS="$JOBS" cargo test -p shared-types runtime_health --lib --no-fail-fast
  CARGO_BUILD_JOBS="$JOBS" cargo test -p api --bin crypto-arb-api venue_credentials --no-fail-fast
  CARGO_BUILD_JOBS="$JOBS" cargo test -p api --bin crypto-arb-api services::hedge_preflight::tests --no-fail-fast
  CARGO_BUILD_JOBS="$JOBS" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib settings --no-fail-fast
  CARGO_BUILD_JOBS="$JOBS" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib risk_preview --no-fail-fast
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-dg -- --workers=1
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-du -- --workers=1
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-bx -- --workers=1
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-dv -- --workers=1
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-eq -- --workers=1
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-ck -- --workers=1
fi

printf 'OK PR-CB Settings credential health and ActionState completion contract\n'
