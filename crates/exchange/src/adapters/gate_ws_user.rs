//! Gate.io futures private WebSocket payloads and event parser.
//!
//! Official docs:
//! - Futures WebSocket API: <https://www.gate.com/docs/developers/futures/ws/en/>
//! - `futures.orders` notification: <https://www.gate.com/docs/developers/futures/ws/en/#orders-api>
//! - `futures.positions` notification: <https://www.gate.com/docs/developers/futures/ws/en/#positions-api>
//! - `futures.balances` notification: <https://www.gate.com/docs/developers/futures/ws/en/#balances-api>
//! - `futures.usertrades` notification: <https://www.gate.com/docs/developers/futures/ws/en/#user-trades-api>

use super::gate_ws_user_data;
use crate::error::{ExchangeError, ExchangeResult};
use crate::signing::gate as sign;
use common::time::now_secs;
use serde::Serialize;
use shared_types::{LiveOrderState, OrderInfo};

pub const GATE_PRIVATE_WS_URL: &str = "wss://fx-ws.gateio.ws/v4/ws/usdt";
pub const GATE_TESTNET_PRIVATE_WS_URL: &str = "wss://ws-testnet.gate.com/v4/ws/futures/usdt";

pub(super) const EXCHANGE: &str = "gate";
const EVENT_SUBSCRIBE: &str = "subscribe";
pub(super) const EVENT_UPDATE: &str = "update";
pub(super) const CHANNEL_ORDERS: &str = "futures.orders";
pub(super) const CHANNEL_POSITIONS: &str = "futures.positions";
pub(super) const CHANNEL_BALANCES: &str = "futures.balances";
pub(super) const CHANNEL_USERTRADES: &str = "futures.usertrades";

#[derive(Debug, Clone, Copy)]
pub struct GateUserWsConfig<'a> {
    pub api_key: &'a str,
    pub api_secret: &'a str,
    pub user_id: &'a str,
    pub time_offset_secs: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GateUserEvent {
    Order(Vec<GateOrderUpdate>),
    Position(Vec<GatePositionDelta>),
    Balance(Vec<GateBalanceDelta>),
    UserTrade(Vec<GateUserTradeDelta>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct GateOrderUpdate {
    pub client_order_id: String,
    pub live_state: LiveOrderState,
    pub order: OrderInfo,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GatePositionDelta {
    pub symbol: String,
    pub side: String,
    pub size: f64,
    pub entry_price: f64,
    pub mark_price: f64,
    pub unrealized_pnl: f64,
    pub leverage: f64,
    pub liquidation_price: Option<f64>,
    pub margin: f64,
    pub maintenance_margin_ratio: f64,
    pub updated_time_ms: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GateBalanceDelta {
    pub currency: String,
    pub balance: f64,
    pub change: f64,
    pub available: f64,
    pub position_margin: f64,
    pub order_margin: f64,
    pub unrealized_pnl: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GateUserTradeDelta {
    pub trade_id: String,
    pub exchange_order_id: String,
    pub symbol: String,
    pub quantity: f64,
    pub price: f64,
    pub fee: f64,
    pub point_fee: f64,
    pub occurred_at_ms: i64,
}

pub fn private_ws_url(testnet: bool) -> &'static str {
    if testnet {
        GATE_TESTNET_PRIVATE_WS_URL
    } else {
        GATE_PRIVATE_WS_URL
    }
}

pub fn subscribe_orders_payload(
    cfg: GateUserWsConfig<'_>,
    contract: &str,
) -> ExchangeResult<String> {
    subscribe_payload(cfg, CHANNEL_ORDERS, contract)
}

pub fn subscribe_positions_payload(
    cfg: GateUserWsConfig<'_>,
    contract: &str,
) -> ExchangeResult<String> {
    subscribe_payload(cfg, CHANNEL_POSITIONS, contract)
}

pub fn subscribe_balances_payload(cfg: GateUserWsConfig<'_>) -> ExchangeResult<String> {
    subscribe_payload(cfg, CHANNEL_BALANCES, "")
}

pub fn subscribe_usertrades_payload(
    cfg: GateUserWsConfig<'_>,
    contract: &str,
) -> ExchangeResult<String> {
    subscribe_payload(cfg, CHANNEL_USERTRADES, contract)
}

pub fn parse_user_event(text: &str) -> ExchangeResult<Option<GateUserEvent>> {
    gate_ws_user_data::parse_user_event(text)
}

fn subscribe_payload(
    cfg: GateUserWsConfig<'_>,
    channel: &'static str,
    contract: &str,
) -> ExchangeResult<String> {
    let user_id = checked_user_id(cfg.user_id)?;
    let time = now_secs().saturating_add(cfg.time_offset_secs);
    let request = WsRequest {
        time,
        channel,
        event: EVENT_SUBSCRIBE,
        payload: subscription_payload(user_id, contract),
        auth: AuthPayload {
            method: "api_key",
            key: cfg.api_key.to_owned(),
            sign: sign::ws_sign(
                cfg.api_secret.as_bytes(),
                channel,
                EVENT_SUBSCRIBE,
                &time.to_string(),
            ),
        },
    };
    serde_json::to_string(&request)
        .map_err(|error| ExchangeError::Parse(format!("gate ws user subscribe payload: {error}")))
}

fn checked_user_id(user_id: &str) -> ExchangeResult<String> {
    let trimmed = user_id.trim();
    if trimmed.is_empty() {
        return Err(ExchangeError::Parse(
            "gate ws user subscription requires official user id".into(),
        ));
    }
    Ok(trimmed.to_owned())
}

fn subscription_payload(user_id: String, contract: &str) -> Vec<String> {
    let trimmed = contract.trim();
    if trimmed.is_empty() {
        vec![user_id]
    } else {
        vec![user_id, trimmed.to_owned()]
    }
}

#[derive(Debug, Serialize)]
struct WsRequest {
    time: i64,
    channel: &'static str,
    event: &'static str,
    payload: Vec<String>,
    auth: AuthPayload,
}

#[derive(Debug, Serialize)]
struct AuthPayload {
    method: &'static str,
    #[serde(rename = "KEY")]
    key: String,
    #[serde(rename = "SIGN")]
    sign: String,
}

#[cfg(test)]
#[path = "gate_ws_user_tests.rs"]
mod tests;
