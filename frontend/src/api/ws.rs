//! WebSocket 客户端（套利 / 资金费率推送）。
//!
//! 协议（与 `crates/api/src/routers/websocket.rs` 同步）：
//! - 客户端发：`{type:"subscribe",channels:["arbitrage"]}` 或 `["funding-rates"]`、`{type:"ping"}`
//! - 服务端推：`{type:"message",channel:<name>,payload:<channel-specific>}`
//! - 服务端可批量推：`{type:"batch",messages:[{channel:<name>,payload:<channel-specific>}]}`
//!
//! [`start_arbitrage_stream`] / [`start_funding_stream`] 都基于内部的 [`start_stream`]
//! 通用函数，会在 WASM 主任务里启动一个长循环：
//! - 建连失败或断开后 5s 自动重连
//! - 每 25s 发一次 ping 维持心跳

use crate::api::rest::{
    portfolio_envelope_degraded_problem, portfolio_envelope_problem, ExecutionRunEvent,
    FundingRatesResponse, OpportunityStreamPayload, OrderStreamPayload, RiskAlertEvent,
};
use crate::api::ws_runtime::{current_ws_runtime, RuntimeSubscription};
use leptos::prelude::*;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::Value;
use shared_types::ApiProblem;
use std::rc::Rc;

const CH_ARBITRAGE: &str = "arbitrage";
const CH_FUNDING_RATES: &str = "funding-rates";
const CH_ORDERS: &str = "orders";
const CH_EXECUTION: &str = "execution";
const CH_PORTFOLIO: &str = "portfolio";
const CH_RISK_ALERTS: &str = "risk-alerts";
const CH_SYSTEM: &str = "system";
const CH_AUTOMATION: &str = "automation";
const CH_ONCHAIN: &str = "onchain";
const CH_STOCKS: &str = "stocks";
const CH_WATCHLIST: &str = "watchlist";
const CH_ALERTS: &str = "alerts";
const CH_REVIEW: &str = "review";
const CH_WEBHOOK: &str = "webhook";
pub(crate) const RECONNECT_DELAY_MS: u32 = 5_000;
pub(crate) const PING_INTERVAL_MS: u32 = 25_000;

/// WebSocket 连接状态（前端 UI 反馈）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WsStatus {
    Connecting,
    Connected,
    Disconnected,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WsChannelState {
    pub channel: String,
    pub status: WsStatus,
    pub subscribed: bool,
    pub last_message_at_ms: Option<u64>,
    pub last_error: Option<ApiProblem>,
    pub retry_after_ms: Option<u64>,
    pub message_count: u64,
    pub problem_count: u64,
    pub last_problem_at_ms: Option<u64>,
}

impl WsChannelState {
    pub fn new(channel: impl Into<String>) -> Self {
        Self {
            channel: channel.into(),
            status: WsStatus::Disconnected,
            subscribed: false,
            last_message_at_ms: None,
            last_error: None,
            retry_after_ms: None,
            message_count: 0,
            problem_count: 0,
            last_problem_at_ms: None,
        }
    }
}

pub struct WsStreamHandle {
    runtime_subscription_id: Option<u64>,
}

impl WsStreamHandle {
    pub(crate) fn runtime(subscription_id: u64) -> Self {
        Self {
            runtime_subscription_id: Some(subscription_id),
        }
    }

    pub(crate) fn inactive() -> Self {
        Self {
            runtime_subscription_id: None,
        }
    }

    pub fn cancel(&self) {
        if let Some(subscription_id) = self.runtime_subscription_id {
            if let Some(runtime) = current_ws_runtime() {
                runtime.unsubscribe(subscription_id);
            }
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ServerEnvelope {
    Ack {
        #[serde(default)]
        subscribed: Vec<String>,
        #[serde(default, rename = "requestId")]
        request_id: Option<String>,
    },
    Message {
        channel: String,
        payload: Value,
    },
    Batch {
        #[serde(default)]
        messages: Vec<ChannelPayload>,
    },
    Pong,
    Error {
        #[serde(default)]
        message: String,
        #[serde(default)]
        channel: Option<String>,
        #[serde(default)]
        code: Option<String>,
        #[serde(default, rename = "retryAfterMs")]
        retry_after_ms: Option<u64>,
        #[serde(default, rename = "requestId")]
        request_id: Option<String>,
    },
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChannelPayload {
    pub(crate) channel: String,
    pub(crate) payload: Value,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum PortfolioStreamPayload {
    CloseRun {
        #[serde(default)]
        event: String,
        #[serde(rename = "closeRun")]
        close_run: Box<shared_types::CloseRun>,
        #[serde(default, rename = "timestampMs")]
        timestamp_ms: Option<i64>,
    },
    Envelope(Box<shared_types::PortfolioSnapshotEnvelope>),
    Snapshot(Box<shared_types::PortfolioSnapshot>),
}

pub fn start_arbitrage_stream_with_state(
    channel_state: RwSignal<WsChannelState>,
    on_payload: impl Fn(OpportunityStreamPayload) + 'static,
    on_problem: impl Fn(ApiProblem) + 'static,
) -> WsStreamHandle {
    start_stream::<OpportunityStreamPayload>(CH_ARBITRAGE, channel_state, on_payload, on_problem)
}

pub fn start_funding_stream_with_state(
    channel_state: RwSignal<WsChannelState>,
    on_payload: impl Fn(FundingRatesResponse) + 'static,
    on_problem: impl Fn(ApiProblem) + 'static,
) -> WsStreamHandle {
    start_stream::<FundingRatesResponse>(CH_FUNDING_RATES, channel_state, on_payload, on_problem)
}

pub fn start_order_stream_with_state(
    channel_state: RwSignal<WsChannelState>,
    on_payload: impl Fn(OrderStreamPayload) + 'static,
    on_problem: impl Fn(ApiProblem) + 'static,
) -> WsStreamHandle {
    start_stream::<OrderStreamPayload>(CH_ORDERS, channel_state, on_payload, on_problem)
}

pub fn start_execution_stream_with_state(
    channel_state: RwSignal<WsChannelState>,
    on_payload: impl Fn(ExecutionRunEvent) + 'static,
    on_problem: impl Fn(ApiProblem) + 'static,
) -> WsStreamHandle {
    start_stream::<ExecutionRunEvent>(CH_EXECUTION, channel_state, on_payload, on_problem)
}

pub fn start_portfolio_stream_with_state(
    channel_state: RwSignal<WsChannelState>,
    on_payload: impl Fn(shared_types::PortfolioSnapshot) + 'static,
    on_close_run: impl Fn(shared_types::CloseRun) + 'static,
    on_problem: impl Fn(ApiProblem) + 'static,
) -> WsStreamHandle {
    start_portfolio_stream_inner(channel_state, on_payload, on_close_run, on_problem)
}

fn start_portfolio_stream_inner(
    channel_state: RwSignal<WsChannelState>,
    on_snapshot: impl Fn(shared_types::PortfolioSnapshot) + 'static,
    on_close_run: impl Fn(shared_types::CloseRun) + 'static,
    on_problem: impl Fn(ApiProblem) + 'static,
) -> WsStreamHandle {
    let on_snapshot = Rc::new(on_snapshot);
    let on_close_run = Rc::new(on_close_run);
    let on_problem = Rc::new(on_problem);
    let stream_problem = Rc::clone(&on_problem);
    start_stream::<PortfolioStreamPayload>(
        CH_PORTFOLIO,
        channel_state,
        move |event| match event {
            PortfolioStreamPayload::Snapshot(snapshot) => on_snapshot.as_ref()(*snapshot),
            PortfolioStreamPayload::Envelope(envelope) => {
                handle_portfolio_envelope(*envelope, on_snapshot.as_ref(), on_problem.as_ref());
            }
            PortfolioStreamPayload::CloseRun {
                close_run,
                event,
                timestamp_ms,
            } => {
                let _ = (event, timestamp_ms);
                on_close_run.as_ref()(*close_run);
            }
        },
        move |problem| stream_problem.as_ref()(problem),
    )
}

fn handle_portfolio_envelope(
    envelope: shared_types::PortfolioSnapshotEnvelope,
    on_snapshot: &impl Fn(shared_types::PortfolioSnapshot),
    on_problem: &impl Fn(ApiProblem),
) {
    let degraded_problem = portfolio_envelope_degraded_problem(&envelope);
    if let Some(snapshot) = envelope.snapshot {
        on_snapshot(snapshot);
        if let Some(problem) = degraded_problem {
            on_problem(problem);
        }
        return;
    }
    on_problem(portfolio_envelope_problem(&envelope));
}

pub fn start_risk_alert_stream_with_state(
    channel_state: RwSignal<WsChannelState>,
    on_payload: impl Fn(RiskAlertEvent) + 'static,
    on_problem: impl Fn(ApiProblem) + 'static,
) -> WsStreamHandle {
    start_stream::<RiskAlertEvent>(CH_RISK_ALERTS, channel_state, on_payload, on_problem)
}

pub fn start_watchlist_stream_with_state(
    channel_state: RwSignal<WsChannelState>,
    on_payload: impl Fn(shared_types::WatchlistStreamEvent) + 'static,
    on_problem: impl Fn(ApiProblem) + 'static,
) -> WsStreamHandle {
    start_stream::<shared_types::WatchlistStreamEvent>(
        CH_WATCHLIST,
        channel_state,
        on_payload,
        on_problem,
    )
}

pub fn start_alert_stream_with_state(
    channel_state: RwSignal<WsChannelState>,
    on_payload: impl Fn(shared_types::AlertStreamEvent) + 'static,
    on_problem: impl Fn(ApiProblem) + 'static,
) -> WsStreamHandle {
    start_stream::<shared_types::AlertStreamEvent>(CH_ALERTS, channel_state, on_payload, on_problem)
}

pub fn start_system_stream_with_state(
    channel_state: RwSignal<WsChannelState>,
    on_payload: impl Fn(shared_types::SystemHealth) + 'static,
    on_problem: impl Fn(ApiProblem) + 'static,
) -> WsStreamHandle {
    start_stream::<shared_types::SystemHealth>(CH_SYSTEM, channel_state, on_payload, on_problem)
}

pub fn start_automation_stream_with_state(
    channel_state: RwSignal<WsChannelState>,
    on_payload: impl Fn(shared_types::AutomationRuntimeStatus) + 'static,
    on_problem: impl Fn(ApiProblem) + 'static,
) -> WsStreamHandle {
    start_stream::<shared_types::AutomationRuntimeStatus>(
        CH_AUTOMATION,
        channel_state,
        on_payload,
        on_problem,
    )
}

pub fn start_onchain_stream_with_state(
    channel_state: RwSignal<WsChannelState>,
    on_payload: impl Fn(shared_types::OnchainComparisonSnapshot) + 'static,
    on_problem: impl Fn(ApiProblem) + 'static,
) -> WsStreamHandle {
    start_stream::<shared_types::OnchainComparisonSnapshot>(
        CH_ONCHAIN,
        channel_state,
        on_payload,
        on_problem,
    )
}

pub fn start_stock_stream_with_state(
    channel_state: RwSignal<WsChannelState>,
    on_payload: impl Fn(shared_types::stocks::StockMarketSnapshot) + 'static,
    on_problem: impl Fn(ApiProblem) + 'static,
) -> WsStreamHandle {
    start_stream::<shared_types::stocks::StockMarketSnapshot>(CH_STOCKS, channel_state, on_payload, on_problem)
}

pub fn start_webhook_stream_with_state(
    channel_state: RwSignal<WsChannelState>,
    on_payload: impl Fn(shared_types::WebhookRuntimeStatus) + 'static,
    on_problem: impl Fn(ApiProblem) + 'static,
) -> WsStreamHandle {
    start_stream::<shared_types::WebhookRuntimeStatus>(
        CH_WEBHOOK,
        channel_state,
        on_payload,
        on_problem,
    )
}

pub fn start_review_stream_with_state(
    channel_state: RwSignal<WsChannelState>,
    on_payload: impl Fn(shared_types::ReviewRuntimeSnapshot) + 'static,
    on_problem: impl Fn(ApiProblem) + 'static,
) -> WsStreamHandle {
    start_stream::<shared_types::ReviewRuntimeSnapshot>(
        CH_REVIEW,
        channel_state,
        on_payload,
        on_problem,
    )
}

fn start_stream<P>(
    channel: &'static str,
    channel_state: RwSignal<WsChannelState>,
    on_payload: impl Fn(P) + 'static,
    on_problem: impl Fn(ApiProblem) + 'static,
) -> WsStreamHandle
where
    P: DeserializeOwned + 'static,
{
    let payload = Rc::new(on_payload);
    let problem = Rc::new(on_problem);
    if let Some(runtime) = current_ws_runtime() {
        let payload_handler = Rc::clone(&payload);
        let value_handler =
            Rc::new(move |value: &Value| decode_payload(channel, value, payload_handler.as_ref()));
        return runtime.subscribe(RuntimeSubscription {
            channel,
            channel_state,
            on_payload: value_handler,
            on_problem: problem,
        });
    }

    let missing = ws_runtime_missing_problem(channel);
    channel_state.update(|state| {
        ensure_channel(state, channel);
        state.status = WsStatus::Disconnected;
        state.subscribed = false;
        mark_channel_problem(state, &missing, now_ms());
    });
    problem(missing);
    WsStreamHandle::inactive()
}

pub(crate) fn ack_includes_channel(subscribed: &[String], channel: &str) -> bool {
    subscribed.iter().any(|item| item == channel)
}

pub(crate) fn ws_problem(channel: &str, code: &str, detail: impl std::fmt::Display) -> ApiProblem {
    ApiProblem::new(code, format!("ws[{channel}] {detail}")).with_source("frontend-ws")
}

pub(crate) fn ws_reconnect_problem(
    channel: &str,
    code: &str,
    detail: impl std::fmt::Display,
) -> ApiProblem {
    ws_problem(channel, code, detail).with_retry_after_ms(Some(RECONNECT_DELAY_MS.into()))
}

pub(crate) fn ws_server_problem(
    channel: &str,
    code: &str,
    message: &str,
    retry_after_ms: Option<u64>,
    request_id: Option<String>,
) -> ApiProblem {
    ws_problem(channel, code, message)
        .with_retry_after_ms(retry_after_ms)
        .with_request_id(request_id)
}

pub(crate) fn ws_runtime_missing_problem(channel: &str) -> ApiProblem {
    ws_problem(
        channel,
        "WS_RUNTIME_MISSING",
        "runtime missing; refusing unauthenticated legacy websocket fallback",
    )
}

pub(crate) fn mark_channel_message(state: &mut WsChannelState, observed_at_ms: u64) {
    state.last_message_at_ms = Some(observed_at_ms);
    state.last_error = None;
    state.retry_after_ms = None;
    state.message_count = state.message_count.saturating_add(1);
}

pub(crate) fn mark_channel_problem(
    state: &mut WsChannelState,
    problem: &ApiProblem,
    observed_at_ms: u64,
) {
    let repeated_unresolved_problem = state.last_error.as_ref() == Some(problem);
    state.last_error = Some(problem.clone());
    state.retry_after_ms = problem.retry_after_ms;
    if !repeated_unresolved_problem {
        state.problem_count = state.problem_count.saturating_add(1);
    }
    state.last_problem_at_ms = Some(observed_at_ms);
}

fn ensure_channel(state: &mut WsChannelState, channel: &str) {
    if state.channel != channel {
        state.channel = channel.to_owned();
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn now_ms() -> u64 {
    js_sys::Date::now().max(0.0).round().min(u64::MAX as f64) as u64
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or_default()
}

pub(crate) fn decode_payload<P, F>(
    channel: &str,
    payload: &Value,
    on_payload: &F,
) -> Result<(), ApiProblem>
where
    P: DeserializeOwned,
    F: Fn(P) + ?Sized,
{
    // 借用反序列化：runtime 分发时不再为每个订阅者深拷贝整个 Value。
    match P::deserialize(payload) {
        Ok(resp) => {
            on_payload(resp);
            Ok(())
        }
        Err(e) => {
            leptos::logging::warn!("ws[{channel}] payload decode failed: {e}");
            Err(ws_problem(channel, "WS_PAYLOAD_DECODE", e))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ws_problem_carries_channel_code_and_source() {
        let problem = ws_problem("arbitrage", "WS_DECODE", "bad json");
        assert_eq!(problem.code, "WS_DECODE");
        assert!(problem.message.contains("arbitrage"));
        assert!(problem.message.contains("bad json"));
        assert_eq!(problem.source.as_deref(), Some("frontend-ws"));
    }

    #[test]
    fn ws_transport_problem_codes_are_distinct() {
        let open = ws_reconnect_problem("portfolio", "WS_OPEN_FAILED", "network down");
        let subscribe = ws_reconnect_problem("portfolio", "WS_SUBSCRIBE_FAILED", "closed");
        let read = ws_reconnect_problem("portfolio", "WS_READ_ERROR", "abnormal close");

        assert_eq!(open.code, "WS_OPEN_FAILED");
        assert_eq!(subscribe.code, "WS_SUBSCRIBE_FAILED");
        assert_eq!(read.code, "WS_READ_ERROR");
        assert_eq!(read.retry_after_ms, Some(5_000));
        assert!(read.message.contains("abnormal close"));
    }

    #[test]
    fn subscribe_ack_must_include_requested_channel() {
        let subscribed = vec![
            "orders".to_string(),
            "execution".to_string(),
            "risk-alerts".to_string(),
        ];

        assert!(ack_includes_channel(&subscribed, "orders"));
        assert!(ack_includes_channel(&subscribed, "execution"));
        assert!(!ack_includes_channel(&subscribed, "portfolio"));
    }

    #[test]
    fn watchlist_alert_stream_contract_is_distinct_from_risk_alerts() {
        assert_ne!(CH_ALERTS, CH_RISK_ALERTS);
        let rules = serde_json::json!({
            "event": "alert_rules_changed",
            "envelope": {
                "rules": [],
                "runtime": {
                    "featureGate": "api_surface.watchlist_alerts",
                    "persistence": "memory",
                    "volatile": true,
                    "restartBehavior": "cleared_on_restart",
                    "publicOrderbookPrewarmLimit": 32,
                    "publicTickerSymbolsPerVenueLimit": 32,
                    "privateWsSymbolsFromWatchlist": 0
                }
            },
            "timestampMs": 42
        });
        let watchlist = serde_json::json!({
            "event": "watchlist_changed",
            "envelope": {
                "items": [],
                "runtime": {
                    "featureGate": "api_surface.watchlist_alerts",
                    "persistence": "memory",
                    "volatile": true,
                    "restartBehavior": "cleared_on_restart",
                    "publicOrderbookPrewarmLimit": 32,
                    "publicTickerSymbolsPerVenueLimit": 32,
                    "privateWsSymbolsFromWatchlist": 0
                }
            },
            "timestampMs": 42
        });

        assert!(serde_json::from_value::<shared_types::AlertStreamEvent>(rules).is_ok());
        assert!(serde_json::from_value::<shared_types::WatchlistStreamEvent>(watchlist).is_ok());
    }

    #[test]
    fn server_error_problem_uses_backend_code_and_retry() {
        let problem = ws_server_problem(
            "system",
            "WS_CHANNEL_REJECTED",
            "rejected channel(s): unknown",
            Some(2_000),
            Some("rid-ws".into()),
        );

        assert_eq!(problem.code, "WS_CHANNEL_REJECTED");
        assert_eq!(problem.retry_after_ms, Some(2_000));
        assert_eq!(problem.request_id.as_deref(), Some("rid-ws"));
        assert!(problem.message.contains("system"));
        assert!(problem.message.contains("rejected channel"));
    }

    #[test]
    fn typed_payload_decode_returns_problem_to_shared_runtime() {
        let decoded = std::cell::Cell::new(0_u32);

        let result =
            decode_payload::<u32, _>("orders", &Value::String("invalid".into()), &|value| {
                decoded.set(value);
            });

        assert_eq!(decoded.get(), 0);
        assert_eq!(
            result.as_ref().err().map(|problem| problem.code.as_str()),
            Some("WS_PAYLOAD_DECODE")
        );
        assert_eq!(
            result
                .as_ref()
                .err()
                .and_then(|problem| problem.source.as_deref()),
            Some("frontend-ws")
        );
    }

    #[test]
    fn review_runtime_payload_decodes_both_projections() {
        let snapshot = shared_types::ReviewRuntimeSnapshot {
            executed: shared_types::ReviewEnvelope::new(
                Vec::<shared_types::ExecutedTrade>::new(),
                42,
                30,
                shared_types::ReviewDataSource::ExecutionLedger,
                Some(shared_types::ReviewLedgerStatus::NoLedgerEvents),
                Vec::new(),
            ),
            strategy_performance: shared_types::ReviewEnvelope::new(
                Vec::<shared_types::StrategyPerformance>::new(),
                42,
                30,
                shared_types::ReviewDataSource::ExecutionLedger,
                Some(shared_types::ReviewLedgerStatus::NoLedgerEvents),
                Vec::new(),
            ),
            generated_at_ms: 42,
        };
        let payload = serde_json::to_value(snapshot).unwrap_or(Value::Null);
        let observed = std::cell::Cell::new(0_i64);

        let result = decode_payload::<shared_types::ReviewRuntimeSnapshot, _>(
            CH_REVIEW,
            &payload,
            &|snapshot| observed.set(snapshot.generated_at_ms),
        );

        assert!(result.is_ok());
        assert_eq!(observed.get(), 42);
    }

    #[test]
    fn missing_runtime_problem_blocks_legacy_fallback() {
        let problem = ws_runtime_missing_problem("orders");

        assert_eq!(problem.code, "WS_RUNTIME_MISSING");
        assert_eq!(problem.source.as_deref(), Some("frontend-ws"));
        assert!(problem.message.contains("orders"));
        assert!(problem.message.contains("legacy websocket fallback"));
    }

    #[test]
    fn portfolio_payload_decodes_close_run_update_without_snapshot_fields() {
        let payload = serde_json::json!({
            "event": "close_run_updated",
            "timestampMs": 42,
            "closeRun": {
                "id": "close-1",
                "scope": "pair",
                "status": "unwind_required",
                "snapshotVersion": "pos-1",
                "expectedLegCount": 2,
                "legs": [],
                "submittedOrderCount": 1,
                "failedLegCount": 1,
                "nakedExposureUsd": 12.0,
                "message": "manual compensation required",
                "startedAtMs": 1,
                "updatedAtMs": 2
            }
        });

        let decoded = serde_json::from_value::<PortfolioStreamPayload>(payload);

        assert!(matches!(
            &decoded,
            Ok(PortfolioStreamPayload::CloseRun { .. })
        ));
        if let Ok(PortfolioStreamPayload::CloseRun { close_run, .. }) = decoded {
            assert_eq!(close_run.id, "close-1");
            assert_eq!(
                close_run.status,
                shared_types::CloseRunStatus::UnwindRequired
            );
        }
    }

    #[test]
    fn portfolio_payload_decodes_error_envelope_without_snapshot() {
        let payload = serde_json::json!({
            "status": "error",
            "source": "portfolio_lifecycle",
            "observedAtMs": 42,
            "problem": {
                "code": "PORTFOLIO_SNAPSHOT_UNAVAILABLE",
                "message": "portfolio snapshot unavailable",
                "retryAfterMs": 2000,
                "source": "portfolio_lifecycle"
            },
            "retryAfterMs": 2000
        });

        let decoded = serde_json::from_value::<PortfolioStreamPayload>(payload);

        assert!(matches!(decoded, Ok(PortfolioStreamPayload::Envelope(_))));
    }

    #[test]
    fn degraded_portfolio_envelope_with_snapshot_reports_problem() {
        let envelope = shared_types::PortfolioSnapshotEnvelope {
            status: shared_types::PortfolioSnapshotStatus::Stale,
            source: "portfolio_lifecycle".to_owned(),
            observed_at_ms: 42,
            snapshot: Some(portfolio_snapshot("pos-ws")),
            problem: None,
            problems: Vec::new(),
            operation_health: Vec::new(),
            retry_after_ms: Some(2_000),
        };
        let snapshot_version = std::cell::RefCell::new(None::<String>);
        let problem_code = std::cell::RefCell::new(None::<String>);

        handle_portfolio_envelope(
            envelope,
            &|snapshot| {
                snapshot_version.replace(Some(snapshot.snapshot_version));
            },
            &|problem| {
                problem_code.replace(Some(problem.code));
            },
        );

        assert_eq!(snapshot_version.borrow().as_deref(), Some("pos-ws"));
        assert_eq!(
            problem_code.borrow().as_deref(),
            Some(shared_types::problem::codes::PORTFOLIO_SNAPSHOT_STALE)
        );
    }

    #[test]
    fn channel_state_tracks_problem_without_clearing_message_time() {
        let mut state = WsChannelState::new("orders");
        state.last_message_at_ms = Some(42);
        let problem = ApiProblem::new("WS_READ_ERROR", "closed").with_retry_after_ms(Some(5_000));

        mark_channel_problem(&mut state, &problem, 84);

        assert_eq!(state.last_message_at_ms, Some(42));
        assert_eq!(
            state
                .last_error
                .as_ref()
                .map(|problem| problem.code.as_str()),
            Some("WS_READ_ERROR")
        );
        assert_eq!(state.retry_after_ms, Some(5_000));
        assert_eq!(state.problem_count, 1);
        assert_eq!(state.last_problem_at_ms, Some(84));

        mark_channel_message(&mut state, 126);
        assert_eq!(state.message_count, 1);
        assert_eq!(state.problem_count, 1);
        assert_eq!(state.last_message_at_ms, Some(126));
        assert!(state.last_error.is_none());
        assert_eq!(state.last_problem_at_ms, Some(84));
    }

    #[test]
    fn channel_state_counts_repeated_unresolved_problem_once() {
        let mut state = WsChannelState::new("arbitrage");
        let problem =
            ApiProblem::new("WS_PAYLOAD_DECODE", "invalid payload").with_source("frontend-ws");

        mark_channel_problem(&mut state, &problem, 84);
        mark_channel_problem(&mut state, &problem, 126);

        assert_eq!(state.problem_count, 1);
        assert_eq!(state.last_problem_at_ms, Some(126));
        assert_eq!(state.last_error.as_ref(), Some(&problem));
    }

    #[test]
    fn channel_state_counts_same_problem_after_successful_frame_again() {
        let mut state = WsChannelState::new("arbitrage");
        let problem =
            ApiProblem::new("WS_PAYLOAD_DECODE", "invalid payload").with_source("frontend-ws");

        mark_channel_problem(&mut state, &problem, 84);
        mark_channel_message(&mut state, 126);
        mark_channel_problem(&mut state, &problem, 168);

        assert_eq!(state.message_count, 1);
        assert_eq!(state.problem_count, 2);
        assert_eq!(state.last_problem_at_ms, Some(168));
    }

    fn portfolio_snapshot(snapshot_version: &str) -> shared_types::PortfolioSnapshot {
        shared_types::PortfolioSnapshot {
            summary: shared_types::PortfolioSummary {
                total_nav_usd: 0.0,
                nav_evidence: shared_types::PortfolioNavEvidence::default(),
                nav_change_24h_pct: Some(0.0),
                net_delta_usd: 0.0,
                net_delta_pct_of_nav: 0.0,
                naked_exposure_usd: 0.0,
                naked_position_count: 0,
                realized_pnl_today_usd: 0.0,
                pnl_breakdown: shared_types::PnlBreakdown::default(),
                updated_at_ms: 1,
            },
            positions: Vec::new(),
            balances: Vec::new(),
            risk: shared_types::RiskSnapshot {
                var_99_1d_usd: 0.0,
                var_pct_of_nav: 0.0,
                var_sample_size: 0,
                funding_clustering: Vec::new(),
                delta_concentration: Vec::new(),
                margin_utilization: Vec::new(),
                hard_limits: shared_types::HardLimitsUsage::default(),
                updated_at_ms: 1,
            },
            server_now_ms: 1,
            snapshot_version: snapshot_version.to_owned(),
            degraded: false,
            problems: Vec::new(),
            operation_health: Vec::new(),
            account_state: shared_types::AccountStateSnapshot::default(),
            recent_close_runs: Vec::new(),
        }
    }
}
