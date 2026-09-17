//! Bitget V3 / UTA private WebSocket payloads + event parser entry-point.
//!
//! V3 differences vs the V2 sibling (`bitget_ws_user.rs`):
//! - URL → `wss://ws.bitget.com/v3/ws/private`.
//! - Topic names are singular: `account / order / position / fill` (V2 used
//!   `account / orders / positions` plural).
//! - All private-data topics are UTA-global and use exactly
//!   `{instType:"UTA",topic}`. Product-scoped V2 wildcards are not valid V3
//!   subscription arguments.
//! - Login payload is unchanged: `{op:"login", args:[{apiKey, passphrase,
//!   timestamp, sign}]}` with sign = `timestamp + "GET" + "/user/verify"`
//!   HMAC-SHA256 base64 of `api_secret`.
//!
//! The private subscription wires account/order/position/fill. Fill uses the
//! official UTA-global topic shape, so fills from spot/margin/futures can be
//! parsed and then filtered by downstream order ids.
//!
//! Official docs:
//! - WebSocket intro (login + heartbeat): <https://www.bitget.com/api-doc/uta/websocket/Intro>
//! - Account topic: <https://www.bitget.com/api-doc/uta/websocket/private/Account-Channel>
//! - Order topic: <https://www.bitget.com/api-doc/uta/websocket/private/Order-Channel>
//! - Position topic: <https://www.bitget.com/api-doc/uta/websocket/private/Positions-Channel>
//! - Fill topic: <https://www.bitget.com/api-doc/uta/websocket/private/Fill-Channel>

use super::bitget_uta_config::PROD_WS_PRIVATE;
use super::bitget_uta_ws_user_data;
use crate::error::{ExchangeError, ExchangeResult};
use crate::signing::bitget as sign;
use common::time::now_ms;
use serde::{Deserialize, Serialize};
use shared_types::{LiveOrderState, OrderInfo};

pub const BITGET_PRIVATE_WS_URL: &str = PROD_WS_PRIVATE;

pub(super) const EXCHANGE: &str = "bitget";
const OP_LOGIN: &str = "login";
const OP_SUBSCRIBE: &str = "subscribe";
const LOGIN_PATH: &str = "/user/verify";

const INST_TYPE_UTA: &str = "UTA";

pub(super) const TOPIC_ACCOUNT: &str = "account";
pub(super) const TOPIC_ORDER: &str = "order";
pub(super) const TOPIC_POSITION: &str = "position";
pub(super) const TOPIC_FILL: &str = "fill";

#[derive(Debug, Clone, Copy)]
pub struct BitgetUserWsConfig<'a> {
    pub api_key: &'a str,
    pub api_secret: &'a str,
    pub passphrase: &'a str,
}

/// Parsed private events emitted by the V3 / UTA private stream.
///
/// Variants intentionally share names with the V2 enum so the trading-service
/// mapper can be flipped to this module by changing only the `use` path.
#[derive(Debug, Clone, PartialEq)]
pub enum BitgetUserEvent {
    Account(BitgetAccountUpdate),
    Order(Vec<BitgetOrderUpdate>),
    Position(BitgetPositionUpdate),
    Fill(Vec<BitgetFillUpdate>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BitgetUserControl {
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

/// Bitget V3 / UTA `account` topic envelope. `data[0]` is the aggregate UTA
/// account state and its nested `coin[]` rows.
/// <https://www.bitget.com/api-doc/uta/websocket/private/Account-Channel>
#[derive(Debug, Clone, PartialEq)]
pub struct BitgetAccountUpdate {
    pub action: String,
    pub total_equity: f64,
    pub effective_equity: f64,
    pub initial_margin: f64,
    pub maintenance_margin: f64,
    pub margin_ratio: f64,
    pub position_margin_ratio: f64,
    pub unrealized_pnl: f64,
    pub accounts: Vec<BitgetAccountDelta>,
}

/// Bitget V3 / UTA `position` topic envelope. The global stream covers every
/// UTA futures category and preserves the native margin coin.
/// <https://www.bitget.com/api-doc/uta/websocket/private/Positions-Channel>
#[derive(Debug, Clone, PartialEq)]
pub struct BitgetPositionUpdate {
    pub action: String,
    pub positions: Vec<BitgetPositionDelta>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BitgetAccountDelta {
    pub coin: String,
    pub frozen: f64,
    pub available: f64,
    pub equity: f64,
    pub usdt_equity: f64,
    pub unrealized_pnl: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BitgetOrderUpdate {
    pub client_order_id: String,
    pub live_state: LiveOrderState,
    pub category: String,
    pub hold_mode: String,
    pub hold_side: String,
    pub trade_side: String,
    pub updated_time_ms: i64,
    pub cancel_reason: Option<String>,
    pub total_profit: f64,
    pub order: OrderInfo,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BitgetPositionDelta {
    pub symbol: String,
    pub margin_coin: String,
    pub margin_size: f64,
    pub margin_mode: String,
    pub hold_mode: String,
    pub position_status: String,
    pub side: String,
    pub size: f64,
    pub available: f64,
    pub frozen: f64,
    pub entry_price: f64,
    pub leverage: f64,
    pub unrealized_pnl: f64,
    pub liquidation_price: Option<f64>,
    pub maintenance_margin_rate: f64,
    pub mark_price: f64,
    pub updated_time_ms: i64,
}

/// Trade fill row from the V3 `fill` topic.
#[derive(Debug, Clone, PartialEq)]
pub struct BitgetFillUpdate {
    pub order_id: String,
    pub client_order_id: String,
    pub exec_id: String,
    pub category: String,
    pub symbol: String,
    pub side: String,
    pub hold_side: String,
    pub trade_side: String,
    pub price: f64,
    pub size: f64,
    pub value: f64,
    pub realized_pnl: f64,
    pub fee: f64,
    pub fee_currency: Option<String>,
    pub trade_time_ms: i64,
    pub updated_time_ms: i64,
    pub is_rpi: Option<bool>,
}

pub fn login_payload(cfg: BitgetUserWsConfig<'_>) -> ExchangeResult<String> {
    serde_json::to_string(&login_request(cfg))
        .map_err(|error| ExchangeError::Parse(format!("bitget uta ws login payload: {error}")))
}

/// Subscribe to the private topics needed for balance, position, order and fill
/// updates.
pub fn subscribe_private_payload() -> ExchangeResult<String> {
    subscribe_payload(&[
        SubscriptionArg::account(),
        SubscriptionArg::order(),
        SubscriptionArg::position(),
        SubscriptionArg::fill(),
    ])
}

pub fn subscribe_payload(args: &[SubscriptionArg]) -> ExchangeResult<String> {
    if args.is_empty() {
        return Err(ExchangeError::Parse(
            "bitget uta ws subscribe requires at least one topic".into(),
        ));
    }
    let request = SubscribeRequest {
        op: OP_SUBSCRIBE,
        args: args.to_vec(),
    };
    serde_json::to_string(&request)
        .map_err(|error| ExchangeError::Parse(format!("bitget uta ws subscribe payload: {error}")))
}

pub fn parse_user_event(text: &str) -> ExchangeResult<Option<BitgetUserEvent>> {
    bitget_uta_ws_user_data::parse_user_event(text)
}

pub fn parse_user_control(text: &str) -> ExchangeResult<Option<BitgetUserControl>> {
    if is_heartbeat_pong(text) {
        return Ok(None);
    }
    let envelope: ControlEnvelope = serde_json::from_str(text).map_err(|error| {
        ExchangeError::Parse(format!("bitget uta user ws control: {error}; body={text}"))
    })?;
    let event = envelope.event.trim();
    if !matches!(event, OP_LOGIN | OP_SUBSCRIBE | "error") {
        return Ok(None);
    }

    let code = envelope.code.as_text();
    let request_id = clean_text(&envelope.id);
    let authentication_failed = event == OP_LOGIN || code == "30005";
    let channel = clean_text(&envelope.arg.topic)
        .or_else(|| authentication_failed.then(|| OP_LOGIN.to_owned()))
        .or_else(|| request_id.clone())
        .unwrap_or_else(|| event.to_owned());
    let acknowledged = (event == OP_LOGIN && (code.is_empty() || code == "0"))
        || (event == OP_SUBSCRIBE && (code.is_empty() || code == "0"));
    if acknowledged {
        return Ok(Some(BitgetUserControl::Acknowledged {
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
    Ok(Some(BitgetUserControl::Rejected {
        channel,
        request_id,
        authentication_failed,
        error,
    }))
}

pub(super) fn is_heartbeat_pong(text: &str) -> bool {
    text.trim().eq_ignore_ascii_case("pong")
}

fn clean_text(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn login_request(cfg: BitgetUserWsConfig<'_>) -> LoginRequest {
    let timestamp = now_ms().to_string();
    LoginRequest {
        op: OP_LOGIN,
        args: [LoginArg {
            api_key: cfg.api_key.to_owned(),
            passphrase: cfg.passphrase.to_owned(),
            timestamp: timestamp.clone(),
            sign: sign::sign(cfg.api_secret.as_bytes(), &timestamp, "GET", LOGIN_PATH, ""),
        }],
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SubscriptionArg {
    #[serde(rename = "instType")]
    inst_type: &'static str,
    topic: &'static str,
}

impl SubscriptionArg {
    pub fn account() -> Self {
        Self {
            inst_type: INST_TYPE_UTA,
            topic: TOPIC_ACCOUNT,
        }
    }

    pub fn order() -> Self {
        Self {
            inst_type: INST_TYPE_UTA,
            topic: TOPIC_ORDER,
        }
    }

    pub fn position() -> Self {
        Self {
            inst_type: INST_TYPE_UTA,
            topic: TOPIC_POSITION,
        }
    }

    pub fn fill() -> Self {
        Self {
            inst_type: INST_TYPE_UTA,
            topic: TOPIC_FILL,
        }
    }
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

#[derive(Debug, Serialize)]
struct SubscribeRequest {
    op: &'static str,
    args: Vec<SubscriptionArg>,
}

#[derive(Debug, Default, Deserialize)]
struct ControlEnvelope {
    #[serde(default)]
    event: String,
    #[serde(default)]
    code: ControlCode,
    #[serde(default)]
    msg: String,
    #[serde(default)]
    id: String,
    #[serde(default)]
    arg: ControlArg,
}

#[derive(Debug, Default, Deserialize)]
struct ControlArg {
    #[serde(default)]
    topic: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(untagged)]
enum ControlCode {
    #[default]
    Missing,
    Text(String),
    Number(i64),
}

impl ControlCode {
    fn as_text(&self) -> String {
        match self {
            Self::Missing => String::new(),
            Self::Text(value) => value.clone(),
            Self::Number(value) => value.to_string(),
        }
    }
}

#[cfg(test)]
#[path = "bitget_uta_ws_user_tests.rs"]
mod tests;
