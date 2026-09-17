//! Gate Spot account-trade WebSocket session.
//!
//! Official channels: `spot.login`, `spot.order_place`, `spot.order_cancel`,
//! `spot.order_status` and `spot.order_list`.
//! <https://www.gate.com/docs/developers/apiv4/ws/en/#spot-account-trade>

use super::gate_spot_trade_data::{
    ack_from_row, cancel_params, order_info, place_params, status_params, GateSpotOrderRow,
};
use super::spot_order_contract::CompiledSpotOrder;
use crate::error::{ExchangeError, ExchangeResult};
use crate::signing::gate as sign;
use crate::ws::trade_session::{session_key, WsLoginSpec, WsSessionSpec, WsTradeSession};
use common::time::now_secs;
use dashmap::DashMap;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use shared_types::{CancelOrderRequest, LiveOrderState, OrderAck, OrderInfo, OrderIntent};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

const EXCHANGE: &str = "gate";
const EVENT_API: &str = "api";
const CHANNEL_LOGIN: &str = "spot.login";
const CHANNEL_ORDER_PLACE: &str = "spot.order_place";
const CHANNEL_ORDER_CANCEL: &str = "spot.order_cancel";
const CHANNEL_ORDER_STATUS: &str = "spot.order_status";

#[derive(Debug, Clone, Copy)]
pub(super) struct WsSpotTradeConfig<'a> {
    pub url: &'a str,
    pub api_key: &'a str,
    pub api_secret: &'a str,
    pub timeout_secs: u64,
    pub time_offset_secs: i64,
}

pub(super) async fn place_order(
    cfg: WsSpotTradeConfig<'_>,
    intent: &OrderIntent,
    compiled: &CompiledSpotOrder,
) -> ExchangeResult<OrderAck> {
    let row: GateSpotOrderRow = send_typed_request(
        cfg,
        api_request(
            CHANNEL_ORDER_PLACE,
            &intent.id,
            place_params(intent, compiled)?,
        ),
    )
    .await?;
    ack_from_row(
        intent.id.clone(),
        intent.client_order_id.clone(),
        &row,
        LiveOrderState::Accepted,
    )
}

pub(super) async fn cancel_order(
    cfg: WsSpotTradeConfig<'_>,
    request: &CancelOrderRequest,
    native_symbol: &str,
) -> ExchangeResult<OrderAck> {
    let row: GateSpotOrderRow = send_typed_request(
        cfg,
        api_request(
            CHANNEL_ORDER_CANCEL,
            &request.internal_order_id,
            cancel_params(request, native_symbol)?,
        ),
    )
    .await?;
    ack_from_row(
        request.internal_order_id.clone(),
        request.client_order_id.clone(),
        &row,
        LiveOrderState::CancelRequested,
    )
}

pub(super) async fn get_order(
    cfg: WsSpotTradeConfig<'_>,
    order_id: &str,
    native_symbol: &str,
) -> ExchangeResult<Option<OrderInfo>> {
    let row: GateSpotOrderRow = send_typed_request(
        cfg,
        api_request(
            CHANNEL_ORDER_STATUS,
            &next_request_id("status"),
            status_params(order_id, native_symbol)?,
        ),
    )
    .await?;
    order_info(row).map(Some)
}

async fn send_typed_request<T: DeserializeOwned>(
    cfg: WsSpotTradeConfig<'_>,
    request: WsRequest,
) -> ExchangeResult<T> {
    let request_id = request.payload.req_id.clone();
    let payload = serde_json::to_string(&request)
        .map_err(|error| ExchangeError::Parse(format!("gate spot ws request: {error}")))?;
    let text = session(cfg)
        .send(
            payload,
            Box::new(move |text| Ok(parse_response(text)?.is_final_for(&request_id))),
        )
        .await?;
    parse_response(&text)?.into_typed_result()
}

fn session(cfg: WsSpotTradeConfig<'_>) -> WsTradeSession {
    static SESSIONS: OnceLock<DashMap<String, WsTradeSession>> = OnceLock::new();
    let offset = cfg.time_offset_secs.to_string();
    let key = session_key(&[
        EXCHANGE,
        "spot",
        cfg.url,
        cfg.api_key,
        cfg.api_secret,
        &offset,
    ]);
    let spec = || {
        let api_key = cfg.api_key.to_owned();
        let api_secret = cfg.api_secret.to_owned();
        let time_offset_secs = cfg.time_offset_secs;
        WsSessionSpec::new(cfg.url, cfg.timeout_secs).with_login(WsLoginSpec::new(
            move || {
                serde_json::to_string(&login_request(WsSpotTradeConfig {
                    url: "",
                    api_key: &api_key,
                    api_secret: &api_secret,
                    timeout_secs: 0,
                    time_offset_secs,
                }))
                .map_err(|error| ExchangeError::Parse(format!("gate spot ws login: {error}")))
            },
            |text| {
                let response = parse_response(text)?;
                if response.header.channel != CHANNEL_LOGIN {
                    return Ok(false);
                }
                response.into_typed_result::<Value>()?;
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

fn login_request(cfg: WsSpotTradeConfig<'_>) -> WsRequest {
    let time = now_secs().saturating_add(cfg.time_offset_secs);
    let req_id = next_request_id("login");
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
            signature: Some(signature),
            timestamp: Some(time.to_string()),
        },
    }
}

fn api_request(channel: &'static str, req_id: &str, params: Value) -> WsRequest {
    WsRequest {
        time: now_secs(),
        channel,
        event: EVENT_API,
        payload: WsPayload {
            req_id: req_id.to_owned(),
            req_param: Some(params),
            api_key: None,
            signature: None,
            timestamp: None,
        },
    }
}

fn next_request_id(operation: &str) -> String {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    format!(
        "cl-{operation}-{:x}-{:x}",
        common::time::now_ms(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

fn parse_response(text: &str) -> ExchangeResult<WsResponse> {
    serde_json::from_str(text).map_err(|error| {
        ExchangeError::Parse(format!("gate spot ws response: {error}; body={text}"))
    })
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
    signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timestamp: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WsResponse {
    #[serde(default)]
    request_id: String,
    #[serde(default)]
    ack: bool,
    #[serde(default)]
    header: WsHeader,
    #[serde(default)]
    data: WsData,
}

impl WsResponse {
    fn is_final_for(&self, request_id: &str) -> bool {
        self.request_id == request_id
            && (!self.ack
                || self.data.errs.is_some()
                || (!self.header.status.is_empty() && self.header.status != "200"))
    }

    fn into_typed_result<T: DeserializeOwned>(self) -> ExchangeResult<T> {
        if let Some(error) = self.data.errs {
            return Err(ExchangeError::Api {
                exchange: EXCHANGE.to_owned(),
                code: error.label,
                message: error.message,
            });
        }
        if !self.header.status.is_empty() && self.header.status != "200" {
            return Err(ExchangeError::Api {
                exchange: EXCHANGE.to_owned(),
                code: self.header.status,
                message: "gate spot WebSocket returned non-200 status".to_owned(),
            });
        }
        let result = self
            .data
            .result
            .ok_or_else(|| ExchangeError::Parse("gate spot ws missing result".to_owned()))?;
        serde_json::from_value(result)
            .map_err(|error| ExchangeError::Parse(format!("gate spot ws result schema: {error}")))
    }
}

#[derive(Debug, Default, Deserialize)]
struct WsHeader {
    #[serde(default)]
    status: String,
    #[serde(default)]
    channel: String,
}

#[derive(Debug, Default, Deserialize)]
struct WsData {
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    errs: Option<WsError>,
}

#[derive(Debug, Deserialize)]
struct WsError {
    #[serde(default)]
    label: String,
    #[serde(default)]
    message: String,
}

#[cfg(test)]
#[path = "gate_spot_ws_trade_tests.rs"]
mod tests;
