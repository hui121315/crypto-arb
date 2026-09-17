#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
SPEC="$ROOT/crates/api/src/services/venue_credentials/specs.rs"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

if [[ "$MODE" == "--self-test" ]]; then
  PR_DG_SKIP_TESTS=1 bash "$0" >/dev/null
  backup="$(mktemp "${TMPDIR:-/tmp}/crossline-pr-dg.XXXXXX")"
  cp "$SPEC" "$backup"
  restore() {
    cp "$backup" "$SPEC"
    rm -f "$backup"
  }
  trap restore EXIT
  python3 - "$SPEC" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = '"vault_address"'
if marker not in source:
    raise SystemExit("PR-DG self-test setup failed: vault field marker missing")
path.write_text(source.replace(marker, '"vault_scope_drift"', 1), encoding="utf-8")
PY
  if PR_DG_SKIP_TESTS=1 bash "$0" >/dev/null 2>&1; then
    printf 'PR-DG completion self-test failed: drifted vault field passed\n' >&2
    exit 1
  fi
  printf 'PR-DG completion self-test passed\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from __future__ import annotations

import csv
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])
title = "PR-DG Settings Credential Runtime Diagnostics & Secret Persistence Contract"
verify_anchor = "`bash scripts/check_pr_dg_completion.sh --self-test`"
evidence_contract = {
    "credential-shared-contract": "shared-types/src/venues/credentials.rs",
    "credential-probe-matrix": "shared-types/src/credential_matrix.rs",
    "dynamic-field-spec": "crates/api/src/services/venue_credentials/specs.rs",
    "atomic-secret-persistence": "crates/api/src/services/venue_credentials/dotenv.rs",
    "keychain-secret-backend": "crates/api/src/services/venue_credentials/keychain.rs",
    "credential-validation-matrix": "crates/api/src/services/venue_credentials/validation/venues.rs",
    "vault-runtime-profile": "crates/api/src/services/trading_credentials.rs",
    "vault-live-route": "crates/api/src/trading_service/live_adapters/routing.rs",
    "vault-private-ws-scope": "crates/api/src/lifecycle/private_ws/plain_venues/hyperliquid.rs",
    "settings-load-state": "frontend/src/panels/modules/settings/data/resources.rs",
    "settings-credential-console": "frontend/src/panels/modules/settings/tabs/venue_credentials.rs",
    "secret-validation-panels": "frontend/src/panels/modules/settings/tabs/venue_credentials/panels.rs",
    "runtime-diagnostics-center": "frontend/src/panels/modules/settings/tabs/diagnostics/runtime_matrix.rs",
    "adapter-semantic-copy": "frontend/src/panels/modules/settings/tabs/adapters.rs",
    "settings-browser-contract": "test/e2e/pr_dg_settings_credentials.spec.ts",
    "completion-governance": "scripts/check_pr_dg_completion.sh",
}


def fail(message: str) -> None:
    raise SystemExit(f"PR-DG completion gate failed: {message}")


doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
row = next((line for line in doc.splitlines() if line.startswith(f"| `{title}`")), None)
if row is None or "✅ 完成" not in row or "剩余：无。" not in row:
    fail("roadmap row must be completed with no local remainder")
if row.count(verify_anchor) != 1:
    fail("roadmap verification must name the destructive completion gate")
queue = doc[doc.index("### 🟡 6.5"):]
if re.search(r"(?m)^\d+\.\s+\*\*PR-DG\b", queue):
    fail("completed PR-DG remains in the local queue")

with (root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv").open(encoding="utf-8", newline="") as handle:
    rows = [row for row in csv.DictReader(handle, delimiter="\t") if row["pr_id"] == "PR-DG"]
indexed = {row["evidence_type"]: row for row in rows}
if len(indexed) != len(rows) or set(indexed) != set(evidence_contract):
    fail(f"evidence type drift: expected={sorted(evidence_contract)}, actual={sorted(indexed)}")
for kind, artifact in evidence_contract.items():
    if indexed[kind]["artifact"] != artifact or not (root / artifact).is_file():
        fail(f"{kind} artifact drifted or is missing")

markers = {
    "shared-types/src/venues/credentials.rs": (
        "pub struct VenueCredentialField {",
        "pub required: bool",
        "pub struct SecretStorageStatus {",
        "pub validation_evidence: Option<VenueCredentialValidationEvidence>",
    ),
    "shared-types/src/credential_matrix.rs": (
        "pub fn live_required() -> [CredentialProbeLink; 5]",
        "fn is_live_trading_ready(&self) -> bool",
        "CredentialReadiness::Incomplete",
    ),
    "crates/api/src/services/venue_credentials/specs.rs": (
        '"vault_address"',
        '"HYPERLIQUID_VAULT_ADDRESS"',
        '"Vault 执行地址（可选）"',
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
    "crates/api/src/services/venue_credentials/validation/venues.rs": (
        "mod hyperliquid;",
        "pub(super) use hyperliquid::*;",
    ),
    "crates/api/src/services/venue_credentials/validation/venues/hyperliquid.rs": (
        'values.optional("vault_address")',
        "optional_hyperliquid_account_relation_probe",
        "optional_safe_order_noop_probe",
    ),
    "crates/api/src/services/trading_credentials.rs": (
        "fn hyperliquid_profile(",
        'lookup("HYPERLIQUID_VAULT_ADDRESS")',
        "value.vault_address.as_deref()",
    ),
    "crates/api/src/trading_service/live_adapters/routing.rs": (
        "credentials.vault_address.clone()",
        "hyperliquid_live_adapter(",
    ),
    "crates/api/src/lifecycle/private_ws/plain_venues/hyperliquid.rs": (
        "hyperliquid_private_subscription_user",
        ".vault_address",
        ".unwrap_or(&credentials.account_address)",
    ),
    "frontend/src/panels/modules/settings/data/resources.rs": (
        "type SettingsResource<T> = RwSignal<LoadState<T>>",
        "state.apply_result(result.map_err(|error| error.problem))",
    ),
    "frontend/src/panels/modules/settings/tabs/venue_credentials.rs": (
        "credential_inputs(",
        "runtime_health_panel(",
        "account_state_evidence_panel(",
    ),
    "frontend/src/panels/modules/settings/tabs/venue_credentials/panels.rs": (
        '"Secret 存储"',
        "validation_evidence_panel(selected_status)",
        'problem_cell("读取凭证状态失败"',
    ),
    "frontend/src/panels/modules/settings/tabs/diagnostics/runtime_matrix.rs": (
        'data-settings-table="venue-runtime-health"',
        '"交易运行状态中心"',
        "operation.problem.as_ref()",
    ),
    "frontend/src/panels/modules/settings/tabs/adapters.rs": (
        '"字段组已补齐"',
        '"可选路由；下单仍需票据级权限与运行态证据"',
    ),
}
for path, required in markers.items():
    source = (root / path).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"missing {path} marker: {marker}")

browser = (root / evidence_contract["settings-browser-contract"]).read_text(encoding="utf-8")
if re.search(r"\btest\.(?:skip|fixme)\s*\(", browser):
    fail("browser fixture must not be skipped")
for marker in (
    "PR-DG persists the dynamic Hyperliquid vault field and keeps readiness fail-closed",
    'getByLabel("Vault 执行地址（可选）")',
    'key: "vault_address"',
    "CREDENTIAL_REGISTRY_UNAVAILABLE",
    "下单仍需票据级权限与运行态证据",
):
    if marker not in browser:
        fail(f"browser fixture missing Settings contract marker: {marker}")

package = (root / "package.json").read_text(encoding="utf-8")
if '"test:e2e:pr-dg"' not in package or "pr_dg_settings_credentials.spec.ts" not in package:
    fail("package script or product suite wiring is missing")

with (root / "docs/PRODUCT_AUDIT_COVERAGE.tsv").open(encoding="utf-8", newline="") as handle:
    coverage = {row["file"]: row for row in csv.DictReader(handle, delimiter="\t")}
for path in set(evidence_contract.values()) | set(markers):
    if coverage.get(path, {}).get("coverage_status") != "exact":
        fail(f"exact coverage missing for {path}")

print(f"OK PR-DG contract ({len(evidence_contract)} evidence types; vault-scoped Settings control plane)")
PY

bash "$ROOT/scripts/check_product_audit_evidence.sh"
bash "$ROOT/scripts/check_product_audit_coverage.sh"

if [[ "${PR_DG_SKIP_TESTS:-0}" != "1" ]]; then
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api --bin crypto-arb-api venue_credentials --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api --bin crypto-arb-api trading_credentials --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test -p api --bin crypto-arb-api private_subscriptions_ --no-fail-fast
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib settings --no-fail-fast
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-dg
fi

printf 'OK PR-DG credential runtime diagnostics and secret persistence contract\n'
