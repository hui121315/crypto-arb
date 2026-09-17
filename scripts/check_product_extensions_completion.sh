#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-check}"
MARKER_FILE="$ROOT/shared-types/src/onchain.rs"

if [[ "$MODE" != "check" && "$MODE" != "--self-test" ]]; then
  printf 'usage: %s [--self-test]\n' "$0" >&2
  exit 2
fi

if [[ "$MODE" == "--self-test" ]]; then
  PRODUCT_EXTENSIONS_SKIP_TESTS=1 bash "$0" >/dev/null
  backup="$(mktemp "${TMPDIR:-/tmp}/crossline-product-extensions.XXXXXX")"
  cp "$MARKER_FILE" "$backup"
  restore() {
    cp "$backup" "$MARKER_FILE"
    rm -f "$backup"
  }
  trap restore EXIT
  python3 - "$MARKER_FILE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text(encoding="utf-8")
marker = "pub read_only: bool"
if marker not in source:
    raise SystemExit("product extensions self-test setup failed: read-only marker missing")
path.write_text(source.replace(marker, "pub read_only_drifted: bool", 1), encoding="utf-8")
PY
  if PRODUCT_EXTENSIONS_SKIP_TESTS=1 bash "$0" >/dev/null 2>&1; then
    printf 'product extensions self-test failed: missing read-only boundary passed\n' >&2
    exit 1
  fi
  printf 'OK product extensions completion self-test\n'
  exit 0
fi

python3 - "$ROOT" <<'PY'
from __future__ import annotations

import sys
from pathlib import Path

root = Path(sys.argv[1])

evidence_contract = {
    "PR-GT": {
        "automation-manifest": "crates/automation/Cargo.toml",
        "automation-crate-root": "crates/automation/src/lib.rs",
        "automation-shared-contract": "shared-types/src/automation.rs",
        "automation-controller": "crates/automation/src/controller.rs",
        "automation-candidate-selector": "crates/automation/src/selector.rs",
        "automation-api-service": "crates/api/src/services/automated_arbitrage.rs",
        "automation-entry-guards": "crates/api/src/services/automated_arbitrage/guards.rs",
        "automation-live-fail-closed": "crates/api/src/services/automated_arbitrage/tests.rs",
        "automation-execution-worker": "crates/api/src/services/automated_arbitrage/worker.rs",
        "automation-action-audit": "crates/api/src/services/action_runs/audit_summary/integrations.rs",
        "shared-hedge-confirm": "crates/api/src/services/hedge_confirm.rs",
        "automation-lifecycle": "crates/api/src/lifecycle/automated_arbitrage.rs",
        "automation-thin-router": "crates/api/src/routers/automation.rs",
        "frontend-integration-client": "frontend/src/api/rest/integrations.rs",
        "automation-module-root": "frontend/src/panels/modules/automation/mod.rs",
        "automation-components-root": "frontend/src/panels/modules/automation/components/mod.rs",
        "automation-appws-data": "frontend/src/panels/modules/automation/data.rs",
        "automation-config-draft": "frontend/src/panels/modules/automation/draft.rs",
        "automation-format": "frontend/src/panels/modules/automation/format.rs",
        "automation-operator-panel": "frontend/src/panels/modules/automation/view.rs",
        "automation-control-rail": "frontend/src/panels/modules/automation/components/control_rail.rs",
        "automation-runtime-board": "frontend/src/panels/modules/automation/components/runtime_board.rs",
        "automation-decision-log": "frontend/src/panels/modules/automation/components/decision_log.rs",
        "automation-protection-controls": "frontend/src/panels/modules/automation/components/protection_controls.rs",
        "automation-paper-closed-loop": "crates/api/src/lifecycle/profit_exit/tests.rs",
        "automation-paper-fixture": "crates/api/src/lifecycle/profit_exit/tests/paper_fixture.rs",
        "automation-runtime-verifier": "scripts/verify_automated_arbitrage_flow.sh",
        "integration-surface-style": "frontend/styles/src/skin/extension-workbench.css",
        "automation-control-style": "frontend/styles/src/skin/automation-control.css",
        "automation-protection-style": "frontend/styles/src/skin/automation-protection.css",
        "automation-surface-style": "frontend/styles/src/skin/automation.css",
        "automation-log-style": "frontend/styles/src/skin/automation-log.css",
        "extension-responsive-style": "frontend/styles/src/skin/extension-workbench-responsive.css",
        "completion-governance": "scripts/check_product_extensions_completion.sh",
    },
    "PR-GU": {
        "onchain-manifest": "crates/onchain-monitor/Cargo.toml",
        "onchain-crate-root": "crates/onchain-monitor/src/lib.rs",
        "onchain-shared-contract": "shared-types/src/onchain.rs",
        "onchain-cost-comparison": "crates/onchain-monitor/src/comparison.rs",
        "onchain-config-runtime": "crates/onchain-monitor/src/runtime.rs",
        "onchain-api-orchestrator": "crates/api/src/services/onchain_comparison.rs",
        "official-jupiter-read-only-reader": "crates/api/src/services/onchain_comparison/quote.rs",
        "onchain-identity-guard": "crates/api/src/services/onchain_comparison/identity.rs",
        "onchain-scenario-matrix": "crates/api/src/services/onchain_comparison/tests.rs",
        "onchain-lifecycle": "crates/api/src/lifecycle/onchain_comparison.rs",
        "onchain-thin-router": "crates/api/src/routers/onchain.rs",
        "onchain-module-root": "frontend/src/panels/modules/onchain/mod.rs",
        "onchain-components-root": "frontend/src/panels/modules/onchain/components/mod.rs",
        "onchain-frontend-data": "frontend/src/panels/modules/onchain/data.rs",
        "onchain-config-draft": "frontend/src/panels/modules/onchain/draft.rs",
        "onchain-format": "frontend/src/panels/modules/onchain/format.rs",
        "onchain-monitor-panel": "frontend/src/panels/modules/onchain/view.rs",
        "onchain-command-rail": "frontend/src/panels/modules/onchain/components/command_rail.rs",
        "onchain-decision-board": "frontend/src/panels/modules/onchain/components/decision_board.rs",
        "onchain-evidence-ledger": "frontend/src/panels/modules/onchain/components/evidence_ledger.rs",
        "onchain-control-style": "frontend/styles/src/skin/onchain-control.css",
        "onchain-surface-style": "frontend/styles/src/skin/onchain.css",
        "onchain-evidence-style": "frontend/styles/src/skin/onchain-evidence.css",
        "completion-governance": "scripts/check_product_extensions_completion.sh",
    },
    "PR-GV": {
        "webhook-manifest": "crates/webhook/Cargo.toml",
        "webhook-crate-root": "crates/webhook/src/lib.rs",
        "webhook-shared-contract": "shared-types/src/webhook.rs",
        "webhook-bounded-dispatcher": "crates/webhook/src/dispatcher.rs",
        "webhook-target-security": "crates/webhook/src/security.rs",
        "webhook-api-service": "crates/api/src/services/webhook.rs",
        "webhook-lifecycle": "crates/api/src/lifecycle/webhook.rs",
        "webhook-event-bridge": "crates/api/src/lifecycle/webhook_events.rs",
        "webhook-thin-router": "crates/api/src/routers/webhook.rs",
        "webhook-frontend-data": "frontend/src/panels/modules/settings/data/webhook.rs",
        "webhook-configuration-panel": "frontend/src/panels/modules/settings/tabs/webhook.rs",
        "webhook-surface-style": "frontend/styles/src/skin/integrations.css",
        "completion-governance": "scripts/check_product_extensions_completion.sh",
    },
    "PR-GW": {
        "multichain-shared-contract": "shared-types/src/onchain.rs",
        "quote-cache-domain": "crates/onchain-monitor/src/quotes.rs",
        "multichain-runtime": "crates/onchain-monitor/src/runtime.rs",
        "low-latency-orchestrator": "crates/api/src/services/onchain_comparison.rs",
        "onchain-config-resolution": "crates/api/src/services/onchain_comparison/config.rs",
        "onchain-snapshot-projection": "crates/api/src/services/onchain_comparison/projection.rs",
        "provider-response-models": "crates/api/src/services/onchain_comparison/provider_types.rs",
        "official-provider-readers": "crates/api/src/services/onchain_comparison/quote.rs",
        "official-token-registry": "crates/api/src/services/onchain_comparison/token_registry.rs",
        "provider-identity-guard": "crates/api/src/services/onchain_comparison/identity.rs",
        "provider-scenario-matrix": "crates/api/src/services/onchain_comparison/tests.rs",
        "cex-ws-hot-refresh": "crates/api/src/services/market_data/cache/spot_orderbook_ws.rs",
        "cex-ws-hot-refresh-test": "crates/api/src/services/market_data/cache/tests/cases_5.rs",
        "binance-ws-control-batching": "crates/exchange/src/adapters/binance_ws_spot_depth.rs",
        "binance-ws-control-payload": "crates/exchange/src/adapters/binance_ws_spot_depth_data.rs",
        "binance-ws-control-test": "crates/exchange/src/adapters/binance_ws_spot_depth_tests.rs",
        "dual-clock-lifecycle": "crates/api/src/lifecycle/onchain_comparison.rs",
        "onchain-realtime-channel": "crates/realtime/src/channels.rs",
        "onchain-appws-publisher": "crates/api/src/services/ws_publish.rs",
        "onchain-appws-replay": "crates/api/src/services/ws_replay.rs",
        "frontend-appws-contract": "frontend/src/api/ws.rs",
        "frontend-ws-first-data": "frontend/src/panels/modules/onchain/data.rs",
        "frontend-token-draft": "frontend/src/panels/modules/onchain/draft.rs",
        "chain-cex-command-rail": "frontend/src/panels/modules/onchain/components/command_rail.rs",
        "dual-source-telemetry": "frontend/src/panels/modules/onchain/components/source_telemetry.rs",
        "dense-decision-table": "frontend/src/panels/modules/onchain/components/decision_board.rs",
        "official-evidence-ledger": "frontend/src/panels/modules/onchain/components/evidence_ledger.rs",
        "onchain-module-assembly": "frontend/src/panels/modules/onchain/view.rs",
        "onchain-control-style": "frontend/styles/src/skin/onchain-control.css",
        "onchain-surface-style": "frontend/styles/src/skin/onchain.css",
        "onchain-responsive-style": "frontend/styles/src/skin/extension-workbench-responsive.css",
        "operator-documentation": "README.md",
        "design-qa": "design-qa.md",
        "completion-governance": "scripts/check_product_extensions_completion.sh",
    },
}

markers = {
    "shared-types/src/automation.rs": (
        "enabled: false",
        "environment: ExecutionEnvironment::Paper",
        "max_concurrent_runs: 1",
    ),
    "crates/api/src/services/automated_arbitrage/guards.rs": (
        "trading kill switch blocks new automated entries",
        "automatic entry requires take-profit, stop-loss, or liquidation protection",
        "maximum concurrent automated positions reached",
    ),
    "crates/api/src/lifecycle/profit_exit/tests/paper_e2e.rs": (
        "paper_automation_opens_protects_closes_and_reaches_review",
        "ExecutionRunState::Closed",
        "CloseRunStatus::Succeeded",
    ),
    "crates/api/src/services/automated_arbitrage/tests.rs": (
        "actual_live_runtime_enters_without_unlock_and_kill_switch_still_wins",
        "trading kill switch blocks new automated entries",
    ),
    "frontend/src/panels/modules/automation/data.rs": (
        "start_automation_stream_with_state",
        "on_cleanup(move || handle.cancel())",
    ),
    "shared-types/src/onchain.rs": (
        "pub read_only: bool",
        "pub executable: bool",
        "pub official_docs_url: String",
        'cex_symbol: "SOL/USDC"',
    ),
    "crates/onchain-monitor/src/runtime.rs": (
        "normalized_pair_symbol",
        "cross_quote_mapping_fails_closed",
        "CEX symbol must match the configured",
    ),
    "crates/api/src/services/onchain_comparison.rs": (
        "project_latest_from_ws",
        "OnchainComparisonQuality::MappingInvalid",
        "refresh_spot_orderbook_from_ws",
    ),
    "crates/api/src/services/onchain_comparison/config.rs": (
        "resolve_changed_token_identity",
        "onchain_known_token",
        "resolve_jupiter_token_identities",
    ),
    "crates/api/src/services/onchain_comparison/projection.rs": (
        "compare_quotes",
        "classify_quality",
        "PROJECTION_INTERVAL_MS: i64 = 100",
        "read_only: true",
    ),
    "crates/api/src/services/onchain_comparison/provider_types.rs": (
        "JupiterOrderQuote",
        "ZeroExPrice",
        "ProviderRuntime",
    ),
    "crates/api/src/services/onchain_comparison/quote.rs": (
        "https://api.jup.ag/swap/v2/order",
        "https://developers.jup.ag/docs/swap/order-and-execute",
        "https://api.0x.org/swap/allowance-holder/price",
    ),
    "crates/api/src/services/onchain_comparison/token_registry.rs": (
        "https://api.jup.ag/tokens/v2/search",
        "row.is_verified",
        "Jupiter 官方 token registry 无法唯一解析",
    ),
    "crates/onchain-monitor/src/quotes.rs": (
        "transaction_requested: false",
        "OnchainQuotePair",
    ),
    "crates/api/src/services/onchain_comparison/identity.rs": (
        "provider quote identity does not match the configured token addresses",
        "CEX orderbook identity does not match the configured spot symbol",
    ),
    "crates/api/src/services/onchain_comparison/tests.rs": (
        "full_snapshot_is_fresh_profitable_and_strictly_read_only",
        "missing_cex_mapping_fails_closed",
        "mismatched_quote_and_orderbook_identities_fail_closed",
        "stale_low_liquidity_and_no_profit_are_distinct_products_states",
        "upstream_failure_snapshot_is_explicit_and_read_only",
    ),
    "frontend/src/panels/modules/onchain/components/command_rail.rs": (
        "永久观察边界",
        "应用并读取",
    ),
    "frontend/src/panels/modules/onchain/draft.rs": (
        "pool_or_route: Some(self.pool_or_route.get_untracked())",
        "base_decimals: self.base_decimals.get_untracked().parse().ok()",
        "symbol: RwSignal::new(config.cex_symbol.clone())",
    ),
    "crates/api/src/services/market_data/cache/spot_orderbook_ws.rs": (
        "refresh_spot_orderbook_from_ws",
        "MarketSource::WsPush",
    ),
    "crates/exchange/src/adapters/binance_ws_spot_depth.rs": (
        "CONTROL_FLUSH_INTERVAL",
        "MAX_STREAMS_PER_CONTROL_MESSAGE",
        'subscription_payload("SUBSCRIBE", &symbols',
    ),
    "crates/exchange/src/adapters/binance_ws_spot_depth_data.rs": (
        'format!("{symbol}@depth20@100ms")',
        '"params": params',
    ),
    "crates/exchange/src/adapters/binance_ws_spot_depth_tests.rs": (
        "subscription_batches_partial_depth_twenty_streams",
        'value["params"][1]',
    ),
    "crates/api/src/lifecycle/onchain_comparison.rs": (
        "Duration::from_millis(100)",
        "project_latest_from_ws",
    ),
    "crates/realtime/src/channels.rs": (
        "pub const ONCHAIN",
        "OnchainComparisonSnapshot",
    ),
    "frontend/src/panels/modules/onchain/components/source_telemetry.rs": (
        "HTTP QUOTE",
        "WS ORDERBOOK",
        "AppWS 已订阅",
    ),
    "frontend/src/panels/modules/onchain/components/decision_board.rs": (
        "双向费后决策表",
        "OBSERVE_ONLY",
    ),
    "shared-types/src/webhook.rs": (
        "WEBHOOK_EVENT_VERSION",
        "queue_capacity: 128",
        "base_backoff_ms: 500",
    ),
    "crates/webhook/src/security.rs": (
        "validate_public_https_target",
        "resolve_public_target",
        "v1={}",
        "rejects_private_and_non_https_targets",
    ),
    "crates/webhook/src/dispatcher.rs": (
        "redirect(reqwest::redirect::Policy::none())",
        "attempt_delivery",
        "idle_queue_poll_returns_at_its_deadline",
        "timeout_is_bounded_and_visible_in_recent_diagnostics",
        "public_status_never_serializes_the_signing_secret",
    ),
    "crates/api/src/lifecycle/webhook.rs": (
        "dispatcher.process_next(POLL_INTERVAL)",
        "idle_worker_records_progress_while_queue_is_empty",
    ),
    "crates/api/src/lifecycle/webhook_events.rs": tuple(
        f"WebhookEventKind::{kind}"
        for kind in (
            "Opportunity",
            "AutomationDecision",
            "ExecutionResult",
            "Compensation",
            "RiskAlert",
            "SystemDegradation",
        )
    ),
    "frontend/src/panels/modules/settings/tabs/webhook.rs": (
        "event_kind_options",
        "base_backoff_ms: backoff.get_untracked().parse().ok()",
        "停用并清除密钥",
    ),
}


def fail(message: str) -> None:
    raise SystemExit(f"product extensions completion gate failed: {message}")


for pr_id, contract in evidence_contract.items():
    for kind, artifact in contract.items():
        if not (root / artifact).is_file():
            fail(f"{pr_id} {kind} artifact is missing: {artifact}")

for path, required in markers.items():
    source = (root / path).read_text(encoding="utf-8")
    for marker in required:
        if marker not in source:
            fail(f"{path} lost required marker: {marker}")

readme = (root / "README.md").read_text(encoding="utf-8")
for marker in (
    "## 自动化套利",
    "## 链上 / CEX 价差监控",
    "## Webhook",
    "https://developers.jup.ag/docs/swap/order-and-execute",
    "https://api.binance.com/api/v3/exchangeInfo?symbol=SOLUSDC",
    "https://docs.0x.org/api-reference/evm-ap-is/swap/allowanceholder-getprice",
    "https://api.jup.ag/tokens/v2/search",
    "https://developers.binance.com/docs/binance-spot-api-docs/web-socket-streams",
):
    if marker not in readme:
        fail(f"README lost product or official-document marker: {marker}")

print("OK automation/on-chain/webhook static product contracts")
PY

if [[ "${PRODUCT_EXTENSIONS_SKIP_TESTS:-0}" != "1" ]]; then
  jobs="${CARGO_BUILD_JOBS:-8}"
  CARGO_BUILD_JOBS="$jobs" cargo test -p shared-types automation --lib --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p automation --lib --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api automated_arbitrage --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api paper_automation_opens_protects_closes_and_reaches_review --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p onchain-monitor --lib --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api onchain_comparison --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p webhook --lib --no-fail-fast
  CARGO_BUILD_JOBS="$jobs" cargo test -p api --bin crypto-arb-api webhook --no-fail-fast
  cargo check --manifest-path frontend/Cargo.toml --target wasm32-unknown-unknown
  bash -n "$ROOT/scripts/verify_automated_arbitrage_flow.sh"
fi

printf 'OK automation/on-chain/webhook product extension gate\n'
