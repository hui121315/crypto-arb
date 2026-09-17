//! Kraken Spot WebSocket v2 authenticated account and order stream.

use super::kraken_config::{KrakenSpotCredentials, SPOT_PRIVATE_WS_URL, SPOT_REST_URL};
use super::kraken_spot_private_data::{
    parse_private_frame, FrameKind, KrakenSpotExecution, SpotPrivateFrame,
};
use super::kraken_spot_rest::fetch_ws_token;
use super::kraken_symbols::spot_symbol;
use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use crate::live::PrivateWsRuntimeStatus;
use crate::ws::trade_session::{session_key, WsSessionSpec, WsTradeSession};
use arc_swap::ArcSwap;
use dashmap::DashMap;
use serde_json::{json, Value};
use shared_types::{
    CancelOrderRequest, LiveOrderState, OrderAck, OrderInfo, OrderIntent, OrderSide, OrderStatus,
    OrderType, TimeInForce, VenueBalanceInfo, VenueOrderIdentityUpdate,
};
use std::cmp::Reverse;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;
use tokio::sync::broadcast;

const VENUE: &str = "kraken:spot";
const MAX_CACHED_ORDERS: usize = 4_096;

#[path = "kraken_stock_receipts/submission.rs"]
mod stock_submission;

#[derive(Debug)]
struct PrivateState {
    stocks: super::kraken_stock_receipts::StockReceipts,
    orders: DashMap<String, OrderInfo>,
    balances: DashMap<String, VenueBalanceInfo>,
    order_sequence: AtomicI64,
    balance_sequence: AtomicI64,
    orders_observed_at_ms: AtomicI64,
    balances_observed_at_ms: AtomicI64,
    executions: broadcast::Sender<KrakenSpotExecution>,
    recent_executions: Mutex<VecDeque<KrakenSpotExecution>>,
}

impl Default for PrivateState {
    fn default() -> Self {
        Self {
            stocks: Default::default(),
            orders: DashMap::new(),
            balances: DashMap::new(),
            order_sequence: AtomicI64::new(0),
            balance_sequence: AtomicI64::new(0),
            orders_observed_at_ms: AtomicI64::new(0),
            balances_observed_at_ms: AtomicI64::new(0),
            executions: broadcast::channel(1024).0,
            recent_executions: Mutex::new(VecDeque::new()),
        }
    }
}

pub(super) struct KrakenSpotPrivateStream {
    session: WsTradeSession,
    token: Arc<ArcSwap<String>>,
    state: Arc<PrivateState>,
}

impl std::fmt::Debug for KrakenSpotPrivateStream {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("KrakenSpotPrivateStream")
            .field("orders", &self.state.orders.len())
            .field("balances", &self.state.balances.len())
            .finish_non_exhaustive()
    }
}

impl KrakenSpotPrivateStream {
    pub(super) fn track_stock_order(
        &self,
        receipt: shared_types::stocks::StockPeerOrderReceipt,
    ) -> ExchangeResult<()> {
        self.state.stocks.track(receipt)
    }

    pub(super) fn stock_order_receipt(
        &self,
        client: &str,
    ) -> Option<shared_types::stocks::StockPeerOrderReceipt> {
        self.state.stocks.get(client)
    }

    pub(super) fn subscribe_stock_receipts(
        &self,
    ) -> broadcast::Receiver<shared_types::stocks::StockPeerOrderReceipt> {
        self.state.stocks.subscribe()
    }

    pub(super) fn release_stock_receipt(
        &self,
        expected: &shared_types::stocks::StockPeerOrderReceipt,
    ) -> ExchangeResult<()> {
        self.state.stocks.release(expected)
    }
    pub(super) async fn validate_stock_order(&self, draft: &shared_types::stocks::StockPeerOrderDraft) -> ExchangeResult<shared_types::stocks::StockPeerOrderCheck> {
        let request_id=next_request_id();
        let token=self.token.clone();
        let request=draft.clone();
        let response=self.session.send_fresh(move || {
            request.kraken_validation(token.load().as_ref(),request_id,common::time::now_ms())
                .map(|v|v.to_string()).map_err(|e|ExchangeError::Parse(e.into()))
        },Box::new(move |text| {
            let v:Value=serde_json::from_str(text).map_err(|_|ExchangeError::Parse("invalid stock validation response".into()))?;
            Ok(v["method"]=="add_order" && v["req_id"].as_u64()==Some(request_id))
        })).await?;
        stock_validation_result(draft,&response,request_id,common::time::now_ms())
    }

    pub(super) fn shared(
        config: &super::kraken_config::KrakenConfig,
        http: &HttpClient,
    ) -> ExchangeResult<Arc<Self>> {
        static STREAMS: OnceLock<DashMap<String, Weak<KrakenSpotPrivateStream>>> = OnceLock::new();
        let credentials = config
            .credentials
            .as_ref()
            .and_then(|credentials| credentials.spot.as_ref())
            .ok_or_else(|| ExchangeError::Auth("Kraken Spot credentials missing".to_owned()))?;
        let ws_url = config
            .spot_private_ws_url_override
            .as_deref()
            .unwrap_or(SPOT_PRIVATE_WS_URL);
        let rest_url = config
            .spot_rest_url_override
            .as_deref()
            .unwrap_or(SPOT_REST_URL);
        let key = session_key(&[
            "kraken",
            "spot-private-v2",
            ws_url,
            rest_url,
            &credentials.api_key,
            &credentials.api_secret,
        ]);
        let streams = STREAMS.get_or_init(DashMap::new);
        if let Some(stream) = streams.get(&key).and_then(|entry| entry.value().upgrade()) {
            return Ok(stream);
        }
        let stream = Arc::new(Self::new(
            ws_url,
            rest_url,
            credentials,
            http,
            config.timeout_secs,
        ));
        let mut entry = streams.entry(key).or_default();
        if let Some(existing) = entry.value().upgrade() {
            return Ok(existing);
        }
        *entry.value_mut() = Arc::downgrade(&stream);
        Ok(stream)
    }

    fn new(
        ws_url: &str,
        rest_url: &str,
        credentials: &KrakenSpotCredentials,
        http: &HttpClient,
        timeout_secs: u64,
    ) -> Self {
        let token = Arc::new(ArcSwap::from_pointee(String::new()));
        let state = Arc::new(PrivateState::default());
        let prepare_token = Arc::clone(&token);
        let prepare_credentials = credentials.clone();
        let prepare_http = http.clone();
        let prepare_url = rest_url.to_owned();
        let mut spec = WsSessionSpec::new(ws_url.to_owned(), timeout_secs)
            .with_connect_prepare(move || {
                let token = Arc::clone(&prepare_token);
                let credentials = prepare_credentials.clone();
                let http = prepare_http.clone();
                let url = prepare_url.clone();
                Box::pin(async move {
                    token.store(Arc::new(fetch_ws_token(&http, &url, &credentials).await?));
                    Ok(())
                })
            })
            .with_heartbeat_interval(Duration::from_secs(25));
        for channel in ["executions", "balances"] {
            let token = Arc::clone(&token);
            spec = spec.with_reconnect_payload(move || {
                subscription_request(channel, token.load().as_ref())
            });
        }
        let push_state = Arc::clone(&state);
        spec = spec.with_incoming_text_handler(move |text| apply_frame(&push_state, text));
        Self {
            session: WsTradeSession::spawn(spec),
            token,
            state,
        }
    }

    pub(super) async fn warm(&self) -> ExchangeResult<()> {
        self.session.warm().await
    }

    pub(super) fn subscribe_executions(&self) -> broadcast::Receiver<KrakenSpotExecution> {
        self.state.executions.subscribe()
    }

    pub(super) fn execution_snapshot(&self) -> Vec<KrakenSpotExecution> {
        self.state
            .recent_executions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .cloned()
            .collect()
    }

    pub(super) async fn place_order(&self, intent: &OrderIntent) -> ExchangeResult<OrderAck> {
        let venue_client_id = crate::client_order_id_policy::required_venue_client_order_id(
            "kraken",
            &intent.client_order_id,
        )?;
        let request_id = next_request_id();
        let response_request_id = request_id;
        let matcher_request_id = request_id;
        let token = Arc::clone(&self.token);
        let intent_for_request = intent.clone();
        let client_for_request = venue_client_id.clone();
        let response = self
            .session
            .send_fresh(
                move || {
                    add_order_request(
                        &intent_for_request,
                        &client_for_request,
                        token.load().as_ref(),
                        request_id,
                    )
                },
                Box::new(move |text| response_matches(text, "add_order", matcher_request_id)),
            )
            .await?;
        let exchange_order_id = response_order_id(&response, "add_order", response_request_id)?;
        Ok(OrderAck {
            internal_order_id: intent.id.clone(),
            exchange_order_id: Some(exchange_order_id.clone()),
            client_order_id: intent.client_order_id.clone(),
            identity_update: VenueOrderIdentityUpdate::from_ids(
                intent.client_order_id.clone(),
                venue_client_id,
                Some(exchange_order_id),
            ),
            state: LiveOrderState::Submitted,
            accepted_at_ms: common::time::now_ms(),
            message: Some("Kraken Spot WS v2 add_order accepted".to_owned()),
            filled_quantity: None,
            filled_price: None,
            filled_fee: None,
        })
    }

    pub(super) async fn cancel_order(
        &self,
        request: &CancelOrderRequest,
    ) -> ExchangeResult<OrderAck> {
        let venue_client_id = crate::client_order_id_policy::required_venue_client_order_id(
            "kraken",
            &request.client_order_id,
        )?;
        let request_id = next_request_id();
        let matcher_request_id = request_id;
        let response_request_id = request_id;
        let token = Arc::clone(&self.token);
        let exchange_order_id = request.exchange_order_id.clone();
        let request_exchange_id = exchange_order_id.clone();
        let client_for_request = venue_client_id.clone();
        let response = self
            .session
            .send_fresh(
                move || {
                    cancel_order_request(
                        request_exchange_id.as_deref(),
                        &client_for_request,
                        token.load().as_ref(),
                        request_id,
                    )
                },
                Box::new(move |text| response_matches(text, "cancel_order", matcher_request_id)),
            )
            .await?;
        response_order_id(&response, "cancel_order", response_request_id)?;
        Ok(OrderAck {
            internal_order_id: request.internal_order_id.clone(),
            exchange_order_id: exchange_order_id.clone(),
            client_order_id: request.client_order_id.clone(),
            identity_update: VenueOrderIdentityUpdate::from_ids(
                request.client_order_id.clone(),
                venue_client_id,
                exchange_order_id,
            ),
            state: LiveOrderState::CancelRequested,
            accepted_at_ms: common::time::now_ms(),
            message: Some("Kraken Spot WS v2 cancel_order accepted".to_owned()),
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
                    OrderStatus::Pending | OrderStatus::Open | OrderStatus::PartiallyFilled
                ) && symbol.is_none_or(|symbol| entry.symbol.eq_ignore_ascii_case(symbol))
            })
            .map(|entry| entry.value().clone())
            .collect::<Vec<_>>();
        rows.sort_by_key(|row| Reverse(row.created_at));
        rows
    }

    pub(super) fn has_order_sample(&self) -> bool {
        self.state.orders_observed_at_ms.load(Ordering::Acquire) > 0
    }

    pub(super) fn has_balance_sample(&self) -> bool {
        self.state.balances_observed_at_ms.load(Ordering::Acquire) > 0
    }

    pub(super) fn runtime_status(&self) -> PrivateWsRuntimeStatus {
        PrivateWsRuntimeStatus {
            sessions: 1,
            subscriptions: 2,
            account_streams: 1,
            account_samples: usize::from(self.has_balance_sample()),
            order_streams: 1,
            order_samples: usize::from(self.has_order_sample()),
        }
    }
}

fn apply_frame(state: &PrivateState, text: &str) -> ExchangeResult<()> {
    reject_subscription_error(text)?;
    let Some(frame) = parse_private_frame(text)? else {
        return Ok(());
    };
    match frame {
        SpotPrivateFrame::Executions {
            kind,
            sequence,
            rows,
        } => {
            if !accept_sequence(&state.order_sequence, kind, sequence, "executions")? {
                return Ok(());
            }
            state.stocks.apply(text);
            if kind == FrameKind::Snapshot {
                state.orders.retain(|_, order| {
                    matches!(
                        order.status,
                        OrderStatus::Filled
                            | OrderStatus::Canceled
                            | OrderStatus::Expired
                            | OrderStatus::Rejected
                    )
                });
            }
            for mut patch in rows {
                let fill = patch.fill.take();
                let current = state
                    .orders
                    .get(&patch.order_id)
                    .map(|row| row.value().clone());
                let order = patch.merge(current);
                if let Some(order) = &order {
                    state.orders.insert(order.order_id.clone(), order.clone());
                }
                if order.is_some() || fill.is_some() {
                    publish_execution(
                        state,
                        KrakenSpotExecution {
                            order,
                            fill,
                            received_at_ms: common::time::now_ms(),
                        },
                    );
                }
            }
            trim_terminal_orders(state);
            state
                .orders_observed_at_ms
                .store(common::time::now_ms(), Ordering::Release);
        }
        SpotPrivateFrame::Balances {
            kind,
            sequence,
            rows,
        } => {
            if !accept_sequence(&state.balance_sequence, kind, sequence, "balances")? {
                return Ok(());
            }
            if kind == FrameKind::Snapshot {
                state.balances.clear();
            }
            for row in rows {
                let frozen = state
                    .balances
                    .get(&row.currency)
                    .map_or(0.0, |value| value.frozen);
                state.balances.insert(
                    row.currency.clone(),
                    VenueBalanceInfo {
                        venue: VENUE.to_owned(),
                        currency: row.currency,
                        total: row.total,
                        available: row.total - frozen,
                        frozen,
                        unrealized_pnl: 0.0,
                    },
                );
            }
            state
                .balances_observed_at_ms
                .store(common::time::now_ms(), Ordering::Release);
        }
    }
    Ok(())
}

fn publish_execution(state: &PrivateState, event: KrakenSpotExecution) {
    let mut recent = state
        .recent_executions
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if recent.len() >= MAX_CACHED_ORDERS {
        recent.pop_front();
    }
    recent.push_back(event.clone());
    drop(recent);
    let _ = state.executions.send(event);
}

fn reject_subscription_error(text: &str) -> ExchangeResult<()> {
    let value: Value = serde_json::from_str(text)
        .map_err(|error| ExchangeError::Parse(format!("kraken spot private frame: {error}")))?;
    if value.get("method").and_then(Value::as_str) == Some("subscribe")
        && value.get("success").and_then(Value::as_bool) == Some(false)
    {
        return Err(ExchangeError::Api {
            exchange: "kraken".to_owned(),
            code: value
                .get("error")
                .and_then(|error| error.get("error_code"))
                .and_then(Value::as_str)
                .unwrap_or("subscribe_failed")
                .to_owned(),
            message: value
                .get("error")
                .and_then(|error| error.get("error_message"))
                .and_then(Value::as_str)
                .or_else(|| value.get("error").and_then(Value::as_str))
                .unwrap_or("private subscription failed")
                .to_owned(),
        });
    }
    Ok(())
}

fn accept_sequence(
    sequence: &AtomicI64,
    kind: FrameKind,
    next: i64,
    channel: &str,
) -> ExchangeResult<bool> {
    let previous = sequence.load(Ordering::Acquire);
    if kind == FrameKind::Snapshot {
        sequence.store(next, Ordering::Release);
        return Ok(true);
    }
    if next <= previous {
        return Ok(false);
    }
    if previous == 0 || next != previous + 1 {
        return Err(ExchangeError::Parse(format!(
            "kraken spot {channel} sequence gap: previous={previous}, next={next}"
        )));
    }
    sequence.store(next, Ordering::Release);
    Ok(true)
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

fn subscription_request(channel: &str, token: &str) -> ExchangeResult<String> {
    if token.is_empty() {
        return Err(ExchangeError::Auth(
            "Kraken Spot websocket token missing".to_owned(),
        ));
    }
    let params = match channel {
        "executions" => json!({
            "channel": channel,
            "token": token,
            "snap_orders": true,
            "snap_trades": true,
            "rebased": true,
            "order_status": true
        }),
        "balances" => json!({ "channel": channel, "token": token }),
        _ => {
            return Err(ExchangeError::Parse(format!(
                "unsupported Kraken private channel {channel}"
            )))
        }
    };
    serde_json::to_string(&json!({ "method": "subscribe", "params": params }))
        .map_err(|error| ExchangeError::Parse(format!("kraken private subscription: {error}")))
}

fn add_order_request(
    intent: &OrderIntent,
    venue_client_id: &str,
    token: &str,
    request_id: u64,
) -> ExchangeResult<String> {
    if !intent.quantity.is_finite() || intent.quantity <= 0.0 {
        return Err(ExchangeError::Parse(
            "Kraken Spot order_qty must be positive".to_owned(),
        ));
    }
    let order_type = match intent.order_type {
        OrderType::Market => "market",
        OrderType::Limit | OrderType::PostOnly => "limit",
    };
    let mut params = json!({
        "order_type": order_type,
        "side": match intent.side { OrderSide::Buy => "buy", OrderSide::Sell => "sell" },
        "order_qty": intent.quantity,
        "symbol": spot_symbol(&intent.symbol),
        "cl_ord_id": venue_client_id,
        "time_in_force": match intent.time_in_force {
            TimeInForce::Ioc => "ioc",
            TimeInForce::Fok => "fok",
            TimeInForce::Gtc | TimeInForce::Gtx => "gtc",
        },
        "post_only": intent.post_only || intent.order_type == OrderType::PostOnly || intent.time_in_force == TimeInForce::Gtx,
        "reduce_only": intent.reduce_only,
        "token": token
    });
    if order_type == "limit" {
        let price = intent
            .price
            .filter(|value| value.is_finite() && *value > 0.0)
            .ok_or_else(|| ExchangeError::Parse("Kraken Spot limit_price missing".to_owned()))?;
        params["limit_price"] = json!(price);
    }
    serde_json::to_string(&json!({ "method": "add_order", "params": params, "req_id": request_id }))
        .map_err(|error| ExchangeError::Parse(format!("kraken add_order request: {error}")))
}

fn cancel_order_request(
    exchange_order_id: Option<&str>,
    venue_client_id: &str,
    token: &str,
    request_id: u64,
) -> ExchangeResult<String> {
    let params = if let Some(order_id) = exchange_order_id.filter(|value| !value.is_empty()) {
        json!({ "order_id": [order_id], "token": token })
    } else {
        json!({ "cl_ord_id": [venue_client_id], "token": token })
    };
    serde_json::to_string(
        &json!({ "method": "cancel_order", "params": params, "req_id": request_id }),
    )
    .map_err(|error| ExchangeError::Parse(format!("kraken cancel_order request: {error}")))
}

fn response_matches(text: &str, method: &str, request_id: u64) -> ExchangeResult<bool> {
    let value: Value = serde_json::from_str(text)
        .map_err(|error| ExchangeError::Parse(format!("kraken {method} response: {error}")))?;
    if value.get("method").and_then(Value::as_str) != Some(method)
        || value.get("req_id").and_then(Value::as_u64) != Some(request_id)
    {
        return Ok(false);
    }
    if value.get("success").and_then(Value::as_bool) == Some(false) {
        return Err(ExchangeError::Api {
            exchange: "kraken".to_owned(),
            code: method.to_owned(),
            message: value
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("request failed")
                .to_owned(),
        });
    }
    Ok(true)
}

fn response_order_id(text: &str, method: &str, request_id: u64) -> ExchangeResult<String> {
    let value: Value = serde_json::from_str(text)
        .map_err(|error| ExchangeError::Parse(format!("kraken {method} response: {error}")))?;
    if value.get("req_id").and_then(Value::as_u64) != Some(request_id) {
        return Err(ExchangeError::Parse(format!(
            "kraken {method} req_id mismatch"
        )));
    }
    value
        .pointer("/result/order_id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| ExchangeError::Parse(format!("kraken {method} order_id missing")))
}

fn next_request_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

fn stock_validation_result(draft: &shared_types::stocks::StockPeerOrderDraft, text:&str, id:u64, now:i64) -> ExchangeResult<shared_types::stocks::StockPeerOrderCheck> {
    use shared_types::stocks::{StockPeerOrderCheck,StockPeerOrderCheckStatus as Status};
    let v:Value=serde_json::from_str(text).map_err(|_|ExchangeError::Parse("invalid stock validation response".into()))?;
    if v["method"]!="add_order" || v["req_id"].as_u64()!=Some(id) {return Err(ExchangeError::Parse("stock validation response identity mismatch".into()));}
    let (status,message)=if v.pointer("/result/order_id").is_some_and(|v|!v.is_null()) {
        (Status::Unknown,"验证回复含订单编号，不能当作不成交验证通过；请检查交易所订单")
    }else if v["success"]==true && v["result"].is_object()
        && v.pointer("/result/cl_ord_id").is_none_or(|v|v.as_str()==Some(format!("sv{id:016x}").as_str())) {
        (Status::Passed,"交易所仅验证当次股票参数通过；未成交，不代表套利执行已就绪")
    }else if v["success"]==false {
        let error=v["error"].as_str().unwrap_or_default();
        let message=if error.contains("Insufficient funds"){"交易所验证拒绝：该股票腿可用余额不足"}
            else if error.contains("Permission denied"){"交易所验证拒绝：缺少所需账户权限"}
            else if error.contains("Invalid arguments"){"交易所验证拒绝：订单参数不符合要求"}
            else{"交易所验证拒绝；请核对现货权限、余额和股票交易条件"};
        (Status::Rejected,message)
    }else {(Status::Unknown,"股票验证回复不完整，未确认通过")};
    Ok(StockPeerOrderCheck {draft:draft.clone(),completed_at_ms:Some(now),status,message:message.into()})
}

#[cfg(test)]
#[path = "kraken_stock_receipts/ws_tests.rs"]
mod stock_receipt_ws_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{ExecutionMode, MarginMode, OrderSource};

    fn stock_draft() -> shared_types::stocks::StockPeerOrderDraft {
        use shared_types::stocks::*;
        let now=common::time::now_ms();
        StockPeerOrderDraft {purpose:StockPeerOrderPurpose::Equity,request:StockPeerOrderCheckRequest {asset:"MU.US".into(),selection:StockPeerSelection {venue:"kraken".into(),product:StockPeerProduct::Spot,native_symbol:"MUx/USD".into()},direction:StockChainDirection::Buy},
            quantity:"1.234567".into(),limit_price:"601.12".into(),quote_asset:"USD".into(),prepared_at_ms:now,source_at_ms:now,metadata_at_ms:now}
    }

    #[test]
    fn stock_validation_reply_never_confuses_an_order_ack_or_echoes_credentials() {
        use shared_types::stocks::StockPeerOrderCheckStatus as Status;
        let draft=stock_draft();
        for (body,status) in [
            (json!({"method":"add_order","req_id":7,"success":true,"result":{"cl_ord_id":"sv0000000000000007"}}),Status::Passed),
            (json!({"method":"add_order","req_id":7,"success":true,"result":{"order_id":"unexpected-order"}}),Status::Unknown),
            (json!({"method":"add_order","req_id":7,"success":true}),Status::Unknown),
            (json!({"method":"add_order","req_id":7,"success":true,"result":{"cl_ord_id":"different-client"}}),Status::Unknown),
            (json!({"method":"add_order","req_id":7,"success":false,"error":"EOrder:Insufficient funds fixture-private-token"}),Status::Rejected),
        ] {
            let result=stock_validation_result(&draft,&body.to_string(),7,common::time::now_ms()).unwrap();
            assert_eq!(result.status,status);assert!(!result.message.contains("fixture-private-token"));
        }
        assert!(stock_validation_result(&draft,r#"{"method":"add_order","req_id":8,"success":true,"result":{}}"#,7,common::time::now_ms()).is_err());
    }

    #[tokio::test]
    async fn stock_validation_ws_roundtrip_reuses_stream_and_never_publishes_fills() {
        use futures_util::{SinkExt,StreamExt};
        use tokio_tungstenite::{accept_async,tungstenite::Message};
        use wiremock::{Mock,MockServer,ResponseTemplate,matchers::{method,path}};
        use shared_types::stocks::{StockChainDirection,StockPeerOrderCheckStatus};
        let http=MockServer::start().await;
        Mock::given(method("POST")).and(path("/0/private/GetWebSocketsToken"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"error":[],"result":{"token":"fixture-private-token","expires":900}})))
            .expect(1).mount(&http).await;
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url=format!("ws://{}",listener.local_addr().unwrap());
        let (stop,mut stopped)=tokio::sync::oneshot::channel::<()>();
        let server=tokio::spawn(async move {
            let (socket,_)=listener.accept().await.unwrap();let mut ws=accept_async(socket).await.unwrap();
            let mut requests=Vec::new();
            loop {
                tokio::select! {
                    _=&mut stopped=>break,
                    message=ws.next()=>{
                        let text=match message {
                            Some(Ok(Message::Text(text)))=>text,
                            Some(Ok(Message::Ping(payload)))=>{ws.send(Message::Pong(payload)).await.unwrap();continue;},
                            Some(Ok(Message::Pong(_)))=>continue,
                            Some(Ok(Message::Close(_)))|None=>break,
                            other=>panic!("unexpected local WS frame: {other:?}"),
                        };
                        let v:Value=serde_json::from_str(&text).unwrap();
                        if v["method"]!="add_order" {continue;}
                        let id=v["req_id"].as_u64().unwrap();
                        assert_eq!(v["params"]["validate"],true);assert_eq!(v["params"]["margin"],false);
                        assert_eq!(v["params"]["time_in_force"],"fok");assert_eq!(v["params"]["fee_preference"],"quote");
                        assert_eq!(v["params"]["order_qty"].to_string(),"1.234567");assert_eq!(v["params"]["symbol"],"MUx/USD");
                        assert_eq!(v["params"]["token"],"fixture-private-token");
                        let deadline=chrono::DateTime::parse_from_rfc3339(v["params"]["deadline"].as_str().unwrap()).unwrap().timestamp_millis();
                        assert!((500..=2000).contains(&(deadline-common::time::now_ms())));
                        requests.push(v.clone());
                        ws.send(Message::Text(json!({"method":"add_order","req_id":id+1,"success":true,"result":{}}).to_string())).await.unwrap();
                        let reply=if requests.len()==1 {json!({"method":"add_order","req_id":id,"success":true,"result":{"cl_ord_id":v["params"]["cl_ord_id"]}})}
                            else {json!({"method":"add_order","req_id":id,"success":false,"error":"EOrder:Insufficient funds fixture-private-token"})};
                        ws.send(Message::Text(reply.to_string())).await.unwrap();
                    }
                }
            }
            requests
        });
        let stream=KrakenSpotPrivateStream::new(&url,&http.uri(),&KrakenSpotCredentials{api_key:"local-fixture".into(),api_secret:"c2VjcmV0".into()},&HttpClient::new("kraken-fixture").unwrap(),3);
        let mut updates=stream.state.executions.subscribe();
        let first=stream.validate_stock_order(&stock_draft()).await.unwrap();assert_eq!(first.status,StockPeerOrderCheckStatus::Passed);
        let mut buy=stock_draft();buy.request.direction=StockChainDirection::Sell;
        let second=stream.validate_stock_order(&buy).await.unwrap();assert_eq!(second.status,StockPeerOrderCheckStatus::Rejected);
        assert!(!second.message.contains("fixture-private-token"));assert!(stream.state.orders.is_empty());assert!(updates.try_recv().is_err());
        assert!(stream.state.recent_executions.lock().unwrap().is_empty());
        let _=stop.send(());let frames=server.await.unwrap();assert_eq!(frames.len(),2);
        assert_eq!(frames[0]["params"]["side"],"sell");assert_eq!(frames[1]["params"]["side"],"buy");
        assert_ne!(frames[0]["req_id"],frames[1]["req_id"]);assert_eq!(http.received_requests().await.unwrap().len(),1);
    }

    #[test]
    fn executions_reach_subscribers_and_replay_is_bounded() {
        let state = PrivateState::default();
        let mut updates = state.executions.subscribe();
        let mut frame: Value = serde_json::from_str(include_str!(
            "../../fixtures/kraken/spot_v2_execution_update.json"
        ))
        .unwrap();
        frame["type"] = json!("snapshot");
        apply_frame(&state, &frame.to_string()).unwrap();
        let first = updates.try_recv().unwrap();
        assert_eq!(
            first.fill.as_ref().unwrap().fee_currency.as_deref(),
            Some("USD")
        );
        assert!(first.order.is_some());
        frame["type"] = json!("update");
        apply_frame(&state, &frame.to_string()).unwrap();
        assert!(updates.try_recv().is_err());
        for _ in 0..MAX_CACHED_ORDERS + 2 {
            publish_execution(&state, first.clone());
        }
        assert_eq!(
            state.recent_executions.lock().unwrap().len(),
            MAX_CACHED_ORDERS
        );
        let subscription: Value =
            serde_json::from_str(&subscription_request("executions", "test").unwrap()).unwrap();
        assert_eq!(subscription["params"]["snap_trades"], true);
    }

    #[test]
    fn reconnect_trade_history_cannot_regress_a_known_terminal_order() {
        let state = PrivateState::default();
        let mut frame: Value = serde_json::from_str(include_str!(
            "../../fixtures/kraken/spot_v2_execution_update.json"
        ))
        .unwrap();
        frame["type"] = json!("snapshot");
        frame["data"][0]["order_status"] = json!("filled");
        frame["data"][0]["cum_qty"] = json!(0.01);
        frame["data"][0]["avg_price"] = json!(27000.0);
        apply_frame(&state, &frame.to_string()).unwrap();
        frame["sequence"] = json!(1);
        frame["data"][0]["order_status"] = json!("partially_filled");
        frame["data"][0]["cum_qty"] = json!(0.005);
        frame["data"][0]["avg_price"] = json!(26000.0);
        apply_frame(&state, &frame.to_string()).unwrap();
        let order = state.orders.get("OK4GJX-KSTLS-7DZZO5").unwrap();
        assert_eq!(order.status, OrderStatus::Filled);
        assert_eq!(order.filled_quantity, 0.01);
        assert_eq!(order.filled_price, 27000.0);
    }

    fn intent() -> OrderIntent {
        OrderIntent {
            id: "internal".to_owned(),
            source: OrderSource::Manual,
            strategy: None,
            mode: ExecutionMode::Live,
            exchange: "kraken".to_owned(),
            symbol: "BTC/USD".to_owned(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            quantity: 0.01,
            price: Some(40_000.0),
            slippage_tolerance_bps: None,
            reduce_only: false,
            time_in_force: TimeInForce::Gtc,
            post_only: true,
            margin_mode: MarginMode::Cross,
            leverage: 1.0,
            client_order_id: "client-1".to_owned(),
            client_order_id_policy: None,
            created_at_ms: 1,
        }
    }

    #[test]
    fn requests_follow_spot_v2_contract() {
        let request: Value =
            serde_json::from_str(&add_order_request(&intent(), "a1b2c3", "token", 7).unwrap())
                .unwrap();
        assert_eq!(request["method"], "add_order");
        assert_eq!(request["params"]["symbol"], "BTC/USD");
        assert_eq!(request["params"]["post_only"], true);
        assert_eq!(request["req_id"], 7);

        let cancel: Value = serde_json::from_str(
            &cancel_order_request(Some("ORDER"), "client", "token", 8).unwrap(),
        )
        .unwrap();
        assert_eq!(cancel["params"]["order_id"][0], "ORDER");
    }

    #[test]
    fn spot_v2_write_responses_parse_official_fixtures() {
        let place = include_str!("../../fixtures/kraken/spot_v2_add_order_ack.json");
        assert!(response_matches(place, "add_order", 7).unwrap());
        assert_eq!(
            response_order_id(place, "add_order", 7).unwrap(),
            "OK4GJX-KSTLS-7DZZO5"
        );

        let cancel = include_str!("../../fixtures/kraken/spot_v2_cancel_order_ack.json");
        assert!(response_matches(cancel, "cancel_order", 8).unwrap());
        assert_eq!(
            response_order_id(cancel, "cancel_order", 8).unwrap(),
            "OK4GJX-KSTLS-7DZZO5"
        );
    }

    #[test]
    fn sequence_gap_fails_closed_and_duplicates_are_ignored() {
        let sequence = AtomicI64::new(0);
        assert!(accept_sequence(&sequence, FrameKind::Snapshot, 4, "test").unwrap());
        assert!(!accept_sequence(&sequence, FrameKind::Update, 4, "test").unwrap());
        assert!(accept_sequence(&sequence, FrameKind::Update, 5, "test").unwrap());
        assert!(accept_sequence(&sequence, FrameKind::Update, 7, "test").is_err());
    }
}
