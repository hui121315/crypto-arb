//! Binance USD-M Futures user data stream parser.
//!
//! Official docs:
//! - User stream base: <https://developers.binance.com/docs/derivatives/usds-margined-futures/user-data-streams>
//! - `/private` migration: <https://developers.binance.com/en/docs/products/derivatives-trading-usds-futures/websocket-market-streams/Important-WebSocket-Change-Notice>
//! - Balance and position update: <https://developers.binance.com/docs/derivatives/usds-margined-futures/user-data-streams/Event-Balance-and-Position-Update>
//! - Order update: <https://developers.binance.com/docs/derivatives/usds-margined-futures/user-data-streams/Event-Order-Update>

use super::binance_ws_user_data;
use crate::error::{ExchangeError, ExchangeResult};
use shared_types::{LiveOrderState, OrderInfo};

pub(super) const EXCHANGE: &str = "binance";
pub(super) const EVENT_ACCOUNT_UPDATE: &str = "ACCOUNT_UPDATE";
pub(super) const EVENT_ORDER_UPDATE: &str = "ORDER_TRADE_UPDATE";
pub const USER_STREAM_KEEPALIVE_INTERVAL_SECS: u64 = 30 * 60;
pub const USER_STREAM_CONNECTION_MAX_SECS: u64 = 23 * 60 * 60;
const USER_STREAM_WS_BASE: &str = "wss://fstream.binance.com/private";
const USER_STREAM_EVENTS: &str = "ORDER_TRADE_UPDATE/ACCOUNT_UPDATE";

#[derive(Debug, Clone, PartialEq)]
pub enum BinanceUserEvent {
    Account(BinanceAccountUpdate),
    Order(Box<BinanceOrderTradeUpdate>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct BinanceAccountUpdate {
    pub event_time_ms: i64,
    pub transaction_time_ms: i64,
    pub reason: String,
    pub balances: Vec<BinanceAccountBalanceDelta>,
    pub positions: Vec<BinancePositionDelta>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BinanceAccountBalanceDelta {
    pub asset: String,
    pub wallet_balance: f64,
    pub cross_wallet_balance: f64,
    pub balance_change: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BinancePositionDelta {
    pub symbol: String,
    pub side: String,
    pub quantity: f64,
    pub entry_price: f64,
    pub accumulated_realized: f64,
    pub unrealized_pnl: f64,
    pub margin_type: String,
    pub isolated_wallet: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BinanceOrderTradeUpdate {
    pub event_time_ms: i64,
    pub transaction_time_ms: i64,
    pub trade_time_ms: i64,
    pub client_order_id: String,
    pub execution_type: String,
    pub order_status: String,
    pub reject_reason: Option<String>,
    pub trade_id: Option<i64>,
    pub last_filled_quantity: f64,
    pub last_filled_price: f64,
    pub commission_asset: Option<String>,
    pub live_state: LiveOrderState,
    pub order: OrderInfo,
}

impl BinanceOrderTradeUpdate {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.live_state,
            LiveOrderState::Filled
                | LiveOrderState::Cancelled
                | LiveOrderState::Rejected
                | LiveOrderState::Failed
        )
    }
}

pub fn parse_user_event(text: &str) -> ExchangeResult<Option<BinanceUserEvent>> {
    binance_ws_user_data::parse_user_event(text)
}

pub fn user_stream_ws_url(listen_key: &str) -> ExchangeResult<String> {
    user_stream_ws_url_with_base(USER_STREAM_WS_BASE, listen_key)
}

pub fn user_stream_ws_url_with_base(base: &str, listen_key: &str) -> ExchangeResult<String> {
    let key = listen_key.trim();
    if key.is_empty() {
        return Err(ExchangeError::Parse(
            "binance user stream empty listenKey".into(),
        ));
    }
    let mut url =
        url::Url::parse(&format!("{}/ws", base.trim_end_matches('/'))).map_err(|error| {
            ExchangeError::Parse(format!(
                "binance user stream invalid websocket base: {error}"
            ))
        })?;
    url.query_pairs_mut()
        .append_pair("listenKey", key)
        .append_pair("events", USER_STREAM_EVENTS);
    Ok(url.into())
}

#[cfg(test)]
#[path = "binance_ws_user_tests.rs"]
mod tests;
