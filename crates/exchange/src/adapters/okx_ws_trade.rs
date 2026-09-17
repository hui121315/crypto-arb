//! OKX WebSocket private trading API.
//!
//! Official docs:
//! - WebSocket login/order/cancel: <https://www.okx.com/docs-v5/en/>
//! - REST/WS signature basis: <https://www.okx.com/docs-v5/en/#rest-api-authentication-signature>

use super::okx_instruments::OkxOrderSizing;
use super::okx_live_config::OkxTdMode;
use super::okx_trade_data::{
    ack_from_item, cancel_order_ws_arg, cancel_spot_order_ws_arg, place_order_ws_arg,
    place_spot_order_ws_arg, OkxPositionMode, OrderAckItem,
};
use crate::error::{ExchangeError, ExchangeResult};
use crate::signing::okx as sign;
use crate::ws::manager::WsHeartbeat;
use crate::ws::trade_session::{session_key, WsLoginSpec, WsSessionSpec, WsTradeSession};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use shared_types::{CancelOrderRequest, OrderAck, OrderIntent};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

const EXCHANGE: &str = "okx";
const OP_LOGIN: &str = "login";
const OP_ORDER: &str = "order";
const OP_CANCEL_ORDER: &str = "cancel-order";
const LOGIN_PATH: &str = "/users/self/verify";
static WS_REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

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
    inst_id_code: u64,
    td_mode: OkxTdMode,
    position_mode: OkxPositionMode,
    sizing: OkxOrderSizing,
) -> ExchangeResult<OrderAck> {
    let request = order_request(
        &next_request_id(),
        OP_ORDER,
        place_order_ws_arg(intent, inst_id_code, td_mode, position_mode, sizing)?,
    );
    let item = send_trade_request(cfg, request).await?;
    Ok(ack_from_item(
        intent.id.clone(),
        intent.client_order_id.clone(),
        item,
    ))
}

pub(super) async fn cancel_order(
    cfg: WsTradeConfig<'_>,
    request: &CancelOrderRequest,
    inst_id_code: u64,
) -> ExchangeResult<OrderAck> {
    let ws_request = order_request(
        &next_request_id(),
        OP_CANCEL_ORDER,
        cancel_order_ws_arg(request, inst_id_code)?,
    );
    let item = send_trade_request(cfg, ws_request).await?;
    Ok(ack_from_item(
        request.internal_order_id.clone(),
        request.client_order_id.clone(),
        item,
    ))
}

pub(super) async fn place_spot_order(
    cfg: WsTradeConfig<'_>,
    intent: &OrderIntent,
    inst_id: String,
    quantity: String,
    price: Option<String>,
) -> ExchangeResult<OrderAck> {
    let request = order_request(
        &next_request_id(),
        OP_ORDER,
        place_spot_order_ws_arg(intent, inst_id, quantity, price)?,
    );
    let item = send_trade_request(cfg, request).await?;
    Ok(ack_from_item(
        intent.id.clone(),
        intent.client_order_id.clone(),
        item,
    ))
}

pub(super) async fn cancel_spot_order(
    cfg: WsTradeConfig<'_>,
    request: &CancelOrderRequest,
    inst_id: String,
) -> ExchangeResult<OrderAck> {
    let ws_request = order_request(
        &next_request_id(),
        OP_CANCEL_ORDER,
        cancel_spot_order_ws_arg(request, inst_id)?,
    );
    let item = send_trade_request(cfg, ws_request).await?;
    Ok(ack_from_item(
        request.internal_order_id.clone(),
        request.client_order_id.clone(),
        item,
    ))
}

async fn send_trade_request(
    cfg: WsTradeConfig<'_>,
    request: WsOpRequest,
) -> ExchangeResult<OrderAckItem> {
    let request_id = request.id.clone();
    let payload = serde_json::to_string(&request)
        .map_err(|error| ExchangeError::Parse(format!("okx ws trade request: {error}")))?;
    let text = session(cfg)
        .send(
            payload,
            Box::new(move |text| Ok(parse_op_response(text)?.id == request_id)),
        )
        .await?;
    parse_op_response(&text)?.into_item()
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
            .with_heartbeat(okx_trade_heartbeat())
            .with_heartbeat_response(okx_trade_heartbeat_response)
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
                        ExchangeError::Parse(format!("okx ws login request: {error}"))
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

fn okx_trade_heartbeat() -> WsHeartbeat {
    WsHeartbeat::Text("ping".to_owned())
}

fn okx_trade_heartbeat_response(text: &str) -> bool {
    text.trim() == "pong"
}

fn login_request(cfg: WsTradeConfig<'_>) -> WsLoginRequest {
    // 修复 P1 9.4：OKX V5 WS login timestamp 官方推荐 "Unix Epoch seconds with milliseconds"
    // 浮点格式（如 `1538054050.975`）。整数秒虽被服务器容忍，但毫秒精度可有效避开 30s
    // skew check 边界并跟其他 V5 客户端示例对齐。
    // 文档：<https://www.okx.com/docs-v5/en/#overview-websocket-login>
    let now = chrono::Utc::now();
    let timestamp = format!("{}.{:03}", now.timestamp(), now.timestamp_subsec_millis());
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

fn order_request(id: &str, op: &'static str, arg: Value) -> WsOpRequest {
    WsOpRequest {
        id: id.to_owned(),
        op,
        args: [arg],
    }
}

/// OKX WS operation IDs are transport correlation IDs, not order identities.
/// Officially they must be 1-32 case-sensitive alphanumeric characters.
fn next_request_id() -> String {
    let timestamp_ms = common::time::now_ms().max(0) as u64;
    let sequence = WS_REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("{timestamp_ms:016x}{sequence:016x}")
}

fn parse_login_response(text: &str) -> ExchangeResult<WsLoginResponse> {
    serde_json::from_str(text).map_err(|error| {
        ExchangeError::Parse(format!("okx ws login response: {error}; body={text}"))
    })
}

fn parse_op_response(text: &str) -> ExchangeResult<WsOpResponse> {
    serde_json::from_str(text)
        .map_err(|error| ExchangeError::Parse(format!("okx ws op response: {error}; body={text}")))
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

#[derive(Debug, Serialize)]
struct WsOpRequest {
    id: String,
    op: &'static str,
    args: [Value; 1],
}

#[derive(Debug, Deserialize)]
struct WsLoginResponse {
    event: Option<String>,
    #[serde(default)]
    code: String,
    #[serde(default)]
    msg: String,
}

impl WsLoginResponse {
    fn into_result(self) -> ExchangeResult<()> {
        if self.code == "0" {
            return Ok(());
        }
        Err(ExchangeError::Api {
            exchange: EXCHANGE.into(),
            code: self.code,
            message: self.msg,
        })
    }
}

#[derive(Debug, Deserialize)]
struct WsOpResponse {
    #[serde(default)]
    id: String,
    code: String,
    #[serde(default)]
    msg: String,
    #[serde(default = "Vec::new")]
    data: Vec<OrderAckItem>,
}

impl WsOpResponse {
    fn into_item(mut self) -> ExchangeResult<OrderAckItem> {
        if self.code != "0" {
            return Err(ExchangeError::Api {
                exchange: EXCHANGE.into(),
                code: self.code,
                message: self.msg,
            });
        }
        self.data
            .pop()
            .ok_or_else(|| ExchangeError::Parse("okx ws missing data".into()))
    }
}

#[cfg(test)]
#[path = "okx_ws_trade_tests.rs"]
mod tests;
