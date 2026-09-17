use super::*;
use shared_types::{
    ExecutionMode, MarginMode, OrderIntent, OrderSide, OrderSource, OrderType, TimeInForce,
};

#[test]
fn order_event_payload_uses_orders_channel_shape() {
    let record = OrderRecord {
        intent: OrderIntent {
            id: "order-1".into(),
            source: OrderSource::ArbitragePreview,
            strategy: None,
            mode: ExecutionMode::DryRun,
            exchange: "okx".into(),
            symbol: "BTC".into(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            quantity: 1.0,
            price: Some(100.0),
            slippage_tolerance_bps: None,
            reduce_only: false,
            time_in_force: TimeInForce::Ioc,
            post_only: false,
            margin_mode: MarginMode::Cross,
            leverage: 1.0,
            client_order_id: "client-1".into(),
            client_order_id_policy: None,
            created_at_ms: 1,
        },
        state: shared_types::LiveOrderState::Submitted,
        risk: None,
        identity: Default::default(),
        last_update_source: Default::default(),
        exchange_order_id: None,
        message: None,
        filled_quantity: None,
        filled_price: None,
        filled_fee: None,
        updated_at_ms: 2,
    };
    let msg = order_event_message("hedge_first_leg_submitted", &record);
    let Some(value) = msg.as_ref().ok().and_then(WsMessage::payload_json) else {
        unreachable!("OrderRecord serializes to a JSON websocket message");
    };

    assert_eq!(value["event"], "hedge_first_leg_submitted");
    assert_eq!(value["record"]["intent"]["id"], "order-1");
    assert!(value["timestampMs"].as_i64().is_some());
}

#[test]
fn execution_run_payload_uses_execution_channel_shape() {
    let run = ExecutionRun {
        run_id: "run-1".into(),
        ticket_id: "ticket-1".into(),
        opportunity_id: "opp-1".into(),
        state: shared_types::ExecutionRunState::Unwinding,
        long_leg: run_leg(shared_types::HedgeLegRole::Long),
        short_leg: run_leg(shared_types::HedgeLegRole::Short),
        net_exposure_usd: 100.0,
        cost_reconciliation: None,
        valuation_problem: None,
        unwind_problem: None,
        finality_problem: None,
        finality_checked_at_ms: None,
        evidence: Default::default(),
        recovery_action: Some(shared_types::RecoveryAction::UnwindLongLeg),
        status_reason: "部分成交，已提交反向 unwind".into(),
        created_at_ms: 1,
        updated_at_ms: 2,
    };
    let msg = execution_run_message("execution_run_updated", &run);
    let Some(value) = msg.as_ref().ok().and_then(WsMessage::payload_json) else {
        unreachable!("ExecutionRun serializes to a JSON websocket message");
    };

    assert_eq!(value["event"], "execution_run_updated");
    assert_eq!(value["executionRun"]["runId"], "run-1");
    assert_eq!(value["executionRun"]["state"], "unwinding");
    assert!(value["timestampMs"].as_i64().is_some());
}

#[test]
fn close_run_payload_uses_portfolio_channel_shape() {
    let run = CloseRun {
        id: "close-1".into(),
        scope: shared_types::CloseRunScope::Pair,
        status: shared_types::CloseRunStatus::UnwindRequired,
        action_run_id: Some("action-1".into()),
        request_id: Some("request-1".into()),
        idempotency_key: None,
        snapshot_version: "snapshot-1".into(),
        expected_leg_count: 2,
        reason: Some("positions.close_pair".into()),
        legs: Vec::new(),
        submitted_order_count: 1,
        failed_leg_count: 1,
        naked_exposure_usd: 25.0,
        message: "manual review required".into(),
        problem: None,
        finality_problem: None,
        finality_checked_at_ms: None,
        unwind_plan: None,
        cost_events: Vec::new(),
        cost_reconciliation: None,
        started_at_ms: 1,
        updated_at_ms: 2,
    };
    let msg = close_run_message("close_run_updated", &run);
    let Some(value) = msg.as_ref().ok().and_then(WsMessage::payload_json) else {
        unreachable!("CloseRunEvent serializes to a JSON websocket message");
    };

    assert_eq!(value["event"], "close_run_updated");
    assert_eq!(value["closeRun"]["id"], "close-1");
    assert_eq!(value["closeRun"]["status"], "unwind_required");
    assert!(value["timestampMs"].as_i64().is_some());
}

#[test]
fn reconcile_event_payload_uses_orders_channel_shape() {
    let diffs = vec![ReconcileDiff {
        kind: trading::ReconcileDiffKind::RemoteMissing,
        exchange_order_id: "x1".into(),
        internal_order_id: Some("order-1".into()),
        local_state: Some(shared_types::LiveOrderState::Accepted),
        remote_state: None,
        local_quantity: Some(1.0),
        remote_quantity: None,
    }];
    let msg = reconcile_event_message("order_reconcile_diff", &diffs);
    let Some(value) = msg.as_ref().ok().and_then(WsMessage::payload_json) else {
        unreachable!("reconcile diff serializes to JSON");
    };

    assert_eq!(value["event"], "order_reconcile_diff");
    assert_eq!(value["diffCount"], 1);
    assert_eq!(value["diffs"][0]["kind"], "remote_missing");
    assert_eq!(value["diffs"][0]["internalOrderId"], "order-1");
}

#[test]
fn risk_event_payload_uses_risk_alert_channel_shape() {
    let risk = TradingRiskStatus {
        live_trading_enabled: false,
        kill_switch_active: true,
        max_order_notional: 1000.0,
        max_open_orders: 10,
        max_hedge_imbalance_pct: 0.01,
        liquidation_warn_pct: 15.0,
        liquidation_danger_pct: 8.0,
        allowed_exchanges: vec!["okx".into()],
        allowed_symbols: vec!["btc".into()],
        protected_positions: Vec::new(),
        auto_profit_close: Default::default(),
    };
    let msg = risk_alert_message("risk_snapshot_replay", risk);
    let Some(value) = msg.as_ref().ok().and_then(WsMessage::payload_json) else {
        unreachable!("RiskAlertEvent serializes to JSON");
    };

    assert_eq!(value["event"], "risk_snapshot_replay");
    assert_eq!(value["risk"]["killSwitchActive"], true);
    assert!(value.get("executionRun").is_none());
    assert!(value["timestampMs"].as_i64().is_some());
}

fn run_leg(role: shared_types::HedgeLegRole) -> shared_types::ExecutionRunLeg {
    shared_types::ExecutionRunLeg {
        role,
        exchange: "okx".into(),
        symbol: "BTC".into(),
        order_ids: Vec::new(),
        identity: None,
        finality_source: None,
        confirmed_filled_at_ms: None,
        state: shared_types::LiveOrderState::Submitted,
        target_quantity: 1.0,
        filled_quantity: None,
        target_notional_usd: 100.0,
        filled_notional_usd: None,
        filled_fee: None,
    }
}
