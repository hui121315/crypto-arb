//! Bybit V5 private WebSocket payloads and event parser.
//!
//! Official docs:
//! - Connect/auth/private URL: <https://bybit-exchange.github.io/docs/v5/ws/connect>
//! - Order stream: <https://bybit-exchange.github.io/docs/v5/websocket/private/order>
//! - Execution stream: <https://bybit-exchange.github.io/docs/v5/websocket/private/execution>
//! - Position stream: <https://bybit-exchange.github.io/docs/v5/websocket/private/position>
//! - Wallet stream: <https://bybit-exchange.github.io/docs/v5/websocket/private/wallet>

use super::bybit_ws_user_data;
use crate::error::{ExchangeError, ExchangeResult};
use crate::signing::bybit as sign;
use common::time::now_ms;
use serde::{Deserialize, Serialize};
use shared_types::{LiveOrderState, OrderInfo};

pub const BYBIT_PRIVATE_WS_URL: &str = "wss://stream.bybit.com/v5/private";
pub const BYBIT_TESTNET_PRIVATE_WS_URL: &str = "wss://stream-testnet.bybit.com/v5/private";

pub(super) const EXCHANGE: &str = "bybit";
const OP_AUTH: &str = "auth";
const OP_SUBSCRIBE: &str = "subscribe";
pub(super) const TOPIC_ORDER: &str = "order";
pub(super) const TOPIC_EXECUTION: &str = "execution";
pub(super) const TOPIC_POSITION: &str = "position";
pub(super) const TOPIC_WALLET: &str = "wallet";

#[derive(Debug, Clone, Copy)]
pub struct BybitUserWsConfig<'a> {
    pub api_key: &'a str,
    pub api_secret: &'a str,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BybitUserEvent {
    Order(Vec<BybitOrderUpdate>),
    Execution(Vec<BybitExecutionUpdate>),
    Position(BybitPositionUpdate),
    Wallet(BybitWalletUpdate),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BybitUserControl {
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

/// Bybit V5 `position` topic envelope。Bybit 在订阅后会按 category 先推一条
/// `type: "snapshot"` 的全量行，随后按 incremental delta 推 `type: "delta"`。
/// 文档：<https://bybit-exchange.github.io/docs/v5/websocket/private/position>
#[derive(Debug, Clone, PartialEq)]
pub struct BybitPositionUpdate {
    pub update_type: String,
    pub positions: Vec<BybitPositionDelta>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BybitOrderUpdate {
    pub client_order_id: String,
    pub live_state: LiveOrderState,
    pub order: OrderInfo,
    pub finality: BybitOrderFinalityEvidence,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BybitExecutionUpdate {
    pub order_id: String,
    pub client_order_id: String,
    pub exec_id: String,
    pub symbol: String,
    pub category: String,
    pub side: String,
    pub price: f64,
    pub size: f64,
    pub fee: Option<f64>,
    pub fee_currency: Option<String>,
    pub fee_rate: Option<f64>,
    pub extra_fees: Vec<BybitExecutionExtraFee>,
    pub trade_time_ms: i64,
    pub is_maker: Option<bool>,
    pub seq: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BybitExecutionExtraFee {
    pub fee_coin: String,
    pub fee_type: String,
    pub sub_fee_type: String,
    pub fee_rate: Option<f64>,
    pub fee: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BybitOrderFinalityEvidence {
    pub position_idx: Option<i32>,
    pub cancel_type: Option<String>,
    pub reject_reason: Option<String>,
    pub leaves_quantity: Option<f64>,
    pub reduce_only: Option<bool>,
    pub time_in_force: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BybitPositionDelta {
    pub symbol: String,
    pub category: String,
    pub side: String,
    pub size: f64,
    pub entry_price: f64,
    pub mark_price: f64,
    pub unrealized_pnl: f64,
    pub leverage: f64,
    pub liquidation_price: Option<f64>,
    pub updated_time_ms: i64,
}

/// Bybit V5 `wallet` topic envelope。订阅后按 `account_type` 先推一条
/// `type: "snapshot"` 的全量钱包行，随后按 incremental delta 推 `type: "delta"`。
/// 文档：<https://bybit-exchange.github.io/docs/v5/websocket/private/wallet>
#[derive(Debug, Clone, PartialEq)]
pub struct BybitWalletUpdate {
    pub update_type: String,
    pub observed_at_ms: i64,
    pub accounts: Vec<BybitWalletAccount>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BybitWalletAccount {
    pub account_type: String,
    pub total_equity: f64,
    pub total_available_balance: f64,
    pub total_initial_margin: f64,
    pub total_maintenance_margin: f64,
    pub account_im_rate: f64,
    pub account_mm_rate: f64,
    pub coins: Vec<BybitWalletCoinDelta>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BybitWalletCoinDelta {
    pub coin: String,
    pub equity: f64,
    pub usd_value: f64,
    pub wallet_balance: f64,
    pub available_to_withdraw: Option<f64>,
    pub locked: f64,
    pub unrealized_pnl: f64,
}

pub fn private_ws_url(testnet: bool) -> &'static str {
    if testnet {
        BYBIT_TESTNET_PRIVATE_WS_URL
    } else {
        BYBIT_PRIVATE_WS_URL
    }
}

pub fn auth_payload(cfg: BybitUserWsConfig<'_>) -> ExchangeResult<String> {
    serde_json::to_string(&auth_request(cfg))
        .map_err(|error| ExchangeError::Parse(format!("bybit ws user auth payload: {error}")))
}

pub fn subscribe_private_payload(req_id: &str) -> ExchangeResult<String> {
    subscribe_payload(
        req_id,
        &[TOPIC_ORDER, TOPIC_EXECUTION, TOPIC_POSITION, TOPIC_WALLET],
    )
}

pub fn subscribe_payload(req_id: &str, topics: &[&str]) -> ExchangeResult<String> {
    let args = subscription_topics(topics)?;
    let request = SubscribeRequest {
        req_id: req_id.to_owned(),
        op: OP_SUBSCRIBE,
        args,
    };
    serde_json::to_string(&request)
        .map_err(|error| ExchangeError::Parse(format!("bybit ws user subscribe payload: {error}")))
}

pub fn parse_user_event(text: &str) -> ExchangeResult<Option<BybitUserEvent>> {
    bybit_ws_user_data::parse_user_event(text)
}

pub fn parse_user_control(text: &str) -> ExchangeResult<Option<BybitUserControl>> {
    let response: ControlResponse = serde_json::from_str(text).map_err(|error| {
        ExchangeError::Parse(format!("bybit user ws control: {error}; body={text}"))
    })?;
    let channel = response.op.trim();
    if !matches!(channel, OP_AUTH | OP_SUBSCRIBE) {
        return Ok(None);
    }
    let request_id = clean_text(&response.request_id);
    if response.success {
        return Ok(Some(BybitUserControl::Acknowledged {
            channel: channel.to_owned(),
            request_id,
        }));
    }
    let message = response.message.trim();
    Ok(Some(BybitUserControl::Rejected {
        channel: channel.to_owned(),
        request_id,
        authentication_failed: channel == OP_AUTH,
        error: if message.is_empty() {
            format!("bybit private ws {channel} rejected")
        } else {
            message.to_owned()
        },
    }))
}

fn clean_text(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn auth_request(cfg: BybitUserWsConfig<'_>) -> AuthRequest {
    let expires = now_ms() + 5_000;
    let expires_text = expires.to_string();
    AuthRequest {
        op: OP_AUTH,
        args: (
            cfg.api_key.to_owned(),
            expires,
            sign::ws_auth_sign(cfg.api_secret.as_bytes(), &expires_text),
        ),
    }
}

fn subscription_topics(topics: &[&str]) -> ExchangeResult<Vec<String>> {
    if topics.is_empty() {
        return Err(ExchangeError::Parse(
            "bybit ws user subscribe requires at least one topic".into(),
        ));
    }
    topics
        .iter()
        .map(|topic| {
            let trimmed = topic.trim();
            if trimmed.is_empty() {
                Err(ExchangeError::Parse(
                    "bybit ws user subscribe empty topic".into(),
                ))
            } else {
                Ok(trimmed.to_owned())
            }
        })
        .collect()
}

#[derive(Debug, Serialize)]
struct AuthRequest {
    op: &'static str,
    args: (String, i64, String),
}

#[derive(Debug, Serialize)]
struct SubscribeRequest {
    req_id: String,
    op: &'static str,
    args: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ControlResponse {
    #[serde(default)]
    success: bool,
    #[serde(default)]
    op: String,
    #[serde(default, rename = "req_id")]
    request_id: String,
    #[serde(default, rename = "ret_msg", alias = "retMsg")]
    message: String,
}

#[cfg(test)]
#[path = "bybit_ws_user_tests.rs"]
mod tests;
