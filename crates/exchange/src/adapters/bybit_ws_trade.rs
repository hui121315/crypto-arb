//! Bybit v5 WebSocket Trade API.
//!
//! Official docs:
//! - Trade guideline: <https://bybit-exchange.github.io/docs/v5/websocket/trade/guideline>
//! - Order create fields: <https://bybit-exchange.github.io/docs/v5/order/create-order>
//! - WebSocket connect/auth: <https://bybit-exchange.github.io/docs/v5/ws/connect>

use super::bybit_trade_data::{
    bybit_order_link_id, bybit_ws_req_id, cancel_order_arg, cancel_spot_order_arg, place_order_arg,
    place_spot_order_arg, validate_ws_req_id,
};
use crate::error::{ExchangeError, ExchangeResult};
use crate::signing::bybit as sign;
use crate::ws::manager::WsHeartbeat;
use crate::ws::trade_session::{session_key, WsLoginSpec, WsSessionSpec, WsTradeSession};
use common::time::now_ms;
use dashmap::DashMap;
use serde::Deserialize;
use serde_json::{json, Value};
use shared_types::{
    CancelOrderRequest, LiveOrderState, OrderAck, OrderIntent, VenueOrderIdentityUpdate,
};
use std::sync::OnceLock;
use std::time::Duration;

const EXCHANGE: &str = "bybit";
const OP_AUTH: &str = "auth";
const OP_ORDER_CREATE: &str = "order.create";
const OP_ORDER_CANCEL: &str = "order.cancel";

#[derive(Debug, Clone, Copy)]
pub(super) struct WsTradeConfig<'a> {
    pub url: &'a str,
    pub api_key: &'a str,
    pub api_secret: &'a str,
    pub recv_window: &'a str,
    pub timeout_secs: u64,
}

pub(super) async fn place_order(
    cfg: WsTradeConfig<'_>,
    intent: &OrderIntent,
    symbol: String,
    position_idx: u8,
) -> ExchangeResult<OrderAck> {
    let venue_client_order_id = bybit_order_link_id(&intent.client_order_id)?;
    let request = order_create_request(cfg, intent, symbol, position_idx)?;
    let result = send_trade_request(cfg, request).await?;
    Ok(ack_from_result(
        intent.id.clone(),
        intent.client_order_id.clone(),
        venue_client_order_id,
        result,
        LiveOrderState::Accepted,
        None,
    ))
}

pub(super) async fn cancel_order(
    cfg: WsTradeConfig<'_>,
    request: &CancelOrderRequest,
    symbol: String,
) -> ExchangeResult<OrderAck> {
    let venue_client_order_id = bybit_order_link_id(&request.client_order_id)?;
    let ws_request = order_cancel_request(cfg, request, symbol)?;
    let result = send_trade_request(cfg, ws_request).await?;
    Ok(ack_from_result(
        request.internal_order_id.clone(),
        request.client_order_id.clone(),
        venue_client_order_id,
        result,
        LiveOrderState::CancelRequested,
        Some("bybit cancel accepted; final state requires order query".to_owned()),
    ))
}

pub(super) async fn place_spot_order(
    cfg: WsTradeConfig<'_>,
    intent: &OrderIntent,
    symbol: String,
) -> ExchangeResult<OrderAck> {
    let venue_client_order_id = bybit_order_link_id(&intent.client_order_id)?;
    let arg = place_spot_order_arg(intent, symbol)?;
    let request = trade_request(
        &bybit_ws_req_id(&intent.id)?,
        cfg.recv_window,
        OP_ORDER_CREATE,
        arg,
    )?;
    let result = send_trade_request(cfg, request).await?;
    Ok(ack_from_result(
        intent.id.clone(),
        intent.client_order_id.clone(),
        venue_client_order_id,
        result,
        LiveOrderState::Accepted,
        None,
    ))
}

pub(super) async fn cancel_spot_order(
    cfg: WsTradeConfig<'_>,
    request: &CancelOrderRequest,
    symbol: String,
) -> ExchangeResult<OrderAck> {
    let venue_client_order_id = bybit_order_link_id(&request.client_order_id)?;
    let arg = cancel_spot_order_arg(request, symbol)?;
    let ws_request = trade_request(
        &bybit_ws_req_id(&request.internal_order_id)?,
        cfg.recv_window,
        OP_ORDER_CANCEL,
        arg,
    )?;
    let result = send_trade_request(cfg, ws_request).await?;
    Ok(ack_from_result(
        request.internal_order_id.clone(),
        request.client_order_id.clone(),
        venue_client_order_id,
        result,
        LiveOrderState::CancelRequested,
        Some("bybit spot cancel accepted; final state requires private order stream".to_owned()),
    ))
}

async fn send_trade_request(
    cfg: WsTradeConfig<'_>,
    request: WsTradeRequest,
) -> ExchangeResult<OrderAckRow> {
    let request_id = request.req_id.clone();
    let payload = serde_json::to_string(&request)
        .map_err(|error| ExchangeError::Parse(format!("bybit ws trade request: {error}")))?;
    let text = session(cfg)
        .send(
            payload,
            Box::new(move |text| Ok(parse_trade_response(text)?.req_id == request_id)),
        )
        .await?;
    parse_trade_response(&text)?.into_result()
}

fn session(cfg: WsTradeConfig<'_>) -> WsTradeSession {
    static SESSIONS: OnceLock<DashMap<String, WsTradeSession>> = OnceLock::new();
    let key = session_key(&[EXCHANGE, cfg.url, cfg.api_key, cfg.api_secret]);
    let spec = || {
        let api_key = cfg.api_key.to_owned();
        let api_secret = cfg.api_secret.to_owned();
        WsSessionSpec::new(cfg.url, cfg.timeout_secs)
            .with_heartbeat(bybit_trade_heartbeat())
            .with_heartbeat_interval(Duration::from_secs(20))
            .with_heartbeat_response(bybit_trade_heartbeat_response)
            .with_login(WsLoginSpec::new(
                move || {
                    let auth = auth_request(WsTradeConfig {
                        url: "",
                        api_key: &api_key,
                        api_secret: &api_secret,
                        recv_window: "",
                        timeout_secs: 0,
                    });
                    serde_json::to_string(&auth).map_err(|error| {
                        ExchangeError::Parse(format!("bybit ws auth request: {error}"))
                    })
                },
                |text| {
                    let response = parse_auth_response(text)?;
                    if response.op.as_deref() != Some(OP_AUTH) {
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

fn bybit_trade_heartbeat() -> WsHeartbeat {
    WsHeartbeat::Text(r#"{"op":"ping"}"#.to_owned())
}

fn bybit_trade_heartbeat_response(text: &str) -> bool {
    serde_json::from_str::<Value>(text)
        .ok()
        .is_some_and(|value| {
            let op = value.get("op").and_then(Value::as_str);
            op == Some("pong")
                || (op == Some("ping")
                    && value.get("ret_msg").and_then(Value::as_str) == Some("pong"))
        })
}

fn auth_request(cfg: WsTradeConfig<'_>) -> WsAuthRequest {
    let expires = (now_ms() + 5_000).to_string();
    let signature = sign::ws_auth_sign(cfg.api_secret.as_bytes(), &expires);
    WsAuthRequest {
        op: OP_AUTH,
        args: [cfg.api_key.to_owned(), expires, signature],
    }
}

fn order_create_request(
    cfg: WsTradeConfig<'_>,
    intent: &OrderIntent,
    symbol: String,
    position_idx: u8,
) -> ExchangeResult<WsTradeRequest> {
    let arg = place_order_arg(intent, symbol, position_idx)?;
    let req_id = bybit_ws_req_id(&intent.id)?;
    trade_request(&req_id, cfg.recv_window, OP_ORDER_CREATE, arg)
}

fn order_cancel_request(
    cfg: WsTradeConfig<'_>,
    request: &CancelOrderRequest,
    symbol: String,
) -> ExchangeResult<WsTradeRequest> {
    let arg = cancel_order_arg(request, symbol)?;
    let req_id = bybit_ws_req_id(&request.internal_order_id)?;
    trade_request(&req_id, cfg.recv_window, OP_ORDER_CANCEL, arg)
}

fn trade_request(
    req_id: &str,
    recv_window: &str,
    op: &'static str,
    arg: Value,
) -> ExchangeResult<WsTradeRequest> {
    validate_ws_req_id(req_id)?;
    Ok(WsTradeRequest {
        req_id: req_id.to_owned(),
        header: json!({
            "X-BAPI-TIMESTAMP": now_ms().to_string(),
            "X-BAPI-RECV-WINDOW": recv_window,
        }),
        op,
        args: [arg],
    })
}

fn parse_auth_response(text: &str) -> ExchangeResult<WsAuthResponse> {
    serde_json::from_str(text).map_err(|error| {
        ExchangeError::Parse(format!("bybit ws auth response: {error}; body={text}"))
    })
}

fn parse_trade_response(text: &str) -> ExchangeResult<WsTradeResponse> {
    serde_json::from_str(text).map_err(|error| {
        ExchangeError::Parse(format!("bybit ws trade response: {error}; body={text}"))
    })
}

fn ack_from_result(
    internal_order_id: String,
    public_client_order_id: String,
    venue_client_order_id: String,
    result: OrderAckRow,
    state: LiveOrderState,
    message: Option<String>,
) -> OrderAck {
    let exchange_order_id = non_empty(result.order_id);
    let venue_client_order_id = non_empty(result.order_link_id).unwrap_or(venue_client_order_id);
    OrderAck {
        internal_order_id,
        exchange_order_id: exchange_order_id.clone(),
        client_order_id: public_client_order_id.clone(),
        identity_update: VenueOrderIdentityUpdate::from_ids(
            public_client_order_id,
            venue_client_order_id,
            exchange_order_id,
        ),
        state,
        accepted_at_ms: now_ms(),
        message,
        filled_quantity: None,
        filled_price: None,
        filled_fee: None,
    }
}

fn non_empty(value: String) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

#[derive(Debug, serde::Serialize)]
struct WsAuthRequest {
    op: &'static str,
    args: [String; 3],
}

#[derive(Debug, serde::Serialize)]
struct WsTradeRequest {
    #[serde(rename = "reqId")]
    req_id: String,
    header: Value,
    op: &'static str,
    args: [Value; 1],
}

#[derive(Debug, Deserialize)]
struct WsAuthResponse {
    op: Option<String>,
    #[serde(default, rename = "retCode")]
    ret_code: Option<i64>,
    #[serde(default, rename = "retMsg")]
    ret_msg: Option<String>,
    #[serde(default, rename = "ret_msg")]
    ret_msg_alt: Option<String>,
    #[serde(default)]
    success: Option<bool>,
}

impl WsAuthResponse {
    fn into_result(self) -> ExchangeResult<()> {
        if self.ret_code == Some(0) || self.success == Some(true) {
            return Ok(());
        }
        Err(ExchangeError::Api {
            exchange: EXCHANGE.into(),
            code: self
                .ret_code
                .map(|code| code.to_string())
                .unwrap_or_else(|| "auth".to_owned()),
            message: self
                .ret_msg
                .or(self.ret_msg_alt)
                .unwrap_or_else(|| "authentication rejected".to_owned()),
        })
    }
}

#[derive(Debug, Deserialize)]
struct WsTradeResponse {
    #[serde(default, rename = "reqId")]
    req_id: String,
    #[serde(rename = "retCode")]
    ret_code: i64,
    #[serde(default, rename = "retMsg")]
    ret_msg: String,
    #[serde(default)]
    data: Option<OrderAckRow>,
}

impl WsTradeResponse {
    fn into_result(self) -> ExchangeResult<OrderAckRow> {
        if self.ret_code == 0 {
            return self
                .data
                .ok_or_else(|| ExchangeError::Parse("bybit ws missing data".into()));
        }
        Err(ExchangeError::Api {
            exchange: EXCHANGE.into(),
            code: self.ret_code.to_string(),
            message: self.ret_msg,
        })
    }
}

#[derive(Debug, Deserialize)]
struct OrderAckRow {
    #[serde(default, rename = "orderId")]
    order_id: String,
    #[serde(default, rename = "orderLinkId")]
    order_link_id: String,
}

#[cfg(test)]
#[path = "bybit_ws_trade_tests.rs"]
mod tests;
