//! Hyperliquid user WebSocket payloads and event parser.
//!
//! Official docs:
//! - Subscriptions: <https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/websocket/subscriptions>
//! - Order status values: <https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/info-endpoint#query-order-status-by-oid-or-cloid>

use super::hyperliquid_ws_user_data;
use crate::error::{ExchangeError, ExchangeResult};
use serde::Serialize;
use shared_types::{LiveOrderState, OrderInfo};

pub const HYPERLIQUID_WS_URL: &str = "wss://api.hyperliquid.xyz/ws";

const METHOD_SUBSCRIBE: &str = "subscribe";
const METHOD_UNSUBSCRIBE: &str = "unsubscribe";

#[derive(Debug, Clone, PartialEq)]
pub enum HyperliquidUserWsEvent {
    Order(Vec<HyperliquidOrderUpdate>),
    OpenOrders(HyperliquidOpenOrdersSnapshot),
    Fill(Vec<HyperliquidFill>),
    Funding(Vec<HyperliquidFunding>),
    Liquidation(HyperliquidLiquidation),
    NonUserCancel(Vec<HyperliquidNonUserCancel>),
    Clearinghouse(HyperliquidClearinghouseState),
    AllDexsClearinghouse(Vec<HyperliquidDexClearinghouseState>),
    SpotState(Vec<HyperliquidSpotBalance>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct HyperliquidOrderUpdate {
    pub client_order_id: String,
    pub live_state: LiveOrderState,
    pub status_timestamp_ms: i64,
    pub order: OrderInfo,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HyperliquidOpenOrdersSnapshot {
    pub venue: String,
    pub orders: Vec<OrderInfo>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HyperliquidFill {
    pub venue: String,
    pub coin: String,
    pub trade_id: Option<i64>,
    pub order_id: String,
    pub side: String,
    pub price: f64,
    pub size: f64,
    pub fee: f64,
    pub fee_token: String,
    pub closed_pnl: f64,
    pub liquidation: Option<HyperliquidFillLiquidation>,
    pub crossed: bool,
    pub time_ms: i64,
    pub tx_hash: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HyperliquidFillLiquidation {
    pub liquidated_user: Option<String>,
    pub mark_price: f64,
    pub method: HyperliquidLiquidationMethod,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HyperliquidLiquidationMethod {
    Market,
    Backstop,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HyperliquidFunding {
    pub venue: String,
    pub coin: String,
    pub usdc: f64,
    pub size: f64,
    pub funding_rate: f64,
    pub time_ms: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HyperliquidLiquidation {
    pub id: i64,
    pub liquidator: String,
    pub liquidated_user: String,
    pub notional_position: f64,
    pub account_value: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HyperliquidNonUserCancel {
    pub venue: String,
    pub coin: String,
    pub order_id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HyperliquidClearinghouseState {
    pub dex: Option<String>,
    pub user: String,
    pub account_value: f64,
    pub total_margin_used: f64,
    pub withdrawable: f64,
    pub positions: Vec<HyperliquidPositionDelta>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HyperliquidDexClearinghouseState {
    pub dex: String,
    pub state: HyperliquidClearinghouseState,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HyperliquidPositionDelta {
    pub coin: String,
    pub side: String,
    pub size: f64,
    pub entry_price: f64,
    pub liquidation_price: Option<f64>,
    pub margin_used: f64,
    pub unrealized_pnl: f64,
    pub leverage: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HyperliquidSpotBalance {
    pub coin: String,
    pub token: u32,
    pub total: f64,
    pub hold: f64,
    pub entry_notional: f64,
}

pub fn subscribe_order_updates_payload(user: &str) -> ExchangeResult<String> {
    user_subscription_payload("orderUpdates", user)
}

pub fn subscribe_open_orders_payload(user: &str, dex: Option<&str>) -> ExchangeResult<String> {
    subscription_payload(METHOD_SUBSCRIBE, Subscription::open_orders(user, dex)?)
}

pub fn subscribe_user_events_payload(user: &str) -> ExchangeResult<String> {
    user_subscription_payload("userEvents", user)
}

pub fn subscribe_user_fills_payload(user: &str, aggregate_by_time: bool) -> ExchangeResult<String> {
    subscription_payload(
        METHOD_SUBSCRIBE,
        Subscription::user_fills(user, aggregate_by_time)?,
    )
}

pub fn subscribe_user_fundings_payload(user: &str) -> ExchangeResult<String> {
    user_subscription_payload("userFundings", user)
}

pub fn subscribe_clearinghouse_payload(user: &str, dex: Option<&str>) -> ExchangeResult<String> {
    subscription_payload(
        METHOD_SUBSCRIBE,
        Subscription::clearinghouse_state(user, dex)?,
    )
}

pub fn subscribe_all_dexs_clearinghouse_payload(user: &str) -> ExchangeResult<String> {
    user_subscription_payload("allDexsClearinghouseState", user)
}

pub fn subscribe_spot_state_payload(
    user: &str,
    is_portfolio_margin: Option<bool>,
) -> ExchangeResult<String> {
    subscription_payload(
        METHOD_SUBSCRIBE,
        Subscription::spot_state(user, is_portfolio_margin)?,
    )
}

pub fn unsubscribe_payload(subscription: Subscription) -> ExchangeResult<String> {
    subscription_payload(METHOD_UNSUBSCRIBE, subscription)
}

pub fn parse_user_event(text: &str) -> ExchangeResult<Option<HyperliquidUserWsEvent>> {
    hyperliquid_ws_user_data::parse_user_event(text)
}

fn user_subscription_payload(
    subscription_type: &'static str,
    user: &str,
) -> ExchangeResult<String> {
    subscription_payload(
        METHOD_SUBSCRIBE,
        Subscription::user(subscription_type, user)?,
    )
}

fn subscription_payload(
    method: &'static str,
    subscription: Subscription,
) -> ExchangeResult<String> {
    let request = WsRequest {
        method,
        subscription,
    };
    serde_json::to_string(&request)
        .map_err(|error| ExchangeError::Parse(format!("hyperliquid ws user payload: {error}")))
}

fn checked_user(user: &str) -> ExchangeResult<String> {
    let trimmed = user.trim();
    if trimmed.len() == 42 && trimmed.starts_with("0x") {
        Ok(trimmed.to_owned())
    } else {
        Err(ExchangeError::Parse(format!(
            "hyperliquid ws user must be 42-char 0x address: {trimmed}"
        )))
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Subscription {
    #[serde(rename = "type")]
    subscription_type: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "aggregateByTime")]
    aggregate_by_time: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "isPortfolioMargin")]
    is_portfolio_margin: Option<bool>,
}

impl Subscription {
    pub fn user(subscription_type: &'static str, user: &str) -> ExchangeResult<Self> {
        Ok(Self {
            subscription_type,
            user: Some(checked_user(user)?),
            dex: None,
            aggregate_by_time: None,
            is_portfolio_margin: None,
        })
    }

    pub fn user_fills(user: &str, aggregate_by_time: bool) -> ExchangeResult<Self> {
        Ok(Self {
            subscription_type: "userFills",
            user: Some(checked_user(user)?),
            dex: None,
            aggregate_by_time: Some(aggregate_by_time),
            is_portfolio_margin: None,
        })
    }

    pub fn clearinghouse_state(user: &str, dex: Option<&str>) -> ExchangeResult<Self> {
        Self::user_dex("clearinghouseState", user, dex)
    }

    pub fn open_orders(user: &str, dex: Option<&str>) -> ExchangeResult<Self> {
        Self::user_dex("openOrders", user, dex)
    }

    fn user_dex(
        subscription_type: &'static str,
        user: &str,
        dex: Option<&str>,
    ) -> ExchangeResult<Self> {
        Ok(Self {
            subscription_type,
            user: Some(checked_user(user)?),
            dex: dex.map(str::to_owned),
            aggregate_by_time: None,
            is_portfolio_margin: None,
        })
    }

    pub fn spot_state(user: &str, is_portfolio_margin: Option<bool>) -> ExchangeResult<Self> {
        Ok(Self {
            subscription_type: "spotState",
            user: Some(checked_user(user)?),
            dex: None,
            aggregate_by_time: None,
            is_portfolio_margin,
        })
    }
}

#[derive(Debug, Serialize)]
struct WsRequest {
    method: &'static str,
    subscription: Subscription,
}

#[cfg(test)]
#[path = "hyperliquid_ws_user_tests.rs"]
mod tests;
