use crate::state::AppState;
use crate::trading_service::private_ws_events::{
    BinanceOrderTradeDelta, PrivateFillDelta, PrivateOrderDelta, PrivateWsEvent,
};
use shared_types::{
    CloseLeg, CloseLegStatus, CloseRun, CloseRunScope, CloseRunStatus, ExecutionCostReconciliation,
    ExecutionLedgerEventType, ExecutionMode, ExecutionRun, ExecutionRunLeg, ExecutionRunState,
    HedgeLegRole, LiveOrderState, MarginMode, OrderInfo, OrderIntent, OrderRecord, OrderSide,
    OrderSource, OrderStatus, OrderType, OrderUpdateSource, PositionSide, StrategyKind,
    TimeInForce,
};

mod order_events;

pub(super) use order_events::{private_order_cancel_event, private_order_filled_event};

pub(super) async fn isolated_private_ws_state() -> anyhow::Result<AppState> {
    isolated_private_ws_state_with_postgres(None).await
}

pub(super) async fn isolated_private_ws_state_with_unavailable_sql() -> anyhow::Result<AppState> {
    isolated_private_ws_state_with_postgres(Some(
        "postgres://127.0.0.1:1/crossline_unavailable".to_owned(),
    ))
    .await
}

async fn isolated_private_ws_state_with_postgres(
    postgres_url: Option<String>,
) -> anyhow::Result<AppState> {
    let mut config = common::config::AppConfig::default();
    config.storage.portfolio_nav_path = None;
    config.storage.execution_run_ledger_path = None;
    config.storage.execution_ledger_path = None;
    config.storage.order_snapshot_path = None;
    config.storage.close_run_ledger_path = None;
    config.storage.postgres_url = postgres_url;
    AppState::new(config).await
}

pub(super) fn private_ws_intent(id: &str, side: OrderSide, reduce_only: bool) -> OrderIntent {
    OrderIntent {
        id: id.to_owned(),
        source: OrderSource::ArbitragePreview,
        strategy: Some(StrategyKind::PerpCross),
        mode: ExecutionMode::DryRun,
        exchange: "mock".into(),
        symbol: "BTC".into(),
        side,
        order_type: if reduce_only {
            OrderType::Market
        } else {
            OrderType::Limit
        },
        quantity: 1.0,
        price: Some(100.0),
        slippage_tolerance_bps: None,
        reduce_only,
        time_in_force: TimeInForce::Ioc,
        post_only: false,
        margin_mode: MarginMode::Cross,
        leverage: 1.0,
        client_order_id: format!("client-{id}"),
        client_order_id_policy: None,
        created_at_ms: common::time::now_ms(),
    }
}

pub(super) fn private_fill(
    record: &OrderRecord,
    venue_event_id: &str,
    price: f64,
    fee_amount: f64,
    occurred_at_ms: i64,
) -> anyhow::Result<PrivateFillDelta> {
    Ok(PrivateFillDelta {
        venue: "mock".into(),
        exchange_order_id: record
            .exchange_order_id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("mock submit did not return an exchange order id"))?,
        client_order_id: Some(record.intent.client_order_id.clone()),
        symbol: Some(record.intent.symbol.clone()),
        side: Some(record.intent.side),
        venue_event_id: venue_event_id.to_owned(),
        quantity: 1.0,
        price,
        fee_amount: Some(fee_amount),
        fee_currency: Some("USDC".into()),
        occurred_at_ms,
    })
}

pub(super) fn binance_private_trade_event(
    record: &OrderRecord,
    venue_event_id: &str,
    price: f64,
    fee_amount: f64,
    occurred_at_ms: i64,
) -> anyhow::Result<PrivateWsEvent> {
    let fill = private_fill(record, venue_event_id, price, fee_amount, occurred_at_ms)?;
    let order_id = fill.exchange_order_id.clone();
    Ok(PrivateWsEvent::BinanceOrderTrade(Box::new(
        BinanceOrderTradeDelta {
            order: PrivateOrderDelta {
                client_order_id: record.intent.client_order_id.clone(),
                order: OrderInfo {
                    execution_style: None,
                    venue_time_in_force: None,
                    client_order_id: Some(record.intent.client_order_id.clone()),
                    reduce_only: Some(record.intent.reduce_only),
                    order_id,
                    symbol: record.intent.symbol.clone(),
                    exchange: record.intent.exchange.clone(),
                    side: record.intent.side,
                    order_type: record.intent.order_type,
                    status: OrderStatus::Filled,
                    quantity: record.intent.quantity,
                    price,
                    filled_quantity: record.intent.quantity,
                    filled_price: price,
                    fees: fee_amount,
                    created_at: chrono::Utc::now(),
                },
                received_at_ms: occurred_at_ms,
            },
            fill: Some(fill),
            execution_type: "TRADE".into(),
            order_status: "FILLED".into(),
            reject_reason: Some("NONE".into()),
            terminal: true,
        },
    )))
}

pub(super) fn binance_review_trade_events(
    long: &OrderRecord,
    short: &OrderRecord,
    occurred_at_ms: i64,
) -> anyhow::Result<Vec<PrivateWsEvent>> {
    Ok(vec![
        binance_private_trade_event(
            long,
            "binance_trade:review-long:1",
            101.0,
            0.1,
            occurred_at_ms,
        )?,
        binance_private_trade_event(
            short,
            "binance_trade:review-short:2",
            99.0,
            0.1,
            occurred_at_ms,
        )?,
    ])
}

pub(super) fn private_ws_unwind_run() -> ExecutionRun {
    ExecutionRun {
        run_id: "run-private-ws".into(),
        ticket_id: "ticket-private-ws".into(),
        opportunity_id: "opportunity-private-ws".into(),
        state: ExecutionRunState::Unwinding,
        long_leg: private_ws_run_leg(HedgeLegRole::Long),
        short_leg: private_ws_run_leg(HedgeLegRole::Short),
        net_exposure_usd: 100.0,
        cost_reconciliation: Some(ExecutionCostReconciliation::default()),
        valuation_problem: None,
        unwind_problem: None,
        finality_problem: None,
        finality_checked_at_ms: None,
        evidence: Default::default(),
        recovery_action: None,
        status_reason: "unwind submitted".into(),
        created_at_ms: 1,
        updated_at_ms: 1,
    }
}

fn private_ws_run_leg(role: HedgeLegRole) -> ExecutionRunLeg {
    ExecutionRunLeg {
        role,
        exchange: "mock".into(),
        symbol: "BTC".into(),
        order_ids: Vec::new(),
        identity: None,
        finality_source: None,
        confirmed_filled_at_ms: None,
        state: LiveOrderState::Submitted,
        target_quantity: 1.0,
        filled_quantity: None,
        target_notional_usd: 100.0,
        filled_notional_usd: None,
        filled_fee: None,
    }
}

pub(super) fn private_ws_close_run(record: &OrderRecord) -> CloseRun {
    let mut order = record.clone();
    order.state = LiveOrderState::Accepted;
    order.filled_quantity = None;
    order.filled_price = None;
    order.filled_fee = None;
    CloseRun {
        id: "close-private-ws".into(),
        scope: CloseRunScope::Single,
        status: CloseRunStatus::Submitted,
        action_run_id: None,
        request_id: None,
        idempotency_key: None,
        snapshot_version: "private-ws".into(),
        expected_leg_count: 1,
        reason: None,
        legs: vec![CloseLeg {
            venue: "mock".into(),
            symbol: "BTC".into(),
            side: PositionSide::Short,
            status: CloseLegStatus::Submitted,
            quantity: 1.0,
            mark_price: 100.0,
            notional_usd: 100.0,
            order: Some(order),
            finality_source: None,
            confirmed_filled_at_ms: None,
            problem: None,
            pair_evidence: None,
            cost_events: Vec::new(),
        }],
        submitted_order_count: 1,
        failed_leg_count: 0,
        naked_exposure_usd: 0.0,
        message: "submitted".into(),
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

pub(super) fn private_fill_projection_count(state: &AppState) -> usize {
    state
        .trading_service()
        .list_execution_ledger_events()
        .iter()
        .filter(|event| {
            matches!(
                event.event_type,
                ExecutionLedgerEventType::FillEvent | ExecutionLedgerEventType::Slippage
            ) && event.source == OrderUpdateSource::PrivateWs
        })
        .count()
}
