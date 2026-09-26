use super::*;
use exchange::{ExchangeCapabilities, ExchangeError, ExchangeResult, LiveTradingAdapter};
use shared_types::{
    CancelOrderRequest, ExecutionMode, LiveOrderState, OrderAck, OrderInfo, OrderIntent, OrderSide,
    OrderSource, OrderStatus, OrderType, PositionInfo, VenueBalanceInfo,
};
use std::fmt::Debug;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

#[allow(clippy::panic)]
fn must_ok<T, E: Debug>(result: Result<T, E>, context: &str) -> T {
    match result {
        Ok(value) => value,
        Err(error) => panic!("{context}: {error:?}"),
    }
}

#[allow(clippy::panic)]
fn must_err<T: Debug, E>(result: Result<T, E>, context: &str) -> E {
    match result {
        Ok(value) => panic!("{context}: unexpected ok {value:?}"),
        Err(error) => error,
    }
}

fn limit_intent(id: &str) -> OrderIntent {
    OrderIntent {
        id: id.into(),
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
        client_order_id: format!("client-{id}"),
        client_order_id_policy: None,
        created_at_ms: 1,
    }
}

fn credentials() -> AdapterCredentials {
    AdapterCredentials {
        binance_live: Some(("lk".into(), "ls".into())),
        bitget_live: Some(("gk".into(), "gs".into(), "gp".into())),
        bybit_live: Some(("bk".into(), "bs".into())),
        gate_live: Some(("gtk".into(), "gts".into())),
        gate_crossex_live: None,
        hyperliquid_live: Some(HyperliquidAdapterCredentials {
            account_address: "0x0000000000000000000000000000000000000001".into(),
            private_key: "0101010101010101010101010101010101010101010101010101010101010101".into(),
            vault_address: None,
        }),
        kucoin_live: Some(("kk".into(), "ks".into(), "kp".into())),
        kraken_live: None,
        okx_live: Some(("lk".into(), "ls".into(), "lp".into())),
    }
}

mod adapters;
mod balance_latency;
mod balances;
mod reconcile;
mod selection;
mod order_accounts;
mod submit_proof;
mod submit_rate_limit_adapter;
mod submit_recovery;
mod support;
