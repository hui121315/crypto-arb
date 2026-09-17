use super::*;

pub(super) fn named(name: &'static str) -> LiveAdapter {
    position_adapter(name, PositionRead::Empty)
}

pub(super) fn position_adapter(name: &'static str, position_read: PositionRead) -> LiveAdapter {
    Arc::new(NamedAdapter {
        name,
        capabilities: empty_capabilities(),
        position_read,
        balance_read: BalanceRead::Empty,
        order_read: OrderRead::Empty,
        funding_payment_read: FundingPaymentRead::Empty,
        funding_payment_symbols: Arc::new(Mutex::new(Vec::new())),
        order_reads: Arc::new(AtomicUsize::new(0)),
        account_mode: None,
    })
}

pub(super) fn balance_adapter(name: &'static str, balance_read: BalanceRead) -> LiveAdapter {
    Arc::new(NamedAdapter {
        name,
        capabilities: empty_capabilities(),
        position_read: PositionRead::Empty,
        balance_read,
        order_read: OrderRead::Empty,
        funding_payment_read: FundingPaymentRead::Empty,
        funding_payment_symbols: Arc::new(Mutex::new(Vec::new())),
        order_reads: Arc::new(AtomicUsize::new(0)),
        account_mode: None,
    })
}

pub(super) fn funding_payment_adapter(
    name: &'static str,
    funding_payment_read: FundingPaymentRead,
) -> LiveAdapter {
    Arc::new(NamedAdapter {
        name,
        capabilities: empty_capabilities(),
        position_read: PositionRead::Empty,
        balance_read: BalanceRead::Empty,
        order_read: OrderRead::Empty,
        funding_payment_read,
        funding_payment_symbols: Arc::new(Mutex::new(Vec::new())),
        order_reads: Arc::new(AtomicUsize::new(0)),
        account_mode: None,
    })
}

pub(super) fn funding_payment_recording_adapter(
    name: &'static str,
    calls: Arc<Mutex<Vec<Option<String>>>>,
) -> LiveAdapter {
    Arc::new(NamedAdapter {
        name,
        capabilities: empty_capabilities(),
        position_read: PositionRead::Empty,
        balance_read: BalanceRead::Empty,
        order_read: OrderRead::Empty,
        funding_payment_read: FundingPaymentRead::Empty,
        funding_payment_symbols: calls,
        order_reads: Arc::new(AtomicUsize::new(0)),
        account_mode: None,
    })
}

pub(super) fn adapter_with_market(name: &'static str, supports_market_orders: bool) -> LiveAdapter {
    let mut capabilities = empty_capabilities();
    capabilities.supports_market_orders = supports_market_orders;
    Arc::new(NamedAdapter {
        name,
        capabilities,
        position_read: PositionRead::Empty,
        balance_read: BalanceRead::Empty,
        order_read: OrderRead::Empty,
        funding_payment_read: FundingPaymentRead::Empty,
        funding_payment_symbols: Arc::new(Mutex::new(Vec::new())),
        order_reads: Arc::new(AtomicUsize::new(0)),
        account_mode: None,
    })
}

pub(super) fn account_mode_adapter(name: &'static str, mode: &'static str) -> LiveAdapter {
    Arc::new(NamedAdapter {
        name,
        capabilities: empty_capabilities(),
        position_read: PositionRead::Empty,
        balance_read: BalanceRead::Empty,
        order_read: OrderRead::Empty,
        funding_payment_read: FundingPaymentRead::Empty,
        funding_payment_symbols: Arc::new(Mutex::new(Vec::new())),
        order_reads: Arc::new(AtomicUsize::new(0)),
        account_mode: Some(mode),
    })
}

pub(super) fn order_adapter(
    name: &'static str,
    order_read: OrderRead,
    order_reads: Arc<AtomicUsize>,
) -> LiveAdapter {
    Arc::new(NamedAdapter {
        name,
        capabilities: empty_capabilities(),
        position_read: PositionRead::Empty,
        balance_read: BalanceRead::Empty,
        order_read,
        funding_payment_read: FundingPaymentRead::Empty,
        funding_payment_symbols: Arc::new(Mutex::new(Vec::new())),
        order_reads,
        account_mode: None,
    })
}

pub(super) fn route_name(router: &LiveVenueRouter, venue: &str) -> ExchangeResult<&'static str> {
    router.route_for(venue).map(|adapter| adapter.name())
}

pub(super) fn position(exchange: &str) -> PositionInfo {
    PositionInfo {
        symbol: "BTCUSDT".into(),
        exchange: exchange.into(),
        side: "long".into(),
        quantity: 1.0,
        entry_price: 100.0,
        mark_price: 101.0,
        unrealized_pnl: 1.0,
        leverage: 2.0,
        liquidation_price: None,
        liquidation_distance_pct: None,
        next_funding_ms: None,
        paired_with: None,
        margin: 50.0,
        maintenance_margin_ratio: 0.01,
        position_mode: None,
        margin_mode: None,
        risk_rate: None,
        available_position: None,
        frozen_position: None,
    }
}

pub(super) fn balance(venue: &str) -> VenueBalanceInfo {
    VenueBalanceInfo {
        venue: venue.to_owned(),
        currency: "USDT".into(),
        total: 100.0,
        available: 90.0,
        frozen: 10.0,
        unrealized_pnl: 0.0,
    }
}

pub(super) fn order_info(exchange: &str) -> OrderInfo {
    OrderInfo {
        execution_style: None,
        venue_time_in_force: None,
        client_order_id: None,
        reduce_only: None,
        order_id: format!("{exchange}-order"),
        symbol: "BTCUSDT".into(),
        exchange: exchange.into(),
        side: OrderSide::Buy,
        order_type: OrderType::Limit,
        status: OrderStatus::Filled,
        quantity: 1.0,
        price: 100.0,
        filled_quantity: 1.0,
        filled_price: 100.0,
        fees: 0.01,
        created_at: chrono::Utc::now(),
    }
}

pub(super) fn funding_payment(venue: &str) -> FundingPaymentData {
    FundingPaymentData {
        venue: venue.to_owned(),
        symbol: "BTCUSDT".into(),
        amount: -0.12,
        currency: "USDT".into(),
        funding_time_ms: 10,
        venue_event_id: format!("{venue}-funding-10"),
    }
}

pub(super) fn order_intent(exchange: &str) -> OrderIntent {
    OrderIntent {
        id: "order-1".into(),
        source: shared_types::OrderSource::Manual,
        strategy: None,
        mode: shared_types::ExecutionMode::Live,
        exchange: exchange.into(),
        symbol: "BTC".into(),
        side: OrderSide::Buy,
        order_type: OrderType::Limit,
        quantity: 1.0,
        price: Some(100.0),
        slippage_tolerance_bps: None,
        reduce_only: false,
        time_in_force: shared_types::TimeInForce::Ioc,
        post_only: false,
        margin_mode: shared_types::MarginMode::Cross,
        leverage: 1.0,
        client_order_id: "client-1".into(),
        client_order_id_policy: None,
        created_at_ms: 1,
    }
}

pub(super) fn account_mode_info(venue: &str, mode: &str) -> VenueAccountModeInfo {
    VenueAccountModeInfo {
        venue: venue.to_owned(),
        mode: mode.to_owned(),
        source: "test".to_owned(),
        checked_at_ms: 1,
        freshness_ms: Some(0),
        account_scope: None,
    }
}
