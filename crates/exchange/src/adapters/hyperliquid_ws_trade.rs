//! Hyperliquid WebSocket post/action trading.
//!
//! Official docs:
//! - WebSocket post requests: <https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/websocket/post-requests>
//! - Exchange endpoint actions: <https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/exchange-endpoint>
//! - Signing: <https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/signing>

use crate::error::{ExchangeError, ExchangeResult};
use crate::signing::hyperliquid::{self, HyperliquidNetwork, HyperliquidSignature};
use crate::ws::manager::WsHeartbeat;
use crate::ws::trade_session::{session_key, WsSessionSpec, WsTradeSession};
use common::time::now_ms;
use dashmap::DashMap;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use shared_types::{
    CancelOrderRequest, LiveOrderState, OrderAck, OrderIntent, OrderTransportMetadata,
    VenueOrderIdentityUpdate,
};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

/// 修复 P2 9.7：WS 请求 id 用独立的单调 counter，与 nonce（HL 签名防重放 token）解耦。
/// 同步发起多个请求时 id 必须唯一；nonce 同样必须单调但语义不同（HL 服务端校验）。
/// 初值 = `now_ms()`，每次 `fetch_add(1)` 保证全局单调递增。
static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(0);
static NONCES: OnceLock<DashMap<String, AtomicU64>> = OnceLock::new();
static SIGNER_SESSION_HEALTH: OnceLock<DashMap<String, HyperliquidSignerSessionHealth>> =
    OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HyperliquidSignerSessionHealth {
    pub account_address: String,
    pub signer_address: String,
    pub vault_address: Option<String>,
    pub network: String,
    pub ownership_boundary: &'static str,
    pub last_nonce: u64,
    pub last_error: Option<String>,
    pub observed_at_ms: i64,
}

pub fn hyperliquid_signer_session_health() -> Vec<HyperliquidSignerSessionHealth> {
    let mut rows: Vec<HyperliquidSignerSessionHealth> = SIGNER_SESSION_HEALTH
        .get()
        .map(|rows| rows.iter().map(|row| row.value().clone()).collect())
        .unwrap_or_default();
    rows.sort_by(|left, right| {
        left.signer_address
            .cmp(&right.signer_address)
            .then_with(|| left.account_address.cmp(&right.account_address))
            .then_with(|| left.vault_address.cmp(&right.vault_address))
    });
    rows
}

fn next_request_id() -> u64 {
    let current = NEXT_REQUEST_ID.load(Ordering::Relaxed);
    if current == 0 {
        // 首次使用：用 now_ms() 作起点，避免重启后 id 冲突
        let init = now_ms().max(1) as u64;
        NEXT_REQUEST_ID
            .compare_exchange(0, init, Ordering::Relaxed, Ordering::Relaxed)
            .ok();
    }
    NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed)
}

/// PR-DA：per-signer 单调 nonce 分配器。Hyperliquid 服务端要求同一签名账户的
/// `nonce` 严格递增，且落在 recency 窗口（约 T-2d..T+1d）内。裸用 `now_ms()` 时，
/// 同一毫秒内并发下单/撤单会得到相同 nonce，被服务端当成重放而拒绝后一笔。这里按
/// signer 维护 high-water mark，返回 `max(now_ms(), prev + 1)`：保持与时钟对齐
/// （仍在 recency 窗口内），同时保证严格递增、永不重复、永不回退。
/// Official guidance recommends one API wallet per trading process; this
/// allocator deliberately does not coordinate the same signer across processes:
/// <https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/nonces-and-api-wallets>
pub(super) const SIGNER_OWNERSHIP_BOUNDARY: &str = "one_api_wallet_per_trading_process";

pub(super) fn next_nonce(signer_key: &str) -> u64 {
    let now = now_ms().max(1) as u64;
    let slot = NONCES
        .get_or_init(DashMap::new)
        .entry(signer_key.to_owned())
        .or_insert_with(|| AtomicU64::new(0));
    // `entry` 持有该 key 所在 shard 的写锁，同 signer 的并发调用在此串行化，
    // 因此下面的 load/store 对彼此是原子的。
    let next = now.max(slot.load(Ordering::Relaxed).saturating_add(1));
    slot.store(next, Ordering::Relaxed);
    next
}

/// Signer-scoped nonce sequence key. Hyperliquid applies one nonce set to the
/// signer across main/sub-account and vault scopes, so those scopes must not
/// create independent counters. Only the derived address is retained.
pub(super) fn signer_nonce_key(network: HyperliquidNetwork, private_key: &str) -> String {
    let signer = hyperliquid::address_from_private_key(private_key)
        .unwrap_or_else(|_| "unknown-signer".to_owned());
    format!("{SIGNER_OWNERSHIP_BOUNDARY}|{network:?}|{signer}")
}

const EXCHANGE: &str = "hyperliquid";
const METHOD_POST: &str = "post";
const REQUEST_TYPE_INFO: &str = "info";
const REQUEST_TYPE_ACTION: &str = "action";

#[derive(Debug, Clone, Copy)]
pub(super) struct WsInfoConfig<'a> {
    pub url: &'a str,
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct WsTradeConfig<'a> {
    pub url: &'a str,
    pub account_address: &'a str,
    pub private_key: &'a str,
    pub timeout_secs: u64,
    pub network: HyperliquidNetwork,
    /// 修复 P2 9.8：可选 vault address，None = 主账户签名。
    pub vault_address: Option<&'a str>,
    /// 修复 P2 9.8：可选 expiresAfter 窗口（毫秒），None = 不携带（永不过期）。
    pub action_expires_after_ms: Option<u64>,
}

pub(super) async fn place_order(
    cfg: WsTradeConfig<'_>,
    intent: &OrderIntent,
    venue_client_order_id: String,
    action: Value,
) -> ExchangeResult<OrderAck> {
    let result = send_action(cfg, action).await?;
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
    venue_client_order_id: String,
    action: Value,
) -> ExchangeResult<OrderAck> {
    let result = send_action(cfg, action).await?;
    Ok(ack_from_result(
        request.internal_order_id.clone(),
        request.client_order_id.clone(),
        venue_client_order_id,
        result,
        LiveOrderState::CancelRequested,
        Some(
            "hyperliquid cancel accepted; final state requires orderUpdates/orderStatus".to_owned(),
        ),
    ))
}

pub(super) async fn post_info<T>(cfg: WsInfoConfig<'_>, payload: Value) -> ExchangeResult<T>
where
    T: DeserializeOwned,
{
    let request_type = payload
        .get("type")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| ExchangeError::Parse("hyperliquid ws info request missing type".into()))?
        .to_owned();
    let id = next_request_id();
    let request = WsInfoPostRequest {
        method: METHOD_POST,
        id,
        request: WsInfoRequestEnvelope {
            request_type: REQUEST_TYPE_INFO,
            payload,
        },
    };
    let text = info_session(cfg)
        .send(
            serde_json::to_string(&request).map_err(|error| {
                ExchangeError::Parse(format!("hyperliquid ws info request: {error}"))
            })?,
            Box::new(move |text| {
                let Some(response) = parse_info_response(text)? else {
                    return Ok(false);
                };
                Ok(response.matches_id(id))
            }),
        )
        .await?;
    parse_info_response(&text)?
        .ok_or_else(|| ExchangeError::Parse("hyperliquid ws missing info response".into()))?
        .into_result(id, &request_type)
}

async fn send_action(cfg: WsTradeConfig<'_>, action: Value) -> ExchangeResult<OrderAckRow> {
    // 修复 P2 9.7：id 与 nonce 解耦。
    // - `nonce`：HL 签名防重放 token，需服务端单调可见，按 signer 单调分配
    //   （见 `next_nonce`），避免同毫秒并发下单 nonce 撞车被当重放拒绝
    // - `id`：客户端 WS request correlation key，用独立 atomic counter
    let signer_key = signer_nonce_key(cfg.network, cfg.private_key);
    let nonce = next_nonce(&signer_key);
    let result = send_action_with_nonce(cfg, action, nonce).await;
    record_signer_session_result(cfg, nonce, result.as_ref().err());
    result
}

async fn send_action_with_nonce(
    cfg: WsTradeConfig<'_>,
    action: Value,
    nonce: u64,
) -> ExchangeResult<OrderAckRow> {
    let id = next_request_id();
    // 修复 P2 9.8：expires_after = now_ms + window（若配置）。签名与 payload 必须一致。
    let expires_after = cfg
        .action_expires_after_ms
        .map(|window| nonce.saturating_add(window));
    let request = signed_post_request(SignedPostParams {
        id,
        private_key: cfg.private_key,
        action,
        nonce,
        network: cfg.network,
        vault_address: cfg.vault_address,
        expires_after,
    })?;
    let request_id = request.id;
    let payload = serde_json::to_string(&request)
        .map_err(|error| ExchangeError::Parse(format!("hyperliquid ws post request: {error}")))?;
    let text = session(cfg)
        .send(
            payload,
            Box::new(move |text| {
                let Some(response) = parse_post_response(text)? else {
                    return Ok(false);
                };
                Ok(response.matches_id(request_id))
            }),
        )
        .await?;
    parse_post_response(&text)?
        .ok_or_else(|| ExchangeError::Parse("hyperliquid ws missing post response".into()))?
        .into_result(request_id)
}

pub(super) fn record_signer_session_result(
    cfg: WsTradeConfig<'_>,
    nonce: u64,
    error: Option<&ExchangeError>,
) {
    let signer_address = hyperliquid::address_from_private_key(cfg.private_key)
        .unwrap_or_else(|_| "unknown-signer".to_owned());
    let scope = format!(
        "{:?}|{}|{}|{}",
        cfg.network,
        cfg.account_address.trim().to_ascii_lowercase(),
        signer_address.to_ascii_lowercase(),
        cfg.vault_address.unwrap_or("").trim().to_ascii_lowercase(),
    );
    SIGNER_SESSION_HEALTH.get_or_init(DashMap::new).insert(
        scope,
        HyperliquidSignerSessionHealth {
            account_address: cfg.account_address.to_owned(),
            signer_address,
            vault_address: cfg.vault_address.map(str::to_owned),
            network: format!("{:?}", cfg.network).to_ascii_lowercase(),
            ownership_boundary: SIGNER_OWNERSHIP_BOUNDARY,
            last_nonce: nonce,
            last_error: error.map(ToString::to_string),
            observed_at_ms: now_ms(),
        },
    );
}

fn session(cfg: WsTradeConfig<'_>) -> WsTradeSession {
    static SESSIONS: OnceLock<DashMap<String, WsTradeSession>> = OnceLock::new();
    let vault = cfg.vault_address.unwrap_or("");
    let expiry = cfg
        .action_expires_after_ms
        .map(|value| value.to_string())
        .unwrap_or_default();
    let key = session_key(&[EXCHANGE, cfg.url, cfg.private_key, vault, &expiry]);
    SESSIONS
        .get_or_init(DashMap::new)
        .entry(key)
        .or_insert_with(|| {
            WsTradeSession::spawn(
                WsSessionSpec::new(cfg.url, cfg.timeout_secs)
                    .with_heartbeat(hyperliquid_trade_heartbeat())
                    .with_heartbeat_response(hyperliquid_trade_heartbeat_response),
            )
        })
        .clone()
}

fn info_session(cfg: WsInfoConfig<'_>) -> WsTradeSession {
    static SESSIONS: OnceLock<DashMap<String, WsTradeSession>> = OnceLock::new();
    let key = session_key(&[EXCHANGE, "info", cfg.url]);
    SESSIONS
        .get_or_init(DashMap::new)
        .entry(key)
        .or_insert_with(|| {
            WsTradeSession::spawn(
                WsSessionSpec::new(cfg.url, cfg.timeout_secs)
                    .with_heartbeat(hyperliquid_trade_heartbeat())
                    .with_heartbeat_response(hyperliquid_trade_heartbeat_response),
            )
        })
        .clone()
}

fn hyperliquid_trade_heartbeat() -> WsHeartbeat {
    WsHeartbeat::Text(r#"{"method":"ping"}"#.to_owned())
}

fn hyperliquid_trade_heartbeat_response(text: &str) -> bool {
    serde_json::from_str::<Value>(text)
        .ok()
        .is_some_and(|value| value.get("channel").and_then(Value::as_str) == Some("pong"))
}

struct SignedPostParams<'a> {
    id: u64,
    private_key: &'a str,
    action: Value,
    nonce: u64,
    network: HyperliquidNetwork,
    vault_address: Option<&'a str>,
    expires_after: Option<u64>,
}

fn signed_post_request(params: SignedPostParams<'_>) -> ExchangeResult<WsPostRequest> {
    // 修复 P2 9.8：签名 hash 和 payload 必须同时包含 vault_address / expires_after，
    // 否则服务端校签失败。`sign_l1_action` 已经在 hash 计算中把这两字段附加到 msgpack
    // 之后（见 `signing/hyperliquid.rs::l1_action_hash`）。
    let SignedPostParams {
        id,
        private_key,
        action,
        nonce,
        network,
        vault_address,
        expires_after,
    } = params;
    let signature = hyperliquid::sign_l1_action(
        private_key,
        &action,
        nonce,
        vault_address,
        expires_after,
        network,
    )
    .map_err(|error| ExchangeError::Auth(format!("hyperliquid signing failed: {error}")))?;
    Ok(WsPostRequest {
        method: METHOD_POST,
        id,
        request: WsRequestEnvelope {
            request_type: REQUEST_TYPE_ACTION,
            payload: WsActionPayload {
                action,
                nonce,
                signature: signature.into(),
                vault_address: vault_address.map(str::to_owned),
                expires_after,
            },
        },
    })
}

fn parse_post_response(text: &str) -> ExchangeResult<Option<WsPostResponse>> {
    let value: Value = serde_json::from_str(text).map_err(|error| {
        ExchangeError::Parse(format!(
            "hyperliquid ws post response: {error}; body={text}"
        ))
    })?;
    if value.get("channel").and_then(Value::as_str) != Some("post") {
        return Ok(None);
    }
    serde_json::from_value(value)
        .map(Some)
        .map_err(|error| ExchangeError::Parse(format!("hyperliquid ws post payload: {error}")))
}

fn parse_info_response(text: &str) -> ExchangeResult<Option<WsInfoPostResponse>> {
    let value: Value = serde_json::from_str(text).map_err(|error| {
        ExchangeError::Parse(format!(
            "hyperliquid ws info response: {error}; body={text}"
        ))
    })?;
    if value.get("channel").and_then(Value::as_str) != Some("post") {
        return Ok(None);
    }
    serde_json::from_value(value)
        .map(Some)
        .map_err(|error| ExchangeError::Parse(format!("hyperliquid ws info payload: {error}")))
}

fn ack_from_result(
    internal_order_id: String,
    public_client_order_id: String,
    venue_client_order_id: String,
    row: OrderAckRow,
    state: LiveOrderState,
    message: Option<String>,
) -> OrderAck {
    let transport_metadata = row.transport_metadata();
    let exchange_order_id = row.order_id;
    OrderAck {
        internal_order_id,
        exchange_order_id: exchange_order_id.clone(),
        client_order_id: public_client_order_id.clone(),
        identity_update: VenueOrderIdentityUpdate::from_ids(
            public_client_order_id,
            venue_client_order_id,
            exchange_order_id,
        )
        .with_transport_metadata(transport_metadata),
        state,
        accepted_at_ms: now_ms(),
        message: message.or(row.message),
        filled_quantity: None,
        filled_price: None,
        filled_fee: None,
    }
}

#[derive(Debug, Serialize)]
struct WsPostRequest {
    method: &'static str,
    id: u64,
    request: WsRequestEnvelope,
}

#[derive(Debug, Serialize)]
struct WsInfoPostRequest {
    method: &'static str,
    id: u64,
    request: WsInfoRequestEnvelope,
}

#[derive(Debug, Serialize)]
struct WsInfoRequestEnvelope {
    #[serde(rename = "type")]
    request_type: &'static str,
    payload: Value,
}

#[derive(Debug, Serialize)]
struct WsRequestEnvelope {
    #[serde(rename = "type")]
    request_type: &'static str,
    payload: WsActionPayload,
}

#[derive(Debug, Serialize)]
struct WsActionPayload {
    action: Value,
    nonce: u64,
    signature: WsSignature,
    #[serde(skip_serializing_if = "Option::is_none", rename = "vaultAddress")]
    vault_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "expiresAfter")]
    expires_after: Option<u64>,
}

#[derive(Debug, Serialize)]
struct WsSignature {
    r: String,
    s: String,
    v: u8,
}

impl From<HyperliquidSignature> for WsSignature {
    fn from(signature: HyperliquidSignature) -> Self {
        Self {
            r: signature.r,
            s: signature.s,
            v: signature.v,
        }
    }
}

#[derive(Debug, Deserialize)]
struct WsPostResponse {
    data: WsPostData,
}

#[derive(Debug, Deserialize)]
struct WsInfoPostResponse {
    data: WsInfoPostData,
}

impl WsInfoPostResponse {
    fn matches_id(&self, id: u64) -> bool {
        self.data.id == id
    }

    fn into_result<T>(self, request_id: u64, request_type: &str) -> ExchangeResult<T>
    where
        T: DeserializeOwned,
    {
        if self.data.id != request_id {
            return Err(ExchangeError::Parse(format!(
                "hyperliquid ws info response id mismatch: request={request_id} response={}",
                self.data.id
            )));
        }
        self.data.response.into_result(request_type)
    }
}

#[derive(Debug, Deserialize)]
struct WsInfoPostData {
    id: u64,
    response: WsInfoResponse,
}

#[derive(Debug, Deserialize)]
struct WsInfoResponse {
    #[serde(rename = "type")]
    response_type: String,
    payload: Value,
}

impl WsInfoResponse {
    fn into_result<T>(self, request_type: &str) -> ExchangeResult<T>
    where
        T: DeserializeOwned,
    {
        if self.response_type == "error" {
            return Err(info_api_error(value_to_string(&self.payload)));
        }
        if self.response_type != REQUEST_TYPE_INFO {
            return Err(ExchangeError::Parse(format!(
                "hyperliquid unexpected post response type: {}",
                self.response_type
            )));
        }
        let response_type = self
            .payload
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ExchangeError::Parse("hyperliquid ws info payload missing type".into())
            })?;
        if response_type != request_type {
            return Err(ExchangeError::Parse(format!(
                "hyperliquid ws info type mismatch: request={request_type} response={response_type}"
            )));
        }
        let data = self.payload.get("data").cloned().ok_or_else(|| {
            ExchangeError::Parse(format!(
                "hyperliquid ws info {request_type} payload missing data"
            ))
        })?;
        serde_json::from_value(data).map_err(|error| {
            ExchangeError::Parse(format!("hyperliquid ws info {request_type} data: {error}"))
        })
    }
}

impl WsPostResponse {
    fn matches_id(&self, id: u64) -> bool {
        self.data.id == id
    }

    fn into_result(self, native_request_id: u64) -> ExchangeResult<OrderAckRow> {
        let native_response_id = self.data.id;
        self.data
            .response
            .into_result()
            .map(|row| row.with_native_ids(native_request_id, native_response_id))
    }
}

#[derive(Debug, Deserialize)]
struct WsPostData {
    id: u64,
    response: WsActionResponse,
}

#[derive(Debug, Deserialize)]
struct WsActionResponse {
    #[serde(rename = "type")]
    response_type: String,
    payload: WsActionResponsePayload,
}

impl WsActionResponse {
    fn into_result(self) -> ExchangeResult<OrderAckRow> {
        if self.response_type != REQUEST_TYPE_ACTION {
            return Err(ExchangeError::Parse(format!(
                "hyperliquid unexpected post response type: {}",
                self.response_type
            )));
        }
        self.payload.into_result()
    }
}

#[derive(Debug, Deserialize)]
struct WsActionResponsePayload {
    status: String,
    #[serde(default)]
    response: Option<Value>,
}

impl WsActionResponsePayload {
    fn into_result(self) -> ExchangeResult<OrderAckRow> {
        if self.status != "ok" {
            return Err(api_error(response_message(self.response.as_ref())));
        }
        if let Some(response) = self.response {
            return row_from_action_response(&response);
        }
        Ok(OrderAckRow {
            order_id: None,
            message: None,
            native_request_id: None,
            native_response_id: None,
        })
    }
}

fn row_from_action_response(response: &Value) -> ExchangeResult<OrderAckRow> {
    let statuses = response
        .get("data")
        .and_then(|data| data.get("statuses"))
        .and_then(Value::as_array);
    let Some(first) = statuses.and_then(|values| values.first()) else {
        return Ok(OrderAckRow {
            order_id: None,
            message: None,
            native_request_id: None,
            native_response_id: None,
        });
    };
    if let Some(error) = first.get("error").and_then(Value::as_str) {
        return Err(api_error(error.to_owned()));
    }
    if first.as_str() == Some("success") || first.get("success").is_some() {
        return Ok(OrderAckRow {
            order_id: None,
            message: Some("success".to_owned()),
            native_request_id: None,
            native_response_id: None,
        });
    }
    for key in ["resting", "filled"] {
        if let Some(row) = first.get(key) {
            return Ok(OrderAckRow {
                order_id: row.get("oid").map(value_to_string),
                message: Some(key.to_owned()),
                native_request_id: None,
                native_response_id: None,
            });
        }
    }
    Ok(OrderAckRow {
        order_id: None,
        message: Some(first.to_string()),
        native_request_id: None,
        native_response_id: None,
    })
}

fn response_message(response: Option<&Value>) -> String {
    response
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| response.map(Value::to_string))
        .unwrap_or_else(|| "hyperliquid action rejected".to_owned())
}

fn value_to_string(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}

fn api_error(message: String) -> ExchangeError {
    ExchangeError::Api {
        exchange: EXCHANGE.into(),
        code: "action".into(),
        message,
    }
}

fn info_api_error(message: String) -> ExchangeError {
    ExchangeError::Api {
        exchange: EXCHANGE.into(),
        code: "info".into(),
        message,
    }
}

struct OrderAckRow {
    order_id: Option<String>,
    message: Option<String>,
    native_request_id: Option<String>,
    native_response_id: Option<String>,
}

impl OrderAckRow {
    fn with_native_ids(mut self, request_id: u64, response_id: u64) -> Self {
        self.native_request_id = Some(request_id.to_string());
        self.native_response_id = Some(response_id.to_string());
        self
    }

    fn transport_metadata(&self) -> OrderTransportMetadata {
        let mut metadata =
            OrderTransportMetadata::default().with_native_transport("hyperliquid_ws_post");
        if let Some(request_id) = self.native_request_id.as_deref() {
            metadata = metadata.with_native_request_id(request_id);
        }
        if let Some(response_id) = self.native_response_id.as_deref() {
            metadata = metadata.with_native_response_id(response_id);
        }
        metadata
    }
}

#[cfg(test)]
#[path = "hyperliquid_ws_trade_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "hyperliquid_ws_info_tests.rs"]
mod info_tests;
