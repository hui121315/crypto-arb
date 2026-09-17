//! Persistent KuCoin Pro WebSocket Spot order writer.
//!
//! The server-first challenge is signed byte-for-byte before the connection
//! becomes usable. The same authenticated socket is reused for place/cancel.

use super::kucoin_spot_trade_data::{cancel_args, place_args};
use super::kucoin_ws_user::{
    pro_challenge_signature, pro_connect_url, pro_spot_cancel_payload, pro_spot_order_payload,
    KucoinUserWsConfig, KUCOIN_PRO_PRIVATE_WS_BASE,
};
use super::spot_order_contract::CompiledSpotOrder;
use crate::error::{ExchangeError, ExchangeResult};
use crate::ws::trade_session::{session_key, WsChallengeLoginSpec, WsSessionSpec, WsTradeSession};
use dashmap::DashMap;
use serde::Deserialize;
use serde_json::Value;
use shared_types::{
    CancelOrderRequest, LiveOrderState, OrderAck, OrderIntent, VenueOrderIdentityUpdate,
};
use std::sync::OnceLock;

const EXCHANGE: &str = "kucoin";
const SUCCESS_CODE: &str = "200000";

#[derive(Debug, Clone, Copy)]
pub(super) struct WsSpotTradeConfig<'a> {
    pub api_key: &'a str,
    pub api_secret: &'a str,
    pub passphrase: &'a str,
    pub timeout_secs: u64,
    pub time_offset_ms: i64,
}

pub(super) async fn place_order(
    cfg: WsSpotTradeConfig<'_>,
    intent: &OrderIntent,
    compiled: &CompiledSpotOrder,
) -> ExchangeResult<OrderAck> {
    let request_id = intent.id.clone();
    let payload = pro_spot_order_payload(&request_id, place_args(intent, compiled)?)?;
    let response = send_request(cfg, payload, request_id).await?;
    Ok(response.into_ack(
        intent.id.clone(),
        intent.client_order_id.clone(),
        LiveOrderState::Accepted,
        None,
    ))
}

pub(super) async fn cancel_order(
    cfg: WsSpotTradeConfig<'_>,
    request: &CancelOrderRequest,
    native_symbol: &str,
) -> ExchangeResult<OrderAck> {
    let request_id = request.internal_order_id.clone();
    let payload = pro_spot_cancel_payload(&request_id, cancel_args(request, native_symbol)?)?;
    let response = send_request(cfg, payload, request_id).await?;
    Ok(response.into_ack(
        request.internal_order_id.clone(),
        request.client_order_id.clone(),
        LiveOrderState::CancelRequested,
        Some("kucoin spot cancel accepted; final state requires private order stream".to_owned()),
    ))
}

async fn send_request(
    cfg: WsSpotTradeConfig<'_>,
    payload: String,
    request_id: String,
) -> ExchangeResult<ProResponse> {
    let text = session(cfg)
        .send(
            payload,
            Box::new(move |text| Ok(parse_response(text)?.id == request_id)),
        )
        .await?;
    parse_response(&text)?.into_result()
}

fn session(cfg: WsSpotTradeConfig<'_>) -> WsTradeSession {
    static SESSIONS: OnceLock<DashMap<String, WsTradeSession>> = OnceLock::new();
    let offset = cfg.time_offset_ms.to_string();
    let key = session_key(&[
        EXCHANGE,
        "spot",
        cfg.api_key,
        cfg.api_secret,
        cfg.passphrase,
        &offset,
    ]);
    let spec = || {
        let time_offset_ms = cfg.time_offset_ms;
        let connect_key = cfg.api_key.to_owned();
        let connect_secret = cfg.api_secret.to_owned();
        let connect_passphrase = cfg.passphrase.to_owned();
        let challenge_secret = connect_secret.clone();
        WsSessionSpec::new(KUCOIN_PRO_PRIVATE_WS_BASE, cfg.timeout_secs)
            .with_fresh_url(move || {
                Ok(pro_connect_url(
                    KucoinUserWsConfig {
                        api_key: &connect_key,
                        api_secret: &connect_secret,
                        passphrase: &connect_passphrase,
                        time_offset_ms,
                    },
                    true,
                ))
            })
            .with_challenge_login(WsChallengeLoginSpec::new(
                is_challenge,
                move |challenge| Ok(pro_challenge_signature(&challenge_secret, challenge)),
                is_welcome,
            ))
    };
    SESSIONS
        .get_or_init(DashMap::new)
        .entry(key)
        .or_insert_with(|| WsTradeSession::spawn(spec()))
        .clone()
}

fn is_challenge(text: &str) -> ExchangeResult<bool> {
    let value: Value = serde_json::from_str(text)
        .map_err(|error| ExchangeError::Parse(format!("kucoin pro challenge: {error}")))?;
    Ok(value.get("sessionId").and_then(Value::as_str).is_some()
        && value.get("timestamp").is_some()
        && value.get("data").is_none())
}

fn is_welcome(text: &str) -> ExchangeResult<bool> {
    let value: Value = serde_json::from_str(text)
        .map_err(|error| ExchangeError::Parse(format!("kucoin pro welcome: {error}")))?;
    Ok(value.get("sessionId").and_then(Value::as_str).is_some()
        && matches!(
            value
                .get("data")
                .or_else(|| value.get("message"))
                .and_then(Value::as_str),
            Some("welcome")
        ))
}

fn parse_response(text: &str) -> ExchangeResult<ProResponse> {
    serde_json::from_str(text).map_err(|error| {
        ExchangeError::Parse(format!("kucoin spot Pro WS response: {error}; body={text}"))
    })
}

#[derive(Debug, Deserialize)]
struct ProResponse {
    #[serde(default)]
    id: String,
    #[serde(default)]
    code: String,
    #[serde(default)]
    msg: String,
    #[serde(default)]
    data: ProOrderData,
}

impl ProResponse {
    fn into_result(self) -> ExchangeResult<Self> {
        if self.code == SUCCESS_CODE {
            Ok(self)
        } else {
            Err(ExchangeError::Api {
                exchange: EXCHANGE.to_owned(),
                code: if self.code.is_empty() {
                    "UNKNOWN".to_owned()
                } else {
                    self.code
                },
                message: self.msg,
            })
        }
    }

    fn into_ack(
        self,
        internal_order_id: String,
        public_client_order_id: String,
        state: LiveOrderState,
        message: Option<String>,
    ) -> OrderAck {
        let venue_client_order_id =
            non_empty(self.data.client_oid).unwrap_or_else(|| public_client_order_id.clone());
        let exchange_order_id = non_empty(self.data.order_id);
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
            accepted_at_ms: common::time::now_ms(),
            message,
            filled_quantity: None,
            filled_price: None,
            filled_fee: None,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct ProOrderData {
    #[serde(default, rename = "clientOid")]
    client_oid: String,
    #[serde(default, rename = "orderId")]
    order_id: String,
}

fn non_empty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

#[cfg(test)]
#[path = "kucoin_spot_ws_trade_tests.rs"]
mod tests;
