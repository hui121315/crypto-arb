//! OKX V5 private WebSocket account/order subscription payloads and parser.
//!
//! Official docs:
//! - WebSocket login: <https://www.okx.com/docs-v5/en/#overview-websocket-login>
//! - Private WS URL: <https://www.okx.com/docs-v5/en/#overview-production-trading-services>
//! - Account channel: <https://www.okx.com/docs-v5/en/#trading-account-websocket-account-channel>
//! - Positions channel: <https://www.okx.com/docs-v5/en/#trading-account-websocket-positions-channel>
//! - Orders channel: <https://www.okx.com/docs-v5/en/#order-book-trading-trade-ws-order-channel>

use super::okx_ws_user_data;
use crate::error::{ExchangeError, ExchangeResult};
use crate::signing::okx as sign;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use shared_types::{LiveOrderState, OrderInfo};

pub const OKX_PRIVATE_WS_URL: &str = "wss://ws.okx.com:8443/ws/v5/private";
pub const OKX_DEMO_PRIVATE_WS_URL: &str = "wss://wspap.okx.com:8443/ws/v5/private";

pub(super) const EXCHANGE: &str = "okx";
const OP_LOGIN: &str = "login";
const OP_SUBSCRIBE: &str = "subscribe";
const LOGIN_PATH: &str = "/users/self/verify";
pub(super) const CHANNEL_ACCOUNT: &str = "account";
pub(super) const CHANNEL_POSITIONS: &str = "positions";
pub(super) const CHANNEL_ORDERS: &str = "orders";

#[derive(Debug, Clone, Copy)]
pub struct OkxUserWsConfig<'a> {
    pub api_key: &'a str,
    pub api_secret: &'a str,
    pub passphrase: &'a str,
}

#[derive(Debug, Clone, PartialEq)]
pub enum OkxUserEvent {
    Account(OkxAccountUpdate),
    Position(OkxPositionUpdate),
    Order(Vec<OkxOrderUpdate>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OkxUserControl {
    Acknowledged {
        channel: String,
        request_id: Option<String>,
    },
    Rejected {
        channel: String,
        request_id: Option<String>,
        authentication_failed: bool,
        error: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct OkxAccountUpdate {
    pub event_type: String,
    pub last_page: bool,
    pub summary: Option<OkxAccountSummaryDelta>,
    pub balances: Vec<OkxBalanceDelta>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OkxAccountSummaryDelta {
    pub total_equity_usd: f64,
    pub total_available_balance_usd: f64,
    pub total_initial_margin_usd: f64,
    pub total_maintenance_margin_usd: f64,
    pub updated_time_ms: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OkxBalanceDelta {
    pub currency: String,
    pub total: f64,
    pub available: f64,
    pub frozen: f64,
    pub unrealized_pnl: f64,
    pub updated_time_ms: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OkxPositionUpdate {
    pub event_type: String,
    pub last_page: bool,
    pub positions: Vec<OkxPositionDelta>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OkxPositionDelta {
    pub symbol: String,
    pub inst_id: String,
    pub inst_type: String,
    pub side: String,
    pub quantity: f64,
    pub entry_price: f64,
    pub mark_price: f64,
    pub unrealized_pnl: f64,
    pub leverage: f64,
    pub liquidation_price: Option<f64>,
    pub margin: f64,
    pub initial_margin: f64,
    pub maintenance_margin_ratio: f64,
    pub updated_time_ms: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OkxOrderUpdate {
    pub client_order_id: String,
    pub live_state: LiveOrderState,
    pub updated_time_ms: i64,
    pub fill: Option<OkxOrderFillUpdate>,
    pub order: OrderInfo,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OkxOrderFillUpdate {
    pub trade_id: String,
    pub fill_price: f64,
    pub fill_size: f64,
    pub fill_fee: Option<f64>,
    pub fill_fee_currency: Option<String>,
    pub fill_time_ms: i64,
}

pub fn login_payload(cfg: OkxUserWsConfig<'_>) -> String {
    serde_json::to_string(&login_request(cfg)).unwrap_or_else(|_| "{}".to_owned())
}

pub fn subscribe_account_payload(id: &str, currency: Option<&str>) -> String {
    subscription_payload(id, &[account_arg(currency)])
}

pub fn subscribe_positions_payload(id: &str) -> String {
    subscription_payload(id, &[inst_type_arg("positions")])
}

pub fn subscribe_orders_payload(id: &str) -> String {
    subscription_payload(id, &[inst_type_arg("orders")])
}

pub fn parse_user_event(text: &str) -> ExchangeResult<Option<OkxUserEvent>> {
    okx_ws_user_data::parse_user_event(text)
}

pub fn parse_user_control(text: &str) -> ExchangeResult<Option<OkxUserControl>> {
    let envelope: ControlEnvelope = serde_json::from_str(text).map_err(|error| {
        ExchangeError::Parse(format!("okx user ws control: {error}; body={text}"))
    })?;
    let event = envelope.event.trim();
    if !matches!(event, OP_LOGIN | OP_SUBSCRIBE | "error") {
        return Ok(None);
    }

    let request_id = clean_text(&envelope.id);
    let code = envelope.code.trim();
    let authentication_failed = event == OP_LOGIN || code == "60009";
    let channel = clean_text(&envelope.arg.channel)
        .or_else(|| request_id.clone())
        .or_else(|| authentication_failed.then(|| OP_LOGIN.to_owned()))
        .ok_or_else(|| {
            ExchangeError::Parse(format!(
                "okx user ws control missing channel/id: event={event}; body={text}"
            ))
        })?;

    if event == OP_LOGIN && code == "0" {
        return Ok(Some(OkxUserControl::Acknowledged {
            channel: OP_LOGIN.to_owned(),
            request_id,
        }));
    }
    if event == OP_SUBSCRIBE && (code.is_empty() || code == "0") {
        return Ok(Some(OkxUserControl::Acknowledged {
            channel,
            request_id,
        }));
    }

    let message = envelope.msg.trim();
    let error = if message.is_empty() {
        format!("code={code}; event={event}")
    } else {
        format!("code={code}; message={message}")
    };
    Ok(Some(OkxUserControl::Rejected {
        channel,
        request_id,
        authentication_failed,
        error,
    }))
}

fn clean_text(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn login_request(cfg: OkxUserWsConfig<'_>) -> LoginRequest {
    let now = chrono::Utc::now();
    let timestamp = format!("{}.{:03}", now.timestamp(), now.timestamp_subsec_millis());
    let sign = sign::sign(cfg.api_secret.as_bytes(), &timestamp, "GET", LOGIN_PATH, "");
    LoginRequest {
        op: OP_LOGIN,
        args: [LoginArg {
            api_key: cfg.api_key.to_owned(),
            passphrase: cfg.passphrase.to_owned(),
            timestamp,
            sign,
        }],
    }
}

fn subscription_payload(id: &str, args: &[Value]) -> String {
    json!({
        "id": id,
        "op": OP_SUBSCRIBE,
        "args": args
    })
    .to_string()
}

fn account_arg(currency: Option<&str>) -> Value {
    match currency {
        Some(ccy) if !ccy.trim().is_empty() => json!({
            "channel": "account",
            "ccy": ccy.trim().to_ascii_uppercase()
        }),
        _ => json!({ "channel": "account" }),
    }
}

fn inst_type_arg(channel: &str) -> Value {
    json!({
        "channel": channel,
        "instType": "ANY"
    })
}

#[derive(Debug, Serialize)]
struct LoginRequest {
    op: &'static str,
    args: [LoginArg; 1],
}

#[derive(Debug, Serialize)]
struct LoginArg {
    #[serde(rename = "apiKey")]
    api_key: String,
    passphrase: String,
    timestamp: String,
    sign: String,
}

#[derive(Debug, Default, Deserialize)]
struct ControlEnvelope {
    #[serde(default)]
    id: String,
    #[serde(default)]
    event: String,
    #[serde(default)]
    code: String,
    #[serde(default)]
    msg: String,
    #[serde(default)]
    arg: ControlArg,
}

#[derive(Debug, Default, Deserialize)]
struct ControlArg {
    #[serde(default)]
    channel: String,
}

#[cfg(test)]
#[path = "okx_ws_user_tests.rs"]
mod tests;
