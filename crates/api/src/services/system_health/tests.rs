use super::*;

mod health;
mod misc;

fn position(next_funding_ms: i64) -> shared_types::PositionRow {
    shared_types::PositionRow {
        venue: "OKX".into(),
        symbol: "BTC".into(),
        origin: Default::default(),
        side: shared_types::PositionSide::Long,
        quantity: 1.0,
        entry_price: 100.0,
        mark_price: 100.0,
        leverage: 2.0,
        unrealized_pnl_usd: 0.0,
        liquidation_price: None,
        liquidation_distance_pct: None,
        next_funding_ms: Some(next_funding_ms),
        funding_rate_8h: 0.0,
        funding_rate_verified: true,
        maintenance_margin_ratio: 0.05,
        pair_evidence: None,
        paired_with: None,
        margin_usd: 50.0,
        severity: shared_types::PositionSeverity::Ok,
        seconds_until_funding: Some(((next_funding_ms - 60_000).max(0) / 1000) as u32),
    }
}

fn order(state: LiveOrderState, created_at_ms: i64, updated_at_ms: i64) -> OrderRecord {
    OrderRecord {
        intent: shared_types::OrderIntent {
            id: "o".into(),
            source: shared_types::OrderSource::Manual,
            strategy: None,
            mode: shared_types::ExecutionMode::DryRun,
            exchange: "mock".into(),
            symbol: "BTCUSDT".into(),
            side: shared_types::OrderSide::Buy,
            order_type: shared_types::OrderType::Limit,
            quantity: 1.0,
            price: Some(10.0),
            slippage_tolerance_bps: None,
            reduce_only: false,
            time_in_force: shared_types::TimeInForce::Ioc,
            post_only: false,
            margin_mode: shared_types::MarginMode::Cross,
            leverage: 1.0,
            client_order_id: "c".into(),
            client_order_id_policy: None,
            created_at_ms,
        },
        state,
        risk: None,
        identity: Default::default(),
        last_update_source: Default::default(),
        exchange_order_id: None,
        message: None,
        filled_quantity: None,
        filled_price: None,
        filled_fee: None,
        updated_at_ms,
    }
}

fn operation_health(
    venue: &str,
    operation: &str,
    status: VenueOperationStatus,
) -> VenueOperationHealth {
    VenueOperationHealth {
        venue: venue.to_owned(),
        operation: operation.to_owned(),
        status,
        source: "test".to_owned(),
        message: "runtime health".to_owned(),
        supported: Some(true),
        configured: Some(true),
        requested: Some(1),
        rows: Some(u64::from(status == VenueOperationStatus::Ok)),
        freshness_ms: Some(250),
        retry_after_ms: None,
        latency_ms: None,
        latency_p95_ms: None,
        error: None,
        evidence: None,
        problem: None,
        observed_at_ms: 1_000,
    }
}

fn unsupported_operation_health(venue: &str, operation: &str) -> VenueOperationHealth {
    VenueOperationHealth {
        supported: Some(false),
        status: VenueOperationStatus::Unsupported,
        ..operation_health(venue, operation, VenueOperationStatus::Unsupported)
    }
}
