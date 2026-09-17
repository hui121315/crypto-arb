//! KuCoin Futures private WebSocket payloads and event parser.
//!
//! Official docs:
//! - Futures private token: <https://www.kucoin.com/docs-new/websocket-api/base-info/get-private-token-futures>
//! - Futures private orders: <https://www.kucoin.com/docs-new/3470090w0>
//! - Futures private positions: <https://www.kucoin.com/docs-new/3470093w0>
//! - Pro WS add order: <https://www.kucoin.com/docs-new/3470252w0>
//! - Pro WS cancel order: <https://www.kucoin.com/docs-new/3470253w0>
//!
//! KuCoin's current production documentation explicitly supports Classic and UTA order writes.
//! This module exposes verified payload/signature helpers, while the production writer remains
//! gated until an authenticated session and live ACK/finality evidence are recorded.

use super::kucoin_ws_user_data;
use crate::adapter::strip_common_suffixes;
use crate::error::{ExchangeError, ExchangeResult};
use crate::signing::kucoin as sign;
use common::time::now_ms;
use serde::Serialize;
use serde_json::Value;
use shared_types::{LiveOrderState, OrderInfo};

pub const KUCOIN_FUTURES_PRIVATE_BULLET_PATH: &str = "/api/v1/bullet-private";
pub const KUCOIN_PRO_PRIVATE_WS_BASE: &str = "wss://wsapi.kucoin.com/v1/private";
pub const KUCOIN_PRO_WS_RUNTIME_EVIDENCE_BLOCKER: &str =
    "KUCOIN_PRO_WS_RUNTIME_EVIDENCE_MISSING: authenticated session, live place/cancel ACK, and order finality evidence are required";

pub(super) const EXCHANGE: &str = "kucoin";
const TYPE_SUBSCRIBE: &str = "subscribe";
pub(super) const TYPE_MESSAGE: &str = "message";
pub(super) const TOPIC_ORDER: &str = "/contractMarket/tradeOrders";
pub(super) const TOPIC_BALANCE: &str = "/contractAccount/wallet";
const TOPIC_POSITION_ALL: &str = "/contract/positionAll";
pub(super) const TOPIC_POSITION_PREFIX: &str = "/contract/position";
const OP_FUTURES_ORDER: &str = "futures.order";
const OP_FUTURES_CANCEL: &str = "futures.cancel";
const OP_SPOT_ORDER: &str = "spot.order";
const OP_SPOT_CANCEL: &str = "spot.cancel";

#[derive(Debug, Clone, Copy)]
pub struct KucoinUserWsConfig<'a> {
    pub api_key: &'a str,
    pub api_secret: &'a str,
    pub passphrase: &'a str,
    pub time_offset_ms: i64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KucoinProWsRuntimeEvidence {
    pub authenticated_session: bool,
    pub live_place_ack: bool,
    pub live_cancel_ack: bool,
    pub order_finality: bool,
}

impl KucoinProWsRuntimeEvidence {
    fn is_complete(self) -> bool {
        self.authenticated_session
            && self.live_place_ack
            && self.live_cancel_ack
            && self.order_finality
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum KucoinUserEvent {
    Order(Box<KucoinOrderUpdate>),
    Balance(KucoinBalanceDelta),
    Position(KucoinPositionDelta),
}

#[derive(Debug, Clone, PartialEq)]
pub struct KucoinOrderUpdate {
    pub client_order_id: String,
    pub event_type: String,
    pub event_time_ms: i64,
    pub terminal: bool,
    pub live_state: LiveOrderState,
    pub order: OrderInfo,
    pub fill: Option<KucoinFillUpdate>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct KucoinFillUpdate {
    pub exchange_order_id: String,
    pub client_order_id: Option<String>,
    pub trade_id: String,
    pub venue_event_id: String,
    pub symbol: String,
    pub side: shared_types::OrderSide,
    pub quantity: f64,
    pub price: f64,
    pub liquidity: String,
    pub fee_type: String,
    /// Classic `tradeOrders` does not publish the charged fee amount.
    pub fee_amount: Option<f64>,
    /// Classic `tradeOrders` does not publish the charged fee currency.
    pub fee_currency: Option<String>,
    pub occurred_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct KucoinBalanceDelta {
    pub subject: String,
    pub currency: String,
    pub total: f64,
    pub available: f64,
    pub hold_balance: f64,
    pub unrealized_pnl: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum KucoinPositionDelta {
    Change {
        native_symbol: String,
        current_contracts: f64,
        updated_time_ms: i64,
    },
    Settlement {
        native_symbol: Option<String>,
        current_contracts: f64,
        updated_time_ms: i64,
    },
    RiskLimitAdjustment {
        success: bool,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct KucoinProOrderAck {
    pub request_id: String,
    pub client_order_id: String,
    pub exchange_order_id: Option<String>,
    pub live_state: LiveOrderState,
    pub message: Option<String>,
}

pub fn subscribe_orders_payload(id: &str) -> ExchangeResult<String> {
    subscribe_payload(id, TOPIC_ORDER, true)
}

pub fn subscribe_balance_payload(id: &str) -> ExchangeResult<String> {
    subscribe_payload(id, TOPIC_BALANCE, true)
}

pub fn subscribe_positions_payload(id: &str) -> ExchangeResult<String> {
    subscribe_payload(id, TOPIC_POSITION_ALL, true)
}

pub fn subscribe_position_payload(id: &str, symbol: &str) -> ExchangeResult<String> {
    let symbol = checked_symbol(symbol)?;
    subscribe_payload(id, &format!("{TOPIC_POSITION_PREFIX}:{symbol}"), true)
}

pub fn pro_connect_url(cfg: KucoinUserWsConfig<'_>, enable_ns: bool) -> String {
    let timestamp = now_ms().saturating_add(cfg.time_offset_ms).to_string();
    let sign = sign::sign_ws_connect(cfg.api_secret.as_bytes(), cfg.api_key, &timestamp);
    let passphrase = sign::encrypt_passphrase(cfg.api_secret.as_bytes(), cfg.passphrase);
    let mut url = format!(
        "{KUCOIN_PRO_PRIVATE_WS_BASE}?apikey={}&sign={}&passphrase={}&timestamp={}",
        encode(cfg.api_key),
        encode(&sign),
        encode(&passphrase),
        encode(&timestamp),
    );
    if enable_ns {
        url.push_str("&enable_ns=true");
    }
    url
}

/// Returns every blocker that prevents KuCoin Pro WS from entering the live writer.
///
/// The public schema is production-published, but static fixtures cannot prove authenticated
/// account permissions or live order finality.
pub fn pro_ws_live_submit_blockers(
    evidence: Option<KucoinProWsRuntimeEvidence>,
) -> Vec<&'static str> {
    if evidence.is_some_and(KucoinProWsRuntimeEvidence::is_complete) {
        Vec::new()
    } else {
        vec![KUCOIN_PRO_WS_RUNTIME_EVIDENCE_BLOCKER]
    }
}

pub fn pro_challenge_signature(api_secret: &str, challenge_json: &str) -> String {
    sign::sign_ws_challenge(api_secret.as_bytes(), challenge_json)
}

pub fn pro_futures_order_payload(id: &str, args: Value) -> ExchangeResult<String> {
    pro_payload(id, OP_FUTURES_ORDER, args)
}

pub fn pro_futures_cancel_payload(id: &str, args: Value) -> ExchangeResult<String> {
    pro_payload(id, OP_FUTURES_CANCEL, args)
}

pub fn pro_spot_order_payload(id: &str, args: Value) -> ExchangeResult<String> {
    pro_payload(id, OP_SPOT_ORDER, args)
}

pub fn pro_spot_cancel_payload(id: &str, args: Value) -> ExchangeResult<String> {
    pro_payload(id, OP_SPOT_CANCEL, args)
}

pub fn parse_user_event(text: &str) -> ExchangeResult<Option<KucoinUserEvent>> {
    kucoin_ws_user_data::parse_user_event(text)
}

pub fn parse_pro_order_ack(text: &str) -> ExchangeResult<Option<KucoinProOrderAck>> {
    kucoin_ws_user_data::parse_pro_order_ack(text)
}

fn subscribe_payload(id: &str, topic: &str, private_channel: bool) -> ExchangeResult<String> {
    let topic = checked_topic(topic)?;
    let request = SubscribeRequest {
        id: id.to_owned(),
        message_type: TYPE_SUBSCRIBE,
        topic,
        private_channel,
        response: true,
    };
    serde_json::to_string(&request)
        .map_err(|error| ExchangeError::Parse(format!("kucoin ws user subscribe payload: {error}")))
}

fn pro_payload(id: &str, op: &'static str, args: Value) -> ExchangeResult<String> {
    let request = ProRequest {
        id: id.to_owned(),
        op,
        args,
    };
    serde_json::to_string(&request)
        .map_err(|error| ExchangeError::Parse(format!("kucoin pro ws payload: {error}")))
}

fn checked_topic(topic: &str) -> ExchangeResult<String> {
    let trimmed = topic.trim();
    if trimmed.is_empty() {
        Err(ExchangeError::Parse("kucoin ws user empty topic".into()))
    } else {
        Ok(trimmed.to_owned())
    }
}

fn checked_symbol(symbol: &str) -> ExchangeResult<String> {
    let trimmed = symbol.trim().to_ascii_uppercase();
    if trimmed.is_empty() {
        return Err(ExchangeError::Parse(
            "kucoin ws user position subscribe empty symbol".into(),
        ));
    }
    Ok(if trimmed.ends_with("USDTM") {
        trimmed
    } else {
        format!("{}USDTM", kucoin_base(&strip_common_suffixes(&trimmed)))
    })
}

fn kucoin_base(base: &str) -> String {
    if base.eq_ignore_ascii_case("BTC") {
        "XBT".to_owned()
    } else {
        base.to_ascii_uppercase()
    }
}

fn encode(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

#[derive(Debug, Serialize)]
struct SubscribeRequest {
    id: String,
    #[serde(rename = "type")]
    message_type: &'static str,
    topic: String,
    #[serde(rename = "privateChannel")]
    private_channel: bool,
    response: bool,
}

#[derive(Debug, Serialize)]
struct ProRequest {
    id: String,
    op: &'static str,
    args: Value,
}

#[cfg(test)]
#[path = "kucoin_ws_user_tests.rs"]
mod tests;
