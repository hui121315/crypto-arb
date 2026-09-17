//! Binance USD-M Futures WebSocket request API.
//!
//! Official docs:
//! - New order: <https://developers.binance.com/docs/derivatives/usds-margined-futures/trade/websocket-api/New-Order>
//! - Cancel order: <https://developers.binance.com/docs/derivatives/usds-margined-futures/trade/websocket-api/Cancel-Order>
//! - Query order and positions: <https://developers.binance.com/en/docs/catalog/core-trading-derivatives-trading-usd-s-m-futures/api/ws-api/trade>
//! - Account and balance: <https://developers.binance.com/en/docs/catalog/core-trading-derivatives-trading-usd-s-m-futures/api/ws-api/account>
//! - User stream lifecycle: <https://developers.binance.com/en/docs/catalog/core-trading-derivatives-trading-usd-s-m-futures/api/ws-api/user-data-streams>
//! - General signing: <https://developers.binance.com/docs/derivatives/usds-margined-futures/websocket-api-general-info>

use super::binance_format::binance_time_in_force;
use super::binance_private_data::{
    parse_account_info, parse_balances, parse_open_order, parse_positions_with_mode, AccountInfoV3,
    BalanceItem, OpenOrderItem, ParsedAccountRead, ParsedPositions, PositionItem,
};
use super::binance_trade_data::{validate_client_order_id, validate_position_side};
use crate::error::{ExchangeError, ExchangeResult};
use crate::live::{venue_balance_rows, VenueAccountRead};
use crate::signing::binance as sign;
use crate::ws::trade_session::{session_key, WsSessionSpec, WsTradeSession};
use common::time::now_ms;
use dashmap::DashMap;
use serde::{de::DeserializeOwned, Deserialize};
use serde_json::{Map, Value};
use shared_types::{
    BalanceInfo, CancelOrderRequest, LiveOrderState, OrderAck, OrderInfo, OrderIntent, OrderSide,
    OrderStatus, OrderType, VenueOrderIdentityUpdate,
};
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

const EXCHANGE: &str = "binance";
const METHOD_ORDER_PLACE: &str = "order.place";
const METHOD_ORDER_CANCEL: &str = "order.cancel";
const METHOD_ORDER_STATUS: &str = "order.status";
const METHOD_ACCOUNT_STATUS_V2: &str = "v2/account.status";
const METHOD_ACCOUNT_BALANCE_V2: &str = "v2/account.balance";
const METHOD_ACCOUNT_POSITION_V2: &str = "v2/account.position";
const METHOD_USER_STREAM_START: &str = "userDataStream.start";
const METHOD_USER_STREAM_PING: &str = "userDataStream.ping";
const METHOD_USER_STREAM_STOP: &str = "userDataStream.stop";
const ACCOUNT_STATUS_SOURCE: &str = "binance.WSS v2/account.status";

#[derive(Debug, Clone, Copy)]
pub(super) struct WsTradeConfig<'a> {
    pub url: &'a str,
    pub api_key: &'a str,
    pub api_secret: &'a str,
    pub timeout_secs: u64,
    pub time_offset_ms: i64,
}

#[derive(Debug, Clone, Copy)]
struct WsSigningConfig<'a> {
    api_key: &'a str,
    api_secret: &'a str,
    time_offset_ms: i64,
}

#[cfg(test)]
impl<'a> WsTradeConfig<'a> {
    fn signing_values(self) -> WsSigningConfig<'a> {
        WsSigningConfig {
            api_key: self.api_key,
            api_secret: self.api_secret,
            time_offset_ms: self.time_offset_ms,
        }
    }
}

pub(super) async fn place_order(
    cfg: WsTradeConfig<'_>,
    intent: &OrderIntent,
    symbol: &str,
    position_side: &str,
) -> ExchangeResult<OrderAck> {
    validate_client_order_id(&intent.client_order_id)?;
    validate_position_side(intent, position_side)?;
    let internal_order_id = intent.id.clone();
    let client_order_id = intent.client_order_id.clone();
    let owned_intent = intent.clone();
    let owned_symbol = symbol.to_owned();
    let owned_position_side = position_side.to_owned();
    let api_key = cfg.api_key.to_owned();
    let api_secret = cfg.api_secret.to_owned();
    let time_offset_ms = cfg.time_offset_ms;
    let result = send_signed_request(cfg, internal_order_id.clone(), move || {
        order_place_request_with_signing(
            WsSigningConfig {
                api_key: &api_key,
                api_secret: &api_secret,
                time_offset_ms,
            },
            &owned_intent,
            &owned_symbol,
            &owned_position_side,
        )
    })
    .await?;
    Ok(ack_from_result(internal_order_id, client_order_id, &result))
}

pub(super) async fn cancel_order(
    cfg: WsTradeConfig<'_>,
    request: &CancelOrderRequest,
    symbol: &str,
) -> ExchangeResult<OrderAck> {
    let internal_order_id = request.internal_order_id.clone();
    let client_order_id = request.client_order_id.clone();
    let owned_request = request.clone();
    let owned_symbol = symbol.to_owned();
    let api_key = cfg.api_key.to_owned();
    let api_secret = cfg.api_secret.to_owned();
    let time_offset_ms = cfg.time_offset_ms;
    let result = send_signed_request(cfg, internal_order_id.clone(), move || {
        Ok(order_cancel_request_with_signing(
            WsSigningConfig {
                api_key: &api_key,
                api_secret: &api_secret,
                time_offset_ms,
            },
            &owned_request,
            &owned_symbol,
        ))
    })
    .await?;
    Ok(ack_from_result(internal_order_id, client_order_id, &result))
}

pub(super) async fn place_spot_order(
    cfg: WsTradeConfig<'_>,
    intent: &OrderIntent,
    symbol: &str,
) -> ExchangeResult<OrderAck> {
    validate_client_order_id(&intent.client_order_id)?;
    let internal_order_id = intent.id.clone();
    let client_order_id = intent.client_order_id.clone();
    let owned_intent = intent.clone();
    let owned_symbol = symbol.to_owned();
    let api_key = cfg.api_key.to_owned();
    let api_secret = cfg.api_secret.to_owned();
    let time_offset_ms = cfg.time_offset_ms;
    let result = send_signed_request(cfg, internal_order_id.clone(), move || {
        spot_order_place_request(
            WsSigningConfig {
                api_key: &api_key,
                api_secret: &api_secret,
                time_offset_ms,
            },
            &owned_intent,
            &owned_symbol,
        )
    })
    .await?;
    Ok(ack_from_result(internal_order_id, client_order_id, &result))
}

pub(super) async fn cancel_spot_order(
    cfg: WsTradeConfig<'_>,
    request: &CancelOrderRequest,
    symbol: &str,
) -> ExchangeResult<OrderAck> {
    validate_client_order_id(&request.client_order_id)?;
    let internal_order_id = request.internal_order_id.clone();
    let client_order_id = request.client_order_id.clone();
    let owned_request = request.clone();
    let owned_symbol = symbol.to_owned();
    let api_key = cfg.api_key.to_owned();
    let api_secret = cfg.api_secret.to_owned();
    let time_offset_ms = cfg.time_offset_ms;
    let result = send_signed_request(cfg, internal_order_id.clone(), move || {
        Ok(order_cancel_request_with_signing(
            WsSigningConfig {
                api_key: &api_key,
                api_secret: &api_secret,
                time_offset_ms,
            },
            &owned_request,
            &owned_symbol,
        ))
    })
    .await?;
    Ok(ack_from_result(internal_order_id, client_order_id, &result))
}

pub(super) async fn get_spot_order(
    cfg: WsTradeConfig<'_>,
    symbol: &str,
    client_order_id: &str,
) -> ExchangeResult<Option<OrderInfo>> {
    validate_client_order_id(client_order_id)?;
    let params = BTreeMap::from([
        ("origClientOrderId", client_order_id.to_owned()),
        ("symbol", symbol.to_owned()),
    ]);
    let item: SpotOrderResult = send_read_request(cfg, METHOD_ORDER_STATUS, params).await?;
    spot_order_info(item).map(Some)
}

pub(super) async fn get_spot_order_by_id(
    cfg: WsTradeConfig<'_>,
    symbol: &str,
    exchange_order_id: &str,
) -> ExchangeResult<Option<OrderInfo>> {
    let order_id = exchange_order_id
        .trim()
        .parse::<u64>()
        .map_err(|_| ExchangeError::Parse("binance spot orderId must be numeric".to_owned()))?;
    let params = BTreeMap::from([
        ("orderId", order_id.to_string()),
        ("symbol", symbol.to_owned()),
    ]);
    let item: SpotOrderResult = send_read_request(cfg, METHOD_ORDER_STATUS, params).await?;
    spot_order_info(item).map(Some)
}

pub(super) async fn account_read(
    cfg: WsTradeConfig<'_>,
    currency: Option<&str>,
    observed_at_ms: i64,
) -> ExchangeResult<VenueAccountRead> {
    let item: AccountInfoV3 =
        send_read_request(cfg, METHOD_ACCOUNT_STATUS_V2, BTreeMap::new()).await?;
    let ParsedAccountRead { balances, summary } =
        parse_account_info(item, currency, observed_at_ms, ACCOUNT_STATUS_SOURCE)?;
    Ok(VenueAccountRead {
        balances: venue_balance_rows(EXCHANGE, balances),
        summaries: vec![summary],
        asset_valuations: Vec::new(),
        issues: Vec::new(),
    })
}

pub(super) async fn balances(
    cfg: WsTradeConfig<'_>,
    currency: Option<&str>,
) -> ExchangeResult<HashMap<String, BalanceInfo>> {
    let items: Vec<BalanceItem> =
        send_read_request(cfg, METHOD_ACCOUNT_BALANCE_V2, BTreeMap::new()).await?;
    parse_balances(items, currency)
}

pub(super) async fn positions(
    cfg: WsTradeConfig<'_>,
    target_exchange_symbol: Option<&str>,
) -> ExchangeResult<ParsedPositions> {
    let mut params = BTreeMap::new();
    if let Some(symbol) = target_exchange_symbol {
        params.insert("symbol", symbol.to_owned());
    }
    let items: Vec<PositionItem> =
        send_read_request(cfg, METHOD_ACCOUNT_POSITION_V2, params).await?;
    parse_positions_with_mode(items, target_exchange_symbol)
}

pub(super) async fn get_order(
    cfg: WsTradeConfig<'_>,
    symbol: &str,
    client_order_id: &str,
) -> ExchangeResult<Option<OrderInfo>> {
    let params = BTreeMap::from([
        ("origClientOrderId", client_order_id.to_owned()),
        ("symbol", symbol.to_owned()),
    ]);
    let item: OpenOrderItem = send_read_request(cfg, METHOD_ORDER_STATUS, params).await?;
    parse_open_order(&item).map(Some)
}

pub(super) async fn start_user_data_stream(cfg: WsTradeConfig<'_>) -> ExchangeResult<String> {
    user_data_stream_listen_key(cfg, METHOD_USER_STREAM_START).await
}

pub(super) async fn keepalive_user_data_stream(cfg: WsTradeConfig<'_>) -> ExchangeResult<String> {
    user_data_stream_listen_key(cfg, METHOD_USER_STREAM_PING).await
}

pub(super) async fn close_user_data_stream(cfg: WsTradeConfig<'_>) -> ExchangeResult<()> {
    let _: Value = send_api_key_request(cfg, METHOD_USER_STREAM_STOP).await?;
    Ok(())
}

async fn user_data_stream_listen_key(
    cfg: WsTradeConfig<'_>,
    method: &'static str,
) -> ExchangeResult<String> {
    let result: UserDataStreamResult = send_api_key_request(cfg, method).await?;
    result
        .listen_key
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| ExchangeError::Parse(format!("binance ws {method} missing listenKey")))
}

async fn send_signed_request<F>(
    cfg: WsTradeConfig<'_>,
    request_id: String,
    build_request: F,
) -> ExchangeResult<WsOrderResult>
where
    F: FnOnce() -> ExchangeResult<WsRequest> + Send + 'static,
{
    let response_request_id = request_id.clone();
    let text = session(cfg)
        .send_fresh(
            move || {
                let request = build_request()?;
                if request.id != request_id {
                    return Err(ExchangeError::Parse(
                        "binance ws request id changed during fresh signing".into(),
                    ));
                }
                serde_json::to_string(&request)
                    .map_err(|error| ExchangeError::Parse(format!("binance ws request: {error}")))
            },
            Box::new(move |text| Ok(parse_response_id(text)? == response_request_id)),
        )
        .await?;
    parse_response(&text)?.into_result()
}

async fn send_read_request<T>(
    cfg: WsTradeConfig<'_>,
    method: &'static str,
    extra_params: BTreeMap<&'static str, String>,
) -> ExchangeResult<T>
where
    T: DeserializeOwned,
{
    let request_id = next_request_id();
    let response_request_id = request_id.clone();
    let api_key = cfg.api_key.to_owned();
    let api_secret = cfg.api_secret.to_owned();
    let time_offset_ms = cfg.time_offset_ms;
    let text = session(cfg)
        .send_fresh(
            move || {
                let request = signed_read_request(
                    WsSigningConfig {
                        api_key: &api_key,
                        api_secret: &api_secret,
                        time_offset_ms,
                    },
                    &request_id,
                    method,
                    extra_params,
                );
                serde_json::to_string(&request)
                    .map_err(|error| ExchangeError::Parse(format!("binance ws request: {error}")))
            },
            Box::new(move |text| Ok(parse_response_id(text)? == response_request_id)),
        )
        .await?;
    parse_typed_response(&text)?.into_result(method)
}

async fn send_api_key_request<T>(cfg: WsTradeConfig<'_>, method: &'static str) -> ExchangeResult<T>
where
    T: DeserializeOwned,
{
    let request_id = next_request_id();
    let response_request_id = request_id.clone();
    let api_key = cfg.api_key.to_owned();
    let text = session(cfg)
        .send_fresh(
            move || {
                serde_json::to_string(&api_key_request(&request_id, method, &api_key))
                    .map_err(|error| ExchangeError::Parse(format!("binance ws request: {error}")))
            },
            Box::new(move |text| Ok(parse_response_id(text)? == response_request_id)),
        )
        .await?;
    parse_typed_response(&text)?.into_result(method)
}

fn next_request_id() -> String {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    let sequence = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    format!("crossline-{}-{sequence}", now_ms())
}

fn signed_read_request(
    cfg: WsSigningConfig<'_>,
    request_id: &str,
    method: &'static str,
    extra_params: BTreeMap<&'static str, String>,
) -> WsRequest {
    let mut params = signed_base_params(cfg);
    params.extend(extra_params);
    signed_request(request_id, method, cfg.api_secret, params)
}

fn api_key_request(request_id: &str, method: &'static str, api_key: &str) -> WsRequest {
    WsRequest {
        id: request_id.to_owned(),
        method,
        params: Map::from_iter([("apiKey".to_owned(), Value::String(api_key.to_owned()))]),
    }
}

fn session(cfg: WsTradeConfig<'_>) -> WsTradeSession {
    static SESSIONS: OnceLock<DashMap<String, WsTradeSession>> = OnceLock::new();
    let key = session_key(&[EXCHANGE, cfg.url, cfg.api_key]);
    SESSIONS
        .get_or_init(DashMap::new)
        .entry(key)
        .or_insert_with(|| WsTradeSession::spawn(WsSessionSpec::new(cfg.url, cfg.timeout_secs)))
        .clone()
}

#[cfg(test)]
fn order_place_request(
    cfg: WsTradeConfig<'_>,
    intent: &OrderIntent,
    symbol: &str,
    position_side: &str,
) -> ExchangeResult<WsRequest> {
    order_place_request_with_signing(cfg.signing_values(), intent, symbol, position_side)
}

fn order_place_request_with_signing(
    cfg: WsSigningConfig<'_>,
    intent: &OrderIntent,
    symbol: &str,
    position_side: &str,
) -> ExchangeResult<WsRequest> {
    validate_client_order_id(&intent.client_order_id)?;
    validate_position_side(intent, position_side)?;
    let mut params = signed_base_params(cfg);
    params.insert("newClientOrderId", intent.client_order_id.clone());
    // Official ACK returns as soon as the venue accepts the request. Terminal fill remains
    // authoritative on the private ORDER_TRADE_UPDATE stream; RESULT would make MARKET orders
    // wait for FILLED and duplicates that finality path.
    params.insert("newOrderRespType", "ACK".to_owned());
    params.insert("positionSide", position_side.to_owned());
    params.insert("quantity", number_param(intent.quantity));
    params.insert("side", binance_side(intent.side).to_owned());
    params.insert("symbol", symbol.to_owned());
    params.insert("type", binance_order_type(intent.order_type).to_owned());
    add_order_type_params(&mut params, intent)?;
    if intent.reduce_only && position_side == "BOTH" {
        params.insert("reduceOnly", "true".to_owned());
    }
    Ok(signed_request(
        &intent.id,
        METHOD_ORDER_PLACE,
        cfg.api_secret,
        params,
    ))
}

fn spot_order_place_request(
    cfg: WsSigningConfig<'_>,
    intent: &OrderIntent,
    symbol: &str,
) -> ExchangeResult<WsRequest> {
    validate_client_order_id(&intent.client_order_id)?;
    let mut params = signed_base_params(cfg);
    params.insert("newClientOrderId", intent.client_order_id.clone());
    params.insert("newOrderRespType", "ACK".to_owned());
    params.insert("quantity", number_param(intent.quantity));
    params.insert("side", binance_side(intent.side).to_owned());
    params.insert("symbol", symbol.to_owned());
    params.insert(
        "type",
        binance_spot_order_type(intent.order_type).to_owned(),
    );
    add_spot_order_type_params(&mut params, intent)?;
    Ok(signed_request(
        &intent.id,
        METHOD_ORDER_PLACE,
        cfg.api_secret,
        params,
    ))
}

#[cfg(test)]
fn order_cancel_request(
    cfg: WsTradeConfig<'_>,
    request: &CancelOrderRequest,
    symbol: &str,
) -> WsRequest {
    order_cancel_request_with_signing(cfg.signing_values(), request, symbol)
}

fn order_cancel_request_with_signing(
    cfg: WsSigningConfig<'_>,
    request: &CancelOrderRequest,
    symbol: &str,
) -> WsRequest {
    let mut params = signed_base_params(cfg);
    params.insert("origClientOrderId", request.client_order_id.clone());
    params.insert("symbol", symbol.to_owned());
    signed_request(
        &request.internal_order_id,
        METHOD_ORDER_CANCEL,
        cfg.api_secret,
        params,
    )
}

fn signed_base_params(cfg: WsSigningConfig<'_>) -> BTreeMap<&'static str, String> {
    BTreeMap::from([
        ("apiKey", cfg.api_key.to_owned()),
        ("recvWindow", "5000".to_owned()),
        (
            "timestamp",
            now_ms().saturating_add(cfg.time_offset_ms).to_string(),
        ),
    ])
}

fn signed_request(
    id: &str,
    method: &'static str,
    secret: &str,
    mut params: BTreeMap<&'static str, String>,
) -> WsRequest {
    let payload = signing_payload(&params);
    params.insert("signature", sign::sign_query(secret.as_bytes(), &payload));
    WsRequest {
        id: id.to_owned(),
        method,
        params: ws_params(params),
    }
}

fn add_order_type_params(
    params: &mut BTreeMap<&'static str, String>,
    intent: &OrderIntent,
) -> ExchangeResult<()> {
    if matches!(intent.order_type, OrderType::Market) {
        return Ok(());
    }
    // 修复 P1 9.3：LIMIT / POST_ONLY 必须携 price，原实现 unwrap_or_default = 0.0 会
    // 被 Binance 拒为 `-1102 Mandatory parameter not sent`，但本地无早期信号。
    // 以 hyperliquid `required_price` 为参考实现同款校验：该 finite 且 > 0。
    let price = match intent.price {
        Some(p) if p.is_finite() && p > 0.0 => p,
        _ => {
            return Err(ExchangeError::Api {
                exchange: "binance".into(),
                code: "validation".into(),
                message: format!(
                    "binance ws trade {:?} order requires positive price; got {:?}",
                    intent.order_type, intent.price
                ),
            });
        }
    };
    params.insert("price", number_param(price));
    if let Some(time_in_force) = binance_time_in_force(intent.order_type, intent.time_in_force) {
        params.insert("timeInForce", time_in_force.to_owned());
    }
    Ok(())
}

fn add_spot_order_type_params(
    params: &mut BTreeMap<&'static str, String>,
    intent: &OrderIntent,
) -> ExchangeResult<()> {
    if matches!(intent.order_type, OrderType::Market) {
        return Ok(());
    }
    let price = match intent.price {
        Some(value) if value.is_finite() && value > 0.0 => value,
        _ => {
            return Err(ExchangeError::Api {
                exchange: EXCHANGE.into(),
                code: "validation".into(),
                message: "binance spot limit order requires positive price".to_owned(),
            });
        }
    };
    params.insert("price", number_param(price));
    if matches!(intent.order_type, OrderType::Limit) {
        let time_in_force = binance_time_in_force(intent.order_type, intent.time_in_force)
            .ok_or_else(|| ExchangeError::Parse("binance spot limit tif missing".to_owned()))?;
        params.insert("timeInForce", time_in_force.to_owned());
    }
    Ok(())
}

fn signing_payload(params: &BTreeMap<&'static str, String>) -> String {
    params
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("&")
}

fn ws_params(params: BTreeMap<&'static str, String>) -> Map<String, Value> {
    params
        .into_iter()
        .map(|(key, value)| (key.to_owned(), ws_param_value(key, value)))
        .collect()
}

fn ws_param_value(key: &str, value: String) -> Value {
    if matches!(key, "recvWindow" | "timestamp") {
        return value
            .parse::<u64>()
            .map(Value::from)
            .unwrap_or_else(|_| Value::String(value));
    }
    Value::String(value)
}

fn parse_response(text: &str) -> ExchangeResult<WsResponse> {
    serde_json::from_str(text).map_err(|error| {
        ExchangeError::Parse(format!("binance ws trade response: {error}; body={text}"))
    })
}

fn parse_response_id(text: &str) -> ExchangeResult<String> {
    serde_json::from_str::<WsResponseId>(text)
        .map(|response| response.id)
        .map_err(|error| {
            ExchangeError::Parse(format!("binance ws response id: {error}; body={text}"))
        })
}

fn parse_typed_response<T>(text: &str) -> ExchangeResult<TypedWsResponse<T>>
where
    T: DeserializeOwned,
{
    serde_json::from_str(text)
        .map_err(|error| ExchangeError::Parse(format!("binance ws response: {error}; body={text}")))
}

fn ack_from_result(
    internal_order_id: String,
    client_order_id: String,
    result: &WsOrderResult,
) -> OrderAck {
    let exchange_order_id = Some(result.order_id.to_string());
    OrderAck {
        internal_order_id,
        exchange_order_id: exchange_order_id.clone(),
        client_order_id: client_order_id.clone(),
        identity_update: VenueOrderIdentityUpdate::from_ids(
            client_order_id.clone(),
            client_order_id,
            exchange_order_id,
        ),
        state: live_state_from_status(&result.status),
        accepted_at_ms: now_ms(),
        message: None,
        filled_quantity: None,
        filled_price: None,
        filled_fee: None,
    }
}

fn binance_side(side: OrderSide) -> &'static str {
    match side {
        OrderSide::Buy => "BUY",
        OrderSide::Sell => "SELL",
    }
}

fn binance_order_type(order_type: OrderType) -> &'static str {
    match order_type {
        OrderType::Market => "MARKET",
        OrderType::Limit | OrderType::PostOnly => "LIMIT",
    }
}

fn binance_spot_order_type(order_type: OrderType) -> &'static str {
    match order_type {
        OrderType::Market => "MARKET",
        OrderType::Limit => "LIMIT",
        OrderType::PostOnly => "LIMIT_MAKER",
    }
}

fn spot_order_info(row: SpotOrderResult) -> ExchangeResult<OrderInfo> {
    let quantity = parse_spot_number("origQty", &row.orig_qty)?;
    let filled_quantity = parse_spot_number("executedQty", &row.executed_qty)?;
    let cumulative_quote = parse_spot_number("cummulativeQuoteQty", &row.cumulative_quote_qty)?;
    let filled_price = if filled_quantity > 0.0 {
        cumulative_quote / filled_quantity
    } else {
        0.0
    };
    let created_at = match chrono::DateTime::from_timestamp_millis(row.time) {
        Some(value) => value,
        None => chrono::Utc::now(),
    };
    Ok(OrderInfo {
        order_id: row.order_id.to_string(),
        symbol: row.symbol,
        exchange: EXCHANGE.to_owned(),
        side: match row.side.as_str() {
            "BUY" => OrderSide::Buy,
            "SELL" => OrderSide::Sell,
            value => return Err(ExchangeError::Parse(format!("binance spot side: {value}"))),
        },
        order_type: match row.order_type.as_str() {
            "MARKET" => OrderType::Market,
            "LIMIT_MAKER" => OrderType::PostOnly,
            "LIMIT" => OrderType::Limit,
            value => {
                return Err(ExchangeError::Parse(format!(
                    "binance spot order type: {value}"
                )));
            }
        },
        status: binance_order_status(&row.status),
        quantity,
        price: parse_spot_number("price", &row.price)?,
        filled_quantity,
        filled_price,
        fees: 0.0,
        created_at,
        execution_style: None,
        venue_time_in_force: Some(row.time_in_force),
        client_order_id: (!row.client_order_id.is_empty()).then_some(row.client_order_id),
        reduce_only: Some(false),
    })
}

fn parse_spot_number(field: &str, value: &str) -> ExchangeResult<f64> {
    value
        .parse::<f64>()
        .map_err(|error| ExchangeError::Parse(format!("binance spot {field}: {error}")))
}

fn binance_order_status(status: &str) -> OrderStatus {
    match status {
        "NEW" | "PENDING_NEW" => OrderStatus::Open,
        "PARTIALLY_FILLED" => OrderStatus::PartiallyFilled,
        "FILLED" => OrderStatus::Filled,
        "CANCELED" | "PENDING_CANCEL" => OrderStatus::Canceled,
        "REJECTED" => OrderStatus::Rejected,
        "EXPIRED" | "EXPIRED_IN_MATCH" => OrderStatus::Expired,
        _ => OrderStatus::Pending,
    }
}

/// 修复 P1 9.2：避免 `f64::to_string` 在小数字上输出科学计数法（如 `1e-8`），
/// Binance API 会以 `-1102 Mandatory parameter not sent` 拒绝。
/// 与 `binance::number_param` 同款：12 位小数 + trim 尾部 0/.，空串/`"-"` 归一为 `"0"`。
/// 文档：<https://binance-docs.github.io/apidocs/futures/en/#general-information-on-endpoints>
fn number_param(value: f64) -> String {
    let formatted = format!("{value:.12}");
    let trimmed = formatted.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() || trimmed == "-" {
        "0".to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn live_state_from_status(status: &str) -> LiveOrderState {
    match status.to_ascii_uppercase().as_str() {
        "NEW" => LiveOrderState::Accepted,
        "PARTIALLY_FILLED" => LiveOrderState::PartiallyFilled,
        "FILLED" => LiveOrderState::Filled,
        "CANCELED" | "CANCELLED" => LiveOrderState::Cancelled,
        "REJECTED" => LiveOrderState::Rejected,
        "EXPIRED" => LiveOrderState::Failed,
        _ => LiveOrderState::Unknown,
    }
}

#[derive(Debug, serde::Serialize)]
struct WsRequest {
    id: String,
    method: &'static str,
    params: Map<String, Value>,
}

#[derive(Debug, Deserialize)]
struct WsResponse {
    status: u16,
    #[serde(default)]
    result: Option<WsOrderResult>,
    #[serde(default)]
    error: Option<WsErrorBody>,
}

#[derive(Debug, Deserialize)]
struct WsResponseId {
    id: String,
}

#[derive(Debug, Deserialize)]
struct TypedWsResponse<T> {
    status: u16,
    result: Option<T>,
    error: Option<WsErrorBody>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UserDataStreamResult {
    #[serde(default)]
    listen_key: Option<String>,
}

impl<T> TypedWsResponse<T> {
    fn into_result(self, method: &str) -> ExchangeResult<T> {
        if (200..300).contains(&self.status) {
            return self.result.ok_or_else(|| {
                ExchangeError::Parse(format!("binance ws {method} missing result"))
            });
        }
        let error = self.error.unwrap_or_default();
        Err(ExchangeError::Api {
            exchange: EXCHANGE.into(),
            code: error.code.to_string(),
            message: error.msg,
        })
    }
}

impl WsResponse {
    fn into_result(self) -> ExchangeResult<WsOrderResult> {
        if (200..300).contains(&self.status) {
            return self
                .result
                .ok_or_else(|| ExchangeError::Parse("binance ws missing result".into()));
        }
        let error = self.error.unwrap_or_default();
        Err(ExchangeError::Api {
            exchange: EXCHANGE.into(),
            code: error.code.to_string(),
            message: error.msg,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WsOrderResult {
    order_id: u64,
    status: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SpotOrderResult {
    order_id: u64,
    #[serde(default)]
    client_order_id: String,
    symbol: String,
    status: String,
    #[serde(rename = "type")]
    order_type: String,
    side: String,
    #[serde(default)]
    price: String,
    #[serde(default)]
    orig_qty: String,
    #[serde(default)]
    executed_qty: String,
    #[serde(default, rename = "cummulativeQuoteQty")]
    cumulative_quote_qty: String,
    #[serde(default)]
    time_in_force: String,
    #[serde(default)]
    time: i64,
}

#[derive(Debug, Default, Deserialize)]
struct WsErrorBody {
    #[serde(default)]
    code: i64,
    #[serde(default)]
    msg: String,
}

#[cfg(test)]
#[path = "binance_ws_trade_tests.rs"]
mod tests;
