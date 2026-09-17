//! Gate futures WebSocket API trading.
//!
//! Official docs:
//! - Futures WebSocket API: <https://www.gate.com/docs/developers/futures/ws/en/>
//! - Futures REST contract/order fields: <https://www.gate.com/docs/developers/apiv4/en/#futures>

use super::gate_private_data::{parse_open_order, OpenOrderItem};
use super::gate_trade_data::{ack_from_row, gate_text, GateOrderAckRow};
use crate::error::{ExchangeError, ExchangeResult};
use crate::signing::gate as sign;
use crate::ws::trade_session::{session_key, WsLoginSpec, WsSessionSpec, WsTradeSession};
use common::time::{now_ms, now_secs};
use dashmap::DashMap;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Map, Value};
use shared_types::{
    CancelOrderRequest, LiveOrderState, OrderAck, OrderInfo, OrderIntent, OrderStatus,
};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

const EXCHANGE: &str = "gate";
const EVENT_API: &str = "api";
const CHANNEL_LOGIN: &str = "futures.login";
const CHANNEL_ORDER_PLACE: &str = "futures.order_place";
const CHANNEL_ORDER_CANCEL: &str = "futures.order_cancel";
const CHANNEL_ORDER_STATUS: &str = "futures.order_status";
const CHANNEL_ORDER_LIST: &str = "futures.order_list";
const GATE_SIZE_DECIMAL_HEADER: &str = "X-Gate-Size-Decimal";

#[derive(Debug, Clone, Copy)]
pub(super) struct WsTradeConfig<'a> {
    pub url: &'a str,
    pub api_key: &'a str,
    pub api_secret: &'a str,
    pub timeout_secs: u64,
    /// 修复 P2 5.9：本地时钟到 Gate 服务时间的偏移（秒），由 `Gate::sync_server_time` 维护。
    pub time_offset_secs: i64,
}

pub(super) async fn place_order(
    cfg: WsTradeConfig<'_>,
    intent: &OrderIntent,
    params: Value,
) -> ExchangeResult<OrderAck> {
    let request = trade_request(CHANNEL_ORDER_PLACE, &intent.id, params);
    let result: OpenOrderItem = send_typed_request(cfg, request).await?;
    place_ack_from_order(intent, &result)
}

pub(super) async fn cancel_order(
    cfg: WsTradeConfig<'_>,
    request: &CancelOrderRequest,
    params: Value,
) -> ExchangeResult<OrderAck> {
    let ws_request = trade_request(CHANNEL_ORDER_CANCEL, &request.internal_order_id, params);
    let result: OpenOrderItem = send_typed_request(cfg, ws_request).await?;
    cancel_ack_from_order(request, &result)
}

pub(super) async fn get_order(
    cfg: WsTradeConfig<'_>,
    order_id: &str,
) -> ExchangeResult<OpenOrderItem> {
    let request = trade_request(
        CHANNEL_ORDER_STATUS,
        &next_request_id("order-status"),
        json!({ "order_id": order_id }),
    );
    send_typed_request(cfg, request).await
}

pub(super) async fn get_open_orders(
    cfg: WsTradeConfig<'_>,
    contract: Option<&str>,
) -> ExchangeResult<Vec<OpenOrderItem>> {
    let mut params = Map::from_iter([("status".to_owned(), Value::String("open".to_owned()))]);
    if let Some(contract) = contract {
        params.insert("contract".to_owned(), Value::String(contract.to_owned()));
    }
    let request = trade_request(
        CHANNEL_ORDER_LIST,
        &next_request_id("order-list"),
        Value::Object(params),
    );
    send_typed_request(cfg, request).await
}

async fn send_typed_request<T: DeserializeOwned>(
    cfg: WsTradeConfig<'_>,
    request: WsRequest,
) -> ExchangeResult<T> {
    send_request(cfg, request).await?.into_typed_result()
}

async fn send_request(cfg: WsTradeConfig<'_>, request: WsRequest) -> ExchangeResult<WsResponse> {
    let request_id = request.payload.req_id.clone();
    let payload = serde_json::to_string(&request)
        .map_err(|error| ExchangeError::Parse(format!("gate ws trade request: {error}")))?;
    let text = session(cfg)
        .send(
            payload,
            Box::new(move |text| Ok(parse_response(text)?.is_final_for(&request_id))),
        )
        .await?;
    parse_response(&text)
}

fn session(cfg: WsTradeConfig<'_>) -> WsTradeSession {
    static SESSIONS: OnceLock<DashMap<String, WsTradeSession>> = OnceLock::new();
    let offset = cfg.time_offset_secs.to_string();
    let key = session_key(&[EXCHANGE, cfg.url, cfg.api_key, cfg.api_secret, &offset]);
    let spec = || {
        let api_key = cfg.api_key.to_owned();
        let api_secret = cfg.api_secret.to_owned();
        let time_offset_secs = cfg.time_offset_secs;
        WsSessionSpec::new(cfg.url, cfg.timeout_secs)
            .with_header(GATE_SIZE_DECIMAL_HEADER, "1")
            .with_login(WsLoginSpec::new(
                move || {
                    let login = login_request(WsTradeConfig {
                        url: "",
                        api_key: &api_key,
                        api_secret: &api_secret,
                        timeout_secs: 0,
                        time_offset_secs,
                    });
                    serde_json::to_string(&login).map_err(|error| {
                        ExchangeError::Parse(format!("gate ws login request: {error}"))
                    })
                },
                |text| {
                    let response = parse_response(text)?;
                    if response.header.channel != CHANNEL_LOGIN
                        && !response.request_id.starts_with("login-")
                    {
                        return Ok(false);
                    }
                    if !response.is_final_for(&response.request_id) {
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

fn login_request(cfg: WsTradeConfig<'_>) -> WsRequest {
    // 修复 P2 5.9：用 server-local offset 补偿本地时钟漂移（与 REST 签名同步）。
    let time = now_secs().saturating_add(cfg.time_offset_secs);
    let req_id = format!("login-{time}");
    let signature = sign::ws_api_sign(
        cfg.api_secret.as_bytes(),
        EVENT_API,
        CHANNEL_LOGIN,
        "",
        &time.to_string(),
    );
    WsRequest {
        time,
        channel: CHANNEL_LOGIN,
        event: EVENT_API,
        payload: WsPayload {
            req_id,
            req_param: None,
            api_key: Some(cfg.api_key.to_owned()),
            timestamp: Some(time.to_string()),
            signature: Some(signature),
        },
    }
}

fn trade_request(channel: &'static str, req_id: &str, params: Value) -> WsRequest {
    WsRequest {
        time: now_secs(),
        channel,
        event: EVENT_API,
        payload: WsPayload {
            req_id: req_id.to_owned(),
            req_param: Some(params),
            api_key: None,
            timestamp: None,
            signature: None,
        },
    }
}

fn next_request_id(operation: &str) -> String {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let operation_code = match operation {
        "order-status" => "os",
        "order-list" => "ol",
        _ => "rq",
    };
    format!(
        "cl-{operation_code}-{:x}-{:x}",
        now_ms(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

fn parse_response(text: &str) -> ExchangeResult<WsResponse> {
    serde_json::from_str(text).map_err(|error| {
        ExchangeError::Parse(format!("gate ws trade response: {error}; body={text}"))
    })
}

fn ack_from_result(
    internal_order_id: String,
    public_client_order_id: String,
    venue_client_order_id: String,
    result: &GateOrderAckRow,
    state: LiveOrderState,
    message: Option<String>,
) -> OrderAck {
    ack_from_row(
        internal_order_id,
        public_client_order_id,
        venue_client_order_id,
        result,
        state,
        message,
    )
}

fn place_ack_from_order(intent: &OrderIntent, row: &OpenOrderItem) -> ExchangeResult<OrderAck> {
    let order = parse_open_order(row)?;
    let state = gate_live_state(order.status);
    let mut ack = ack_from_projected_order(
        intent.id.clone(),
        intent.client_order_id.clone(),
        &order,
        state,
        None,
    )?;
    if order.filled_quantity > 0.0 {
        let fill_ratio = order.filled_quantity / order.quantity;
        let filled_quantity = intent.quantity * fill_ratio;
        if !filled_quantity.is_finite() || !(0.0..=intent.quantity).contains(&filled_quantity) {
            return Err(ExchangeError::Parse(format!(
                "gate order {} produced invalid normalized fill quantity {filled_quantity}",
                order.order_id
            )));
        }
        ack.filled_quantity = Some(filled_quantity);
        ack.filled_price = (order.filled_price > 0.0).then_some(order.filled_price);
    }
    Ok(ack)
}

fn cancel_ack_from_order(
    request: &CancelOrderRequest,
    row: &OpenOrderItem,
) -> ExchangeResult<OrderAck> {
    let order = parse_open_order(row)?;
    let projected = gate_live_state(order.status);
    let (state, message) = match projected {
        LiveOrderState::Accepted | LiveOrderState::PartiallyFilled => (
            LiveOrderState::CancelRequested,
            Some("gate cancel response remains non-terminal; order query required".to_owned()),
        ),
        terminal => (terminal, None),
    };
    ack_from_projected_order(
        request.internal_order_id.clone(),
        request.client_order_id.clone(),
        &order,
        state,
        message,
    )
}

fn ack_from_projected_order(
    internal_order_id: String,
    public_client_order_id: String,
    order: &OrderInfo,
    state: LiveOrderState,
    message: Option<String>,
) -> ExchangeResult<OrderAck> {
    let venue_client_order_id = gate_text(&public_client_order_id)?;
    if order.client_order_id.as_deref() != Some(venue_client_order_id.as_str()) {
        return Err(ExchangeError::Parse(format!(
            "gate websocket order {} client id mismatch: expected {venue_client_order_id:?}, returned {:?}",
            order.order_id, order.client_order_id
        )));
    }
    let order_id = order.order_id.parse::<i64>().map_err(|_| {
        ExchangeError::Parse(format!(
            "gate websocket returned non-numeric order id {:?}",
            order.order_id
        ))
    })?;
    Ok(ack_from_result(
        internal_order_id,
        public_client_order_id,
        venue_client_order_id,
        &GateOrderAckRow {
            order_id: Some(order_id),
        },
        state,
        message,
    ))
}

fn gate_live_state(status: OrderStatus) -> LiveOrderState {
    match status {
        OrderStatus::Pending | OrderStatus::Open => LiveOrderState::Accepted,
        OrderStatus::PartiallyFilled => LiveOrderState::PartiallyFilled,
        OrderStatus::Filled => LiveOrderState::Filled,
        OrderStatus::Canceled => LiveOrderState::Cancelled,
        OrderStatus::Rejected => LiveOrderState::Rejected,
        OrderStatus::Expired => LiveOrderState::Failed,
    }
}

#[derive(Debug, Serialize)]
struct WsRequest {
    time: i64,
    channel: &'static str,
    event: &'static str,
    payload: WsPayload,
}

#[derive(Debug, Serialize)]
struct WsPayload {
    req_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    req_param: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timestamp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    signature: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WsResponse {
    #[serde(default, rename = "request_id")]
    request_id: String,
    #[serde(default)]
    ack: bool,
    #[serde(default)]
    header: WsHeader,
    #[serde(default)]
    data: WsData,
}

/// 修复 P2 9.9：解析 Gate WS 顶层 `header` 字段（`status` 是权威 final 信号）。
///
/// 文档：<https://www.gate.com/docs/developers/futures/ws/en/#api-message>
/// > 每个 API 响应都包含 `header.status` 字符串（如 "200" / "400" / "500"）。
/// > 客户端确认 final 应同时看 `header.status` 而不仅是 `ack=false`。
#[derive(Debug, Default, Deserialize)]
struct WsHeader {
    /// HTTP 风格状态码字符串（"200" = success final，4xx/5xx = error final）
    #[serde(default)]
    status: String,
    /// 频道名（如 `futures.order_place`）
    #[serde(default)]
    channel: String,
}

impl WsResponse {
    /// 修复 P2 9.9：终极版判定。
    ///
    /// 之前实现：`request_id == req_id && !ack` —— 漏掉 ack=true + errs 的错误路径，
    /// 客户端会永远超时。
    ///
    /// 现在三条任一即为 final：
    /// 1. `!ack`（原逻辑：明确的最终响应）
    /// 2. `data.errs` 非空（错误响应可能在 ack=true 阶段就返回）
    /// 3. `header.status` 非 "200"（HTTP 风格 4xx/5xx 错误）
    fn is_final_for(&self, req_id: &str) -> bool {
        if self.request_id != req_id {
            return false;
        }
        if !self.ack {
            return true;
        }
        if self.data.errs.is_some() {
            return true;
        }
        if !self.header.status.is_empty() && self.header.status != "200" {
            return true;
        }
        false
    }

    fn into_result(self) -> ExchangeResult<GateOrderAckRow> {
        let result: WsResult = self.into_typed_result()?;
        Ok(GateOrderAckRow {
            order_id: result.id,
        })
    }

    fn into_typed_result<T: DeserializeOwned>(self) -> ExchangeResult<T> {
        if let Some(error) = self.data.errs {
            return Err(ExchangeError::Api {
                exchange: EXCHANGE.into(),
                code: error.code.to_string(),
                message: error.message,
            });
        }
        // 修复 P2 9.9：若 errs 缺但 header.status 非 200，也归类为错误，避免静默通过。
        if !self.header.status.is_empty() && self.header.status != "200" {
            return Err(ExchangeError::Api {
                exchange: EXCHANGE.into(),
                code: self.header.status.clone(),
                message: format!(
                    "gate ws non-200 status without errs body; status={}",
                    self.header.status
                ),
            });
        }
        let result = self
            .data
            .result
            .ok_or_else(|| ExchangeError::Parse("gate ws missing result".into()))?;
        serde_json::from_value(result).map_err(|error| {
            ExchangeError::Parse(format!("gate ws result schema mismatch: {error}"))
        })
    }
}

#[derive(Debug, Default, Deserialize)]
struct WsData {
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    errs: Option<WsError>,
}

#[derive(Debug, Deserialize)]
struct WsResult {
    #[serde(default)]
    id: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct WsError {
    #[serde(default)]
    code: i64,
    #[serde(default)]
    message: String,
}

#[cfg(test)]
#[path = "gate_ws_trade_tests.rs"]
mod tests;
