#![allow(clippy::panic)]

use super::super::*;
use super::fixtures::*;
use shared_types::{
    ActionMutationChange, AutoProfitCloseConfigPatch, ProtectedPositionFingerprint,
};

#[tokio::test]
async fn risk_config_action_run_detail_restores_status_payload() {
    let state = test_state().await;

    let Json(response) = update_risk_config(
        State(state.clone()),
        HeaderMap::new(),
        Json(RiskConfigPatch {
            max_order_notional: Some(12_345.0),
            max_open_orders: Some(7),
            max_hedge_imbalance_pct: None,
            allowed_exchanges: Some(vec!["mock".to_owned()]),
            allowed_symbols: Some(vec!["ETHUSDT".to_owned()]),
            protected_positions: Some(vec![protected_position()]),
            auto_profit_close: Some(AutoProfitCloseConfigPatch {
                enabled: Some(true),
                min_net_profit_usd: Some(8.0),
                min_roi_bps: Some(15.0),
                exit_buffer_bps: Some(8.0),
                stop_loss_enabled: Some(true),
                max_net_loss_usd: Some(30.0),
                max_loss_roi_bps: Some(125.0),
                liquidation_guard_enabled: Some(true),
                liquidation_exit_distance_pct: Some(9.0),
                confirmation_samples: Some(4),
                cooldown_secs: Some(90),
            }),
        }),
    )
    .await
    .unwrap_or_else(|error| panic!("risk config update failed: {error}"));
    let run = find_action_run(&state, ActionRunKind::TradingRiskConfigUpdate);

    let restored = action_runs::replay_payload::<TradingStatusResponse>(&run)
        .unwrap_or_else(|error| panic!("risk config action payload missing: {error}"));

    assert_eq!(restored.risk.max_order_notional, 12_345.0);
    assert_eq!(restored.risk.max_open_orders, 7);
    assert!(restored.risk.auto_profit_close.enabled);
    assert!(restored.risk.auto_profit_close.stop_loss_enabled);
    assert!(restored.risk.auto_profit_close.liquidation_guard_enabled);
    assert_eq!(restored.risk.protected_positions.len(), 1);
    assert_eq!(
        restored.risk.protected_positions[0].native_symbol,
        "btcusdt"
    );
    assert_eq!(
        restored
            .risk
            .auto_profit_close
            .liquidation_exit_distance_pct,
        9.0
    );
    assert_eq!(restored.risk, response.risk);
    assert_eq!(response.action_run_id.as_deref(), Some(run.id.as_str()));
    assert_eq!(run.mutation, response.mutation);
    let mutation = response
        .mutation
        .as_ref()
        .unwrap_or_else(|| panic!("risk mutation diff missing"));
    assert!(mutation.changes.iter().any(|change| matches!(
        change,
        ActionMutationChange::MaxOrderNotional { after, .. } if *after == 12_345.0
    )));
    assert!(mutation.changes.iter().any(|change| matches!(
        change,
        ActionMutationChange::AllowedSymbols { after, .. }
            if after == &["ethusdt".to_owned()]
    )));
    assert!(mutation.changes.iter().any(|change| matches!(
        change,
        ActionMutationChange::ProtectedPositions { after, .. }
            if after.len() == 1 && after[0].opening_identity == "preexisting-binance-btc-long"
    )));
    assert!(mutation.changes.iter().any(|change| matches!(
        change,
        ActionMutationChange::AutoProfitClose { after, .. }
            if after.enabled && after.min_net_profit_usd == 8.0
    )));
}

#[tokio::test]
async fn auto_exit_config_change_wakes_portfolio_evaluation() {
    let state = test_state().await;

    let Json(_) = update_risk_config(
        State(state.clone()),
        HeaderMap::new(),
        Json(RiskConfigPatch {
            max_order_notional: None,
            max_open_orders: None,
            max_hedge_imbalance_pct: None,
            allowed_exchanges: None,
            allowed_symbols: None,
            protected_positions: None,
            auto_profit_close: Some(AutoProfitCloseConfigPatch {
                enabled: Some(true),
                ..Default::default()
            }),
        }),
    )
    .await
    .unwrap_or_else(|error| panic!("risk config update failed: {error}"));

    assert!(tokio::time::timeout(
        std::time::Duration::from_millis(50),
        state.portfolio_refresh_signal().notified(),
    )
    .await
    .is_ok());
}

fn protected_position() -> ProtectedPositionFingerprint {
    ProtectedPositionFingerprint {
        venue: "binance".to_owned(),
        canonical_symbol: "BTC".to_owned(),
        native_symbol: "BTCUSDT".to_owned(),
        side: "long".to_owned(),
        quantity: 0.232,
        entry_price: 64_456.2,
        position_mode: Some("both".to_owned()),
        opening_identity: "preexisting-binance-btc-long".to_owned(),
        source: "account_position_runtime".to_owned(),
        captured_at_ms: 42,
    }
}

#[tokio::test]
async fn risk_config_action_run_preserves_explicit_idempotency_key_through_success() {
    let state = test_state().await;

    let Json(_) = update_risk_config(
        State(state.clone()),
        idempotency_headers(HEADER_IDEMPOTENCY_KEY, "risk-config-audit-1"),
        Json(RiskConfigPatch {
            max_order_notional: Some(12_345.0),
            max_open_orders: Some(7),
            max_hedge_imbalance_pct: None,
            allowed_exchanges: Some(vec!["mock".to_owned()]),
            allowed_symbols: Some(vec!["BTCUSDT".to_owned()]),
            protected_positions: None,
            auto_profit_close: None,
        }),
    )
    .await
    .unwrap_or_else(|error| panic!("risk config update failed: {error}"));
    let run = find_action_run(&state, ActionRunKind::TradingRiskConfigUpdate);

    assert_eq!(run.status, ActionRunStatus::Succeeded);
    assert_eq!(run.idempotency_key.as_deref(), Some("risk-config-audit-1"));
}

#[tokio::test]
async fn risk_config_replay_returns_first_payload_without_overwriting_newer_config() {
    let state = test_state().await;
    let first_headers = idempotency_headers(HEADER_IDEMPOTENCY_KEY, "risk-config-replay-k1");
    let second_headers = idempotency_headers(HEADER_IDEMPOTENCY_KEY, "risk-config-replay-k2");

    let Json(first) = update_risk_config(
        State(state.clone()),
        first_headers.clone(),
        Json(RiskConfigPatch {
            max_order_notional: Some(1_000.0),
            max_open_orders: Some(3),
            max_hedge_imbalance_pct: None,
            allowed_exchanges: None,
            allowed_symbols: None,
            protected_positions: None,
            auto_profit_close: None,
        }),
    )
    .await
    .unwrap_or_else(|error| panic!("first risk config update failed: {error}"));
    let Json(second) = update_risk_config(
        State(state.clone()),
        second_headers,
        Json(RiskConfigPatch {
            max_order_notional: Some(2_000.0),
            max_open_orders: Some(5),
            max_hedge_imbalance_pct: None,
            allowed_exchanges: None,
            allowed_symbols: None,
            protected_positions: None,
            auto_profit_close: None,
        }),
    )
    .await
    .unwrap_or_else(|error| panic!("second risk config update failed: {error}"));
    let Json(replayed) = update_risk_config(
        State(state.clone()),
        first_headers,
        Json(RiskConfigPatch {
            max_order_notional: Some(9_999.0),
            max_open_orders: Some(99),
            max_hedge_imbalance_pct: None,
            allowed_exchanges: None,
            allowed_symbols: None,
            protected_positions: None,
            auto_profit_close: None,
        }),
    )
    .await
    .unwrap_or_else(|error| panic!("risk config replay failed: {error}"));

    assert_eq!(replayed, first);
    assert_eq!(
        state.trading_service().risk_config().max_order_notional,
        2_000.0
    );
    assert_eq!(state.trading_service().risk_config().max_open_orders, 5);
    assert_eq!(second.risk.max_order_notional, 2_000.0);
    let runs = action_runs::recent(&state);
    assert_eq!(
        runs.iter()
            .filter(|run| run.kind == ActionRunKind::TradingRiskConfigUpdate)
            .count(),
        2,
        "replay must not create a third action run"
    );
}
