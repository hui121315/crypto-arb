//! Bitget V3 / UTA WebSocket private trade API.
//!
//! V3 differences vs V2 (`bitget_ws_trade.rs`):
//! - URL → `wss://ws.bitget.com/v3/ws/private`.
//! - Request envelope is flat: `{op, id, category, topic, args:[<params>]}`
//!   instead of V2's `{op:"trade", args:[{id, instType, instId, channel, params}]}`.
//!   `category` (lower-case `usdt-futures`) and `topic`
//!   (`place-order` / `cancel-order`) live at the top level; the order body in
//!   `args[0]` no longer repeats them (see ccxt-bridge
//!   `WebsocketClientV3::sendWSAPIRequest` for the canonical shape).
//! - Login args are unchanged from V2 (`{apiKey, passphrase, timestamp, sign}`
//!   with `sign = HMAC-SHA256-base64(timestamp + "GET" + "/user/verify")`).
//! - Response envelope is flat: `{event:"trade", id, code, msg,
//!   args:[{orderId, clientOid}]}`. We also accept `data` for compatibility
//!   with older examples. `code` may be a string or number.
//!
//! Official docs:
//! - WS API intro (login + heartbeat): <https://www.bitget.com/api-doc/uta/websocket/Intro>
//! - Place order: <https://www.bitget.com/api-doc/uta/websocket/private/Place-Order-Channel>
//! - Cancel order: <https://www.bitget.com/api-doc/uta/websocket/private/Cancel-Order-Channel>

use super::bitget_uta_trade_data::{
    ack_from_row, cancel_order_params, place_order_params, IntoBitgetOrderContext, UtaOrderAckRow,
};
use crate::adapters::bitget_config::BitgetMarginMode;
use crate::adapters::bitget_order_compiler::CompiledBitgetOrder;
use crate::adapters::bitget_uta_config::BitgetUtaCategory;
use crate::error::{ExchangeError, ExchangeResult};
use crate::signing::bitget as sign;
use crate::ws::trade_session::{session_key, WsLoginSpec, WsSessionSpec, WsTradeSession};
use crate::ws::WsHeartbeat;
use common::time::now_ms;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use shared_types::{CancelOrderRequest, LiveOrderState, OrderAck, OrderIntent};
use std::sync::OnceLock;

const EXCHANGE: &str = "bitget";
const OP_LOGIN: &str = "login";
const OP_TRADE: &str = "trade";
const EVENT_ERROR: &str = "error";
const TOPIC_PLACE_ORDER: &str = "place-order";
const TOPIC_CANCEL_ORDER: &str = "cancel-order";
const LOGIN_PATH: &str = "/user/verify";
/// Bitget limits `id` to ~40 chars on the WS API (V2 docs §4.1 still applies
/// in V3). Longer caller ids are reversibly squashed to `bg-<sha256-prefix>`.
const MAX_REQUEST_ID_LEN: usize = 40;

#[derive(Debug, Clone, Copy)]
pub(super) struct WsTradeConfig<'a> {
    pub url: &'a str,
    pub api_key: &'a str,
    pub api_secret: &'a str,
    pub passphrase: &'a str,
    pub timeout_secs: u64,
}

pub(super) async fn place_order(
    cfg: WsTradeConfig<'_>,
    intent: &OrderIntent,
    compiled: &CompiledBitgetOrder,
    margin_mode: BitgetMarginMode,
) -> ExchangeResult<OrderAck> {
    let request = order_place_request(intent, compiled, margin_mode)?;
    let row = send_trade_request(cfg, request).await?;
    Ok(ack_from_row(
        intent.id.clone(),
        intent.client_order_id.clone(),
        row,
        LiveOrderState::Accepted,
        None,
    ))
}

pub(super) async fn cancel_order(
    cfg: WsTradeConfig<'_>,
    request: &CancelOrderRequest,
    category: BitgetUtaCategory,
    symbol: String,
) -> ExchangeResult<OrderAck> {
    let ws_request = order_cancel_request_for(request, category, symbol)?;
    let row = send_trade_request(cfg, ws_request).await?;
    Ok(ack_from_row(
        request.internal_order_id.clone(),
        request.client_order_id.clone(),
        row,
        LiveOrderState::CancelRequested,
        Some("bitget cancel accepted; final state requires order query".to_owned()),
    ))
}

async fn send_trade_request(
    cfg: WsTradeConfig<'_>,
    request: WsTradeRequest,
) -> ExchangeResult<UtaOrderAckRow> {
    let matcher = request_matcher(&request);
    let payload = serde_json::to_string(&request)
        .map_err(|error| ExchangeError::Parse(format!("bitget uta ws trade request: {error}")))?;
    let text = session(cfg)
        .send(
            payload,
            Box::new(move |text| {
                let response = parse_trade_response(text)?;
                Ok(response.matches_parts(&matcher))
            }),
        )
        .await?;
    parse_trade_response(&text)?.into_result()
}

fn session(cfg: WsTradeConfig<'_>) -> WsTradeSession {
    static SESSIONS: OnceLock<DashMap<String, WsTradeSession>> = OnceLock::new();
    let key = session_key(&[
        EXCHANGE,
        cfg.url,
        cfg.api_key,
        cfg.api_secret,
        cfg.passphrase,
    ]);
    let spec = || {
        let api_key = cfg.api_key.to_owned();
        let api_secret = cfg.api_secret.to_owned();
        let passphrase = cfg.passphrase.to_owned();
        WsSessionSpec::new(cfg.url, cfg.timeout_secs)
            .with_heartbeat(bitget_trade_heartbeat())
            .with_heartbeat_response(bitget_trade_heartbeat_response)
            .with_login(WsLoginSpec::new(
                move || {
                    let login = login_request(WsTradeConfig {
                        url: "",
                        api_key: &api_key,
                        api_secret: &api_secret,
                        passphrase: &passphrase,
                        timeout_secs: 0,
                    });
                    serde_json::to_string(&login).map_err(|error| {
                        ExchangeError::Parse(format!("bitget uta ws login request: {error}"))
                    })
                },
                |text| {
                    let response = parse_login_response(text)?;
                    if response.event.as_deref() != Some(OP_LOGIN) {
                        return Ok(false);
                    }
                    response.into_result()?;
                    Ok(true)
                },
            ))
    };
    SESSIONS
        .get_or_init(DashMap::new)
        .entry(key)
        .or_insert_with(|| WsTradeSession::spawn(spec()))
        .clone()
}

fn bitget_trade_heartbeat() -> WsHeartbeat {
    WsHeartbeat::Text("ping".to_owned())
}

fn bitget_trade_heartbeat_response(text: &str) -> bool {
    text.trim() == "pong"
}

fn login_request(cfg: WsTradeConfig<'_>) -> WsLoginRequest {
    let timestamp = now_ms().to_string();
    let signature = sign::sign(cfg.api_secret.as_bytes(), &timestamp, "GET", LOGIN_PATH, "");
    WsLoginRequest {
        op: OP_LOGIN,
        args: [WsLoginArg {
            api_key: cfg.api_key.to_owned(),
            passphrase: cfg.passphrase.to_owned(),
            timestamp,
            sign: signature,
        }],
    }
}

fn order_place_request<C>(
    intent: &OrderIntent,
    context: C,
    margin_mode: BitgetMarginMode,
) -> ExchangeResult<WsTradeRequest>
where
    C: IntoBitgetOrderContext,
{
    let id = checked_request_id(&intent.id)?;
    let compiled = context.into_context(intent);
    let params = place_order_params(intent, &compiled, margin_mode)?;
    Ok(trade_request(
        id,
        compiled.category,
        TOPIC_PLACE_ORDER,
        params,
    ))
}

fn order_cancel_request_for(
    request: &CancelOrderRequest,
    category: BitgetUtaCategory,
    _symbol: String,
) -> ExchangeResult<WsTradeRequest> {
    let id = checked_request_id(&request.internal_order_id)?;
    let params = cancel_order_params(request)?;
    Ok(trade_request(
        id,
        category,
        TOPIC_CANCEL_ORDER,
        Value::Object(params),
    ))
}

#[cfg(test)]
fn order_cancel_request(
    request: &CancelOrderRequest,
    symbol: String,
) -> ExchangeResult<WsTradeRequest> {
    order_cancel_request_for(request, BitgetUtaCategory::UsdtFutures, symbol)
}

fn checked_request_id(id: &str) -> ExchangeResult<String> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return Err(ExchangeError::Api {
            exchange: EXCHANGE.into(),
            code: "validation".into(),
            message: "bitget uta ws request id must be non-empty".to_owned(),
        });
    }
    if trimmed.len() <= MAX_REQUEST_ID_LEN {
        return Ok(trimmed.to_owned());
    }
    let digest = Sha256::digest(trimmed.as_bytes());
    Ok(format!("bg-{}", hex::encode(&digest[..8])))
}

fn trade_request(
    id: String,
    category: BitgetUtaCategory,
    topic: &'static str,
    params: Value,
) -> WsTradeRequest {
    WsTradeRequest {
        op: OP_TRADE,
        id,
        category: category.as_ws_inst_type(),
        topic,
        args: [params],
    }
}

fn request_matcher(request: &WsTradeRequest) -> WsRequestMatcher {
    WsRequestMatcher {
        id: request.id.clone(),
        topic: request.topic,
    }
}

fn parse_login_response(text: &str) -> ExchangeResult<WsLoginResponse> {
    serde_json::from_str(text).map_err(|error| {
        ExchangeError::Parse(format!(
            "bitget uta ws login response: {error}; body={text}"
        ))
    })
}

fn parse_trade_response(text: &str) -> ExchangeResult<WsTradeResponse> {
    serde_json::from_str(text).map_err(|error| {
        ExchangeError::Parse(format!(
            "bitget uta ws trade response: {error}; body={text}"
        ))
    })
}

#[derive(Debug, Serialize)]
struct WsLoginRequest {
    op: &'static str,
    args: [WsLoginArg; 1],
}

#[derive(Debug, Serialize)]
struct WsLoginArg {
    #[serde(rename = "apiKey")]
    api_key: String,
    passphrase: String,
    timestamp: String,
    sign: String,
}

/// V3 trade envelope: `op`, `id`, `category`, `topic`, `args[<params>]`.
#[derive(Debug, Serialize)]
struct WsTradeRequest {
    op: &'static str,
    id: String,
    category: &'static str,
    topic: &'static str,
    args: [Value; 1],
}

#[derive(Debug, Deserialize)]
struct WsLoginResponse {
    event: Option<String>,
    #[serde(default)]
    code: Value,
    #[serde(default)]
    msg: String,
}

impl WsLoginResponse {
    fn into_result(self) -> ExchangeResult<()> {
        if code_is_success(&self.code) {
            return Ok(());
        }
        Err(ExchangeError::Api {
            exchange: EXCHANGE.into(),
            code: code_text(&self.code),
            message: self.msg,
        })
    }
}

/// V3 trade response envelope.
///
/// Compared to V2, `id` / `topic` are top-level instead of nested under
/// `arg[]`, and the official order ack payload sits in top-level `args`.
/// We accept legacy `data` and `op` echoes for compatibility.
#[derive(Debug, Deserialize)]
struct WsTradeResponse {
    #[serde(default)]
    event: String,
    #[serde(default)]
    op: String,
    #[serde(default)]
    id: String,
    #[serde(default)]
    topic: String,
    #[serde(default)]
    code: Value,
    #[serde(default)]
    msg: String,
    #[serde(default)]
    args: Vec<Map<String, Value>>,
    #[serde(default)]
    data: Vec<Map<String, Value>>,
}

impl WsTradeResponse {
    fn matches_parts(&self, matcher: &WsRequestMatcher) -> bool {
        if self.id != matcher.id {
            return false;
        }
        if self.event == EVENT_ERROR {
            return self.topic.is_empty() || self.topic == matcher.topic;
        }
        let envelope_is_trade = self.event == OP_TRADE || self.op == OP_TRADE;
        envelope_is_trade && self.topic == matcher.topic
    }

    fn into_result(self) -> ExchangeResult<UtaOrderAckRow> {
        if code_is_success(&self.code) {
            let rows = if self.args.is_empty() {
                self.data
            } else {
                self.args
            };
            return rows
                .into_iter()
                .next()
                .map(|params| row_from_params(&params))
                .ok_or_else(|| ExchangeError::Parse("bitget uta ws missing ack row".into()));
        }
        Err(ExchangeError::Api {
            exchange: EXCHANGE.into(),
            code: code_text(&self.code),
            message: self.msg,
        })
    }
}

struct WsRequestMatcher {
    id: String,
    topic: &'static str,
}

fn row_from_params(params: &Map<String, Value>) -> UtaOrderAckRow {
    UtaOrderAckRow {
        order_id: string_field(params, "orderId"),
        client_oid: string_field(params, "clientOid"),
    }
}

fn string_field(params: &Map<String, Value>, key: &str) -> String {
    params
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn code_is_success(value: &Value) -> bool {
    match value {
        Value::String(code) => code == "0" || code == "00000",
        Value::Number(code) => code.as_i64() == Some(0),
        _ => false,
    }
}

fn code_text(value: &Value) -> String {
    match value {
        Value::String(code) => code.clone(),
        Value::Number(code) => code.to_string(),
        _ => "unknown".to_owned(),
    }
}

#[cfg(test)]
#[path = "bitget_uta_ws_trade_tests.rs"]
mod tests;
