#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

python3 - "$ROOT" "$MODE" <<'PY'
from __future__ import annotations

import csv
import re
import shutil
import sys
import tempfile
from pathlib import Path


PR_TITLE = "PR-CF CEX Credential Validation & Account Mode Evidence Matrix"
VERIFY_ANCHOR = "`bash scripts/check_pr_cf_completion.sh --self-test`"

EVIDENCE = {
    "successor-pr-dg": ("scripts/check_pr_dg_completion.sh", "check_pr_dg_completion.sh"),
    "successor-pr-du": ("scripts/check_pr_du_completion.sh", "check_pr_du_completion.sh"),
    "successor-pr-eg": ("scripts/check_pr_eg_completion.sh", "check_pr_eg_completion.sh"),
    "successor-pr-ek": ("scripts/check_pr_ek_completion.sh", "check_pr_ek_completion.sh"),
    "successor-pr-el": ("scripts/check_pr_el_completion.sh", "check_pr_el_completion.sh"),
    "successor-pr-em": ("scripts/check_pr_em_completion.sh", "check_pr_em_completion.sh"),
    "successor-pr-en": ("scripts/check_pr_en_completion.sh", "check_pr_en_completion.sh"),
    "successor-pr-eo": ("scripts/check_pr_eo_completion.sh", "check_pr_eo_completion.sh"),
    "successor-pr-ep": ("scripts/check_pr_ep_completion.sh", "check_pr_ep_completion.sh"),
    "successor-pr-er": ("scripts/check_pr_er_completion.sh", "check_pr_er_completion.sh"),
    "credential-shared-contract": ("shared-types/src/venues/credentials.rs", "credential_probe"),
    "credential-readiness-matrix": ("shared-types/src/credential_matrix.rs", "credential"),
    "account-mode-contract": ("shared-types/src/live_trading.rs", "VenueAccountModeInfo"),
    "save-validation-seven-cex": ("crates/api/src/services/venue_credentials/validation/venues/cex.rs", "venue_credentials"),
    "optional-probe-classification": ("crates/api/src/services/venue_credentials/validation/probes.rs", "optional_probe"),
    "account-mode-projection": ("crates/api/src/services/venue_credentials/validation/account_mode.rs", "account_mode_probe"),
    "operation-health-projection": ("crates/api/src/services/venue_operation_health/snapshot/part_15.rs", "venue_operation_health"),
    "frontend-validation-matrix": ("frontend/src/panels/modules/settings/tabs/venue_credentials/validation.rs", "venue_credentials"),
    "frontend-runtime-evidence": ("frontend/src/panels/modules/settings/tabs/venue_credentials/panels/operation_health.rs", "venue_credentials"),
    "settings-browser": ("test/e2e/pr_dg_settings_credentials.spec.ts", "test:e2e:pr-dg"),
    "control-plane-browser": ("test/e2e/pr_du_settings_control_plane.spec.ts", "test:e2e:pr-du"),
    "runtime-health-browser": ("test/e2e/pr_eg_runtime_health.spec.ts", "test:e2e:pr-eg"),
    "binance-account-mode": ("crates/exchange/src/adapters/binance.rs", "exchange_account_mode"),
    "binance-account-tests": ("crates/exchange/src/adapters/binance_private_data_tests.rs", "binance_position_mode"),
    "okx-account-mode": ("crates/exchange/src/adapters/okx_live.rs", "account_mode"),
    "okx-account-tests": ("crates/exchange/src/adapters/okx_live_tests.rs", "account_config"),
    "bybit-account-mode": ("crates/exchange/src/adapters/bybit.rs", "live_account_mode"),
    "bybit-account-tests": ("crates/exchange/src/adapters/bybit_private_data_tests.rs", "bybit_account_summary"),
    "bitget-account-mode": ("crates/exchange/src/adapters/bitget.rs", "account_hold_mode"),
    "bitget-account-tests": ("crates/exchange/src/adapters/bitget_uta_private_data_pr_en_tests.rs", "account_summary"),
    "gate-account-mode": ("crates/exchange/src/adapters/gate.rs", "account_position_mode"),
    "gate-account-tests": ("crates/exchange/src/adapters/gate_private_rest.rs", "account_position_mode"),
    "kucoin-account-mode": ("crates/exchange/src/adapters/kucoin.rs", "live_account_mode"),
    "kucoin-account-tests": ("crates/exchange/src/adapters/kucoin_private_data_tests.rs", "available_margin"),
    "htx-account-mode": ("crates/exchange/src/adapters/htx.rs", "live_account_mode"),
    "htx-account-tests": ("crates/exchange/tests/htx_test.rs", "live_account_mode"),
    "completion-governance": ("scripts/check_pr_cf_completion.sh", "check_pr_cf_completion.sh --self-test"),
}

SUCCESSORS = (
    "PR-DG Settings Credential Runtime Diagnostics & Secret Persistence Contract",
    "PR-DU Settings Credential UX & Runtime Health Contract",
    "PR-EG API Operation Health & Status Center Contract",
    "PR-EK OKX V5 Order, Account & Private WS Semantics Contract",
    "PR-EL Binance USD-M Order, Account & User Stream Semantics Contract",
    "PR-EM Bybit V5 Order, Account & Private WS Semantics Contract",
    "PR-EN Bitget UTA V3 Order, Account & Private WS Semantics Contract",
    "PR-EO KuCoin Futures Order, Account & Private WS Semantics Contract",
    "PR-EP HTX USDT-M Swap Order, Account & Private WS Semantics Contract",
    "PR-ER Gate Futures Order, Account & Private WS Semantics Contract",
)

QUEUE_SUCCESSORS = (
    ("PR-CE Hyperliquid Account Mode & Agent Evidence", "PR-CE"),
    ("PR-AA Verification Gate & Evidence CI", "PR-AA"),
    ("PR-AB Trading Ledger Persistence", "PR-AB"),
    ("PR-AC API Security & Secret Governance", "PR-AC"),
    ("PR-AD API Surface & Legacy Feature Gate", "PR-AD"),
)

VALIDATOR_MARKERS = {
    "binance": ("validate_binance", "/fapi/v1/positionSide/dual", "/fapi/v1/order/test"),
    "okx": ("validate_okx", "/api/v5/account/config", "/api/v5/trade/order-precheck"),
    "bybit": ("validate_bybit", "/v5/order/pre-check"),
    "bitget": ("validate_bitget", "/api/v3/account/settings", "/api/v3/trade/cancel-order"),
    "gate": ("validate_gate", "/api/v4/futures/usdt/accounts", "/api/v4/futures/usdt/orders/{order_id}"),
    "htx": ("validate_htx", "validate_api_ordering_account_type", "validate_api_order_permission_status"),
    "kucoin": ("validate_kucoin", "/api/v2/position/getPositionMode", "/api/v1/trade-fees"),
}

RUNNABLE = (
    ("shared-types/src/credential_matrix.rs", "all_required_ok_is_live_ready"),
    ("shared-types/src/credential_matrix.rs", "failed_link_blocks_even_with_others_ok"),
    ("shared-types/src/credential_matrix.rs", "empty_evidence_blocks_all_links"),
    ("crates/api/src/services/venue_credentials/validation/probes/tests.rs", "balance_probe_rejects_empty_target_currency"),
    ("crates/api/src/services/venue_credentials/validation/probes/tests.rs", "optional_probe_auth_failure_is_failed_not_unknown"),
    ("crates/api/src/services/venue_credentials/validation/probes/tests.rs", "optional_probe_transient_errors_stay_unknown"),
    ("crates/api/src/services/venue_credentials/validation/account_mode.rs", "evidence_adds_not_probed_account_mode_probe"),
    ("crates/api/src/services/venue_credentials/validation/account_mode.rs", "kucoin_account_mode_probe_explains_classic_futures_scope"),
    ("crates/api/src/services/venue_credentials/validation/account_mode.rs", "binance_account_mode_probe_preserves_official_position_side_source"),
    ("crates/api/src/services/venue_credentials/validation/account_mode.rs", "okx_account_mode_probe_preserves_account_config_source"),
    ("crates/api/src/services/venue_operation_health/snapshot/tests/part_01.rs", "credential_validation_probe_rows_preserve_unknown_order_permission"),
    ("crates/api/src/services/venue_operation_health/snapshot/tests/part_01.rs", "credential_private_read_probe_uses_endpoint_evidence_registry"),
    ("crates/api/src/services/venue_operation_health/snapshot/tests/part_02.rs", "configured_private_ws_without_runtime_stays_unknown"),
    ("crates/api/src/services/venue_operation_health/snapshot/tests/part_04.rs", "order_write_credentials_do_not_become_live_proof"),
    ("frontend/src/panels/modules/settings/tabs/venue_credentials/validation.rs", "validation_summary_is_fail_closed_for_missing_required_links"),
    ("frontend/src/panels/modules/settings/tabs/venue_credentials/validation.rs", "validation_probe_message_preserves_kucoin_classic_futures_explanation"),
    ("frontend/src/panels/modules/settings/tabs/venue_credentials/tests_runtime/health.rs", "current_availability_requires_every_runtime_link_to_be_ok"),
    ("frontend/src/panels/modules/settings/tabs/venue_credentials/tests_credentials/summary.rs", "selected_summary_separates_saved_validation_from_current_availability"),
    ("crates/exchange/src/adapters/binance_private_data_tests.rs", "binance_position_mode_parses_official_fixture"),
    ("crates/exchange/tests/binance_test.rs", "exchange_account_mode_reads_official_position_side_dual_endpoint"),
    ("crates/exchange/src/adapters/okx_live_tests.rs", "okx_account_config_parses_official_fixture_position_mode"),
    ("crates/exchange/tests/okx_account_mode_test.rs", "okx_account_mode_reads_signed_account_config_without_demo_header"),
    ("crates/exchange/src/adapters/bybit_private_data_tests.rs", "bybit_account_summary_parses_equity_margin_rates_and_source"),
    ("crates/exchange/tests/bybit_test.rs", "live_account_mode_reads_hedge_position_idx"),
    ("crates/exchange/src/adapters/bitget_uta_private_data_pr_en_tests.rs", "official_account_fixture_projects_complete_uta_summary"),
    ("crates/exchange/src/adapters/bitget_uta_private_rest.rs", "account_hold_mode_reads_official_uta_settings_path"),
    ("crates/exchange/src/adapters/gate_private_rest.rs", "account_position_mode_reads_futures_account_evidence"),
    ("crates/exchange/src/adapters/gate_private_rest.rs", "account_position_mode_rejects_unknown_semantics"),
    ("crates/exchange/src/adapters/kucoin_private_data_tests.rs", "parse_balance_response_uses_available_margin_for_buying_power"),
    ("crates/exchange/tests/kucoin_test.rs", "live_account_mode_reads_hedge_position_mode"),
    ("crates/exchange/tests/htx_test.rs", "live_account_mode_reads_non_unified_account_type"),
    ("crates/exchange/tests/htx_test.rs", "live_account_mode_blocks_unknown_account_type"),
)

BROWSERS = (
    ("test/e2e/pr_dg_settings_credentials.spec.ts", "PR-DG persists the dynamic Hyperliquid vault field and keeps readiness fail-closed"),
    ("test/e2e/pr_dg_settings_credentials.spec.ts", "PR-DG credential cold error exposes typed request and retry context"),
    ("test/e2e/pr_du_settings_control_plane.spec.ts", "PR-DU refreshes the selected venue credential and runtime evidence together"),
    ("test/e2e/pr_du_settings_control_plane.spec.ts", "PR-DU keeps request context and reuses idempotency after an in-flight save"),
    ("test/e2e/pr_du_settings_control_plane.spec.ts", "PR-DU keeps execution environment read-only and legacy readiness absent"),
    ("test/e2e/pr_eg_runtime_health.spec.ts", "PR-EG Settings consumes typed venue runtime health and fails closed without evidence"),
)


def fail(message: str) -> None:
    raise ValueError(message)


def table_rows(path: Path) -> list[dict[str, str]]:
    with path.open(encoding="utf-8", newline="") as handle:
        return list(csv.DictReader(handle, delimiter="\t"))


def require_markers(root: Path, relative: str, *markers: str) -> str:
    source = (root / relative).read_text(encoding="utf-8")
    for marker in markers:
        if marker not in source:
            fail(f"missing {relative} marker: {marker}")
    return source


def check(root: Path) -> None:
    doc = (root / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").read_text(encoding="utf-8")
    row = next((line for line in doc.splitlines() if line.startswith(f"| `{PR_TITLE}`")), None)
    if row is None or "✅ 完成" not in row or "剩余：无。" not in row or row.count(VERIFY_ANCHOR) != 1:
        fail("roadmap row must be complete, remaining-none and bound to the destructive gate")

    queue = doc[doc.index("### 🟡 6.5"):]
    if re.search(r"(?m)^\d+\.\s+\*\*PR-CF\b", queue):
        fail("completed PR-CF remains in the queue")
    expected_head = None
    for successor_title, successor_id in QUEUE_SUCCESSORS:
        successor_row = next(
            (line for line in doc.splitlines() if line.startswith(f"| `{successor_title}`")),
            None,
        )
        completed = successor_row is not None and "✅ 完成" in successor_row and "剩余：无。" in successor_row
        queued = re.search(rf"(?m)^\d+\.\s+\*\*{re.escape(successor_id)}\b", queue) is not None
        if completed and queued:
            fail(f"completed queue successor remains queued: {successor_id}")
        if not completed and expected_head is None:
            expected_head = successor_id
    if expected_head is not None and not re.search(rf"(?m)^1\.\s+\*\*{re.escape(expected_head)}\b", queue):
        fail(f"first incomplete successor must inherit the local queue head: {expected_head}")
    queue_items = re.findall(r"(?m)^\d+\.\s+\*\*", queue)
    if not queue_items:
        fail("local queue plus external pool must not be empty")

    for successor in SUCCESSORS:
        successor_row = next((line for line in doc.splitlines() if line.startswith(f"| `{successor}`")), None)
        if successor_row is None or "✅ 完成" not in successor_row or "剩余：无。" not in successor_row:
            fail(f"completed successor authority drifted: {successor}")

    history = (root / "docs/audit_history/PRODUCT_AUDIT_HISTORY.md").read_text(encoding="utf-8")
    if "## 2026-07-17 PR-CF Seven-CEX Credential and Account Mode Closure" not in history:
        fail("PR-CF closure appendix is missing")

    selected = [item for item in table_rows(root / "docs/PRODUCT_AUDIT_EVIDENCE.tsv") if item["pr_id"] == "PR-CF"]
    indexed = {item["evidence_type"]: item for item in selected}
    if len(indexed) != len(selected) or set(indexed) != set(EVIDENCE):
        fail(f"evidence type drift: expected={sorted(EVIDENCE)}, actual={sorted(indexed)}")
    for evidence_type, (artifact, command_anchor) in EVIDENCE.items():
        item = indexed[evidence_type]
        if item["artifact"] != artifact or command_anchor not in item["command"] or not item["notes"].strip():
            fail(f"evidence anchor drifted: {evidence_type}")
        if not (root / artifact).is_file():
            fail(f"evidence artifact missing: {artifact}")

    coverage = {item["file"]: item for item in table_rows(root / "docs/PRODUCT_AUDIT_COVERAGE.tsv")}
    for artifact, _ in EVIDENCE.values():
        item = coverage.get(artifact)
        if item is None or item["coverage_status"] != "exact" or not item["evidence"].startswith("line:"):
            fail(f"exact coverage missing for {artifact}")

    shared_contracts = "\n".join(
        path.read_text(encoding="utf-8") for path in (root / "shared-types/src").rglob("*.rs")
    )
    if shared_contracts.count("pub struct VenueAccountModeInfo") != 1:
        fail("VenueAccountModeInfo must remain a single shared contract")
    if shared_contracts.count("pub struct VenueCredentialValidationEvidence") != 1:
        fail("VenueCredentialValidationEvidence must remain a single shared contract")

    require_markers(
        root,
        "shared-types/src/credential_matrix.rs",
        "CredentialProbeLink::live_required()",
        "Self::BalanceRead => \"balance_read\"",
        "Self::PositionsRead => \"positions_read\"",
        "Self::OpenOrdersRead => \"open_orders_read\"",
        "Self::OrderPermission => \"order_permission\"",
        "Self::AccountModeRead => \"account_mode_read\"",
        "CredentialReadiness::Blocked",
        "CredentialReadiness::Incomplete",
    )
    validators = require_markers(
        root,
        "crates/api/src/services/venue_credentials/validation/venues/cex.rs",
        "validated_private_read_evidence",
    )
    for venue, markers in VALIDATOR_MARKERS.items():
        for marker in markers:
            if marker not in validators:
                fail(f"{venue} credential validator lost marker: {marker}")
    require_markers(
        root,
        "crates/api/src/services/venue_credentials/validation/probes.rs",
        "validate_balance",
        "optional_private_read_probes",
        'probe.kind == "order_permission"',
        "account_mode_not_probed_probe",
        "classify_optional_probe_error",
        "VenueCredentialProbeStatus::Failed",
        "VenueCredentialProbeStatus::Unknown",
    )
    require_markers(
        root,
        "crates/api/src/services/venue_credentials/validation/account_mode.rs",
        "optional_account_mode_probe",
        "account_mode_probe_from_info",
        "account_mode_not_probed_probe",
        '"account mode is not proven by credential save"',
    )
    require_markers(
        root,
        "crates/api/src/services/venue_operation_health/snapshot/part_15.rs",
        '"account_mode_read" => {',
        "EndpointDataKind::AccountConfig",
        "account_mode_endpoint_matches_probe",
        "does_not_grant_live_write=true",
    )
    require_markers(
        root,
        "frontend/src/panels/modules/settings/tabs/venue_credentials/validation.rs",
        "CredentialProbeLink::live_required()",
        "CredentialReadiness::LiveReady",
        "CredentialReadiness::Blocked",
        "CredentialReadiness::Incomplete",
    )
    require_markers(
        root,
        "frontend/src/panels/modules/settings/tabs/venue_credentials/panels/operation_health.rs",
        "当前可用性仅由运行态链路判定",
        "private_order_stream",
        "order_finality",
    )

    invalid_test = re.compile(r"#\[\s*(?:ignore|should_panic)|#\[\s*cfg_attr[^\]]*ignore")
    for relative, test_name in RUNNABLE:
        source = (root / relative).read_text(encoding="utf-8")
        match = re.search(rf"#\[(?:tokio::)?test\][\s\S]{{0,180}}?(?:async\s+)?fn\s+{re.escape(test_name)}\b", source)
        if match is None or invalid_test.search(source[max(0, match.start() - 160):match.end()]):
            fail(f"runnable test is missing or skipped: {relative}:{test_name}")

    for relative, title in BROWSERS:
        source = (root / relative).read_text(encoding="utf-8")
        escaped = re.escape(title)
        if re.search(rf'test\.(?:skip|fixme)\(\s*["\']{escaped}["\']', source):
            fail(f"browser proof is skipped: {title}")
        if not re.search(rf'test\(\s*["\']{escaped}["\']', source):
            fail(f"browser proof is missing: {title}")

    repo_gate = (root / "scripts/verify_repo_gates.sh").read_text(encoding="utf-8")
    if repo_gate.count("check_pr_cf_completion.sh") != 2:
        fail("repo gate must execute PR-CF in docs and full scopes")


def assert_rejected(root: Path, relative: str, transform, label: str) -> None:
    path = root / relative
    baseline = path.read_text(encoding="utf-8")
    changed = transform(baseline)
    if changed == baseline:
        fail(f"self-test setup drifted: {label}")
    path.write_text(changed, encoding="utf-8")
    try:
        check(root)
    except ValueError:
        pass
    else:
        fail(f"self-test accepted {label}")
    finally:
        path.write_text(baseline, encoding="utf-8")


def self_test(source_root: Path) -> None:
    paths = {
        "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md",
        "docs/PRODUCT_AUDIT_EVIDENCE.tsv",
        "docs/PRODUCT_AUDIT_COVERAGE.tsv",
        "docs/audit_history/PRODUCT_AUDIT_HISTORY.md",
        "scripts/verify_repo_gates.sh",
        "crates/api/src/services/venue_credentials/validation/tests.rs",
        "crates/api/src/services/venue_operation_health/snapshot/tests/part_01.rs",
        "crates/api/src/services/venue_operation_health/snapshot/tests/part_02.rs",
        "crates/api/src/services/venue_operation_health/snapshot/tests/part_04.rs",
        "frontend/src/panels/modules/settings/tabs/venue_credentials/tests_runtime/health.rs",
        "frontend/src/panels/modules/settings/tabs/venue_credentials/tests_credentials/summary.rs",
        "crates/exchange/tests/binance_test.rs",
        "crates/exchange/tests/okx_account_mode_test.rs",
        "crates/exchange/tests/bybit_test.rs",
        "crates/exchange/src/adapters/bitget_uta_private_rest.rs",
        "crates/exchange/tests/kucoin_test.rs",
    }
    paths.update(artifact for artifact, _ in EVIDENCE.values())
    paths.update(relative for relative, _ in RUNNABLE)
    paths.update(relative for relative, _ in BROWSERS)
    with tempfile.TemporaryDirectory(prefix="crossline-pr-cf-completion-") as temp:
        root = Path(temp) / "repo"
        for relative in paths:
            target = root / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source_root / relative, target)
        check(root)
        assert_rejected(root, "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md", lambda text: text.replace(f"| `{PR_TITLE}` | ✅ 完成 |", f"| `{PR_TITLE}` | 🟡 部分完成 |", 1), "a downgraded roadmap row")
        assert_rejected(root, "docs/PRODUCT_AUDIT_EVIDENCE.tsv", lambda text: "\n".join(line for line in text.splitlines() if not line.startswith("PR-CF\tcredential-readiness-matrix\t")) + "\n", "an incomplete evidence matrix")
        assert_rejected(root, "shared-types/src/credential_matrix.rs", lambda text: text.replace('Self::AccountModeRead => "account_mode_read"', 'Self::AccountModeRead => "removed_account_mode_read"', 1), "a detached account-mode readiness link")
        assert_rejected(root, "crates/api/src/services/venue_credentials/validation/probes.rs", lambda text: text.replace("VenueCredentialProbeStatus::Failed", "VenueCredentialProbeStatus::Unknown"), "credential rejection downgraded to unknown")
        assert_rejected(root, "crates/api/src/services/venue_credentials/validation/venues/cex.rs", lambda text: text.replace("validate_gate", "missing_cex_validator", 1), "a missing CEX validator")
        assert_rejected(root, "crates/api/src/services/venue_operation_health/snapshot/part_15.rs", lambda text: text.replace('"account_mode_read" => {', '"removed_account_mode_read" => {', 1), "account-mode health projection removed")
        assert_rejected(root, "crates/exchange/src/adapters/binance_private_data_tests.rs", lambda text: text.replace("#[test]\nfn binance_position_mode_parses_official_fixture", "#[test]\n#[ignore]\nfn binance_position_mode_parses_official_fixture", 1), "a skipped venue account-mode fixture")
        assert_rejected(root, "test/e2e/pr_dg_settings_credentials.spec.ts", lambda text: text.replace('test("PR-DG persists the dynamic', 'test.skip("PR-DG persists the dynamic', 1), "a skipped Settings credential browser proof")
        assert_rejected(root, "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md", lambda text: text.replace("### 🟡 6.5 下一步执行队列", "### 🟡 6.5 下一步执行队列\n\n1. **PR-CF stale**", 1), "completed PR-CF returned to the queue")
        assert_rejected(root, "docs/PRODUCT_AUDIT_COVERAGE.tsv", lambda text: "\n".join(line for line in text.splitlines() if not line.startswith("crates/api/src/services/venue_credentials/validation/account_mode.rs\t")) + "\n", "missing exact account-mode coverage")
        assert_rejected(root, "scripts/verify_repo_gates.sh", lambda text: text.replace("check_pr_cf_completion.sh", "removed_pr_cf_completion.sh", 1), "single-scope repo wiring")
    print("PR-CF completion destructive self-test passed")


try:
    root = Path(sys.argv[1])
    if sys.argv[2] == "--self-test":
        self_test(root)
    else:
        check(root)
        print(
            f"OK PR-CF static contract ({len(EVIDENCE)} evidence types; "
            f"{len(RUNNABLE)} runnable anchors; {len(BROWSERS)} browser anchors; "
            f"{len(VALIDATOR_MARKERS)} CEX validators)"
        )
except (OSError, KeyError, ValueError) as exc:
    raise SystemExit(f"PR-CF completion gate failed: {exc}") from exc
PY

if [[ "$MODE" == "--self-test" ]]; then
  exit 0
fi

if [[ "${PR_CF_SKIP_BROWSER_LIST:-0}" != "1" ]]; then
  npx playwright test \
    "$ROOT/test/e2e/pr_dg_settings_credentials.spec.ts" \
    "$ROOT/test/e2e/pr_du_settings_control_plane.spec.ts" \
    "$ROOT/test/e2e/pr_eg_runtime_health.spec.ts" \
    --list >/dev/null
fi

if [[ "${PR_CF_SKIP_UPSTREAM:-0}" != "1" ]]; then
  PR_DG_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_dg_completion.sh"
  PR_DU_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_du_completion.sh"
  PR_EG_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_eg_completion.sh"
  PR_EK_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_ek_completion.sh"
  bash "$ROOT/scripts/check_pr_el_completion.sh"
  PR_EM_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_em_completion.sh"
  PR_EN_SKIP_TESTS=1 bash "$ROOT/scripts/check_pr_en_completion.sh"
  bash "$ROOT/scripts/check_pr_eo_completion.sh"
  bash "$ROOT/scripts/check_pr_ep_completion.sh"
  bash "$ROOT/scripts/check_pr_er_completion.sh"
  bash "$ROOT/scripts/check_product_audit_progress.sh"
  bash "$ROOT/scripts/check_product_audit_evidence.sh"
  bash "$ROOT/scripts/check_product_audit_evidence_index.sh"
  bash "$ROOT/scripts/check_product_audit_coverage.sh"
fi

if [[ "${PR_CF_SKIP_TESTS:-0}" != "1" ]]; then
  jobs="${CARGO_BUILD_JOBS:-10}"
  CARGO_BUILD_JOBS="$jobs" cargo test -p shared-types --lib credential --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api venue_credentials --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api venue_operation_health --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --lib position_mode --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --lib account_summary --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --lib account_hold_mode --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --lib account_position_mode --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --test binance_test exchange_account_mode --no-fail-fast -- --test-threads=1
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --test okx_account_mode_test --no-fail-fast -- --test-threads=1
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --test bybit_test live_account_mode --no-fail-fast -- --test-threads=1
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --test kucoin_test live_account_mode --no-fail-fast -- --test-threads=1
  CARGO_BUILD_JOBS="$jobs" cargo test -p exchange --test htx_test live_account_mode --no-fail-fast -- --test-threads=1
  CARGO_BUILD_JOBS="$jobs" cargo test --manifest-path "$ROOT/frontend/Cargo.toml" --lib venue_credentials --no-fail-fast
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-dg -- --workers=1
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-du -- --workers=1
  CI=1 npm --prefix "$ROOT" run test:e2e:pr-eg -- --workers=1
fi

printf 'PR-CF seven-CEX credential and account-mode completion passed\n'
