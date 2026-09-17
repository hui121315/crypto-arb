use super::*;
use shared_types::{TradingRiskStatus, TradingWsChannels};

#[test]
fn kill_switch_request_includes_current_snapshot() {
    let status = trading_status(false, 4);

    let request = kill_switch_request(&status, true);

    assert!(request.active);
    assert_eq!(request.expected_active, Some(false));
    assert_eq!(request.expected_open_order_count, Some(4));
    assert_eq!(request.reason, "settings.kill_switch.enable");
}

#[test]
fn risk_status_stale_message_keeps_request_and_retry_context() {
    let problem = ApiProblem::new("TRADING_STATUS_RATE_LIMITED", "rate limited")
        .with_status(429)
        .with_request_id(Some("req-risk-1".into()))
        .with_retry_after_ms(Some(4_000));

    let message = risk_status_stale_message(Some(&problem)).unwrap_or_default();

    assert!(message.contains("风控状态刷新失败，显示上次结果"));
    assert!(message.contains("rate limited"));
    assert!(message.contains("HTTP 429"));
    assert!(message.contains("request_id req-risk-1"));
    assert!(message.contains("retry 4000ms"));
}

#[test]
fn risk_status_stale_message_is_absent_without_problem() {
    assert!(risk_status_stale_message(None).is_none());
}

#[test]
fn environment_label_is_explicitly_backend_and_read_only() {
    assert_eq!(
        environment_label(ExecutionEnvironment::Paper),
        "后端模拟环境"
    );
    assert_eq!(
        environment_label(ExecutionEnvironment::Live),
        "后端实盘环境"
    );
}

#[test]
fn risk_patch_includes_auto_profit_close_contract() {
    let risk = RiskThresholdInputs {
        max_order: "1000".to_owned(),
        max_open: "4".to_owned(),
        imbalance_pct: "1".to_owned(),
        allowed_exchanges: "okx, binance".to_owned(),
        allowed_symbols: "BTCUSDT".to_owned(),
    };
    let auto_close = AutoProfitCloseInputs {
        enabled: true,
        min_net_profit_usd: "8".to_owned(),
        min_roi_pct: "0.15".to_owned(),
        exit_buffer_pct: "0.08".to_owned(),
        stop_loss_enabled: true,
        max_net_loss_usd: "30".to_owned(),
        max_loss_roi_pct: "1.25".to_owned(),
        liquidation_guard_enabled: true,
        liquidation_exit_distance_pct: "9".to_owned(),
        confirmation_samples: "4".to_owned(),
        cooldown_secs: "90".to_owned(),
    };

    let result = risk_patch_from_inputs(&risk, &auto_close);
    assert!(result.is_ok(), "valid risk patch should parse");
    let Ok(patch) = result else {
        return;
    };
    assert!(
        patch.auto_profit_close.is_some(),
        "auto close patch should be present"
    );
    let Some(auto) = patch.auto_profit_close else {
        return;
    };

    assert_eq!(auto.enabled, Some(true));
    assert_eq!(auto.min_net_profit_usd, Some(8.0));
    assert_eq!(auto.min_roi_bps, Some(15.0));
    assert_eq!(auto.exit_buffer_bps, Some(8.0));
    assert_eq!(auto.stop_loss_enabled, Some(true));
    assert_eq!(auto.max_net_loss_usd, Some(30.0));
    assert_eq!(auto.max_loss_roi_bps, Some(125.0));
    assert_eq!(auto.liquidation_guard_enabled, Some(true));
    assert_eq!(auto.liquidation_exit_distance_pct, Some(9.0));
    assert_eq!(auto.confirmation_samples, Some(4));
    assert_eq!(auto.cooldown_secs, Some(90));
}

fn trading_status(active: bool, open_orders: usize) -> TradingStatusResponse {
    TradingStatusResponse {
        adapter: "mock".into(),
        environment: ExecutionEnvironment::Paper,
        open_order_count: open_orders,
        risk: TradingRiskStatus {
            live_trading_enabled: false,
            kill_switch_active: active,
            max_order_notional: 1.0,
            max_open_orders: 10,
            max_hedge_imbalance_pct: 0.01,
            liquidation_warn_pct: 0.2,
            liquidation_danger_pct: 0.1,
            allowed_exchanges: Vec::new(),
            allowed_symbols: Vec::new(),
            protected_positions: Vec::new(),
            auto_profit_close: Default::default(),
        },
        ws_channels: TradingWsChannels {
            orders: "orders".into(),
            execution: "execution".into(),
            risk_alerts: "risk-alerts".into(),
        },
        action_run_id: None,
        request_id: None,
        idempotency_key: None,
        mutation: None,
    }
}
