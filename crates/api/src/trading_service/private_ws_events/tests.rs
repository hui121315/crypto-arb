#![allow(clippy::expect_used)]

use super::*;
use crate::trading_service::AdapterCredentials;
use chrono::Utc;
use shared_types::{
    ExecutionLedgerEventType, ExecutionLedgerPayload, ExecutionMode, OrderSide, OrderSource,
    OrderStatus, OrderType, StrategyKind,
};

#[derive(Clone, Copy)]
struct FillFixture<'a> {
    venue: &'a str,
    exchange_order_id: &'a str,
    venue_event_id: &'a str,
    quantity: f64,
    price: f64,
    fee_amount: f64,
    occurred_at_ms: i64,
}

#[derive(Clone, Copy)]
struct FundingVenueCase<'a> {
    order_venue: &'a str,
    event_venue: &'a str,
    order_symbol: &'a str,
    fill_symbol: &'a str,
    funding_symbol: &'a str,
    currency: &'a str,
}

mod cache_patch;
mod cache_seed;
mod deltas;
mod funding_cross_venue;
mod kraken_spot;
mod pnl;
mod pnl_hyperliquid;

fn intent(id: &str, client_id: &str) -> shared_types::OrderIntent {
    shared_types::OrderIntent {
        id: id.into(),
        source: OrderSource::Manual,
        strategy: None,
        mode: ExecutionMode::DryRun,
        exchange: "mock".into(),
        symbol: "BTC".into(),
        side: OrderSide::Buy,
        order_type: OrderType::Limit,
        quantity: 1.0,
        price: Some(50_000.0),
        slippage_tolerance_bps: None,
        reduce_only: false,
        time_in_force: shared_types::hedge::TimeInForce::Gtc,
        post_only: false,
        margin_mode: shared_types::hedge::MarginMode::Cross,
        leverage: 1.0,
        client_order_id: client_id.into(),
        client_order_id_policy: None,
        created_at_ms: 1,
    }
}

fn arbitrage_intent(id: &str, client_id: &str, side: OrderSide) -> shared_types::OrderIntent {
    arbitrage_intent_on(id, client_id, side, "mock", "BTC")
}

fn arbitrage_intent_on(
    id: &str,
    client_id: &str,
    side: OrderSide,
    exchange: &str,
    symbol: &str,
) -> shared_types::OrderIntent {
    let mut intent = intent(id, client_id);
    intent.source = OrderSource::ArbitragePreview;
    intent.strategy = Some(StrategyKind::PerpCross);
    intent.side = side;
    intent.exchange = exchange.into();
    intent.symbol = symbol.into();
    intent
}

fn seed_accepted_order(
    service: &TradingService,
    intent: shared_types::OrderIntent,
    exchange_order_id: &str,
) {
    let internal_order_id = intent.id.clone();
    let client_order_id = intent.client_order_id.clone();
    service.journal.insert_created(intent, 1);
    service.journal.mark_risk_checked(
        &internal_order_id,
        shared_types::RiskDecision::allow(50_000.0),
        2,
    );
    service.journal.mark_submitted(&internal_order_id, 3);
    service.journal.apply_ack(&shared_types::OrderAck {
        internal_order_id,
        exchange_order_id: Some(exchange_order_id.into()),
        client_order_id,
        identity_update: Default::default(),
        state: shared_types::LiveOrderState::Accepted,
        accepted_at_ms: 4,
        message: None,
        filled_quantity: None,
        filled_price: None,
        filled_fee: None,
    });
}

fn private_fill(
    exchange_order_id: &str,
    venue_event_id: &str,
    quantity: f64,
    price: f64,
    fee_amount: f64,
    occurred_at_ms: i64,
) -> PrivateFillDelta {
    private_fill_on(FillFixture {
        venue: "mock",
        exchange_order_id,
        venue_event_id,
        quantity,
        price,
        fee_amount,
        occurred_at_ms,
    })
}

fn private_fill_on(fixture: FillFixture<'_>) -> PrivateFillDelta {
    private_fill_on_symbol(fixture, "BTC")
}

fn private_fill_on_symbol(fixture: FillFixture<'_>, symbol: &str) -> PrivateFillDelta {
    PrivateFillDelta {
        venue: fixture.venue.into(),
        exchange_order_id: fixture.exchange_order_id.into(),
        client_order_id: None,
        symbol: Some(symbol.into()),
        side: None,
        venue_event_id: fixture.venue_event_id.into(),
        quantity: fixture.quantity,
        price: fixture.price,
        fee_amount: Some(fixture.fee_amount),
        fee_currency: Some("USDC".into()),
        occurred_at_ms: fixture.occurred_at_ms,
    }
}

fn filled_order_info(
    order_id: &str,
    side: OrderSide,
    filled_quantity: f64,
    filled_price: f64,
    fees: f64,
) -> OrderInfo {
    OrderInfo {
        execution_style: None,
        venue_time_in_force: None,
        client_order_id: None,
        reduce_only: None,
        order_id: order_id.into(),
        symbol: "BTC".into(),
        exchange: "mock".into(),
        side,
        order_type: OrderType::Limit,
        status: OrderStatus::Filled,
        quantity: filled_quantity,
        price: filled_price,
        filled_quantity,
        filled_price,
        fees,
        created_at: Utc::now(),
    }
}

async fn mark_all_seeded_orders_filled(service: &TradingService, received_at_ms: i64) {
    for record in service.list_orders() {
        let exchange_order_id = record
            .exchange_order_id
            .as_deref()
            .expect("seeded exchange order id");
        let mut order = filled_order_info(
            exchange_order_id,
            record.intent.side,
            record.intent.quantity,
            record.intent.price.unwrap_or(1.0),
            0.0,
        );
        order.exchange = record.intent.exchange;
        order.symbol = record.intent.symbol;
        let outcome = service
            .apply_private_ws_event(PrivateWsEvent::Order(PrivateOrderDelta {
                client_order_id: String::new(),
                order,
                received_at_ms,
            }))
            .await;
        assert_eq!(
            outcome.order.as_ref().map(|updated| updated.state),
            Some(shared_types::LiveOrderState::Filled)
        );
    }
}

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1e-9,
        "actual={actual}, expected={expected}"
    );
}

async fn isolated_app_state() -> anyhow::Result<crate::state::AppState> {
    let mut config = common::config::AppConfig::default();
    config.storage.portfolio_nav_path = None;
    config.storage.execution_run_ledger_path = None;
    config.storage.execution_ledger_path = None;
    config.storage.order_snapshot_path = None;
    config.storage.close_run_ledger_path = None;
    crate::state::AppState::new(config).await
}

fn position_info(exchange: &str, symbol: &str, side: &str, quantity: f64) -> PositionInfo {
    PositionInfo {
        symbol: symbol.into(),
        exchange: exchange.into(),
        side: side.into(),
        quantity,
        entry_price: 1.0,
        mark_price: 1.0,
        unrealized_pnl: 0.0,
        leverage: 1.0,
        liquidation_price: None,
        liquidation_distance_pct: None,
        next_funding_ms: None,
        paired_with: None,
        margin: 0.0,
        maintenance_margin_ratio: 0.0,
        position_mode: None,
        margin_mode: None,
        risk_rate: None,
        available_position: None,
        frozen_position: None,
    }
}

fn balance_info(venue: &str, currency: &str, total: f64) -> VenueBalanceInfo {
    VenueBalanceInfo {
        venue: venue.into(),
        currency: currency.into(),
        total,
        available: total,
        frozen: 0.0,
        unrealized_pnl: 0.0,
    }
}

fn order_info(status: OrderStatus) -> OrderInfo {
    OrderInfo {
        execution_style: None,
        venue_time_in_force: None,
        client_order_id: None,
        reduce_only: None,
        order_id: "e1".into(),
        symbol: "BTC".into(),
        exchange: "mock".into(),
        side: OrderSide::Buy,
        order_type: OrderType::Limit,
        status,
        quantity: 1.0,
        price: 50_000.0,
        filled_quantity: 1.0,
        filled_price: 50_000.0,
        fees: 0.1,
        created_at: Utc::now(),
    }
}
