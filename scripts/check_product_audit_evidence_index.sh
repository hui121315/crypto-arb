#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DOC="$ROOT/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
LEDGER="$ROOT/docs/PRODUCT_AUDIT_EVIDENCE.tsv"
MODE="${1:-check}"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

if [[ "$MODE" == "--self-test" ]]; then
  python3 - "$ROOT" <<'PY'
from pathlib import Path
import csv
import shutil
import subprocess
import sys
import tempfile


root = Path(sys.argv[1])
script = root / "scripts/check_product_audit_evidence_index.sh"
browser_path = "test/e2e/pr_fa_runtime.spec.ts"
browser_titles = (
    "account evidence isolates section freshness and rejects trusted warming empties",
    "gzip hot-path transfer records browser, Wasm, and bounded render metrics",
)
fj_required_evidence = {
    "mutation-route-matrix-gate": (
        "scripts/check_mutation_audit_contract.sh",
        "bash scripts/check_mutation_audit_contract.sh",
    ),
    "runtime-audit-persistence-restart-redaction-gate": (
        "scripts/verify_api_security_runtime_smoke.sh",
        "bash scripts/verify_api_security_runtime_smoke.sh",
    ),
    "route-registry-browser-gate": (
        "test/e2e/route_registry.spec.ts",
        "playwright test test/e2e/route_registry.spec.ts",
    ),
    "product-security-browser-gate": (
        "test/e2e/data_pipeline.spec.ts",
        "CI=1 npm run test:e2e:data-pipeline",
    ),
    "action-run-terminal-correlation-gate": (
        "crates/api/src/services/action_runs/tests/terminal_contract.rs",
        "core_high_risk_mutations_preserve_identity_across_terminal_outcomes",
    ),
    "completion-governance-gate": (
        "scripts/check_pr_fj_completion.sh",
        "bash scripts/check_pr_fj_completion.sh --self-test",
    ),
}
et_required_evidence = {
    "simulation-shared-boundary-gate": (
        "shared-types/src/simulation.rs",
        "contract_types_remain_in_the_simulation_namespace",
    ),
    "opportunity-detail-load-state-gate": (
        "frontend/src/panels/modules/opportunities/data/tests/detail_problem.rs",
        "segment_problem_keeps_partial_detail_stale_instead_of_blank",
    ),
    "execution-preview-selection-gate": (
        "frontend/src/panels/modules/execution/data/preview_tests/state.rs",
        "execution_preview_refresh_keeps_same_query_as_explicit_stale",
    ),
    "funding-single-flight-health-gate": (
        "crates/api/src/data_source_tests.rs",
        "concurrent_cold_reads_start_one_funding_refresh",
    ),
    "funding-product-browser-gate": (
        "test/e2e/pr_et_runtime.spec.ts",
        "PR-ET funding cold start stays warming instead of becoming a healthy empty state",
    ),
    "dto-duplication-regression-gate": (
        "scripts/check_frontend_module_boundaries_self_test.sh",
        "expect_failure bad_dto_mirror",
    ),
    "direct-client-module-boundary-regression-gate": (
        "scripts/check_frontend_module_boundaries_self_test.sh",
        "expect_failure bad_component_client",
    ),
    "crate-root-boundary-regression-gate": (
        "scripts/check_crate_root_boundaries_self_test.sh",
        "expect_failure bad_logic",
    ),
    "completion-governance-gate": (
        "scripts/check_pr_et_completion.sh",
        "bash scripts/check_pr_et_completion.sh --self-test",
    ),
}
required_evidence = {
    "account-binding-credential-generation-gate": (
        "crates/api/src/services/account_binding.rs",
        "verified_scope_requires_probe_and_current_credential_fingerprint",
        "cargo test -p api verified_scope_requires_probe_and_current_credential_fingerprint --no-fail-fast",
    ),
    "partial-failure-isolation-gate": (
        "crates/api/src/services/account_positions/tests/error_paths.rs",
        "partial_position_envelope_keeps_rows_and_binds_each_venue",
        "cargo test -p api partial_position_envelope_keeps_rows_and_binds_each_venue --no-fail-fast",
    ),
    "typed-problem-browser-gate": (
        browser_path,
        browser_titles[0],
        f'node --check {browser_path}; CI=1 npm run test:e2e:pr-fa -- --grep "{browser_titles[0]}"',
    ),
    "runtime-contract": (
        "scripts/verify_runtime_contracts.sh",
        "bash scripts/verify_runtime_contracts.sh",
        "bash -n scripts/verify_runtime_contracts.sh; ALLOW_RUNTIME_SKIP=1 bash scripts/verify_runtime_contracts.sh; bash scripts/check_product_audit_evidence_index.sh",
    ),
    "frontend-source-freshness-gate": (
        "frontend/src/panels/modules/positions/components/account_evidence.rs",
        "binding_detail_keeps_scope_fingerprint_and_problem_context",
        "cargo test --manifest-path frontend/Cargo.toml binding_detail_keeps_scope_fingerprint_and_problem_context --no-fail-fast",
    ),
    "wasm-browser-performance-gate": (
        browser_path,
        browser_titles[1],
        f'node --check {browser_path}; CI=1 npm run test:e2e:pr-fa -- --grep "{browser_titles[1]}"',
    ),
    "release-runtime-shutdown": (
        "crates/api/src/lifecycle/shutdown.rs",
        "check_release_qa_contract.sh --release",
        "bash scripts/check_release_qa_contract.sh --release",
    ),
    "completion-governance-gate": (
        "scripts/check_pr_fa_completion.sh",
        "bash scripts/check_pr_fa_completion.sh --self-test",
        "bash -n scripts/check_pr_fa_completion.sh; bash scripts/check_pr_fa_completion.sh --self-test; bash scripts/check_pr_fa_completion.sh",
    ),
}


def write_ledger(path: Path, rows: list[list[str]]) -> None:
    with path.open("w", encoding="utf-8", newline="") as handle:
        csv.writer(handle, delimiter="\t", lineterminator="\n").writerows(rows)


def write_fixture(fixture: Path, rows: list[list[str]]) -> None:
    if fixture.exists():
        shutil.rmtree(fixture)
    (fixture / "scripts").mkdir(parents=True)
    (fixture / "docs").mkdir(parents=True)
    shutil.copy2(script, fixture / "scripts/check_product_audit_evidence_index.sh")
    (fixture / "docs/PRODUCT_FULL_AUDIT_REFINEMENT.md").write_text(
        "### 🟡 6.3 建议整改顺序\n\n"
        "| PR | 状态 | 范围 | 验收标准 |\n"
        "|---|---|---|---|\n"
        "| `PR-FA Runtime Smoke, Partial Failure Isolation & Payload Budget Gate` | 🟡 部分完成 | self-test | self-test |\n\n"
        "| `PR-FJ Security Verification, CI Gate & Runtime Smoke Contract` | 🟡 部分完成 | self-test | self-test |\n\n"
        "| `PR-ET Frontend Boundary, DTO Single Source & Hard-Gate Contract` | ✅ 完成 | self-test | self-test |\n\n"
        "### 🟡 6.4 其它\n",
        encoding="utf-8",
    )
    write_ledger(
        fixture / "docs/PRODUCT_AUDIT_EVIDENCE.tsv",
        [["pr_id", "evidence_type", "artifact", "command", "notes"], *rows],
    )
    for artifact, _, _ in required_evidence.values():
        path = fixture / artifact
        path.parent.mkdir(parents=True, exist_ok=True)
        if artifact == browser_path:
            path.write_text(
                "\n".join(f'test("{title}", async () => {{}});' for title in browser_titles)
                + "\n",
                encoding="utf-8",
            )
        elif path != fixture / "scripts/check_product_audit_evidence_index.sh":
            path.write_text("#!/usr/bin/env bash\n", encoding="utf-8")
    for artifact, _ in fj_required_evidence.values():
        path = fixture / artifact
        path.parent.mkdir(parents=True, exist_ok=True)
        if not path.exists():
            path.write_text("#!/usr/bin/env bash\n", encoding="utf-8")
    for artifact, _ in et_required_evidence.values():
        path = fixture / artifact
        path.parent.mkdir(parents=True, exist_ok=True)
        if not path.exists():
            path.write_text("#!/usr/bin/env bash\n", encoding="utf-8")


def run_gate(fixture: Path, expected: str) -> None:
    result = subprocess.run(
        ["bash", "scripts/check_product_audit_evidence_index.sh"],
        cwd=fixture,
        text=True,
        capture_output=True,
        check=False,
    )
    if expected == "pass" and result.returncode != 0:
        raise SystemExit(f"PR-FA evidence-index self-test baseline failed: {result.stderr}")
    if expected == "fail" and result.returncode == 0:
        raise SystemExit("PR-FA evidence-index self-test negative fixture unexpectedly passed")


base_rows = [
    ["PR-FA", evidence_type, artifact, command, "self-test"]
    for evidence_type, (artifact, _, command) in required_evidence.items()
] + [
    ["PR-FJ", evidence_type, artifact, f"verify {anchor}", "self-test"]
    for evidence_type, (artifact, anchor) in fj_required_evidence.items()
] + [
    [
        "PR-ET",
        evidence_type,
        artifact,
        (
            "bash -n scripts/check_pr_et_completion.sh; "
            "bash scripts/check_pr_et_completion.sh --self-test; "
            "bash scripts/check_pr_et_completion.sh"
            if evidence_type == "completion-governance-gate"
            else f"verify {anchor}"
        ),
        "self-test",
    ]
    for evidence_type, (artifact, anchor) in et_required_evidence.items()
]
with tempfile.TemporaryDirectory(prefix="crossline-pr-fa-evidence-index-") as temp:
    fixture = Path(temp) / "fixture"
    write_fixture(fixture, base_rows)
    run_gate(fixture, "pass")

    for index in range(len(base_rows)):
        write_fixture(fixture, base_rows[:index] + base_rows[index + 1 :])
        run_gate(fixture, "fail")

        drifted = [row.copy() for row in base_rows]
        drifted[index][2] = "scripts/check_product_audit_evidence_index.sh"
        write_fixture(fixture, drifted)
        run_gate(fixture, "fail")

        drifted = [row.copy() for row in base_rows]
        source = {
            "PR-FA": required_evidence,
            "PR-FJ": fj_required_evidence,
            "PR-ET": et_required_evidence,
        }[drifted[index][0]]
        anchor = source[drifted[index][1]][1]
        drifted[index][3] = drifted[index][3].replace(anchor, "drifted-anchor")
        write_fixture(fixture, drifted)
        run_gate(fixture, "fail")

    write_fixture(fixture, [*base_rows, base_rows[0].copy()])
    run_gate(fixture, "fail")

    extra = ["PR-FA", "extra-pr-fa-evidence", "scripts/check_product_audit_evidence_index.sh", "true", "self-test"]
    write_fixture(fixture, [*base_rows, extra])
    run_gate(fixture, "fail")

print("OK PR-FA/PR-FJ/PR-ET product audit evidence index self-test")
PY
  exit 0
fi

python3 - "$ROOT" "$DOC" "$LEDGER" <<'PY'
from pathlib import Path
import csv
import re
import shlex
import subprocess
import sys

SUPPORTED_LIVE_SAMPLE_VENUES = {
    "binance",
    "okx",
    "bybit",
    "bitget",
    "gate",
    "htx",
    "kucoin",
    "hyperliquid",
}
HYPERLIQUID_BUILDER_VENUE = re.compile(r"^hyperliquid:[a-z0-9][a-z0-9_-]*$")
PR_FW_GENERIC_EVIDENCE_TYPES = {
    "code",
    "test",
    "tests",
    "docs",
    "doc",
    "evidence",
    "gate",
    "misc",
}
PR_FA_BROWSER_PATH = "test/e2e/pr_fa_runtime.spec.ts"
PR_FA_BROWSER_TITLES = (
    "account evidence isolates section freshness and rejects trusted warming empties",
    "gzip hot-path transfer records browser, Wasm, and bounded render metrics",
)
PR_FA_REQUIRED_EVIDENCE = {
    "account-binding-credential-generation-gate": (
        "crates/api/src/services/account_binding.rs",
        "verified_scope_requires_probe_and_current_credential_fingerprint",
    ),
    "partial-failure-isolation-gate": (
        "crates/api/src/services/account_positions/tests/error_paths.rs",
        "partial_position_envelope_keeps_rows_and_binds_each_venue",
    ),
    "typed-problem-browser-gate": (PR_FA_BROWSER_PATH, PR_FA_BROWSER_TITLES[0]),
    "runtime-contract": (
        "scripts/verify_runtime_contracts.sh",
        "bash scripts/verify_runtime_contracts.sh",
    ),
    "frontend-source-freshness-gate": (
        "frontend/src/panels/modules/positions/components/account_evidence.rs",
        "binding_detail_keeps_scope_fingerprint_and_problem_context",
    ),
    "wasm-browser-performance-gate": (
        PR_FA_BROWSER_PATH,
        PR_FA_BROWSER_TITLES[1],
    ),
    "release-runtime-shutdown": (
        "crates/api/src/lifecycle/shutdown.rs",
        "check_release_qa_contract.sh --release",
    ),
    "completion-governance-gate": (
        "scripts/check_pr_fa_completion.sh",
        "bash scripts/check_pr_fa_completion.sh --self-test",
    ),
}
PR_FJ_REQUIRED_EVIDENCE = {
    "mutation-route-matrix-gate": (
        "scripts/check_mutation_audit_contract.sh",
        "bash scripts/check_mutation_audit_contract.sh",
    ),
    "runtime-audit-persistence-restart-redaction-gate": (
        "scripts/verify_api_security_runtime_smoke.sh",
        "bash scripts/verify_api_security_runtime_smoke.sh",
    ),
    "route-registry-browser-gate": (
        "test/e2e/route_registry.spec.ts",
        "playwright test test/e2e/route_registry.spec.ts",
    ),
    "product-security-browser-gate": (
        "test/e2e/data_pipeline.spec.ts",
        "CI=1 npm run test:e2e:data-pipeline",
    ),
    "action-run-terminal-correlation-gate": (
        "crates/api/src/services/action_runs/tests/terminal_contract.rs",
        "core_high_risk_mutations_preserve_identity_across_terminal_outcomes",
    ),
    "completion-governance-gate": (
        "scripts/check_pr_fj_completion.sh",
        "bash scripts/check_pr_fj_completion.sh --self-test",
    ),
}
PR_ET_REQUIRED_EVIDENCE = {
    "simulation-shared-boundary-gate": (
        "shared-types/src/simulation.rs",
        "contract_types_remain_in_the_simulation_namespace",
    ),
    "opportunity-detail-load-state-gate": (
        "frontend/src/panels/modules/opportunities/data/tests/detail_problem.rs",
        "segment_problem_keeps_partial_detail_stale_instead_of_blank",
    ),
    "execution-preview-selection-gate": (
        "frontend/src/panels/modules/execution/data/preview_tests/state.rs",
        "execution_preview_refresh_keeps_same_query_as_explicit_stale",
    ),
    "funding-single-flight-health-gate": (
        "crates/api/src/data_source_tests.rs",
        "concurrent_cold_reads_start_one_funding_refresh",
    ),
    "funding-product-browser-gate": (
        "test/e2e/pr_et_runtime.spec.ts",
        "PR-ET funding cold start stays warming instead of becoming a healthy empty state",
    ),
    "dto-duplication-regression-gate": (
        "scripts/check_frontend_module_boundaries_self_test.sh",
        "expect_failure bad_dto_mirror",
    ),
    "direct-client-module-boundary-regression-gate": (
        "scripts/check_frontend_module_boundaries_self_test.sh",
        "expect_failure bad_component_client",
    ),
    "crate-root-boundary-regression-gate": (
        "scripts/check_crate_root_boundaries_self_test.sh",
        "expect_failure bad_logic",
    ),
    "completion-governance-gate": (
        "scripts/check_pr_et_completion.sh",
        "bash scripts/check_pr_et_completion.sh --self-test",
    ),
}
PR_BX_REQUIRED_EVIDENCE = {
    "venue-runtime-health-contract-gate": (
        "shared-types/src/venues/tests_runtime_health.rs",
        "snapshot_groups_normalized_venues_into_all_runtime_slots",
    ),
    "venue-runtime-health-api-gate": (
        "crates/api/src/routers/system.rs",
        "api_projects_existing_operation_snapshot_without_external_probes",
    ),
    "scoped-two-leg-preflight-gate": (
        "crates/api/src/services/hedge_preflight/tests/cases_live_b.rs",
        "live_operation_health_guard_scopes_all_evidence_to_ticket_venues",
    ),
    "system-health-usability-gate": (
        "crates/api/src/services/system_health/tests/health.rs",
        "api_health_requires_ok_rows_to_be_configured_and_supported",
    ),
    "settings-runtime-separation-gate": (
        "frontend/src/panels/modules/settings/tabs/diagnostics/tests/search.rs",
        "operation_health_capability_is_distinct_and_searchable",
    ),
    "selected-venue-usability-gate": (
        "frontend/src/panels/modules/settings/tabs/venue_credentials/tests_runtime/health.rs",
        "ok_but_unconfigured_runtime_link_is_not_currently_usable",
    ),
    "four-runtime-slots-browser-gate": (
        "test/e2e/pr_bx_runtime.spec.ts",
        "PR-BX status bar keeps market, trading, private WS, and app WS sources distinct",
    ),
    "completion-governance-gate": (
        "scripts/check_pr_bx_completion.sh",
        "bash scripts/check_pr_bx_completion.sh --self-test",
    ),
}
PR_ER_REQUIRED_EVIDENCE = {
    "order-compiler-identity": (
        "crates/exchange/src/adapters/gate_trade_data_tests.rs",
        "place_order_converts_base_qty_to_contracts",
    ),
    "native-settle-instrument-spec": (
        "crates/exchange/src/adapters/gate_contracts_tests.rs",
        "official_fixture_closes_native_identity_and_contract_spec",
    ),
    "contract-fail-closed": (
        "crates/exchange/src/adapters/gate_contracts_tests.rs",
        "canonical_fallback_is_not_executable_until_officially_verified",
    ),
    "finality-reconciliation": (
        "crates/exchange/src/adapters/gate_tests.rs",
        "numeric_order_finality_uses_order_id_and_enriches_fill_fees",
    ),
    "account-maintenance-quality": (
        "crates/api/src/services/account_positions/tests/venue_quality.rs",
        "gate_maintenance_quality_tracks_actual_estimated_and_unknown_provenance",
    ),
    "private-fill-fee-ledger": (
        "crates/exchange/src/adapters/gate_fill_evidence_tests.rs",
        "parses_official_my_trades_fixture_without_combining_fee_units",
    ),
    "private-ws-operation-health": (
        "crates/api/src/services/private_ws_health/tests.rs",
        "gate_subscription_requires_every_server_ack",
    ),
    "official-fixture-registry": (
        "crates/exchange/src/venue_spec.rs",
        "gate_my_trades_evidence_uses_recorded_fixture_metadata",
    ),
    "product-browser": (
        "test/e2e/pr_er_gate_runtime.spec.ts",
        "npm run test:e2e:pr-er",
    ),
    "completion-governance": (
        "scripts/check_pr_er_completion.sh",
        "bash scripts/check_pr_er_completion.sh --self-test",
    ),
}
PR_EL_REQUIRED_EVIDENCE = {
    "native-symbol-usdc-registry": (
        "crates/exchange/src/adapters/binance_exchange_info_tests.rs",
        "registry_projection_uses_compiled_usdt_and_usdc_specs",
    ),
    "order-identity-contract": (
        "shared-types/src/order_identity.rs",
        "verified_usdc_native_identity_is_execution_ready",
    ),
    "client-order-id-fail-closed": (
        "crates/exchange/src/adapters/binance_trade_data_tests.rs",
        "rest_place_order_validates_official_client_order_id_rule",
    ),
    "private-rest-parser-registry": (
        "crates/exchange/src/venue_spec.rs",
        "binance_pr_el_private_rest_registry_is_recorded",
    ),
    "commission-rate-fixture": (
        "crates/exchange/src/adapters/binance_fee_evidence_tests.rs",
        "parses_official_commission_fixture_without_zeroing_rates",
    ),
    "order-trade-update-finality-ledger": (
        "crates/api/src/trading_service/private_ws_events/tests/deltas/order_updates/part_02.rs",
        "binance_order_trade_fill_is_one_identity_preserving_ledger_outcome",
    ),
    "private-ws-durable-health": (
        "crates/api/src/lifecycle/private_ws/tests/projection.rs",
        "binance_terminal_fill_projects_execution_and_health_once_after_ack",
    ),
    "official-fixture-operation-registry": (
        "crates/exchange/src/venue_spec.rs",
        "binance_commission_rate_evidence_uses_recorded_fixture_metadata",
    ),
    "product-browser": (
        "test/e2e/pr_el_binance_identity.spec.ts",
        "npm run test:e2e:pr-el",
    ),
    "completion-governance": (
        "scripts/check_pr_el_completion.sh",
        "bash scripts/check_pr_el_completion.sh --self-test",
    ),
}
PR_EO_REQUIRED_EVIDENCE = {
    "scoped-position-mode-preflight": (
        "crates/api/src/services/hedge_preflight/tests/cases_account.rs",
        "account_mode_guard_records_kucoin_scope",
    ),
    "native-contract-order-compiler": (
        "crates/exchange/src/adapters/kucoin_instruments_tests.rs",
        "official_matrix_maps_usdt_usdc_and_verified_equity_contracts",
    ),
    "cancel-identity-finality-reconciliation": (
        "crates/exchange/src/adapters/kucoin_private_rest.rs",
        "cancel_target_prefers_exchange_order_id_then_client_oid_fallback",
    ),
    "private-rest-parser-registry": (
        "crates/exchange/src/venue_spec.rs",
        "kucoin_pr_eo_private_evidence_is_recorded",
    ),
    "private-fill-fee-ledger": (
        "crates/exchange/src/adapters/kucoin_private_data_tests.rs",
        "kucoin_fills_parse_official_fixture_without_defaulting_fee",
    ),
    "classic-ws-finality-ledger": (
        "crates/api/src/trading_service/private_ws_mapper/tests/cases_kucoin.rs",
        "kucoin_classic_match_projects_durable_fill_once",
    ),
    "private-ws-durable-health": (
        "crates/api/src/lifecycle/private_ws/tests/kucoin_finality.rs",
        "kucoin_terminal_fill_projects_execution_run_once_after_durable_ack",
    ),
    "pro-ws-beta-fail-closed": (
        "crates/exchange/tests/ws_trading_specs_test.rs",
        "kucoin_pro_ws_beta_stays_unavailable_without_runtime_evidence",
    ),
    "official-fixture-operation-registry": (
        "crates/exchange/src/venue_spec.rs",
        "kucoin_pr_eo_private_evidence_is_recorded",
    ),
    "product-browser-and-completion-governance": (
        "scripts/check_pr_eo_completion.sh",
        "bash scripts/check_pr_eo_completion.sh --self-test",
    ),
}
PR_EP_REQUIRED_EVIDENCE = {
    "account-position-order-preflight": (
        "crates/exchange/tests/htx_compiler_test.rs",
        "place_uses_verified_native_contract_size_and_position_mode",
    ),
    "native-contract-order-compiler": (
        "crates/exchange/tests/htx_compiler_test.rs",
        "preflight_cold_cache_uses_native_alias_and_cross_flat_context",
    ),
    "market-like-offset-order-compiler": (
        "crates/exchange/src/adapters/htx_trade_data_tests.rs",
        "market_like_styles_cover_official_bbo_and_optimal_families",
    ),
    "private-rest-parser-registry": (
        "crates/exchange/src/venue_spec.rs",
        "htx_pr_ep_private_trade_rest_registry_contains_recorded_core",
    ),
    "notification-ws-fill-fee-ledger": (
        "crates/api/src/trading_service/private_ws_mapper/tests/cases_htx.rs",
        "htx_settled_order_maps_stable_fill_identity_and_reported_fee",
    ),
    "private-ws-durable-finality-health": (
        "crates/api/src/lifecycle/private_ws/tests/htx_finality.rs",
        "htx_terminal_fill_projects_execution_run_once_after_durable_ack",
    ),
    "trade-ws-authenticated-runtime-fail-closed": (
        "crates/exchange/tests/ws_trading_specs_test.rs",
        "htx_schema_or_announcement_cannot_enable_live_submit_without_authenticated_runtime",
    ),
    "official-fixture-operation-registry": (
        "crates/exchange/src/venue_spec.rs",
        "htx_pr_ep_private_trade_rest_registry_contains_recorded_core",
    ),
    "product-browser-and-completion-governance": (
        "scripts/check_pr_ep_completion.sh",
        "bash scripts/check_pr_ep_completion.sh --self-test",
    ),
}

PR_EJ_REQUIRED_EVIDENCE = {
    "authority-refinement-truth",
    "actionable-status-governance",
    "product-boundary",
    "external-api-evidence",
    "problem-loadstate-action-boundary",
    "high-risk-audit-idempotency",
    "dto-single-source",
    "wasm-three-stage-budget",
    "completion-governance",
}

PR_EK_REQUIRED_EVIDENCE = {
    "ack-fail-closed",
    "order-compiler-account-mode",
    "instrument-sizing-native-contract",
    "private-rest-official-registry",
    "private-ws-server-ack-health",
    "private-ws-fill-fee-finality",
    "official-fixture-operation-registry",
    "product-browser-and-completion-governance",
}

PR_EM_REQUIRED_EVIDENCE = {
    "account-summary-rest-ws-contract": (
        "crates/exchange/src/adapters/bybit_private_data_tests.rs",
        "bybit_account_summary_parses_equity_margin_rates_and_source",
    ),
    "account-state-source-freshness-problem-ui": (
        "crates/api/src/services/account_state/tests.rs",
        "bybit_unified_summary_problem_degrades_all_account_facts",
    ),
    "native-symbol-settle-rwa-contract": (
        "crates/exchange/src/adapters/bybit_instruments_tests.rs",
        "bybit_identity_matrix_preserves_usdt_usdc_and_rwa_contracts",
    ),
    "private-read-settle-fanout": (
        "crates/exchange/src/adapters/bybit_tests.rs",
        "private_account_reads_fan_out_usdt_and_usdc_settles",
    ),
    "order-compiler-position-mode": (
        "crates/exchange/src/adapters/bybit_trade_data_tests.rs",
        "hedge_position_idx_is_serialized",
    ),
    "order-identity-execution-readiness": (
        "crates/api/src/services/hedge_preview/guards.rs",
        "bybit_identity_constraints_produce_execution_ready_usdc_plan",
    ),
    "private-rest-ws-fail-closed": (
        "crates/exchange/src/adapters/bybit_private_data_tests.rs",
        "bybit_private_strict_fixture_sweep_fails_closed",
    ),
    "private-ws-server-ack-health": (
        "crates/api/src/lifecycle/private_ws/plain_venues_tests.rs",
        "bybit_private_ws_waits_for_auth_and_subscription_acknowledgements",
    ),
    "private-ws-fill-fee-finality": (
        "crates/api/src/trading_service/private_ws_events/tests/deltas/order_updates/part_04.rs",
        "bybit_private_fill_and_order_finality_project_once",
    ),
    "official-fixture-operation-registry": (
        "crates/exchange/tests/ws_trading_specs_test.rs",
        "bybit_private_stream_fixtures_are_recorded",
    ),
    "product-browser": (
        "test/e2e/pr_em_bybit_account.spec.ts",
        "npm run test:e2e:pr-em",
    ),
    "completion-governance": (
        "scripts/check_pr_em_completion.sh",
        "bash scripts/check_pr_em_completion.sh --self-test",
    ),
}


def supported_live_sample_venue(venue: str) -> bool:
    return venue in SUPPORTED_LIVE_SAMPLE_VENUES or bool(
        HYPERLIQUID_BUILDER_VENUE.fullmatch(venue)
    )


root = Path(sys.argv[1])
doc = Path(sys.argv[2])
ledger = Path(sys.argv[3])

if not doc.exists():
    raise SystemExit("product audit evidence index gate failed: missing product audit doc")
if not ledger.exists():
    raise SystemExit("product audit evidence index gate failed: missing docs/PRODUCT_AUDIT_EVIDENCE.tsv")

text = doc.read_text(encoding="utf-8")
start = text.find("### 🟡 6.3")
end = text.find("### 🟡 6.4", start)
if start == -1 or end == -1:
    raise SystemExit("product audit evidence index gate failed: missing 6.3 roadmap section")

active_prs: set[str] = set()
completed: set[str] = set()
for line in text[start:end].splitlines():
    if not line.startswith("| `PR-"):
        continue
    cells = [cell.strip() for cell in line.strip().strip("|").split("|")]
    if len(cells) < 2:
        continue
    name = cells[0].strip("`")
    pr_id = name.split()[0]
    active_prs.add(pr_id)
    if "| ✅ 完成 |" in line:
        completed.add(pr_id)

with ledger.open(encoding="utf-8", newline="") as handle:
    rows = list(csv.DictReader(handle, delimiter="\t"))

required = {"pr_id", "evidence_type", "artifact", "command", "notes"}
if not rows:
    raise SystemExit("product audit evidence index gate failed: ledger has no evidence rows")
if set(rows[0].keys() or []) != required:
    actual = sorted(rows[0].keys() or [])
    raise SystemExit(
        "product audit evidence index gate failed: unexpected header "
        f"{actual}; expected {sorted(required)}"
    )

by_pr: dict[str, list[dict[str, str]]] = {}
failures: list[str] = []
non_local_command = re.compile(r"(^|[;&|]\s*)gh(\s|$)")
pr_fw_evidence_seen: dict[str, int] = {}
pr_fa_evidence_seen: dict[str, int] = {}
pr_fj_evidence_seen: dict[str, int] = {}
pr_et_evidence_seen: dict[str, int] = {}
pr_bx_evidence_seen: dict[str, int] = {}
pr_er_evidence_seen: dict[str, int] = {}
pr_el_evidence_seen: dict[str, int] = {}
pr_eo_evidence_seen: dict[str, int] = {}
pr_ep_evidence_seen: dict[str, int] = {}

for index, row in enumerate(rows, start=2):
    pr_id = row["pr_id"].strip()
    evidence_type = row["evidence_type"].strip()
    artifact = row["artifact"].strip()
    command = row["command"].strip()
    if not re.fullmatch(r"PR-[A-Z0-9]+", pr_id):
        failures.append(f"{ledger.name}:{index} invalid pr_id {pr_id!r}")
    elif pr_id not in active_prs:
        failures.append(
            f"{ledger.name}:{index} {pr_id} is not in 6.3 active roadmap"
        )
    if not evidence_type:
        failures.append(f"{ledger.name}:{index} {pr_id} has empty evidence_type")
    elif not re.fullmatch(r"[a-z0-9][a-z0-9:_-]*", evidence_type):
        failures.append(
            f"{ledger.name}:{index} {pr_id} has invalid evidence_type {evidence_type!r}"
        )
    elif pr_id == "PR-BX":
        previous = pr_bx_evidence_seen.get(evidence_type)
        if previous is not None:
            failures.append(
                f"{ledger.name}:{index} PR-BX duplicates evidence_type "
                f"{evidence_type!r}; first seen at line {previous}"
            )
        else:
            pr_bx_evidence_seen[evidence_type] = index
    elif pr_id == "PR-ET":
        previous = pr_et_evidence_seen.get(evidence_type)
        if previous is not None:
            failures.append(
                f"{ledger.name}:{index} PR-ET duplicates evidence_type "
                f"{evidence_type!r}; first seen at line {previous}"
            )
        else:
            pr_et_evidence_seen[evidence_type] = index
    elif pr_id == "PR-ER":
        previous = pr_er_evidence_seen.get(evidence_type)
        if previous is not None:
            failures.append(
                f"{ledger.name}:{index} PR-ER duplicates evidence_type "
                f"{evidence_type!r}; first seen at line {previous}"
            )
        else:
            pr_er_evidence_seen[evidence_type] = index
    elif pr_id == "PR-EL":
        previous = pr_el_evidence_seen.get(evidence_type)
        if previous is not None:
            failures.append(
                f"{ledger.name}:{index} PR-EL duplicates evidence_type "
                f"{evidence_type!r}; first seen at line {previous}"
            )
        else:
            pr_el_evidence_seen[evidence_type] = index
    elif pr_id == "PR-EO":
        previous = pr_eo_evidence_seen.get(evidence_type)
        if previous is not None:
            failures.append(
                f"{ledger.name}:{index} PR-EO duplicates evidence_type "
                f"{evidence_type!r}; first seen at line {previous}"
            )
        else:
            pr_eo_evidence_seen[evidence_type] = index
    elif pr_id == "PR-EP":
        previous = pr_ep_evidence_seen.get(evidence_type)
        if previous is not None:
            failures.append(
                f"{ledger.name}:{index} PR-EP duplicates evidence_type "
                f"{evidence_type!r}; first seen at line {previous}"
            )
        else:
            pr_ep_evidence_seen[evidence_type] = index
    elif pr_id == "PR-FA":
        previous = pr_fa_evidence_seen.get(evidence_type)
        if previous is not None:
            failures.append(
                f"{ledger.name}:{index} PR-FA duplicates evidence_type "
                f"{evidence_type!r}; first seen at line {previous}"
            )
        else:
            pr_fa_evidence_seen[evidence_type] = index
    elif pr_id == "PR-FJ":
        previous = pr_fj_evidence_seen.get(evidence_type)
        if previous is not None:
            failures.append(
                f"{ledger.name}:{index} PR-FJ duplicates evidence_type "
                f"{evidence_type!r}; first seen at line {previous}"
            )
        else:
            pr_fj_evidence_seen[evidence_type] = index
    elif pr_id == "PR-FW":
        if evidence_type in PR_FW_GENERIC_EVIDENCE_TYPES:
            failures.append(
                f"{ledger.name}:{index} PR-FW evidence_type must be endpoint/gate "
                f"specific, got {evidence_type!r}"
            )
        previous = pr_fw_evidence_seen.get(evidence_type)
        if previous is not None:
            failures.append(
                f"{ledger.name}:{index} PR-FW duplicates evidence_type "
                f"{evidence_type!r}; first seen at line {previous}"
            )
        else:
            pr_fw_evidence_seen[evidence_type] = index
    if not artifact:
        failures.append(f"{ledger.name}:{index} {pr_id} has empty artifact")
    elif artifact.startswith(("http://", "https://")):
        pass
    elif not (root / artifact).exists():
        failures.append(f"{ledger.name}:{index} {pr_id} artifact does not exist: {artifact}")
    if not command:
        failures.append(f"{ledger.name}:{index} {pr_id} has empty command")
    else:
        result = subprocess.run(
            ["bash", "-n", "-c", command],
            cwd=root,
            text=True,
            capture_output=True,
            check=False,
        )
        if result.returncode != 0:
            detail = (result.stderr or result.stdout).strip().splitlines()
            suffix = f": {detail[0]}" if detail else ""
            failures.append(
                f"{ledger.name}:{index} {pr_id} command is not bash-parseable{suffix}"
            )
        if non_local_command.search(command):
            failures.append(
                f"{ledger.name}:{index} {pr_id} command uses non-local GitHub CLI evidence"
            )
    by_pr.setdefault(pr_id, []).append(row)

missing = sorted(completed.difference(by_pr))
if missing:
    failures.append(
        "completed roadmap PRs lack evidence index rows: " + ", ".join(missing)
    )

required_governance = {"PR-CL", "PR-EJ"}
missing_governance = sorted(
    pr_id for pr_id in required_governance if pr_id in active_prs and pr_id not in by_pr
)
if missing_governance:
    failures.append(
        "active governance PRs lack evidence index rows: "
        + ", ".join(missing_governance)
    )

required_evidence = {
    "PR-EJ": set(PR_EJ_REQUIRED_EVIDENCE),
    "PR-EK": set(PR_EK_REQUIRED_EVIDENCE),
    "PR-EM": set(PR_EM_REQUIRED_EVIDENCE),
    "PR-BX": set(PR_BX_REQUIRED_EVIDENCE),
    "PR-ER": set(PR_ER_REQUIRED_EVIDENCE),
    "PR-EL": set(PR_EL_REQUIRED_EVIDENCE),
    "PR-EO": set(PR_EO_REQUIRED_EVIDENCE),
    "PR-EP": set(PR_EP_REQUIRED_EVIDENCE),
    "PR-FA": set(PR_FA_REQUIRED_EVIDENCE),
    "PR-FJ": set(PR_FJ_REQUIRED_EVIDENCE),
    "PR-EX": {
        "api-base-auth-multiplex-channel-state-gate",
        "resource-envelope-runtime-gate",
        "symbol-search-local-state-gate",
    },
    "PR-FR": {
        "settings-credential-static-adapter-copy-boundary-gate",
        "settings-selected-venue-trading-runtime-missing-evidence-browser-gate",
        "settings-private-order-stream-ok-capture-readiness-browser-gate",
        "settings-selected-venue-trading-runtime-all-ok-local-capture-shaped-browser-gate",
        "private-ws-order-stream-topbar-browser-gate",
        "private-order-stream-live-sample-acceptance-gate",
        "private-order-stream-live-sample-index-gate",
    },
    "PR-FZ": {
        "private-funding-payment-eight-venue-matrix-gate",
        "private-funding-payment-runtime-router-gate",
        "private-funding-payment-ingest-idempotency-gate",
        "sql-ledger-funding-slippage-replay-gate",
        "execution-run-cost-replay-gate",
        "close-run-cost-replay-gate",
        "sql-ledger-commit-ack-integrity-gate",
        "run-cost-fact-rebuild-gate",
        "postgres-roundtrip-restart-gate",
    },
    "PR-FW": {
        "exchange-evidence",
        "exchange-evidence-should-panic-gate",
        "exchange-adapter-coverage-exactness",
        "close-position-display-only-registry-boundary-gate",
        "endpoint-spec-checked-at-not-future-gate",
        "endpoint-spec-doc-url-domain-gate",
        "endpoint-spec-file-bound-test-identity-gate",
        "endpoint-spec-fixture-venue-path-gate",
        "endpoint-spec-fixture-parser-binding-gate",
        "endpoint-spec-parser-request-non-skip-body-gate",
        "endpoint-spec-request-builder-path-binding-gate",
        "endpoint-spec-runtime-evidence-parity-gate",
        "endpoint-spec-unrecorded-probe-explicit-identity-gate",
        "endpoint-spec-unique-unrecorded-provenance-gate",
        "evidence-runnable-attribute-block-comment-gate",
        "evidence-runnable-parent-module-scope-gate",
        "operation-diagnostic-fixture-closure-gate",
        "operation-diagnostic-full-venue-fixture-coverage",
        "operation-diagnostic-fixture-venue-binding-gate",
        "operation-diagnostic-fixture-venue-path-gate",
        "operation-diagnostic-fixture-hash-format-gate",
        "operation-evidence-matrix",
        "operation-evidence-matrix-baseline-required-gate",
        "operation-evidence-matrix-non-skipping-fixture-gate",
        "operation-matrix-fixture-hash-uniqueness-gate",
        "operation-matrix-api-route-test-anchor-gate",
        "operation-matrix-rest-allowlist-bridge-gate",
        "operation-matrix-rest-runtime-registry-parity-gate",
        "operation-matrix-rest-tsv-derived-tests-gate",
        "operation-matrix-rest-transport-route-parity-gate",
        "operation-matrix-route-summary-tsv-parity-gate",
        "operation-matrix-runtime-registry-parity-gate",
        "operation-evidence-matrix-should-panic-gate",
        "private-ws-evidence-venue-file-binding-gate",
        "product-audit-evidence-type-specificity-gate",
        "safe-probe-request-builder-test-identity-gate",
        "transport-registry-ack-not-final-boundary-gate",
        "transport-registry-rest-recorded-metadata-completeness-gate",
        "transport-registry-rest-operation-summary-gate",
        "transport-registry-summary-boundary-gate",
        "ws-evidence-helper-attribute-block-comment-gate",
        "ws-operation-evidence-scope-boundary-gate",
        "ws-operation-evidence-registry-projection-parity-gate",
        "ws-operation-fixture-hash-pin-gate",
        "ws-operation-parser-fixture-binding-gate",
        "ws-operation-parser-non-skip-body-gate",
    },
    "PR-G": {
        "credential-safe-probe-readiness-boundary",
        "credential-safe-probe-runtime-readiness-boundary",
        "credential-update-invalidates-finality-runtime",
        "credential-update-router-family-replay-invalidation-gate",
    },
    "PR-M": {
        "runtime-acceptance-live-order-proof",
        "live-sample-acceptance-readiness",
        "live-sample-artifact-redaction-readiness",
        "live-sample-hashed-identity-readiness",
        "live-sample-committed-hash-identity-gate",
        "live-sample-evidence-index-verifier-execution",
        "live-sample-verifier-shape-gate",
        "live-sample-artifact-integrity-gate",
        "live-sample-git-tracked-artifact-gate",
        "live-sample-artifact-venue-binding-gate",
        "live-sample-command-artifact-exact-gate",
        "live-sample-envelope-raw-id-gate",
        "live-sample-checked-at-timeline-gate",
        "live-sample-symbol-context-gate",
        "live-sample-committed-capture-context-gate",
        "live-sample-committed-epoch-timestamp-gate",
        "live-sample-committed-timestamp-window-gate",
        "live-sample-companion-row-all-matches-gate",
        "live-sample-companion-row-shape-gate",
        "live-sample-self-test-fixture-reuse-gate",
        "live-sample-evidence-contract-gate",
        "live-sample-evidence-schema-exact-gate",
        "live-sample-snapshot-row-schema-exact-gate",
        "live-sample-native-transport-metadata-gate",
        "live-sample-static-usability-gate",
        "live-sample-supported-venue-suffix-gate",
        "live-order-proof-runtime-request-context-allowlist-gate",
        "live-sample-proof-count-context-gate",
        "live-sample-identity-family-consistency-gate",
        "live-sample-identity-family-completeness-gate",
        "live-sample-request-context-allowlist-gate",
        "live-sample-request-context-non-empty-value-gate",
        "live-sample-request-id-placeholder-gate",
        "live-sample-non-live-marker-gate",
        "live-sample-accepted-row-clean-status-gate",
        "live-sample-evidence-request-id-match-gate",
        "live-sample-committed-cancel-finality-source-gate",
        "live-sample-hash-algorithm-prefix-gate",
        "live-order-proof-runtime-identity-family-parity-gate",
        "credential-update-invalidates-live-order-proof",
    },
}
missing_evidence: list[str] = []
for pr_id, evidence_types in required_evidence.items():
    if pr_id not in active_prs:
        continue
    present = {
        row["evidence_type"].strip()
        for row in by_pr.get(pr_id, [])
        if row["evidence_type"].strip()
    }
    missing = sorted(evidence_types.difference(present))
    for evidence_type in missing:
        missing_evidence.append(f"{pr_id}/{evidence_type}")
    if pr_id in {"PR-FA", "PR-FJ"}:
        extra = sorted(present.difference(evidence_types))
        for evidence_type in extra:
            missing_evidence.append(f"{pr_id}/extra:{evidence_type}")
if missing_evidence:
    failures.append(
        "active roadmap PRs lack required evidence rows: "
        + ", ".join(missing_evidence)
    )

if "PR-ET" in completed:
    present = {
        row["evidence_type"].strip()
        for row in by_pr.get("PR-ET", [])
        if row["evidence_type"].strip()
    }
    expected = set(PR_ET_REQUIRED_EVIDENCE)
    missing = sorted(expected.difference(present))
    extra = sorted(present.difference(expected))
    if missing or extra:
        failures.append(
            "completed PR-ET evidence type drift: "
            f"missing={missing}, extra={extra}"
        )

if "PR-BX" in completed:
    present = {
        row["evidence_type"].strip()
        for row in by_pr.get("PR-BX", [])
        if row["evidence_type"].strip()
    }
    expected = set(PR_BX_REQUIRED_EVIDENCE)
    missing = sorted(expected.difference(present))
    extra = sorted(present.difference(expected))
    if missing or extra:
        failures.append(
            "completed PR-BX evidence type drift: "
            f"missing={missing}, extra={extra}"
        )

if "PR-ER" in completed:
    present = {
        row["evidence_type"].strip()
        for row in by_pr.get("PR-ER", [])
        if row["evidence_type"].strip()
    }
    expected = set(PR_ER_REQUIRED_EVIDENCE)
    missing = sorted(expected.difference(present))
    extra = sorted(present.difference(expected))
    if missing or extra:
        failures.append(
            "completed PR-ER evidence type drift: "
            f"missing={missing}, extra={extra}"
        )

if "PR-EL" in completed:
    present = {
        row["evidence_type"].strip()
        for row in by_pr.get("PR-EL", [])
        if row["evidence_type"].strip()
    }
    expected = set(PR_EL_REQUIRED_EVIDENCE)
    missing = sorted(expected.difference(present))
    extra = sorted(present.difference(expected))
    if missing or extra:
        failures.append(
            "completed PR-EL evidence type drift: "
            f"missing={missing}, extra={extra}"
        )

if "PR-EM" in completed:
    present = {
        row["evidence_type"].strip()
        for row in by_pr.get("PR-EM", [])
        if row["evidence_type"].strip()
    }
    expected = set(PR_EM_REQUIRED_EVIDENCE)
    missing = sorted(expected.difference(present))
    extra = sorted(present.difference(expected))
    if missing or extra:
        failures.append(
            "completed PR-EM evidence type drift: "
            f"missing={missing}, extra={extra}"
        )

if "PR-EO" in completed:
    present = {
        row["evidence_type"].strip()
        for row in by_pr.get("PR-EO", [])
        if row["evidence_type"].strip()
    }
    expected = set(PR_EO_REQUIRED_EVIDENCE)
    missing = sorted(expected.difference(present))
    extra = sorted(present.difference(expected))
    if missing or extra:
        failures.append(
            "completed PR-EO evidence type drift: "
            f"missing={missing}, extra={extra}"
        )

if "PR-EP" in completed:
    present = {
        row["evidence_type"].strip()
        for row in by_pr.get("PR-EP", [])
        if row["evidence_type"].strip()
    }
    expected = set(PR_EP_REQUIRED_EVIDENCE)
    missing = sorted(expected.difference(present))
    extra = sorted(present.difference(expected))
    if missing or extra:
        failures.append(
            "completed PR-EP evidence type drift: "
            f"missing={missing}, extra={extra}"
        )


SHELL_ASSIGNMENT = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*=.*$")


def shell_command_segments(command: str) -> list[list[str]]:
    lexer = shlex.shlex(command, posix=True, punctuation_chars=";&|")
    lexer.whitespace_split = True
    lexer.commenters = ""
    segments: list[list[str]] = []
    current: list[str] = []
    try:
        tokens = list(lexer)
    except ValueError:
        return []
    for token in tokens:
        if token and all(char in ";&|" for char in token):
            if current:
                segments.append(current)
                current = []
        else:
            current.append(token)
    if current:
        segments.append(current)
    return segments


def command_words(segment: list[str]) -> tuple[list[str], list[str]]:
    split = 0
    while split < len(segment) and SHELL_ASSIGNMENT.fullmatch(segment[split]):
        split += 1
    return segment[:split], segment[split:]


def runs_command(
    segments: list[list[str]], expected: list[str], *, exact: bool = False
) -> bool:
    for segment in segments:
        _, words = command_words(segment)
        if (exact and words == expected) or (
            not exact and words[: len(expected)] == expected
        ):
            return True
    return False


def runs_docs_index(segments: list[list[str]]) -> bool:
    if runs_command(segments, ["bash", "scripts/check_product_audit_evidence_index.sh"]):
        return True
    for segment in segments:
        assignments, words = command_words(segment)
        if (
            "VERIFY_REPO_GATES_SCOPE=docs" in assignments
            and words[:2] == ["bash", "scripts/verify_repo_gates.sh"]
        ):
            return True
    return False


def runs_focused_pr_fa_browser(
    segments: list[list[str]], title: str
) -> bool:
    for segment in segments:
        _, words = command_words(segment)
        if title not in words:
            continue
        npm_gate = words[:3] == ["npm", "run", "test:e2e:pr-fa"]
        playwright_gate = words[:4] == [
            "npx",
            "playwright",
            "test",
            PR_FA_BROWSER_PATH,
        ]
        grep_index = next(
            (index for index, word in enumerate(words) if word in {"--grep", "-g"}),
            None,
        )
        if (
            (npm_gate or playwright_gate)
            and grep_index is not None
            and words[grep_index + 1 : grep_index + 2] == [title]
        ):
            return True
    return False


for row in by_pr.get("PR-FA", []):
    evidence_type = row["evidence_type"].strip()
    expected = PR_FA_REQUIRED_EVIDENCE.get(evidence_type)
    if expected is None:
        continue
    expected_artifact, command_anchor = expected
    artifact = row["artifact"].strip()
    command = row["command"].strip()
    if artifact != expected_artifact:
        failures.append(
            f"PR-FA {evidence_type} artifact must be {expected_artifact}"
        )
    if command_anchor not in command:
        failures.append(
            f"PR-FA {evidence_type} command must include anchor: {command_anchor}"
        )
    segments = shell_command_segments(command)
    if evidence_type == "runtime-contract":
        if not runs_command(
            segments, ["bash", "-n", "scripts/verify_runtime_contracts.sh"]
        ):
            failures.append(
                "PR-FA runtime-contract command must run bash -n "
                "scripts/verify_runtime_contracts.sh"
            )
        if not runs_command(
            segments, ["bash", "scripts/verify_runtime_contracts.sh"]
        ):
            failures.append(
                "PR-FA runtime-contract command must execute the managed/local "
                "runtime contract anchor"
            )
        if not runs_docs_index(segments):
            failures.append(
                "PR-FA runtime-contract command must execute the docs evidence index"
            )
    if evidence_type in {
        "typed-problem-browser-gate",
        "wasm-browser-performance-gate",
    }:
        if not runs_command(
            segments, ["node", "--check", PR_FA_BROWSER_PATH]
        ):
            failures.append(
                f"PR-FA {evidence_type} command must run node --check "
                f"{PR_FA_BROWSER_PATH}"
            )
        if not runs_focused_pr_fa_browser(segments, command_anchor):
            failures.append(
                f"PR-FA {evidence_type} command must run focused Playwright grep: "
                f"{command_anchor}"
            )
    if evidence_type == "completion-governance-gate":
        completion = "scripts/check_pr_fa_completion.sh"
        if not runs_command(segments, ["bash", "-n", completion]):
            failures.append(
                "PR-FA completion-governance-gate command must run bash -n"
            )
        if not runs_command(
            segments, ["bash", completion, "--self-test"], exact=True
        ):
            failures.append(
                "PR-FA completion-governance-gate command must run --self-test"
            )
        completion_check = any(
            command_words(segment)[1] in (
                ["bash", completion],
                ["bash", completion, "check"],
            )
            for segment in segments
        )
        if not completion_check:
            failures.append(
                "PR-FA completion-governance-gate command must run the completion check"
            )

for row in by_pr.get("PR-FJ", []):
    evidence_type = row["evidence_type"].strip()
    expected = PR_FJ_REQUIRED_EVIDENCE.get(evidence_type)
    if expected is None:
        continue
    expected_artifact, command_anchor = expected
    if row["artifact"].strip() != expected_artifact:
        failures.append(
            f"PR-FJ {evidence_type} artifact must be {expected_artifact}"
        )
    if command_anchor not in row["command"]:
        failures.append(
            f"PR-FJ {evidence_type} command must include anchor: {command_anchor}"
        )

for row in by_pr.get("PR-ET", []):
    evidence_type = row["evidence_type"].strip()
    expected = PR_ET_REQUIRED_EVIDENCE.get(evidence_type)
    if expected is None:
        continue
    expected_artifact, command_anchor = expected
    if row["artifact"].strip() != expected_artifact:
        failures.append(
            f"PR-ET {evidence_type} artifact must be {expected_artifact}"
        )
    if command_anchor not in row["command"]:
        failures.append(
            f"PR-ET {evidence_type} command must include anchor: {command_anchor}"
        )
    if evidence_type == "completion-governance-gate":
        completion = "scripts/check_pr_et_completion.sh"
        segments = shell_command_segments(row["command"].strip())
        if not runs_command(segments, ["bash", "-n", completion]):
            failures.append(
                "PR-ET completion-governance-gate command must run bash -n"
            )
        if not runs_command(
            segments, ["bash", completion, "--self-test"], exact=True
        ):
            failures.append(
                "PR-ET completion-governance-gate command must run --self-test"
            )
        completion_check = any(
            command_words(segment)[1]
            in (["bash", completion], ["bash", completion, "check"])
            for segment in segments
        )
        if not completion_check:
            failures.append(
                "PR-ET completion-governance-gate command must run the completion check"
            )

for row in by_pr.get("PR-BX", []):
    evidence_type = row["evidence_type"].strip()
    expected = PR_BX_REQUIRED_EVIDENCE.get(evidence_type)
    if expected is None:
        continue
    expected_artifact, command_anchor = expected
    if row["artifact"].strip() != expected_artifact:
        failures.append(
            f"PR-BX {evidence_type} artifact must be {expected_artifact}"
        )
    if command_anchor not in row["command"]:
        failures.append(
            f"PR-BX {evidence_type} command must include anchor: {command_anchor}"
        )
    if evidence_type == "completion-governance-gate":
        completion = "scripts/check_pr_bx_completion.sh"
        segments = shell_command_segments(row["command"].strip())
        if not runs_command(segments, ["bash", "-n", completion]):
            failures.append(
                "PR-BX completion-governance-gate command must run bash -n"
            )
        if not runs_command(
            segments, ["bash", completion, "--self-test"], exact=True
        ):
            failures.append(
                "PR-BX completion-governance-gate command must run --self-test"
            )
        completion_check = any(
            command_words(segment)[1]
            in (["bash", completion], ["bash", completion, "check"])
            for segment in segments
        )
        if not completion_check:
            failures.append(
                "PR-BX completion-governance-gate command must run the completion check"
            )

for row in by_pr.get("PR-ER", []):
    evidence_type = row["evidence_type"].strip()
    expected = PR_ER_REQUIRED_EVIDENCE.get(evidence_type)
    if expected is None:
        continue
    expected_artifact, command_anchor = expected
    if row["artifact"].strip() != expected_artifact:
        failures.append(
            f"PR-ER {evidence_type} artifact must be {expected_artifact}"
        )
    if command_anchor not in row["command"]:
        failures.append(
            f"PR-ER {evidence_type} command must include anchor: {command_anchor}"
        )
    if evidence_type == "completion-governance":
        completion = "scripts/check_pr_er_completion.sh"
        segments = shell_command_segments(row["command"].strip())
        if not runs_command(segments, ["bash", "-n", completion]):
            failures.append("PR-ER completion command must run bash -n")
        if not runs_command(
            segments, ["bash", completion, "--self-test"], exact=True
        ):
            failures.append("PR-ER completion command must run --self-test")
        completion_check = any(
            command_words(segment)[1]
            in (["bash", completion], ["bash", completion, "check"])
            for segment in segments
        )
        if not completion_check:
            failures.append("PR-ER completion command must run the completion check")

for row in by_pr.get("PR-EL", []):
    evidence_type = row["evidence_type"].strip()
    expected = PR_EL_REQUIRED_EVIDENCE.get(evidence_type)
    if expected is None:
        continue
    expected_artifact, command_anchor = expected
    if row["artifact"].strip() != expected_artifact:
        failures.append(
            f"PR-EL {evidence_type} artifact must be {expected_artifact}"
        )
    if command_anchor not in row["command"]:
        failures.append(
            f"PR-EL {evidence_type} command must include anchor: {command_anchor}"
        )
    if evidence_type == "completion-governance":
        completion = "scripts/check_pr_el_completion.sh"
        segments = shell_command_segments(row["command"].strip())
        if not runs_command(segments, ["bash", "-n", completion]):
            failures.append("PR-EL completion command must run bash -n")
        if not runs_command(
            segments, ["bash", completion, "--self-test"], exact=True
        ):
            failures.append("PR-EL completion command must run --self-test")
        completion_check = any(
            command_words(segment)[1]
            in (["bash", completion], ["bash", completion, "check"])
            for segment in segments
        )
        if not completion_check:
            failures.append("PR-EL completion command must run the completion check")

for row in by_pr.get("PR-EM", []):
    evidence_type = row["evidence_type"].strip()
    expected = PR_EM_REQUIRED_EVIDENCE.get(evidence_type)
    if expected is None:
        continue
    expected_artifact, command_anchor = expected
    if row["artifact"].strip() != expected_artifact:
        failures.append(
            f"PR-EM {evidence_type} artifact must be {expected_artifact}"
        )
    if command_anchor not in row["command"]:
        failures.append(
            f"PR-EM {evidence_type} command must include anchor: {command_anchor}"
        )
    if evidence_type == "completion-governance":
        completion = "scripts/check_pr_em_completion.sh"
        segments = shell_command_segments(row["command"].strip())
        if not runs_command(segments, ["bash", "-n", completion]):
            failures.append("PR-EM completion command must run bash -n")
        if not runs_command(
            segments, ["bash", completion, "--self-test"], exact=True
        ):
            failures.append("PR-EM completion command must run --self-test")
        completion_check = any(
            command_words(segment)[1]
            in (["bash", completion], ["bash", completion, "check"])
            for segment in segments
        )
        if not completion_check:
            failures.append("PR-EM completion command must run the completion check")

for row in by_pr.get("PR-EO", []):
    evidence_type = row["evidence_type"].strip()
    expected = PR_EO_REQUIRED_EVIDENCE.get(evidence_type)
    if expected is None:
        continue
    expected_artifact, command_anchor = expected
    if row["artifact"].strip() != expected_artifact:
        failures.append(
            f"PR-EO {evidence_type} artifact must be {expected_artifact}"
        )
    if command_anchor not in row["command"]:
        failures.append(
            f"PR-EO {evidence_type} command must include anchor: {command_anchor}"
        )
    if evidence_type == "product-browser-and-completion-governance":
        completion = "scripts/check_pr_eo_completion.sh"
        segments = shell_command_segments(row["command"].strip())
        if not runs_command(segments, ["bash", "-n", completion]):
            failures.append("PR-EO completion command must run bash -n")
        if not runs_command(
            segments, ["bash", completion, "--self-test"], exact=True
        ):
            failures.append("PR-EO completion command must run --self-test")
        completion_check = any(
            command_words(segment)[1]
            in (["bash", completion], ["bash", completion, "check"])
            for segment in segments
        )
        if not completion_check:
            failures.append("PR-EO completion command must run the completion check")

for row in by_pr.get("PR-EP", []):
    evidence_type = row["evidence_type"].strip()
    expected = PR_EP_REQUIRED_EVIDENCE.get(evidence_type)
    if expected is None:
        continue
    expected_artifact, command_anchor = expected
    if row["artifact"].strip() != expected_artifact:
        failures.append(
            f"PR-EP {evidence_type} artifact must be {expected_artifact}"
        )
    if command_anchor not in row["command"]:
        failures.append(
            f"PR-EP {evidence_type} command must include anchor: {command_anchor}"
        )
    if evidence_type == "product-browser-and-completion-governance":
        completion = "scripts/check_pr_ep_completion.sh"
        segments = shell_command_segments(row["command"].strip())
        if not runs_command(segments, ["bash", "-n", completion]):
            failures.append("PR-EP completion command must run bash -n")
        if not runs_command(
            segments, ["bash", completion, "--self-test"], exact=True
        ):
            failures.append("PR-EP completion command must run --self-test")
        completion_check = any(
            command_words(segment)[1]
            in (["bash", completion], ["bash", completion, "check"])
            for segment in segments
        )
        if not completion_check:
            failures.append("PR-EP completion command must run the completion check")

for row in by_pr.get("PR-FW", []):
    evidence_type = row["evidence_type"].strip()
    if evidence_type != "endpoint-spec-parser-request-non-skip-body-gate":
        continue
    artifact = row["artifact"].strip()
    command = row["command"].strip()
    expected_command = "bash -n scripts/check_exchange_evidence_debt.sh; bash scripts/check_exchange_evidence_debt.sh --self-test; bash scripts/check_exchange_evidence_debt.sh; bash scripts/check_product_audit_evidence_index.sh"
    if artifact != "scripts/check_exchange_evidence_debt.sh":
        failures.append(
            "PR-FW endpoint-spec-parser-request-non-skip-body-gate artifact "
            "must be scripts/check_exchange_evidence_debt.sh"
        )
    if command != expected_command:
        failures.append(
            "PR-FW endpoint-spec-parser-request-non-skip-body-gate command "
            f"must equal: {expected_command}"
        )
    debt_gate = root / "scripts/check_exchange_evidence_debt.sh"
    debt_text = debt_gate.read_text(encoding="utf-8") if debt_gate.exists() else ""
    required_markers = [
        ("skip marker registry", "TEST_SKIP_MARKERS="),
        ("registry env wiring", "CROSSLINE_TEST_SKIP_MARKERS"),
        ("skip marker scanner", "sub body_has_skip_marker"),
        ("parser skip marker gate", 'test_body_has_no_skip_marker "$parser_test"'),
        (
            "request-builder skip marker gate",
            'test_body_has_no_skip_marker "$request_builder_test"',
        ),
        ("string marker self-test", "accepted_fixture_string_marker_test"),
        ("return-ok self-test", "fixture_return_ok_fails"),
        ("request temp-dir self-test", "request_path_temp_dir_fails"),
    ]
    for label, marker in required_markers:
        if marker not in debt_text:
            failures.append(
                "PR-FW endpoint-spec parser/request non-skip gate missing "
                + label
            )

for row in by_pr.get("PR-EX", []):
    evidence_type = row["evidence_type"].strip()
    if evidence_type != "resource-envelope-runtime-gate":
        continue
    artifact = row["artifact"].strip()
    command = row["command"].strip()
    expected_command = "cargo fmt --manifest-path frontend/Cargo.toml --all -- --check; CARGO_BUILD_JOBS=8 cargo test --manifest-path frontend/Cargo.toml --lib polling --no-fail-fast; CARGO_BUILD_JOBS=8 cargo test --manifest-path frontend/Cargo.toml --lib opportunity_envelope --no-fail-fast; CARGO_BUILD_JOBS=8 cargo test --manifest-path frontend/Cargo.toml --lib detail_state --no-fail-fast; bash scripts/check_product_audit_evidence_index.sh"
    if artifact != "frontend/src/state/polling.rs":
        failures.append(
            "PR-EX resource-envelope-runtime-gate artifact must be "
            "frontend/src/state/polling.rs"
        )
    if command != expected_command:
        failures.append(
            "PR-EX resource-envelope-runtime-gate command must equal: "
            + expected_command
        )
    marker_files = {
        "polling": root / "frontend/src/state/polling.rs",
        "opportunity_envelope": root / "frontend/src/panels/modules/opportunity_envelope.rs",
        "detail": root
        / "frontend/src/panels/modules/opportunities/components/detail_panel.rs",
    }
    marker_text = {
        name: path.read_text(encoding="utf-8") if path.exists() else ""
        for name, path in marker_files.items()
    }
    required_markers = [
        ("ResourceEnvelope trait", "polling", "pub trait ResourceEnvelope"),
        ("use_resource_envelope helper", "polling", "pub fn use_resource_envelope"),
        (
            "explicit publish helper",
            "polling",
            "pub fn apply_resource_envelope_state",
        ),
        ("stale state preservation", "polling", "LoadState::Stale"),
        ("cold error state", "polling", "LoadState::Error"),
        ("typed problem contract", "polling", "ApiProblem"),
        ("request id regression", "polling", "req-envelope-1"),
        ("retry_after regression", "polling", "retry_after_ms"),
        (
            "opportunity envelope impl",
            "opportunity_envelope",
            "impl ResourceEnvelope for OpportunityListEnvelope",
        ),
        (
            "opportunity publish rows bridge",
            "opportunity_envelope",
            "apply_resource_envelope_state(state, envelope, (), published_rows)",
        ),
        (
            "detail state prior anchor",
            "detail",
            "stale_empty_detail_state_keeps_problem_visible",
        ),
    ]
    for label, key, marker in required_markers:
        if marker not in marker_text.get(key, ""):
            failures.append(f"PR-EX resource envelope runtime gate missing {label}")

for row in by_pr.get("PR-EX", []):
    evidence_type = row["evidence_type"].strip()
    if evidence_type != "symbol-search-local-state-gate":
        continue
    artifact = row["artifact"].strip()
    command = row["command"].strip()
    expected_command = "cargo fmt --manifest-path frontend/Cargo.toml --all -- --check; CARGO_BUILD_JOBS=8 cargo test --manifest-path frontend/Cargo.toml --lib symbol_search --no-fail-fast; CARGO_BUILD_JOBS=8 cargo test --manifest-path frontend/Cargo.toml --lib symbol_opportunity --no-fail-fast; CARGO_BUILD_JOBS=8 cargo test --manifest-path frontend/Cargo.toml --lib symbol_futures --no-fail-fast; bash scripts/check_product_audit_evidence_index.sh"
    if artifact != "frontend/src/panels/modules/instrument_search.rs":
        failures.append(
            "PR-EX symbol-search-local-state-gate artifact must be "
            "frontend/src/panels/modules/instrument_search.rs"
        )
    if command != expected_command:
        failures.append(
            "PR-EX symbol-search-local-state-gate command must equal: "
            + expected_command
        )
    marker_files = {
        "instrument_search": root / "frontend/src/panels/modules/instrument_search.rs",
        "opportunities_list": root / "frontend/src/panels/modules/opportunities/data/list.rs",
        "opportunities_tests": root
        / "frontend/src/panels/modules/opportunities/data/tests/behavior.rs",
        "opportunities_runtime_tests": root
        / "frontend/src/panels/modules/opportunities/data/tests/runtime.rs",
        "futures_search": root / "frontend/src/panels/modules/futures/data/search.rs",
        "futures_tests": root
        / "frontend/src/panels/modules/futures/data/tests/selection.rs",
        "futures_runtime_tests": root
        / "frontend/src/panels/modules/futures/data/tests/runtime.rs",
        "workstation": root / "frontend/src/panels/workstation.rs",
    }
    marker_text = {
        name: path.read_text(encoding="utf-8") if path.exists() else ""
        for name, path in marker_files.items()
    }
    required_markers = [
        (
            "symbol search problem helper",
            "instrument_search",
            "pub(crate) fn symbol_search_problem",
        ),
        ("symbol query details", "instrument_search", '"symbolSearch"'),
        ("backend detail nesting", "instrument_search", '"backendDetails"'),
        ("request id regression", "instrument_search", "req-symbol"),
        ("retry_after regression", "instrument_search", "with_retry_after_ms(Some(1_500))"),
        (
            "opportunities error bridge",
            "opportunities_list",
            "symbol_search_problem(error.problem, query, cursor)",
        ),
        (
            "opportunities stale problem context",
            "opportunities_tests",
            "symbol_opportunity_search_error_keeps_stale_rows",
        ),
        (
            "opportunities remount gate",
            "opportunities_runtime_tests",
            "opportunity_symbol_search_local_state_survives_runtime_remount",
        ),
        (
            "futures error bridge",
            "futures_search",
            "symbol_search_problem(error.problem, query, cursor)",
        ),
        (
            "futures stale problem context",
            "futures_tests",
            "symbol_futures_search_error_keeps_stale_rows",
        ),
        (
            "futures remount gate",
            "futures_runtime_tests",
            "pr_dx_shared_opportunity_runtime_survives_futures_remount",
        ),
        ("workstation opportunities runtime", "workstation", "create_opportunities_runtime()"),
        (
            "workstation futures runtime",
            "workstation",
            "create_futures_runtime(opportunities_runtime)",
        ),
    ]
    for label, key, marker in required_markers:
        if marker not in marker_text.get(key, ""):
            failures.append(f"PR-EX symbol search local state gate missing {label}")

for row in by_pr.get("PR-EX", []):
    evidence_type = row["evidence_type"].strip()
    if evidence_type != "api-base-auth-multiplex-channel-state-gate":
        continue
    artifact = row["artifact"].strip()
    command = row["command"].strip()
    expected_command = "cargo fmt --manifest-path frontend/Cargo.toml --all -- --check; CARGO_BUILD_JOBS=8 cargo test --manifest-path frontend/Cargo.toml --lib ws_runtime --no-fail-fast; CARGO_BUILD_JOBS=8 cargo test --manifest-path frontend/Cargo.toml --lib api_base_validate --no-fail-fast; CARGO_BUILD_JOBS=8 cargo clippy --manifest-path frontend/Cargo.toml --target wasm32-unknown-unknown --all-targets -- -D warnings; bash scripts/check_product_audit_evidence_index.sh"
    if artifact != "frontend/src/api/ws_runtime.rs":
        failures.append(
            "PR-EX api-base-auth-multiplex-channel-state-gate artifact must be "
            "frontend/src/api/ws_runtime.rs"
        )
    if command != expected_command:
        failures.append(
            "PR-EX api-base-auth-multiplex-channel-state-gate command must equal: "
            + expected_command
        )
    marker_files = {
        "ws_runtime": root / "frontend/src/api/ws_runtime.rs",
        "ws": root / "frontend/src/api/ws.rs",
        "settings_actions_api_base": root
        / "frontend/src/panels/modules/settings/data/api_base.rs",
        "settings_api_base_tests": root
        / "frontend/src/panels/modules/settings/data/tests/api_base.rs",
    }
    marker_text = {
        name: path.read_text(encoding="utf-8") if path.exists() else ""
        for name, path in marker_files.items()
    }
    required_markers = [
        (
            "request-correlated pending subscribe queue",
            "ws_runtime",
            "pending_subscribe_batches: RefCell<VecDeque<PendingSubscribe>>",
        ),
        (
            "initial subscribe request-id queue",
            "ws_runtime",
            "self.queue_pending_subscribe(request_id, channels.clone())",
        ),
        (
            "ack uses request-id correlation",
            "ws_runtime",
            "let pending = match self.take_pending_subscribe(request_id)",
        ),
        (
            "runtime-owned channel state",
            "ws_runtime",
            "channel_states: RefCell<BTreeMap<String, WsChannelState>>",
        ),
        (
            "subscriber full-state sync",
            "ws_runtime",
            "fn sync_subscriber_state",
        ),
        (
            "hot subscribe ack regression",
            "ws_runtime",
            "runtime_hot_subscribe_ack_missing_requested_channel_reports_problem",
        ),
        (
            "late subscriber full-state regression",
            "ws_runtime",
            "runtime_existing_channel_inherits_full_state_without_resubscribe",
        ),
        (
            "non-wasm ws timestamp fallback",
            "ws",
            "#[cfg(not(target_arch = \"wasm32\"))]",
        ),
        (
            "api validation result",
            "settings_actions_api_base",
            "struct ApiBaseValidationResult",
        ),
        (
            "ws ticket validation call",
            "settings_actions_api_base",
            "validation_request_with_timeout(client.ws_ticket(), \"/api/auth/ws-ticket\")",
        ),
        (
            "empty ticket fail closed",
            "settings_actions_api_base",
            "WS_TICKET_EMPTY",
        ),
        (
            "ticket readiness copy",
            "settings_actions_api_base",
            "/api/auth/ws-ticket 探测通过",
        ),
        (
            "ticket secrecy regression",
            "settings_api_base_tests",
            "api_base_validate_success_message_reports_ws_ticket_without_secret",
        ),
        (
            "bearer secrecy regression",
            "settings_api_base_tests",
            "!message.contains(\"Bearer\")",
        ),
    ]
    for label, key, marker in required_markers:
        if marker not in marker_text.get(key, ""):
            failures.append(
                f"PR-EX api base/auth multiplex channel gate missing {label}"
            )

for row in by_pr.get("PR-FR", []):
    evidence_type = row["evidence_type"].strip()
    artifact = row["artifact"].strip()
    command = row["command"].strip()
    if evidence_type == "settings-credential-static-adapter-copy-boundary-gate":
        expected_command = 'node --check test/e2e/mock_api.mjs; node --check test/e2e/data_pipeline.spec.ts; CI=1 npm run test:e2e:data-pipeline -- --grep "settings credentials keep static adapter copy separate from runtime readiness"; bash scripts/check_product_audit_evidence_index.sh'
        if artifact != "test/e2e/data_pipeline.spec.ts":
            failures.append(
                "PR-FR settings-credential-static-adapter-copy-boundary-gate "
                "artifact must be test/e2e/data_pipeline.spec.ts"
            )
        if command != expected_command:
            failures.append(
                "PR-FR settings-credential-static-adapter-copy-boundary-gate "
                f"command must equal: {expected_command}"
            )
        marker_files = {
            "mock": root / "test/e2e/mock_api.mjs",
            "spec": root / "test/e2e/data_pipeline.spec.ts",
            "format": root
            / "frontend/src/panels/modules/settings/tabs/venue_credentials/format.rs",
            "format_status": root
            / "frontend/src/panels/modules/settings/tabs/venue_credentials/format/status.rs",
            "capability": root
            / "frontend/src/panels/modules/settings/tabs/venue_credentials/capability.rs",
            "credential_tests": root
            / "frontend/src/panels/modules/settings/tabs/venue_credentials/tests_credentials/ws.rs",
            "adapter": root / "frontend/src/panels/modules/settings/tabs/adapters.rs",
            "panels": root
            / "frontend/src/panels/modules/settings/tabs/venue_credentials/panels.rs",
            "panels_operation_health": root
            / "frontend/src/panels/modules/settings/tabs/venue_credentials/panels/operation_health.rs",
            "api_adapter": root / "crates/api/src/routers/trading/adapters.rs",
            "operation_labels": root / "shared-types/src/venues/operation_kind_labels.rs",
            "copy_gate": root / "scripts/product_copy_gate.sh",
        }
        marker_text = {
            name: path.read_text(encoding="utf-8") if path.exists() else ""
            for name, path in marker_files.items()
        }
        required_markers = [
            (
                "mock scenario prefix",
                "mock",
                '"/e2e-settings-credential-static-adapter-boundary"',
            ),
            ("mock validation evidence", "mock", "validationEvidence"),
            ("mock order permission kind", "mock", 'kind: "order_permission"'),
            ("mock unknown order permission", "mock", 'status: "unknown"'),
            ("mock configured adapter", "mock", "credentialsAvailable: true"),
            ("mock static live write", "mock", "liveWrite: true"),
            (
                "browser gate title",
                "spec",
                "settings credentials keep static adapter copy separate from runtime readiness",
            ),
            (
                "credential summary boundary",
                "format_status",
                "{configured}/{} 字段已填写 / {missing} / {validation} / 当前状态待运行态证据 / {write_support} / {}",
            ),
            (
                "static capability boundary",
                "capability",
                "静态写单声明；仍需 order_permission/private WS/order_finality 证据。",
            ),
            (
                "credential field label test",
                "credential_tests",
                "credential_field_label_never_claims_validation",
            ),
            (
                "static capability test",
                "capability",
                "static_capability_summary_does_not_claim_live_readiness",
            ),
            (
                "adapter field group copy",
                "adapter",
                "字段组已补齐",
            ),
            (
                "adapter runtime evidence copy",
                "adapter",
                "可选路由；下单仍需票据级权限与运行态证据",
            ),
            (
                "order permission safe-noop boundary",
                "panels_operation_health",
                "safe/noop 不授予 live_write",
            ),
            (
                "order write runtime evidence boundary",
                "panels_operation_health",
                "写单运行态需 live place/cancel/finality 证据",
            ),
            (
                "api adapter disabled copy",
                "api_adapter",
                "至少补齐一个交易所 API 字段组",
            ),
            (
                "operation label copy",
                "operation_labels",
                'Self::OrderWrite => "写单运行态"',
            ),
            (
                "copy gate API/static readiness scan",
                "copy_gate",
                "Settings copy must keep credential/static adapter/runtime evidence separate",
            ),
        ]
        for label, key, marker in required_markers:
            if marker not in marker_text.get(key, ""):
                failures.append(f"PR-FR settings static adapter boundary gate missing {label}")
        for check in (
            ["node", "--check", str(root / "test/e2e/mock_api.mjs")],
            ["node", "--check", str(root / "test/e2e/data_pipeline.spec.ts")],
        ):
            result = subprocess.run(
                check,
                cwd=root,
                text=True,
                capture_output=True,
                check=False,
            )
            if result.returncode != 0:
                detail = (result.stderr or result.stdout).strip().splitlines()
                suffix = f": {detail[0]}" if detail else ""
                failures.append(
                    "PR-FR settings static adapter browser fixture syntax failed"
                    + suffix
                )
    if evidence_type == "settings-selected-venue-trading-runtime-missing-evidence-browser-gate":
        expected_command = 'node --check test/e2e/mock_api.mjs; node --check test/e2e/data_pipeline.spec.ts; CI=1 npm run test:e2e:data-pipeline -- --grep "settings credentials keep static adapter copy separate from runtime readiness"; bash scripts/check_product_audit_evidence_index.sh'
        if artifact != "test/e2e/data_pipeline.spec.ts":
            failures.append(
                "PR-FR settings-selected-venue-trading-runtime-missing-evidence-browser-gate "
                "artifact must be test/e2e/data_pipeline.spec.ts"
            )
        if command != expected_command:
            failures.append(
                "PR-FR settings-selected-venue-trading-runtime-missing-evidence-browser-gate "
                f"command must equal: {expected_command}"
            )
        marker_files = {
            "mock": root / "test/e2e/mock_api.mjs",
            "spec": root / "test/e2e/data_pipeline.spec.ts",
            "format": root
            / "frontend/src/panels/modules/settings/tabs/venue_credentials/format.rs",
            "format_status": root
            / "frontend/src/panels/modules/settings/tabs/venue_credentials/format/status.rs",
            "panels": root
            / "frontend/src/panels/modules/settings/tabs/venue_credentials/panels.rs",
            "panels_operation_health": root
            / "frontend/src/panels/modules/settings/tabs/venue_credentials/panels/operation_health.rs",
        }
        marker_text = {
            name: path.read_text(encoding="utf-8") if path.exists() else ""
            for name, path in marker_files.items()
        }
        required_markers = [
            (
                "mock scenario prefix",
                "mock",
                '"/e2e-settings-credential-static-adapter-boundary"',
            ),
            (
                "runtime summary formatter",
                "format_status",
                "当前运行态 {ready}/{total} 正常 · {attention}/{total} 待处理",
            ),
            (
                "browser gate selected venue summary",
                "spec",
                "okx · 当前运行态 0/4 正常 · 4/4 待处理",
            ),
            (
                "browser gate private stream variable",
                "spec",
                "const privateOrderStreamRow = page",
            ),
            (
                "browser gate private stream operation",
                "spec",
                "private_ws_order_stream",
            ),
            (
                "browser gate private stream source",
                "spec",
                "private_ws_runtime",
            ),
            (
                "browser gate private stream missing message",
                "spec",
                "okx 暂无私有订单流运行态记录",
            ),
            (
                "browser gate private stream expected evidence",
                "spec",
                "私有订单事件流需要运行态样本",
            ),
            (
                "browser gate private stream no tradable copy",
                "spec",
                'expect(privateOrderStreamRow).not.toContainText("可下单")',
            ),
            (
                "browser gate finality variable",
                "spec",
                "const orderFinalityRow = page",
            ),
            (
                "browser gate finality operation",
                "spec",
                "order_finality",
            ),
            (
                "browser gate finality source",
                "spec",
                "run_finality",
            ),
            (
                "browser gate finality missing message",
                "spec",
                "okx 暂无订单终态回查运行态记录",
            ),
            (
                "browser gate finality expected evidence",
                "spec",
                "未决订单产生后由 REST/WS 终态回查写入",
            ),
            (
                "browser gate finality no verified copy",
                "spec",
                'expect(orderFinalityRow).not.toContainText("权限验证完整")',
            ),
            (
                "panels private stream expected source",
                "panels_operation_health",
                'VenueOperationKind::PrivateWsOrderStream => "private_ws_runtime"',
            ),
            (
                "panels finality expected source",
                "panels_operation_health",
                'VenueOperationKind::OrderFinality => "run_finality"',
            ),
        ]
        for label, key, marker in required_markers:
            if marker not in marker_text.get(key, ""):
                failures.append(f"PR-FR selected venue runtime missing gate missing {label}")
        for check in (
            ["node", "--check", str(root / "test/e2e/mock_api.mjs")],
            ["node", "--check", str(root / "test/e2e/data_pipeline.spec.ts")],
        ):
            result = subprocess.run(
                check,
                cwd=root,
                text=True,
                capture_output=True,
                check=False,
            )
            if result.returncode != 0:
                detail = (result.stderr or result.stdout).strip().splitlines()
                suffix = f": {detail[0]}" if detail else ""
                failures.append(
                    "PR-FR selected venue runtime missing browser fixture syntax failed"
                    + suffix
                )
    if evidence_type == "settings-private-order-stream-ok-capture-readiness-browser-gate":
        expected_command = 'node --check test/e2e/mock_api.mjs; node --check test/e2e/data_pipeline.spec.ts; CI=1 npm run test:e2e:data-pipeline -- --grep "settings credentials surface private order stream capture readiness"; bash scripts/check_product_audit_evidence_index.sh'
        if artifact != "test/e2e/data_pipeline.spec.ts":
            failures.append(
                "PR-FR settings-private-order-stream-ok-capture-readiness-browser-gate "
                "artifact must be test/e2e/data_pipeline.spec.ts"
            )
        if command != expected_command:
            failures.append(
                "PR-FR settings-private-order-stream-ok-capture-readiness-browser-gate "
                f"command must equal: {expected_command}"
            )
        marker_files = {
            "mock": root / "test/e2e/mock_api.mjs",
            "spec": root / "test/e2e/data_pipeline.spec.ts",
        }
        marker_text = {
            name: path.read_text(encoding="utf-8") if path.exists() else ""
            for name, path in marker_files.items()
        }
        required_markers = [
            (
                "mock scenario prefix",
                "mock",
                '"/e2e-settings-private-order-stream-ok-capture-readiness"',
            ),
            (
                "mock ok private stream row",
                "mock",
                "function settingsPrivateOrderStreamOk()",
            ),
            (
                "mock ok private stream source",
                "mock",
                'source: "private_ws_runtime"',
            ),
            (
                "mock official order stream evidence",
                "mock",
                '"official_evidence=order_state_stream"',
            ),
            (
                "mock request id",
                "mock",
                '"req-settings-private-order-stream-ok"',
            ),
            (
                "browser gate title",
                "spec",
                "settings credentials surface private order stream capture readiness",
            ),
            (
                "browser gate selected venue summary",
                "spec",
                "okx · 当前运行态 1/4 正常 · 3/4 待处理",
            ),
            (
                "browser gate trading runtime panel scope",
                "spec",
                "const tradingRuntimePanel = page",
            ),
            (
                "browser gate private stream variable",
                "spec",
                "const privateOrderStreamRow = tradingRuntimePanel",
            ),
            (
                "browser gate private stream ok source",
                "spec",
                "private_ws_runtime",
            ),
            (
                "browser gate freshness",
                "spec",
                "freshness 800ms",
            ),
            (
                "browser gate sample rows",
                "spec",
                "3/3",
            ),
            (
                "browser gate request id",
                "spec",
                "request_id req-settings-private-order-stream-ok",
            ),
            (
                "browser gate official evidence",
                "spec",
                "official_evidence=order_state_stream",
            ),
            (
                "browser gate no tradable copy",
                "spec",
                'expect(privateOrderStreamRow).not.toContainText("可下单")',
            ),
            (
                "browser gate no full permission copy",
                "spec",
                'expect(privateOrderStreamRow).not.toContainText("权限验证完整")',
            ),
            (
                "browser gate order write gap",
                "spec",
                "const writeRow = tradingRuntimePanel.locator",
            ),
            (
                "browser gate finality gap",
                "spec",
                "const orderFinalityRow = tradingRuntimePanel",
            ),
        ]
        for label, key, marker in required_markers:
            if marker not in marker_text.get(key, ""):
                failures.append(f"PR-FR private stream capture readiness gate missing {label}")
        for check in (
            ["node", "--check", str(root / "test/e2e/mock_api.mjs")],
            ["node", "--check", str(root / "test/e2e/data_pipeline.spec.ts")],
        ):
            result = subprocess.run(
                check,
                cwd=root,
                text=True,
                capture_output=True,
                check=False,
            )
            if result.returncode != 0:
                detail = (result.stderr or result.stdout).strip().splitlines()
                suffix = f": {detail[0]}" if detail else ""
                failures.append(
                    "PR-FR private stream capture readiness browser fixture syntax failed"
                    + suffix
                )
    if evidence_type == "settings-selected-venue-trading-runtime-all-ok-local-capture-shaped-browser-gate":
        expected_command = 'node --check test/e2e/mock_api.mjs; node --check test/e2e/data_pipeline.spec.ts; CI=1 npm run test:e2e:data-pipeline -- --grep "settings credentials surface local capture-shaped trading runtime 4/4 ok gate"; bash scripts/check_product_audit_evidence_index.sh'
        if artifact != "test/e2e/data_pipeline.spec.ts":
            failures.append(
                "PR-FR settings-selected-venue-trading-runtime-all-ok-local-capture-shaped-browser-gate "
                "artifact must be test/e2e/data_pipeline.spec.ts"
            )
        if command != expected_command:
            failures.append(
                "PR-FR settings-selected-venue-trading-runtime-all-ok-local-capture-shaped-browser-gate "
                f"command must equal: {expected_command}"
            )
        marker_files = {
            "mock": root / "test/e2e/mock_api.mjs",
            "spec": root / "test/e2e/data_pipeline.spec.ts",
        }
        marker_text = {
            name: path.read_text(encoding="utf-8") if path.exists() else ""
            for name, path in marker_files.items()
        }
        required_markers = [
            (
                "mock all-ok scenario prefix",
                "mock",
                '"/e2e-settings-selected-venue-trading-runtime-all-ok-local-capture-shaped"',
            ),
            (
                "mock order permission ok row",
                "mock",
                "function settingsCredentialOrderPermissionOkLocalCapture()",
            ),
            (
                "mock order write ok row",
                "mock",
                "function settingsOrderWriteOkLocalCapture()",
            ),
            (
                "mock private stream ok row",
                "mock",
                "function settingsPrivateOrderStreamOkLocalCapture()",
            ),
            (
                "mock order finality ok row",
                "mock",
                "function settingsOrderFinalityOkLocalCapture()",
            ),
            ("mock credential source", "mock", 'source: "credential_validation"'),
            ("mock write source", "mock", 'source: "live_order_proof_runtime"'),
            ("mock stream source", "mock", 'source: "private_ws_runtime"'),
            ("mock finality source", "mock", 'source: "run_finality_runtime"'),
            (
                "mock local gate marker",
                "mock",
                '"local_ui_ci_gate=true"',
            ),
            (
                "mock non-live marker",
                "mock",
                '"not_real_exchange_live_sample=true"',
            ),
            (
                "mock order write request id",
                "mock",
                '"req-settings-runtime-all-ok-order-write"',
            ),
            (
                "browser gate title",
                "spec",
                "settings credentials surface local capture-shaped trading runtime 4/4 ok gate",
            ),
            (
                "browser gate selected venue all-ok summary",
                "spec",
                "okx · 当前运行态 4/4 正常 · 0/4 待处理",
            ),
            (
                "browser gate completed status",
                "spec",
                "当前可用",
            ),
            (
                "browser gate trading runtime panel scope",
                "spec",
                "const tradingRuntimePanel = page",
            ),
            (
                "browser gate permission variable",
                "spec",
                "const permissionRow = tradingRuntimePanel",
            ),
            (
                "browser gate write variable",
                "spec",
                "const writeRow = tradingRuntimePanel",
            ),
            (
                "browser gate private stream variable",
                "spec",
                "const privateOrderStreamRow = tradingRuntimePanel",
            ),
            (
                "browser gate finality variable",
                "spec",
                "const orderFinalityRow = tradingRuntimePanel",
            ),
            ("browser gate permission operation", "spec", "credential_probe:order_permission"),
            ("browser gate write operation", "spec", "order_write"),
            ("browser gate private stream operation", "spec", "private_ws_order_stream"),
            ("browser gate finality operation", "spec", "order_finality"),
            ("browser gate credential source", "spec", "credential_validation"),
            ("browser gate write source", "spec", "live_order_proof_runtime"),
            ("browser gate stream source", "spec", "private_ws_runtime"),
            ("browser gate finality source", "spec", "run_finality_runtime"),
            (
                "browser gate permission request id",
                "spec",
                "request_id req-settings-runtime-all-ok-order-permission",
            ),
            (
                "browser gate write request id",
                "spec",
                "request_id req-settings-runtime-all-ok-order-write",
            ),
            (
                "browser gate stream request id",
                "spec",
                "request_id req-settings-runtime-all-ok-private-stream",
            ),
            (
                "browser gate finality request id",
                "spec",
                "request_id req-settings-runtime-all-ok-order-finality",
            ),
            (
                "browser gate local marker",
                "spec",
                "local_ui_ci_gate=true",
            ),
            (
                "browser gate non-live marker",
                "spec",
                "not_real_exchange_live_sample=true",
            ),
        ]
        for label, key, marker in required_markers:
            if marker not in marker_text.get(key, ""):
                failures.append(f"PR-FR selected venue all-ok local capture gate missing {label}")
        for check in (
            ["node", "--check", str(root / "test/e2e/mock_api.mjs")],
            ["node", "--check", str(root / "test/e2e/data_pipeline.spec.ts")],
        ):
            result = subprocess.run(
                check,
                cwd=root,
                text=True,
                capture_output=True,
                check=False,
            )
            if result.returncode != 0:
                detail = (result.stderr or result.stdout).strip().splitlines()
                suffix = f": {detail[0]}" if detail else ""
                failures.append(
                    "PR-FR selected venue all-ok local capture browser fixture syntax failed"
                    + suffix
                )
    if evidence_type == "private-ws-order-stream-topbar-browser-gate":
        expected_command = 'node --check test/e2e/mock_api.mjs; node --check test/e2e/data_pipeline.spec.ts; CI=1 npm run test:e2e:data-pipeline -- --grep "top status bar surfaces private order stream runtime warning"; bash scripts/check_product_audit_evidence_index.sh'
        if artifact != "test/e2e/data_pipeline.spec.ts":
            failures.append(
                "PR-FR private-ws-order-stream-topbar-browser-gate artifact "
                "must be test/e2e/data_pipeline.spec.ts"
            )
        if command != expected_command:
            failures.append(
                "PR-FR private-ws-order-stream-topbar-browser-gate command "
                f"must equal: {expected_command}"
            )
        mock_api = root / "test/e2e/mock_api.mjs"
        spec = root / "test/e2e/data_pipeline.spec.ts"
        mock_text = mock_api.read_text(encoding="utf-8") if mock_api.exists() else ""
        spec_text = spec.read_text(encoding="utf-8") if spec.exists() else ""
        required_markers = [
            ("mock scenario prefix", '"/e2e-private-order-stream-warning"'),
            (
                "mock scenario removes private_ws_subscribe blocker",
                'scenario === "private-order-stream-warning" ? [] : [privateWsAuthFailure()]',
            ),
            (
                "mock private order stream request id",
                '"req-private-order-stream-stale"',
            ),
            (
                "browser gate title assertion",
                "top status bar surfaces private order stream runtime warning",
            ),
        ]
        for label, marker in required_markers:
            haystack = spec_text if label == "browser gate title assertion" else mock_text
            if marker not in haystack:
                failures.append(f"PR-FR private order stream topbar gate missing {label}")
        for check in (["node", "--check", str(mock_api)], ["node", "--check", str(spec)]):
            result = subprocess.run(
                check,
                cwd=root,
                text=True,
                capture_output=True,
                check=False,
            )
            if result.returncode != 0:
                detail = (result.stderr or result.stdout).strip().splitlines()
                suffix = f": {detail[0]}" if detail else ""
                failures.append(
                    "PR-FR private order stream topbar browser fixture syntax failed"
                    + suffix
                )
    if evidence_type != "private-order-stream-live-sample-acceptance-gate":
        if evidence_type == "private-order-stream-live-sample-index-gate":
            expected_command = "bash -n scripts/check_product_audit_evidence_index_private_order_stream_self_test.sh; python3 -m py_compile scripts/check_private_order_stream_live_sample_acceptance.py; bash scripts/check_product_audit_evidence_index_private_order_stream_self_test.sh; bash scripts/check_product_audit_evidence_index.sh"
            if artifact != "scripts/check_product_audit_evidence_index_private_order_stream_self_test.sh":
                failures.append(
                    "PR-FR private-order-stream-live-sample-index-gate artifact "
                    "must be scripts/check_product_audit_evidence_index_private_order_stream_self_test.sh"
                )
            if command != expected_command:
                failures.append(
                    "PR-FR private-order-stream-live-sample-index-gate command "
                    f"must equal: {expected_command}"
                )
        continue
    expected_command = "python3 scripts/check_private_order_stream_live_sample_acceptance.py --self-test; bash scripts/check_product_audit_evidence_index.sh"
    if artifact != "scripts/check_private_order_stream_live_sample_acceptance.py":
        failures.append(
            "PR-FR private-order-stream-live-sample-acceptance-gate artifact "
            "must be scripts/check_private_order_stream_live_sample_acceptance.py"
        )
    if command != expected_command:
        failures.append(
            "PR-FR private-order-stream-live-sample-acceptance-gate command "
            f"must equal: {expected_command}"
        )
    verifier = root / "scripts/check_private_order_stream_live_sample_acceptance.py"
    if verifier.exists():
        result = subprocess.run(
            [sys.executable, str(verifier), "--self-test"],
            cwd=root,
            text=True,
            capture_output=True,
            check=False,
        )
        if result.returncode != 0:
            detail = (result.stderr or result.stdout).strip().splitlines()
            suffix = f": {detail[0]}" if detail else ""
            failures.append(
                "PR-FR private order stream live sample verifier self-test failed"
                + suffix
            )

for row in by_pr.get("PR-FR", []):
    evidence_type = row["evidence_type"].strip()
    if not evidence_type.startswith("private-order-stream-live-sample-acceptance:"):
        continue
    venue = evidence_type.split(":", 1)[1].strip().lower()
    artifact = row["artifact"].strip()
    command = row["command"].strip()
    row_failures: list[str] = []
    if not re.fullmatch(r"[a-z0-9_:-]+", venue):
        row_failures.append(
            f"PR-FR private order stream live sample has invalid venue suffix: {evidence_type}"
        )
    elif not supported_live_sample_venue(venue):
        expected = ", ".join(sorted(SUPPORTED_LIVE_SAMPLE_VENUES))
        row_failures.append(
            f"PR-FR {evidence_type} venue suffix is not a supported venue/family; "
            f"expected one of {expected} or hyperliquid:<builder>"
        )
    artifact_path = Path(artifact)
    live_sample_dir = (root / "docs/live_samples").resolve()
    resolved_artifact: Path | None = None
    if artifact_path.is_absolute() or ".." in artifact_path.parts:
        row_failures.append(
            f"PR-FR {evidence_type} artifact must be a relative path inside "
            f"docs/live_samples/: {artifact}"
        )
    else:
        resolved_artifact = (root / artifact_path).resolve()
        try:
            resolved_artifact.relative_to(live_sample_dir)
        except ValueError:
            row_failures.append(
                f"PR-FR {evidence_type} artifact must live under "
                f"docs/live_samples/: {artifact}"
            )
    if artifact_path.suffix != ".json":
        row_failures.append(
            f"PR-FR {evidence_type} artifact must be a JSON snapshot: {artifact}"
        )
    artifact_stem = artifact_path.stem.lower()
    if artifact_stem and not (
        artifact_stem == venue
        or artifact_stem.startswith(f"{venue}-")
        or artifact_stem.startswith(f"{venue}_")
    ):
        row_failures.append(
            f"PR-FR {evidence_type} artifact basename must be venue-bound "
            f"({venue}.json, {venue}-*.json, or {venue}_*.json): {artifact}"
        )
    if resolved_artifact is not None:
        tracked = subprocess.run(
            ["git", "-C", str(root), "ls-files", "--error-unmatch", artifact],
            cwd=root,
            text=True,
            capture_output=True,
            check=False,
        )
        if tracked.returncode != 0:
            row_failures.append(
                f"PR-FR {evidence_type} artifact must be git-tracked: {artifact}"
            )
    if "scripts/check_private_order_stream_live_sample_acceptance.py" not in command:
        row_failures.append(
            f"PR-FR {evidence_type} command must run "
            "scripts/check_private_order_stream_live_sample_acceptance.py"
        )
    if artifact and artifact not in command:
        row_failures.append(
            f"PR-FR {evidence_type} command must verify its artifact {artifact}"
        )
    venue_pattern = re.compile(
        r"(^|\s)--venue\s+" + re.escape(venue) + r"(\s|;|&|\||$)"
    )
    if not venue_pattern.search(command):
        row_failures.append(
            f"PR-FR {evidence_type} command must pass --venue {venue}"
        )
    hashed_identity_pattern = re.compile(
        r"(^|\s)--require-hashed-identity(\s|;|&|\||$)"
    )
    if not hashed_identity_pattern.search(command):
        row_failures.append(
            f"PR-FR {evidence_type} command must pass --require-hashed-identity"
        )
    expected_command = [
        "python3",
        "scripts/check_private_order_stream_live_sample_acceptance.py",
        artifact,
        "--venue",
        venue,
        "--require-hashed-identity",
    ]
    try:
        parsed_command = shlex.split(command)
    except ValueError as exc:
        parsed_command = []
        row_failures.append(
            f"PR-FR {evidence_type} command is not shell-tokenizable: {exc}"
        )
    if parsed_command and parsed_command != expected_command:
        row_failures.append(
            "PR-FR "
            f"{evidence_type} command must equal: {' '.join(expected_command)}"
        )
    failures.extend(row_failures)
    if row_failures:
        continue

    verifier = root / "scripts/check_private_order_stream_live_sample_acceptance.py"
    result = subprocess.run(
        [
            sys.executable,
            str(verifier),
            str(resolved_artifact or (root / artifact)),
            "--venue",
            venue,
            "--require-hashed-identity",
        ],
        cwd=root,
        text=True,
        capture_output=True,
        check=False,
    )
    if result.returncode != 0:
        detail = (result.stderr or result.stdout).strip().splitlines()
        suffix = f": {detail[0]}" if detail else ""
        failures.append(
            f"PR-FR {evidence_type} verifier failed for {artifact}{suffix}"
        )

pr_m_evidence = {
    row["evidence_type"].strip()
    for row in by_pr.get("PR-M", [])
    if row["evidence_type"].strip()
}
if "PR-M" in completed and not any(
    evidence_type.startswith("live-sample-acceptance:")
    for evidence_type in pr_m_evidence
):
    failures.append(
        "PR-M is marked complete without venue-specific "
        "live-sample-acceptance:<venue> evidence"
    )

for row in by_pr.get("PR-M", []):
    evidence_type = row["evidence_type"].strip()
    if not evidence_type.startswith("live-sample-acceptance:"):
        continue
    venue = evidence_type.split(":", 1)[1].strip().lower()
    artifact = row["artifact"].strip()
    command = row["command"].strip()
    row_failures: list[str] = []
    if not re.fullmatch(r"[a-z0-9_:-]+", venue):
        row_failures.append(
            f"PR-M live sample evidence has invalid venue suffix: {evidence_type}"
        )
    elif not supported_live_sample_venue(venue):
        expected = ", ".join(sorted(SUPPORTED_LIVE_SAMPLE_VENUES))
        row_failures.append(
            f"PR-M {evidence_type} venue suffix is not a supported venue/family; "
            f"expected one of {expected} or hyperliquid:<builder>"
        )
    artifact_path = Path(artifact)
    live_sample_dir = (root / "docs/live_samples").resolve()
    resolved_artifact: Path | None = None
    if artifact_path.is_absolute() or ".." in artifact_path.parts:
        row_failures.append(
            f"PR-M {evidence_type} artifact must be a relative path inside "
            f"docs/live_samples/: {artifact}"
        )
    else:
        resolved_artifact = (root / artifact_path).resolve()
        try:
            resolved_artifact.relative_to(live_sample_dir)
        except ValueError:
            row_failures.append(
                f"PR-M {evidence_type} artifact must live under "
                f"docs/live_samples/: {artifact}"
            )
    if artifact_path.suffix != ".json":
        row_failures.append(
            f"PR-M {evidence_type} artifact must be a JSON snapshot: {artifact}"
        )
    artifact_stem = artifact_path.stem.lower()
    if artifact_stem and not (
        artifact_stem == venue
        or artifact_stem.startswith(f"{venue}-")
        or artifact_stem.startswith(f"{venue}_")
    ):
        row_failures.append(
            f"PR-M {evidence_type} artifact basename must be venue-bound "
            f"({venue}.json, {venue}-*.json, or {venue}_*.json): {artifact}"
        )
    if resolved_artifact is not None:
        tracked = subprocess.run(
            ["git", "-C", str(root), "ls-files", "--error-unmatch", artifact],
            cwd=root,
            text=True,
            capture_output=True,
            check=False,
        )
        if tracked.returncode != 0:
            row_failures.append(
                f"PR-M {evidence_type} artifact must be git-tracked: {artifact}"
            )
    if "scripts/check_live_order_runtime_acceptance.py" not in command:
        row_failures.append(
            f"PR-M {evidence_type} command must run "
            "scripts/check_live_order_runtime_acceptance.py"
        )
    if artifact and artifact not in command:
        row_failures.append(
            f"PR-M {evidence_type} command must verify its artifact {artifact}"
        )
    venue_pattern = re.compile(
        r"(^|\s)--venue\s+" + re.escape(venue) + r"(\s|;|&|\||$)"
    )
    if not venue_pattern.search(command):
        row_failures.append(
            f"PR-M {evidence_type} command must pass --venue {venue}"
        )
    hashed_identity_pattern = re.compile(
        r"(^|\s)--require-hashed-identity(\s|;|&|\||$)"
    )
    if not hashed_identity_pattern.search(command):
        row_failures.append(
            f"PR-M {evidence_type} command must pass --require-hashed-identity"
        )
    expected_command = [
        "python3",
        "scripts/check_live_order_runtime_acceptance.py",
        artifact,
        "--venue",
        venue,
        "--require-hashed-identity",
    ]
    try:
        parsed_command = shlex.split(command)
    except ValueError as exc:
        parsed_command = []
        row_failures.append(
            f"PR-M {evidence_type} command is not shell-tokenizable: {exc}"
        )
    if parsed_command and parsed_command != expected_command:
        row_failures.append(
            "PR-M "
            f"{evidence_type} command must equal: {' '.join(expected_command)}"
        )
    failures.extend(row_failures)
    if row_failures:
        continue

    verifier = root / "scripts/check_live_order_runtime_acceptance.py"
    result = subprocess.run(
        [
            sys.executable,
            str(verifier),
            str(resolved_artifact or (root / artifact)),
            "--venue",
            venue,
            "--require-hashed-identity",
        ],
        cwd=root,
        text=True,
        capture_output=True,
        check=False,
    )
    if result.returncode != 0:
        detail = (result.stderr or result.stdout).strip().splitlines()
        suffix = f": {detail[0]}" if detail else ""
        failures.append(
            f"PR-M {evidence_type} verifier failed for {artifact}{suffix}"
        )

if failures:
    raise SystemExit(
        "product audit evidence index gate failed\n"
        + "\n".join(f"  {failure}" for failure in failures)
    )

print(
    "OK product audit evidence index "
    f"({len(completed)} completed roadmap PRs indexed; "
    f"{len(rows)} evidence rows mapped to {len(active_prs)} active roadmap PRs)"
)
PY
