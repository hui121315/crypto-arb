use shared_types::{ExecutionMode, OrderIntent, OrderSide, OrderSource, OrderType};

mod gating;
mod market_and_hedge;
mod order_validation;

fn limit_intent() -> OrderIntent {
    OrderIntent {
        id: "intent-1".into(),
        source: OrderSource::Manual,
        strategy: None,
        mode: ExecutionMode::Testnet,
        exchange: "mock".into(),
        symbol: "BTC".into(),
        side: OrderSide::Buy,
        order_type: OrderType::Limit,
        quantity: 0.01,
        price: Some(50_000.0),
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

fn market_intent() -> OrderIntent {
    OrderIntent {
        id: "intent-mkt".into(),
        source: OrderSource::Manual,
        strategy: None,
        mode: ExecutionMode::Testnet,
        exchange: "mock".into(),
        symbol: "BTC".into(),
        side: OrderSide::Sell,
        order_type: OrderType::Market,
        quantity: 0.01,
        price: Some(50_000.0),
        slippage_tolerance_bps: None,
        reduce_only: true,
        time_in_force: shared_types::TimeInForce::Ioc,
        post_only: false,
        margin_mode: shared_types::MarginMode::Cross,
        leverage: 1.0,
        client_order_id: "client-mkt".into(),
        client_order_id_policy: None,
        created_at_ms: 1,
    }
}
