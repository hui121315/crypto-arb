//! Kraken Derivatives challenge-authenticated private WebSocket stream.

use super::kraken_config::{KrakenConfig, KrakenFuturesCredentials, FUTURES_WS_URL};
use super::kraken_futures_private_data::{parse_private_frame, FuturesPrivateFrame};
use crate::error::{ExchangeError, ExchangeResult};
use crate::live::PrivateWsRuntimeStatus;
use crate::signing::kraken::futures_ws_challenge_sign;
use crate::ws::manager::WsHeartbeat;
use crate::ws::trade_session::{session_key, WsChallengeLoginSpec, WsSessionSpec, WsTradeSession};
use arc_swap::ArcSwap;
use dashmap::DashMap;
use serde_json::{json, Value};
use shared_types::{OrderInfo, OrderStatus, PositionInfo, VenueBalanceInfo};
use std::cmp::Reverse;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, OnceLock, Weak};
use std::time::Duration;

const MAX_CACHED_ORDERS: usize = 4_096;

#[derive(Debug, Clone, Default)]
struct SessionAuth {
    challenge: String,
    signature: String,
}

#[derive(Debug, Default)]
struct PrivateState {
    orders: DashMap<String, OrderInfo>,
    balances: DashMap<String, VenueBalanceInfo>,
    positions: DashMap<String, PositionInfo>,
    balance_sequence: AtomicI64,
    fill_sequence: AtomicI64,
    order_sample: AtomicBool,
    balance_sample: AtomicBool,
    position_sample: AtomicBool,
    fill_sample: AtomicBool,
}

pub(super) struct KrakenFuturesPrivateStream {
    session: WsTradeSession,
    state: Arc<PrivateState>,
}

impl std::fmt::Debug for KrakenFuturesPrivateStream {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("KrakenFuturesPrivateStream")
            .field("orders", &self.state.orders.len())
            .field("balances", &self.state.balances.len())
            .field("positions", &self.state.positions.len())
            .finish_non_exhaustive()
    }
}

impl KrakenFuturesPrivateStream {
    pub(super) fn shared(config: &KrakenConfig) -> ExchangeResult<Arc<Self>> {
        static STREAMS: OnceLock<DashMap<String, Weak<KrakenFuturesPrivateStream>>> =
            OnceLock::new();
        let credentials = config
            .credentials
            .as_ref()
            .and_then(|credentials| credentials.futures.as_ref())
            .ok_or_else(|| ExchangeError::Auth("Kraken Futures credentials missing".to_owned()))?;
        let url = config
            .futures_ws_url_override
            .as_deref()
            .unwrap_or(FUTURES_WS_URL);
        let key = session_key(&[
            "kraken",
            "futures-private-v1",
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

    fn new(url: &str, credentials: &KrakenFuturesCredentials, timeout_secs: u64) -> Self {
        let state = Arc::new(PrivateState::default());
        let auth = Arc::new(ArcSwap::from_pointee(SessionAuth::default()));
        let challenge_credentials = credentials.clone();
        let challenge_auth = Arc::clone(&auth);
        let login = WsChallengeLoginSpec::new(
            challenge_response,
            move |text| build_first_subscription(text, &challenge_credentials, &challenge_auth),
            first_subscription_response,
        )
        .with_request({
            let credentials = credentials.clone();
            move || challenge_request(&credentials.api_key)
        });
        let mut spec = WsSessionSpec::new(url.to_owned(), timeout_secs)
            .with_challenge_login(login)
            .with_heartbeat(WsHeartbeat::Text(r#"{"event":"ping"}"#.to_owned()))
            .with_heartbeat_interval(Duration::from_secs(25))
            .with_heartbeat_response(|text| {
                serde_json::from_str::<Value>(text)
                    .ok()
                    .and_then(|value| {
                        value
                            .get("event")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                    })
                    .as_deref()
                    == Some("pong")
            });
        for feed in ["fills", "balances", "open_positions"] {
            let credentials = credentials.clone();
            let auth = Arc::clone(&auth);
            spec = spec.with_reconnect_payload(move || {
                subscription_request(feed, &credentials.api_key, auth.load().as_ref())
            });
        }
        let push_state = Arc::clone(&state);
        spec = spec.with_incoming_text_handler(move |text| apply_frame(&push_state, text));
        Self {
            session: WsTradeSession::spawn(spec),
            state,
        }
    }

    pub(super) async fn warm(&self) -> ExchangeResult<()> {
        self.session.warm().await
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
            .map(|row| row.value().clone())
    }

    pub(super) fn open_orders(&self, symbol: Option<&str>) -> Vec<OrderInfo> {
        let mut rows = self
            .state
            .orders
            .iter()
            .filter(|entry| {
                matches!(
                    entry.status,
                    OrderStatus::Pending | OrderStatus::Open | OrderStatus::PartiallyFilled
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
            .filter(|entry| {
                currency.is_none_or(|currency| entry.currency.eq_ignore_ascii_case(currency))
            })
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
            .filter(|entry| symbol.is_none_or(|symbol| entry.symbol.eq_ignore_ascii_case(symbol)))
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
            subscriptions: 4,
            account_streams: 2,
            account_samples: usize::from(self.has_balance_sample())
                + usize::from(self.has_position_sample()),
            order_streams: 2,
            order_samples: usize::from(self.has_order_sample())
                + usize::from(self.has_fill_sample()),
        }
    }
}

fn apply_frame(state: &PrivateState, text: &str) -> ExchangeResult<()> {
    reject_protocol_error(text)?;
    let Some(frame) = parse_private_frame(text)? else {
        return Ok(());
    };
    match frame {
        FuturesPrivateFrame::OrdersSnapshot(rows) => {
            state.orders.clear();
            for row in rows {
                state.orders.insert(row.order_id.clone(), row);
            }
            state.order_sample.store(true, Ordering::Release);
        }
        FuturesPrivateFrame::OrderDelta {
            order,
            order_id,
            terminal_status,
        } => {
            if let Some(order) = order {
                state.orders.insert(order.order_id.clone(), order);
            } else if let Some(status) = terminal_status {
                if let Some(mut row) = state.orders.get_mut(&order_id) {
                    row.status = status;
                }
            }
            state.order_sample.store(true, Ordering::Release);
        }
        FuturesPrivateFrame::Fills(mut fills) => {
            fills.sort_by_key(|fill| fill.sequence);
            for fill in fills {
                let previous = state.fill_sequence.load(Ordering::Acquire);
                if fill.sequence <= previous {
                    continue;
                }
                if let Some(mut order) = state.orders.get_mut(&fill.order_id) {
                    if order.quantity <= 0.0 {
                        order.quantity = fill.quantity + fill.remaining_quantity;
                    }
                    order.filled_quantity = (order.quantity - fill.remaining_quantity)
                        .max(fill.quantity)
                        .max(0.0);
                    order.filled_price = fill.price;
                    order.fees += fill.fee;
                    if fill.client_order_id.is_some() {
                        order.client_order_id = fill.client_order_id;
                    }
                    order.status = if fill.remaining_quantity <= 0.0 {
                        OrderStatus::Filled
                    } else {
                        OrderStatus::PartiallyFilled
                    };
                }
                state.fill_sequence.store(fill.sequence, Ordering::Release);
            }
            state.fill_sample.store(true, Ordering::Release);
        }
        FuturesPrivateFrame::Balances {
            snapshot,
            sequence,
            rows,
        } => {
            if !accept_balance_sequence(state, snapshot, sequence)? {
                return Ok(());
            }
            if snapshot {
                state.balances.clear();
            }
            for row in rows {
                state.balances.insert(row.currency.clone(), row);
            }
            state.balance_sample.store(true, Ordering::Release);
        }
        FuturesPrivateFrame::Positions(rows) => {
            state.positions.clear();
            for row in rows {
                state.positions.insert(position_key(&row), row);
            }
            state.position_sample.store(true, Ordering::Release);
        }
    }
    trim_terminal_orders(state);
    Ok(())
}

fn accept_balance_sequence(
    state: &PrivateState,
    snapshot: bool,
    next: i64,
) -> ExchangeResult<bool> {
    let previous = state.balance_sequence.load(Ordering::Acquire);
    if snapshot {
        state.balance_sequence.store(next, Ordering::Release);
        return Ok(true);
    }
    if state.balance_sample.load(Ordering::Acquire) && next <= previous {
        return Ok(false);
    }
    if !state.balance_sample.load(Ordering::Acquire) || next != previous + 1 {
        return Err(ExchangeError::Parse(format!(
            "kraken futures balance sequence gap: previous={previous}, next={next}"
        )));
    }
    state.balance_sequence.store(next, Ordering::Release);
    Ok(true)
}

fn position_key(row: &PositionInfo) -> String {
    format!("{}:{}", row.symbol, row.side)
}

fn trim_terminal_orders(state: &PrivateState) {
    if state.orders.len() <= MAX_CACHED_ORDERS {
        return;
    }
    let mut terminal = state
        .orders
        .iter()
        .filter(|row| {
            !matches!(
                row.status,
                OrderStatus::Pending | OrderStatus::Open | OrderStatus::PartiallyFilled
            )
        })
        .map(|row| (row.order_id.clone(), row.created_at))
        .collect::<Vec<_>>();
    terminal.sort_by_key(|(_, created_at)| *created_at);
    for (order_id, _) in terminal
        .into_iter()
        .take(state.orders.len() - MAX_CACHED_ORDERS)
    {
        state.orders.remove(&order_id);
    }
}

fn challenge_request(api_key: &str) -> ExchangeResult<String> {
    serde_json::to_string(&json!({ "event": "challenge", "api_key": api_key }))
        .map_err(|error| ExchangeError::Parse(format!("kraken challenge request: {error}")))
}

fn challenge_response(text: &str) -> ExchangeResult<bool> {
    let value: Value = serde_json::from_str(text)
        .map_err(|error| ExchangeError::Parse(format!("kraken challenge response: {error}")))?;
    if value.get("event").and_then(Value::as_str) == Some("error") {
        return Err(protocol_error(&value));
    }
    Ok(
        value.get("event").and_then(Value::as_str) == Some("challenge")
            && value.get("message").and_then(Value::as_str).is_some(),
    )
}

fn build_first_subscription(
    text: &str,
    credentials: &KrakenFuturesCredentials,
    auth: &ArcSwap<SessionAuth>,
) -> ExchangeResult<String> {
    let value: Value = serde_json::from_str(text)
        .map_err(|error| ExchangeError::Parse(format!("kraken challenge json: {error}")))?;
    let challenge = value
        .get("message")
        .and_then(Value::as_str)
        .ok_or_else(|| ExchangeError::Parse("kraken challenge message missing".to_owned()))?;
    let signature =
        futures_ws_challenge_sign(&credentials.api_secret, challenge).map_err(|error| {
            ExchangeError::Auth(format!("kraken futures challenge signing: {error}"))
        })?;
    let session_auth = SessionAuth {
        challenge: challenge.to_owned(),
        signature,
    };
    let request = subscription_request("open_orders", &credentials.api_key, &session_auth)?;
    auth.store(Arc::new(session_auth));
    Ok(request)
}

fn first_subscription_response(text: &str) -> ExchangeResult<bool> {
    let value: Value = serde_json::from_str(text)
        .map_err(|error| ExchangeError::Parse(format!("kraken subscription response: {error}")))?;
    match value.get("event").and_then(Value::as_str) {
        Some("subscribed") if value.get("feed").and_then(Value::as_str) == Some("open_orders") => {
            Ok(true)
        }
        Some("error" | "subscribed_failed") => Err(protocol_error(&value)),
        _ => Ok(false),
    }
}

fn subscription_request(feed: &str, api_key: &str, auth: &SessionAuth) -> ExchangeResult<String> {
    if auth.challenge.is_empty() || auth.signature.is_empty() {
        return Err(ExchangeError::Auth(
            "Kraken Futures challenge state missing".to_owned(),
        ));
    }
    serde_json::to_string(&json!({
        "event": "subscribe",
        "feed": feed,
        "api_key": api_key,
        "original_challenge": auth.challenge,
        "signed_challenge": auth.signature
    }))
    .map_err(|error| ExchangeError::Parse(format!("kraken {feed} subscription: {error}")))
}

fn reject_protocol_error(text: &str) -> ExchangeResult<()> {
    let value: Value = serde_json::from_str(text)
        .map_err(|error| ExchangeError::Parse(format!("kraken futures frame: {error}")))?;
    if matches!(
        value.get("event").and_then(Value::as_str),
        Some("error" | "subscribed_failed")
    ) {
        return Err(protocol_error(&value));
    }
    Ok(())
}

fn protocol_error(value: &Value) -> ExchangeError {
    ExchangeError::Api {
        exchange: "kraken".to_owned(),
        code: value
            .get("event")
            .and_then(Value::as_str)
            .unwrap_or("ws_error")
            .to_owned(),
        message: value
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("private websocket error")
            .to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn challenge_and_subscription_follow_official_contract() {
        let credentials = KrakenFuturesCredentials {
            api_key: "key".to_owned(),
            api_secret: "7zxMEF5p/Z8l2p2U7Ghv6x14Af+Fx+92tPgUdVQ748FOIrEoT9bgT+bTRfXc5pz8na+hL/QdrCVG7bh9KpT0eMTm".to_owned(),
        };
        let auth = ArcSwap::from_pointee(SessionAuth::default());
        let request = build_first_subscription(
            r#"{"event":"challenge","message":"c100b894-1729-464d-ace1-52dbce11db42"}"#,
            &credentials,
            &auth,
        )
        .unwrap();
        let request: Value = serde_json::from_str(&request).unwrap();
        assert_eq!(request["feed"], "open_orders");
        assert_eq!(request["signed_challenge"], "4JEpF3ix66GA2B+ooK128Ift4XQVtc137N9yeg4Kqsn9PI0Kpzbysl9M1IeCEdjg0zl00wkVqcsnG4bmnlMb3A==");
    }

    #[test]
    fn balance_sequence_rejects_gap_after_snapshot() {
        let state = PrivateState::default();
        assert!(accept_balance_sequence(&state, true, 4).unwrap());
        state.balance_sample.store(true, Ordering::Release);
        assert!(accept_balance_sequence(&state, false, 5).unwrap());
        assert!(accept_balance_sequence(&state, false, 7).is_err());
    }
}
