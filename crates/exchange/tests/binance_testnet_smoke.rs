#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]
//! Binance Futures Testnet smoke tests.
//!
//! These tests are ignored by default and require explicit environment variables.

use exchange::{Binance, BinanceConfig, BinanceCredentials, LiveTradingAdapter};
use shared_types::{ExecutionMode, LiveOrderState, OrderIntent, OrderSide, OrderSource, OrderType};
use std::env;
use std::time::{SystemTime, UNIX_EPOCH};

const CONFIRM_ENV: &str = "CRYPTO_ARB_RUN_BINANCE_TESTNET_SMOKE";
const CONFIRM_VALUE: &str = "I_UNDERSTAND_THIS_PLACES_TESTNET_ORDERS";
const BINANCE_FUTURES_MIN_NOTIONAL: f64 = 50.0;

#[tokio::test]
#[ignore = "requires Binance Futures Testnet credentials and explicit confirmation"]
async fn binance_futures_testnet_place_query_cancel_limit_order() {
    require_confirmation();

    let api_key = required_env("BINANCE_FUTURES_TESTNET_API_KEY");
    let api_secret = required_env("BINANCE_FUTURES_TESTNET_API_SECRET");
    let symbol = required_env("BINANCE_FUTURES_TESTNET_SYMBOL");
    let quantity = required_env("BINANCE_FUTURES_TESTNET_QUANTITY")
        .parse::<f64>()
        .expect("BINANCE_FUTURES_TESTNET_QUANTITY must be a positive number");
    let price = required_env("BINANCE_FUTURES_TESTNET_LIMIT_PRICE")
        .parse::<f64>()
        .expect("BINANCE_FUTURES_TESTNET_LIMIT_PRICE must be a positive number");

    assert!(quantity > 0.0, "quantity must be positive");
    assert!(price > 0.0, "price must be positive");
    assert!(
        quantity * price >= BINANCE_FUTURES_MIN_NOTIONAL,
        "quantity * price must be at least {BINANCE_FUTURES_MIN_NOTIONAL} USDT"
    );

    let adapter = Binance::new(BinanceConfig {
        credentials: Some(BinanceCredentials {
            api_key,
            api_secret,
        }),
        testnet: true,
        allow_live_writes: false,
        timeout_secs: 10,
        qps: 5,
        base_url_override: None,
    })
    .expect("binance testnet adapter");

    let internal_id = format!("smoke-{}", now_ms());
    let client_order_id = format!("crypto-arb-{internal_id}");
    let intent = OrderIntent {
        id: internal_id.clone(),
        source: OrderSource::Manual,
        strategy: None,
        mode: ExecutionMode::Testnet,
        exchange: "binance".into(),
        symbol: symbol.clone(),
        side: OrderSide::Buy,
        order_type: OrderType::Limit,
        quantity,
        price: Some(price),
        slippage_tolerance_bps: None,
        reduce_only: false,
        time_in_force: shared_types::TimeInForce::Ioc,
        post_only: false,
        margin_mode: shared_types::MarginMode::Cross,
        leverage: 1.0,
        client_order_id: client_order_id.clone(),
        client_order_id_policy: None,
        created_at_ms: now_ms(),
    };

    let placed = adapter.place_order(&intent).await.expect("place order");
    assert_eq!(placed.internal_order_id, internal_id);
    assert_eq!(placed.client_order_id, client_order_id);
    assert!(matches!(
        placed.state,
        LiveOrderState::Accepted | LiveOrderState::PartiallyFilled | LiveOrderState::Filled
    ));

    let queried = adapter
        .get_order(&symbol, &client_order_id)
        .await
        .expect("query order")
        .expect("order should exist after placement");
    assert_eq!(queried.symbol, symbol.to_ascii_uppercase());

    let cancel_result = adapter
        .cancel_order(&shared_types::CancelOrderRequest {
            exchange: "binance".into(),
            symbol: symbol.clone(),
            internal_order_id: internal_id,
            exchange_order_id: placed.exchange_order_id,
            client_order_id,
        })
        .await;

    if queried.status != shared_types::OrderStatus::Filled {
        let cancelled = cancel_result.expect("cancel order");
        assert!(matches!(
            cancelled.state,
            LiveOrderState::Cancelled | LiveOrderState::Filled | LiveOrderState::Unknown
        ));
    }

    let _open_orders = LiveTradingAdapter::get_open_orders(&adapter, Some(&symbol))
        .await
        .expect("open orders");
    let _positions = LiveTradingAdapter::get_positions(&adapter, Some(&symbol))
        .await
        .expect("positions");
}

fn require_confirmation() {
    let actual = required_env(CONFIRM_ENV);
    assert_eq!(
        actual, CONFIRM_VALUE,
        "set {CONFIRM_ENV}={CONFIRM_VALUE} to run this smoke test"
    );
}

fn required_env(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing required env var: {name}"))
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_millis() as i64
}
