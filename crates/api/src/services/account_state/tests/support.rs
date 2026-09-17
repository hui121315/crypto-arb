use super::*;

pub(super) fn open_order_row(venue: &str) -> shared_types::OrderInfo {
    shared_types::OrderInfo {
        execution_style: None,
        venue_time_in_force: None,
        client_order_id: None,
        reduce_only: None,
        order_id: "o-1".to_owned(),
        symbol: "BTCUSDT".to_owned(),
        exchange: venue.to_owned(),
        side: shared_types::OrderSide::Buy,
        order_type: shared_types::OrderType::Limit,
        status: shared_types::OrderStatus::Open,
        quantity: 1.0,
        price: 10.0,
        filled_quantity: 0.0,
        filled_price: 0.0,
        fees: 0.0,
        created_at: chrono::Utc::now(),
    }
}

pub(super) fn balance_row(venue: &str) -> shared_types::VenueBalanceInfo {
    shared_types::VenueBalanceInfo {
        venue: venue.to_owned(),
        currency: "USDT".to_owned(),
        total: 100.0,
        available: 90.0,
        frozen: 10.0,
        unrealized_pnl: 0.0,
    }
}

pub(super) fn position_row(venue: &str) -> shared_types::PositionInfo {
    shared_types::PositionInfo {
        exchange: venue.to_owned(),
        symbol: "BTCUSDT".to_owned(),
        side: "long".to_owned(),
        quantity: 1.0,
        entry_price: 10.0,
        mark_price: 11.0,
        unrealized_pnl: 1.0,
        leverage: 1.0,
        liquidation_price: None,
        liquidation_distance_pct: None,
        next_funding_ms: None,
        paired_with: None,
        margin: 10.0,
        maintenance_margin_ratio: 0.0,
        position_mode: None,
        margin_mode: None,
        risk_rate: None,
        available_position: None,
        frozen_position: None,
    }
}

pub(super) fn health_row(
    venue: &str,
    operation: &str,
    status: VenueOperationStatus,
) -> VenueOperationHealth {
    VenueOperationHealth {
        venue: venue.to_owned(),
        operation: operation.to_owned(),
        status,
        source: "test".to_owned(),
        message: "test".to_owned(),
        supported: Some(true),
        configured: Some(true),
        requested: None,
        rows: Some(1),
        freshness_ms: Some(1),
        retry_after_ms: None,
        latency_ms: None,
        latency_p95_ms: None,
        error: None,
        evidence: None,
        problem: None,
        observed_at_ms: 10,
    }
}
