use super::*;
use shared_types::{
    ExecutionLedgerEventType, ExecutionLedgerOrderRef, ExecutionLedgerQuality, ExecutionMode,
    LiveOrderState, OrderIntent, OrderType, OrderUpdateSource, StrategyKind, TimeInForce,
    VenueOrderIdentity,
};

mod evidence;
mod fills;
mod pnl;

fn hedge_orders() -> Vec<OrderRecord> {
    vec![
        order("hedge-1-long", OrderSide::Buy),
        order("hedge-1-short", OrderSide::Sell),
    ]
}

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1e-9,
        "actual={actual}, expected={expected}"
    );
}

fn order(id: &str, side: OrderSide) -> OrderRecord {
    order_with_price(id, side, Some(100.0))
}

fn order_with_price(id: &str, side: OrderSide, price: Option<f64>) -> OrderRecord {
    let intent = OrderIntent {
        id: id.into(),
        source: OrderSource::ArbitragePreview,
        strategy: Some(StrategyKind::PerpCross),
        mode: ExecutionMode::DryRun,
        exchange: "mock".into(),
        symbol: "BTC".into(),
        side,
        order_type: OrderType::Limit,
        quantity: 1.0,
        price,
        slippage_tolerance_bps: None,
        reduce_only: false,
        time_in_force: TimeInForce::Ioc,
        post_only: false,
        margin_mode: shared_types::MarginMode::Cross,
        leverage: 1.0,
        client_order_id: format!("{id}-client"),
        client_order_id_policy: None,
        created_at_ms: 0,
    };
    OrderRecord {
        identity: VenueOrderIdentity::from_intent(&intent),
        intent,
        state: LiveOrderState::Filled,
        risk: None,
        last_update_source: OrderUpdateSource::PrivateWs,
        exchange_order_id: None,
        message: None,
        filled_quantity: None,
        filled_price: None,
        filled_fee: None,
        updated_at_ms: 0,
    }
}

fn fill_event(
    order: &OrderRecord,
    price: f64,
    fee: f64,
    occurred_at_ms: i64,
    event_id: &str,
) -> ExecutionLedgerEvent {
    fill_event_with_confidence(
        order,
        price,
        fee,
        occurred_at_ms,
        event_id,
        shared_types::ExecutionFillConfidence::VenueOrderSnapshot,
    )
}

fn fill_event_with_confidence(
    order: &OrderRecord,
    price: f64,
    fee: f64,
    occurred_at_ms: i64,
    event_id: &str,
    confidence: shared_types::ExecutionFillConfidence,
) -> ExecutionLedgerEvent {
    ExecutionLedgerEvent {
        event_id: event_id.into(),
        event_type: ExecutionLedgerEventType::FillSnapshot,
        source: OrderUpdateSource::PrivateWs,
        order: ExecutionLedgerOrderRef {
            run_id: None,
            ticket_id: None,
            leg_role: None,
            reduce_only: None,
            exchange: order.intent.exchange.clone(),
            symbol: order.intent.symbol.clone(),
            side: order.intent.side,
            identity: order.identity_snapshot(),
        },
        payload: ExecutionLedgerPayload::FillSnapshot(FillLedgerSnapshot {
            quantity: 1.0,
            average_price: price,
            quote_value: price,
            quality: ExecutionLedgerQuality::Actual,
            confidence,
            fee: Some(FeeLedgerSnapshot {
                amount: fee,
                currency: Some("USDT".into()),
                quality: ExecutionLedgerQuality::Actual,
            }),
        }),
        occurred_at_ms,
        captured_at_ms: occurred_at_ms,
    }
}

fn incremental_fill_event(
    order: &OrderRecord,
    price: f64,
    fee: f64,
    occurred_at_ms: i64,
    event_id: &str,
) -> ExecutionLedgerEvent {
    let mut event = fill_event(order, price, fee, occurred_at_ms, event_id);
    if let ExecutionLedgerPayload::FillSnapshot(fill) = &mut event.payload {
        fill.confidence = shared_types::ExecutionFillConfidence::VenueFill;
    }
    ExecutionLedgerEvent {
        event_type: ExecutionLedgerEventType::FillEvent,
        ..event
    }
}

fn funding_event(
    order: &OrderRecord,
    amount: f64,
    occurred_at_ms: i64,
    event_id: &str,
) -> ExecutionLedgerEvent {
    ExecutionLedgerEvent {
        event_id: event_id.into(),
        event_type: ExecutionLedgerEventType::FundingPayment,
        source: OrderUpdateSource::PrivateWs,
        order: ExecutionLedgerOrderRef {
            run_id: None,
            ticket_id: None,
            leg_role: None,
            reduce_only: None,
            exchange: order.intent.exchange.clone(),
            symbol: order.intent.symbol.clone(),
            side: order.intent.side,
            identity: order.identity_snapshot(),
        },
        payload: ExecutionLedgerPayload::FundingPayment(FundingPaymentLedgerRecord {
            amount,
            currency: "USDT".into(),
            funding_time_ms: occurred_at_ms,
            quality: ExecutionLedgerQuality::Actual,
        }),
        occurred_at_ms,
        captured_at_ms: occurred_at_ms,
    }
}

fn slippage_event(
    order: &OrderRecord,
    amount_usd: f64,
    occurred_at_ms: i64,
    event_id: &str,
) -> ExecutionLedgerEvent {
    ExecutionLedgerEvent {
        event_id: event_id.into(),
        event_type: ExecutionLedgerEventType::Slippage,
        source: OrderUpdateSource::PrivateWs,
        order: ExecutionLedgerOrderRef {
            run_id: None,
            ticket_id: None,
            leg_role: None,
            reduce_only: None,
            exchange: order.intent.exchange.clone(),
            symbol: order.intent.symbol.clone(),
            side: order.intent.side,
            identity: order.identity_snapshot(),
        },
        payload: ExecutionLedgerPayload::Slippage(shared_types::SlippageLedgerRecord {
            amount_usd,
            reference_price: order.intent.price.unwrap_or(1.0),
            fill_price: order.intent.price.unwrap_or(1.0),
            quantity: 1.0,
            quality: ExecutionLedgerQuality::Actual,
        }),
        occurred_at_ms,
        captured_at_ms: occurred_at_ms,
    }
}

fn orderbook_event(
    order: &OrderRecord,
    occurred_at_ms: i64,
    event_id: &str,
) -> ExecutionLedgerEvent {
    ExecutionLedgerEvent {
        event_id: event_id.into(),
        event_type: ExecutionLedgerEventType::OrderbookEvidence,
        source: OrderUpdateSource::Internal,
        order: ExecutionLedgerOrderRef {
            run_id: None,
            ticket_id: None,
            leg_role: None,
            reduce_only: None,
            exchange: order.intent.exchange.clone(),
            symbol: order.intent.symbol.clone(),
            side: order.intent.side,
            identity: order.identity_snapshot(),
        },
        payload: ExecutionLedgerPayload::OrderbookEvidence(Box::new(
            shared_types::OrderbookDepthLedgerRecord {
                reference_price: Some(100.0),
                bid: Some(99.9),
                ask: Some(100.1),
                mid: Some(100.0),
                open_vwap_price: Some(100.1),
                open_slippage_bps: Some(1.0),
                close_vwap_price: Some(99.9),
                close_slippage_bps: Some(1.0),
                depth_usd_5bps: Some(500.0),
                depth_usd_10bps: Some(1000.0),
                depth_usd_20bps: Some(1500.0),
                max_notional_usd: Some(1500.0),
                market_timestamp_ms: Some(occurred_at_ms),
                health: None,
                reason: None,
                quality: ExecutionLedgerQuality::Actual,
            },
        )),
        occurred_at_ms,
        captured_at_ms: occurred_at_ms,
    }
}
