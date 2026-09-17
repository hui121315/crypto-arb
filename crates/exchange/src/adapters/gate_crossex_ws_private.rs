//! Persistent Gate `CrossEx` private stream and account-trade requests.

use super::gate_crossex_config::{
    GateCrossExConfig, GateCrossExCredentials, CROSSEX_PRIVATE_WS_URL,
};
use super::gate_crossex_private_data::{
    ack_from_api, is_api_response_for, parse_api_ack, parse_private_push, CompiledOrder,
    FillUpdate, PrivatePush,
};
use crate::error::{ExchangeError, ExchangeResult};
use crate::live::PrivateWsRuntimeStatus;
use crate::signing::gate::ws_api_sign;
use crate::ws::trade_session::{session_key, WsLoginSpec, WsSessionSpec, WsTradeSession};
use dashmap::DashMap;
use serde::Serialize;
use serde_json::Value;
use shared_types::{
    CancelOrderRequest, OrderAck, OrderInfo, OrderIntent, PositionInfo, VenueBalanceInfo,
};
use std::cmp::Reverse;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, Weak};
use std::time::Duration;

const EXCHANGE: &str = "gate_crossex";
const CHANNELS: &[&str] = &["order", "asset", "usertrades", "position"];
const EVENT_API: &str = "api";
const EVENT_LOGIN: &str = "login";
const EVENT_SUBSCRIBE: &str = "subscribe";
const CHANNEL_PLACE: &str = "place_order";
const CHANNEL_CANCEL: &str = "cancel_order";

#[derive(Debug, Default)]
struct PrivateState {
    orders: DashMap<String, OrderInfo>,
    balances: DashMap<String, VenueBalanceInfo>,
    positions: DashMap<String, PositionInfo>,
    fills: DashMap<String, FillUpdate>,
    fill_totals: DashMap<String, FillAggregate>,
    order_sample: AtomicBool,
    balance_sample: AtomicBool,
    position_sample: AtomicBool,
    fill_sample: AtomicBool,
}

#[derive(Debug, Clone, Copy, Default)]
struct FillAggregate {
    quantity: f64,
    notional: f64,
    fees: f64,
}

pub(super) struct GateCrossExPrivateStream {
    session: WsTradeSession,
    state: Arc<PrivateState>,
}

impl std::fmt::Debug for GateCrossExPrivateStream {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GateCrossExPrivateStream")
            .field("orders", &self.state.orders.len())
            .field("balances", &self.state.balances.len())
            .field("positions", &self.state.positions.len())
            .finish_non_exhaustive()
    }
}

impl GateCrossExPrivateStream {
    pub(super) fn shared(config: &GateCrossExConfig) -> ExchangeResult<Arc<Self>> {
        static STREAMS: OnceLock<DashMap<String, Weak<GateCrossExPrivateStream>>> = OnceLock::new();
        let credentials = config
            .credentials
            .as_ref()
            .ok_or_else(|| ExchangeError::Auth("Gate CrossEx credentials missing".to_owned()))?;
        let url = config
            .private_ws_url_override
            .as_deref()
            .unwrap_or(CROSSEX_PRIVATE_WS_URL);
        let key = session_key(&[
            EXCHANGE,
            "private",
            url,
            &credentials.api_key,
            &credentials.api_secret,
        ]);
        let streams = STREAMS.get_or_init(DashMap::new);
        if let Some(stream) = streams.get(&key).and_then(|entry| entry.value().upgrade()) {
            return Ok(stream);
        }
        let stream = Arc::new(Self::new(url, credentials, config.timeout_secs));
        let mut entry = streams.entry(key).or_default();
        if let Some(existing) = entry.value().upgrade() {
            return Ok(existing);
        }
        *entry.value_mut() = Arc::downgrade(&stream);
        Ok(stream)
    }

    fn new(url: &str, credentials: &GateCrossExCredentials, timeout_secs: u64) -> Self {
        let state = Arc::new(PrivateState::default());
        let login_credentials = credentials.clone();
        let mut spec = WsSessionSpec::new(url.to_owned(), timeout_secs)
            .with_login(WsLoginSpec::new(
                move || login_request(&login_credentials),
                login_response,
            ))
            .with_heartbeat_interval(Duration::from_secs(20));
        for channel in CHANNELS {
            let channel = (*channel).to_owned();
            spec = spec.with_reconnect_payload(move || subscription_request(&channel));
        }
        let push_state = Arc::clone(&state);
        spec = spec.with_incoming_text_handler(move |text| apply_push(&push_state, text));
        Self {
            session: WsTradeSession::spawn(spec),
            state,
        }
    }

    pub(super) async fn warm(&self) -> ExchangeResult<()> {
        self.session.warm().await
    }

    pub(super) async fn place_order(
        &self,
        intent: &OrderIntent,
        compiled: &CompiledOrder,
    ) -> ExchangeResult<OrderAck> {
        let request_id = next_request_id("place");
        let matcher_id = request_id.clone();
        let response_id = matcher_id.clone();
        let compiled_for_request = compiled.clone();
        let response = self
            .session
            .send_fresh(
                move || api_request(CHANNEL_PLACE, &request_id, &compiled_for_request),
                Box::new(move |text| is_api_response_for(text, CHANNEL_PLACE, &matcher_id)),
            )
            .await?;
        let ack = parse_api_ack(&response, CHANNEL_PLACE, &response_id)?;
        Ok(ack_from_api(intent, compiled, ack))
    }

    pub(super) async fn cancel_order(
        &self,
        request: &CancelOrderRequest,
    ) -> ExchangeResult<OrderAck> {
        let target = request
            .exchange_order_id
            .as_deref()
            .unwrap_or(&request.client_order_id)
            .to_owned();
        let request_id = next_request_id("cancel");
        let matcher_id = request_id.clone();
        let response_id = matcher_id.clone();
        let response = self
            .session
            .send_fresh(
                move || api_request(CHANNEL_CANCEL, &request_id, &target),
                Box::new(move |text| is_api_response_for(text, CHANNEL_CANCEL, &matcher_id)),
            )
            .await?;
        let ack = parse_api_ack(&response, CHANNEL_CANCEL, &response_id)?;
        let exchange_order_id = request.exchange_order_id.clone();
        Ok(OrderAck {
            internal_order_id: request.internal_order_id.clone(),
            exchange_order_id: exchange_order_id.clone(),
            client_order_id: request.client_order_id.clone(),
            identity_update: shared_types::VenueOrderIdentityUpdate::from_ids(
                request.client_order_id.clone(),
                request.client_order_id.clone(),
                exchange_order_id,
            ),
            state: shared_types::LiveOrderState::CancelRequested,
            accepted_at_ms: common::time::now_ms(),
            message: Some(ack.message),
            filled_quantity: None,
            filled_price: None,
            filled_fee: None,
        })
    }

    pub(super) fn order_by_client_id(&self, client_order_id: &str) -> Option<OrderInfo> {
        self.state
            .orders
            .iter()
            .find(|entry| entry.client_order_id.as_deref() == Some(client_order_id))
            .map(|entry| entry.value().clone())
    }

    pub(super) fn order_by_exchange_id(&self, exchange_order_id: &str) -> Option<OrderInfo> {
        self.state
            .orders
            .get(exchange_order_id)
            .map(|entry| entry.value().clone())
    }

    pub(super) fn open_orders(&self, symbol: Option<&str>) -> Vec<OrderInfo> {
        let mut rows = self
            .state
            .orders
            .iter()
            .filter(|entry| {
                matches!(
                    entry.status,
                    shared_types::OrderStatus::Open | shared_types::OrderStatus::PartiallyFilled
                ) && symbol.is_none_or(|symbol| entry.symbol.eq_ignore_ascii_case(symbol))
            })
            .map(|entry| entry.value().clone())
            .collect::<Vec<_>>();
        rows.sort_by_key(|row| Reverse(row.created_at));
        rows
    }

    pub(super) fn balances(&self, currency: Option<&str>) -> Vec<VenueBalanceInfo> {
        let mut rows = self
            .state
            .balances
            .iter()
            .filter(|entry| currency.is_none_or(|value| entry.currency.eq_ignore_ascii_case(value)))
            .map(|entry| entry.value().clone())
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| left.currency.cmp(&right.currency));
        rows
    }

    pub(super) fn positions(&self, symbol: Option<&str>) -> Vec<PositionInfo> {
        let mut rows = self
            .state
            .positions
            .iter()
            .filter(|entry| symbol.is_none_or(|value| entry.symbol.eq_ignore_ascii_case(value)))
            .map(|entry| entry.value().clone())
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| left.symbol.cmp(&right.symbol));
        rows
    }

    pub(super) fn has_order_sample(&self) -> bool {
        self.state.order_sample.load(Ordering::Acquire)
    }

    pub(super) fn has_balance_sample(&self) -> bool {
        self.state.balance_sample.load(Ordering::Acquire)
    }

    pub(super) fn has_position_sample(&self) -> bool {
        self.state.position_sample.load(Ordering::Acquire)
    }

    pub(super) fn has_fill_sample(&self) -> bool {
        self.state.fill_sample.load(Ordering::Acquire)
    }

    pub(super) fn runtime_status(&self) -> PrivateWsRuntimeStatus {
        PrivateWsRuntimeStatus {
            sessions: 1,
            subscriptions: CHANNELS.len(),
            account_streams: 2,
            account_samples: usize::from(self.has_balance_sample())
                + usize::from(self.has_position_sample()),
            order_streams: 2,
            order_samples: usize::from(self.has_order_sample())
                + usize::from(self.has_fill_sample()),
        }
    }
}

fn apply_push(state: &PrivateState, text: &str) -> ExchangeResult<()> {
    let Some(push) = parse_private_push(text)? else {
        return Ok(());
    };
    match push {
        PrivatePush::Order(order) => {
            let order_id = order.order_id.clone();
            state.orders.insert(order_id.clone(), order);
            apply_fill_total(state, &order_id);
            state.order_sample.store(true, Ordering::Release);
        }
        PrivatePush::Balance(balance) => {
            state
                .balances
                .insert(format!("{}:{}", balance.venue, balance.currency), balance);
            state.balance_sample.store(true, Ordering::Release);
        }
        PrivatePush::Position(update) => {
            if let Some(position) = update.row {
                state.positions.insert(update.key, position);
            } else {
                state.positions.remove(&update.key);
            }
            state.position_sample.store(true, Ordering::Release);
        }
        PrivatePush::Fill(fill) => {
            if state
                .fills
                .insert(fill.transaction_id.clone(), fill.clone())
                .is_none()
            {
                let mut aggregate = state.fill_totals.entry(fill.order_id.clone()).or_default();
                aggregate.quantity += fill.filled_quantity;
                aggregate.notional += fill.filled_quantity * fill.filled_price;
                aggregate.fees += fill.fee;
                drop(aggregate);
                apply_fill_total(state, &fill.order_id);
            }
            state.fill_sample.store(true, Ordering::Release);
        }
    }
    Ok(())
}

fn apply_fill_total(state: &PrivateState, order_id: &str) {
    let Some(aggregate) = state.fill_totals.get(order_id) else {
        return;
    };
    let Some(mut order) = state.orders.get_mut(order_id) else {
        return;
    };
    let filled_quantity = if order.quantity > 0.0 {
        aggregate.quantity.min(order.quantity)
    } else {
        aggregate.quantity
    };
    if filled_quantity > order.filled_quantity {
        order.filled_quantity = filled_quantity;
        if aggregate.quantity > 0.0 {
            order.filled_price = aggregate.notional / aggregate.quantity;
        }
    }
    if aggregate.fees.abs() > order.fees.abs() {
        order.fees = aggregate.fees;
    }
    if order.quantity > 0.0 && order.filled_quantity >= order.quantity {
        order.status = shared_types::OrderStatus::Filled;
    } else if order.filled_quantity > 0.0 {
        order.status = shared_types::OrderStatus::PartiallyFilled;
    }
}

fn login_request(credentials: &GateCrossExCredentials) -> ExchangeResult<String> {
    let time = common::time::now_secs();
    let sign = ws_api_sign(
        credentials.api_secret.as_bytes(),
        EVENT_LOGIN,
        "",
        "",
        &time.to_string(),
    );
    serde_json::to_string(&LoginRequest {
        time,
        event: EVENT_LOGIN,
        request_id: next_request_id("login"),
        payload: LoginPayload {
            method: "api_key",
            api_key: &credentials.api_key,
            sign,
        },
    })
    .map_err(|error| ExchangeError::Parse(format!("Gate CrossEx login request: {error}")))
}

fn login_response(text: &str) -> ExchangeResult<bool> {
    let value: Value = serde_json::from_str(text)
        .map_err(|error| ExchangeError::Parse(format!("Gate CrossEx login response: {error}")))?;
    if value.get("event").and_then(Value::as_str) != Some(EVENT_LOGIN) {
        return Ok(false);
    }
    let code = value
        .pointer("/result/code")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if code != super::gate_crossex_private_data::SUCCESS_CODE {
        return Err(ExchangeError::Api {
            exchange: EXCHANGE.to_owned(),
            code: code.to_owned(),
            message: value
                .pointer("/result/message")
                .and_then(Value::as_str)
                .unwrap_or("login failed")
                .to_owned(),
        });
    }
    Ok(true)
}

fn subscription_request(channel: &str) -> ExchangeResult<String> {
    serde_json::to_string(&SubscriptionRequest {
        time: common::time::now_secs(),
        event: EVENT_SUBSCRIBE,
        channel,
        request_id: next_request_id("subscribe"),
        payload: ["!all"],
    })
    .map_err(|error| ExchangeError::Parse(format!("Gate CrossEx subscription: {error}")))
}

fn api_request<T: Serialize>(
    channel: &str,
    request_id: &str,
    payload: &T,
) -> ExchangeResult<String> {
    serde_json::to_string(&ApiRequest {
        time: common::time::now_secs(),
        event: EVENT_API,
        channel,
        request_id,
        payload,
    })
    .map_err(|error| ExchangeError::Parse(format!("Gate CrossEx api request: {error}")))
}

fn next_request_id(operation: &str) -> String {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    format!(
        "crossline-{operation}-{:x}-{:x}",
        common::time::now_ms(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

#[derive(Serialize)]
struct LoginRequest<'a> {
    time: i64,
    event: &'static str,
    request_id: String,
    payload: LoginPayload<'a>,
}

#[derive(Serialize)]
struct LoginPayload<'a> {
    method: &'static str,
    api_key: &'a str,
    sign: String,
}

#[derive(Serialize)]
struct SubscriptionRequest<'a> {
    time: i64,
    event: &'static str,
    channel: &'a str,
    request_id: String,
    payload: [&'static str; 1],
}

#[derive(Serialize)]
struct ApiRequest<'a, T> {
    time: i64,
    event: &'static str,
    channel: &'a str,
    request_id: &'a str,
    payload: &'a T,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_and_subscriptions_match_official_envelope() {
        let login = login_request(&GateCrossExCredentials {
            api_key: "key".to_owned(),
            api_secret: "secret".to_owned(),
        })
        .unwrap();
        let login: Value = serde_json::from_str(&login).unwrap();
        assert_eq!(login["event"], "login");
        assert_eq!(login["payload"]["method"], "api_key");
        assert_eq!(login["payload"]["sign"].as_str().unwrap().len(), 128);

        let subscription: Value =
            serde_json::from_str(&subscription_request("position").unwrap()).unwrap();
        assert_eq!(subscription["channel"], "position");
        assert_eq!(subscription["payload"][0], "!all");
    }

    #[test]
    fn trade_requests_match_official_envelope() {
        let compiled = CompiledOrder {
            text: "cx-0123456789ab".to_owned(),
            symbol: "GATE_FUTURE_ETH_USDT".to_owned(),
            side: "BUY",
            order_type: "LIMIT",
            time_in_force: "GTC",
            qty: Some("0.01".to_owned()),
            quote_qty: None,
            price: Some("2800".to_owned()),
            reduce_only: false,
            position_side: "NONE",
        };
        let place: Value =
            serde_json::from_str(&api_request(CHANNEL_PLACE, "request-1", &compiled).unwrap())
                .unwrap();
        assert_eq!(place["event"], "api");
        assert_eq!(place["channel"], "place_order");
        assert_eq!(place["payload"]["symbol"], "GATE_FUTURE_ETH_USDT");
        assert_eq!(place["payload"]["qty"], "0.01");

        let cancel: Value = serde_json::from_str(
            &api_request(CHANNEL_CANCEL, "request-2", &"2065058175870464").unwrap(),
        )
        .unwrap();
        assert_eq!(cancel["event"], "api");
        assert_eq!(cancel["channel"], "cancel_order");
        assert_eq!(cancel["payload"], "2065058175870464");
    }

    #[test]
    fn push_cache_removes_flat_position() {
        let state = PrivateState::default();
        apply_push(
            &state,
            include_str!("../../fixtures/gate_crossex/private_position_update.json"),
        )
        .unwrap();
        assert_eq!(state.positions.len(), 1);
        assert!(state.position_sample.load(Ordering::Acquire));
        assert!(!state.order_sample.load(Ordering::Acquire));
    }

    #[test]
    fn duplicate_fill_is_applied_once_to_cached_order() {
        let state = PrivateState::default();
        apply_push(
            &state,
            include_str!("../../fixtures/gate_crossex/private_order_update.json"),
        )
        .unwrap();
        let fill = serde_json::json!({
            "channel": "usertrades",
            "event": "subscribe",
            "payload": {
                "transaction_id": "fill-1",
                "order_id": "2048529119934720",
                "qty": "6",
                "price": "0.6",
                "fee": "0.2"
            },
            "result": { "code": "100000", "message": "success" },
            "time": 1756434168,
            "time_ms": 1_756_434_168_984_i64
        })
        .to_string();

        apply_push(&state, &fill).unwrap();
        apply_push(&state, &fill).unwrap();

        let order = state.orders.get("2048529119934720").unwrap();
        assert_eq!(order.filled_quantity, 6.0);
        assert_eq!(order.filled_price, 0.6);
        assert_eq!(order.fees, 0.2);
        assert_eq!(order.status, shared_types::OrderStatus::PartiallyFilled);
        assert_eq!(state.fills.len(), 1);
    }
}
