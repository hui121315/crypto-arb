#![allow(clippy::panic)]
use super::super::*;
use common::config::AppConfig;
use shared_types::{
    ExecutionLedgerOrderRef, ExecutionLedgerQuality, FeeLedgerSnapshot, FundingPaymentLedgerRecord,
    OrderIntent, SlippageLedgerRecord,
};
use std::path::PathBuf;

pub(super) fn close_run(id: &str, leg: CloseLeg) -> CloseRun {
    CloseRun {
        id: id.to_owned(),
        scope: shared_types::CloseRunScope::Single,
        status: CloseRunStatus::Submitted,
        action_run_id: None,
        request_id: None,
        idempotency_key: None,
        snapshot_version: "pos-1".to_owned(),
        expected_leg_count: 1,
        reason: None,
        legs: vec![leg],
        submitted_order_count: 1,
        failed_leg_count: 0,
        naked_exposure_usd: 0.0,
        message: "submitted".to_owned(),
        problem: None,
        finality_problem: None,
        finality_checked_at_ms: None,
        unwind_plan: None,
        cost_events: Vec::new(),
        cost_reconciliation: None,
        started_at_ms: 1,
        updated_at_ms: 1,
    }
}

pub(super) fn close_leg(order_id: &str, status: CloseLegStatus) -> CloseLeg {
    CloseLeg {
        venue: "binance".to_owned(),
        symbol: "MUUSDT".to_owned(),
        side: PositionSide::Long,
        status,
        quantity: 1.0,
        mark_price: 100.0,
        notional_usd: 100.0,
        order: Some(order_record(order_id, LiveOrderState::Submitted)),
        finality_source: None,
        confirmed_filled_at_ms: None,
        problem: None,
        pair_evidence: None,
        cost_events: Vec::new(),
    }
}

pub(super) fn order_record(order_id: &str, state: LiveOrderState) -> OrderRecord {
    OrderRecord {
        intent: OrderIntent {
            id: order_id.to_owned(),
            source: OrderSource::Manual,
            strategy: None,
            mode: ExecutionMode::DryRun,
            exchange: "binance".to_owned(),
            symbol: "MUUSDT".to_owned(),
            side: OrderSide::Sell,
            order_type: OrderType::Market,
            quantity: 1.0,
            price: Some(100.0),
            slippage_tolerance_bps: None,
            reduce_only: true,
            time_in_force: TimeInForce::Ioc,
            post_only: false,
            margin_mode: MarginMode::Cross,
            leverage: 1.0,
            client_order_id: format!("client-{order_id}"),
            client_order_id_policy: None,
            created_at_ms: 1,
        },
        state,
        risk: None,
        identity: Default::default(),
        last_update_source: OrderUpdateSource::OrderQuery,
        exchange_order_id: Some(format!("ex-{order_id}")),
        message: None,
        filled_quantity: None,
        filled_price: None,
        filled_fee: None,
        updated_at_ms: 2,
    }
}

pub(super) fn compensation_order_record(close_run_id: &str, state: LiveOrderState) -> OrderRecord {
    let order_id = format!("cu-{close_run_id}-order-3");
    let mut record = order_record(&order_id, state);
    record.intent.side = OrderSide::Buy;
    record.intent.reduce_only = false;
    record.intent.source = OrderSource::CloseRunCompensation;
    record
}

pub(super) fn orderbook(exchange: &str, symbol: &str) -> OrderBookInfo {
    OrderBookInfo {
        symbol: symbol.to_owned(),
        exchange: exchange.to_owned(),
        bids: vec![[99.9, 1.0]],
        asks: vec![[100.1, 1.0]],
        timestamp: common::time::now_ms(),
    }
}

pub(super) fn ledger_fill_event(order_id: &str, quantity: f64) -> ExecutionLedgerEvent {
    ExecutionLedgerEvent {
        event_id: format!("fill:{order_id}:{quantity}"),
        event_type: ExecutionLedgerEventType::FillEvent,
        source: OrderUpdateSource::PrivateWs,
        order: ledger_order_ref(order_id),
        payload: ExecutionLedgerPayload::FillSnapshot(FillLedgerSnapshot {
            quantity,
            average_price: 100.0,
            quote_value: 100.0 * quantity,
            quality: ExecutionLedgerQuality::Actual,
            confidence: shared_types::ExecutionFillConfidence::VenueFill,
            fee: Some(FeeLedgerSnapshot {
                amount: 0.01,
                currency: Some("USDT".to_owned()),
                quality: ExecutionLedgerQuality::Actual,
            }),
        }),
        occurred_at_ms: 10,
        captured_at_ms: 11,
    }
}

pub(super) fn ledger_state_event(order_id: &str, state: LiveOrderState) -> ExecutionLedgerEvent {
    ExecutionLedgerEvent {
        event_id: format!("state:{order_id}:{state:?}"),
        event_type: ExecutionLedgerEventType::OrderState,
        source: OrderUpdateSource::OrderQuery,
        order: ledger_order_ref(order_id),
        payload: ExecutionLedgerPayload::OrderState {
            state,
            message: None,
        },
        occurred_at_ms: 10,
        captured_at_ms: 11,
    }
}

pub(super) fn ledger_fee_event(order_id: &str, amount: f64) -> ExecutionLedgerEvent {
    ExecutionLedgerEvent {
        event_id: format!("fee:{order_id}:{amount}"),
        event_type: ExecutionLedgerEventType::FeeSnapshot,
        source: OrderUpdateSource::PrivateWs,
        order: ledger_order_ref(order_id),
        payload: ExecutionLedgerPayload::FeeSnapshot(FeeLedgerSnapshot {
            amount,
            currency: Some("USDT".to_owned()),
            quality: ExecutionLedgerQuality::Actual,
        }),
        occurred_at_ms: 12,
        captured_at_ms: 13,
    }
}

pub(super) fn ledger_slippage_event(order_id: &str, amount_usd: f64) -> ExecutionLedgerEvent {
    ExecutionLedgerEvent {
        event_id: format!("slippage:{order_id}:{amount_usd}"),
        event_type: ExecutionLedgerEventType::Slippage,
        source: OrderUpdateSource::PrivateWs,
        order: ledger_order_ref(order_id),
        payload: ExecutionLedgerPayload::Slippage(SlippageLedgerRecord {
            amount_usd,
            reference_price: 100.0,
            fill_price: 100.0 + amount_usd,
            quantity: 1.0,
            quality: ExecutionLedgerQuality::Actual,
        }),
        occurred_at_ms: 14,
        captured_at_ms: 15,
    }
}

pub(super) fn ledger_funding_event(
    order_id: &str,
    run_id: Option<&str>,
    ticket_id: Option<&str>,
    role: Option<HedgeLegRole>,
    amount: f64,
) -> ExecutionLedgerEvent {
    let mut order = ledger_order_ref(order_id);
    order.run_id = run_id.map(str::to_owned);
    order.ticket_id = ticket_id.map(str::to_owned);
    order.leg_role = role;
    ExecutionLedgerEvent {
        event_id: format!("funding:{order_id}:{amount}"),
        event_type: ExecutionLedgerEventType::FundingPayment,
        source: OrderUpdateSource::PrivateWs,
        order,
        payload: ExecutionLedgerPayload::FundingPayment(FundingPaymentLedgerRecord {
            amount,
            currency: "USDT".to_owned(),
            funding_time_ms: 16,
            quality: ExecutionLedgerQuality::Actual,
        }),
        occurred_at_ms: 16,
        captured_at_ms: 17,
    }
}

pub(super) fn ledger_order_ref(order_id: &str) -> ExecutionLedgerOrderRef {
    ExecutionLedgerOrderRef {
        run_id: None,
        ticket_id: None,
        leg_role: None,
        reduce_only: None,
        exchange: "binance".to_owned(),
        symbol: "MUUSDT".to_owned(),
        side: OrderSide::Sell,
        identity: VenueOrderIdentity {
            internal_order_id: order_id.to_owned(),
            public_client_order_id: format!("client-{order_id}"),
            venue_client_order_id: None,
            exchange_order_id: Some(format!("ex-{order_id}")),
            product: shared_types::FeeProduct::Perp,
            client_order_id_policy: None,
            transport_metadata: Default::default(),
        },
    }
}

pub(super) async fn test_state() -> AppState {
    AppState::new(test_config())
        .await
        .unwrap_or_else(|error| panic!("state init failed: {error}"))
}

pub(super) fn action_run_status(state: &AppState, action_run_id: &str) -> ActionRunStatus {
    action_runs::recent(state)
        .into_iter()
        .find(|run| run.id == action_run_id)
        .map(|run| run.status)
        .unwrap_or_else(|| panic!("action run missing: {action_run_id}"))
}

pub(super) fn test_config() -> AppConfig {
    let mut config = AppConfig::default();
    config.history.enabled = false;
    config.storage.portfolio_nav_path = None;
    config.storage.close_run_ledger_path = None;
    config
}

pub(super) fn temp_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "crossline-close-runs-{label}-{}-{}.jsonl",
        std::process::id(),
        common::time::now_ms()
    ))
}
