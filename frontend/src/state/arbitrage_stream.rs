use crate::api::rest::OpportunityStreamPayload;
use crate::api::ws::{start_arbitrage_stream_with_state, WsChannelState, WsStatus};
use crate::state::load_state::LoadState;
use crate::state::polling::now_ms;
use crate::state::read_scope::ReadScope;
use crate::state::section::{problem_detail_context, recovery_action_context};
use gloo_timers::callback::Timeout;
use leptos::prelude::*;
#[cfg(test)]
use shared_types::OpportunityStreamEventKind;
use shared_types::{ApiProblem, OpportunityListRow, OpportunityStreamEvent};
use std::collections::{HashMap, HashSet};
use std::time::Duration;

const WS_CONNECT_GRACE: Duration = Duration::from_secs(5);
const WS_STALE_AFTER: Duration = Duration::from_secs(10);

#[derive(Clone, Copy)]
pub struct ArbitrageStream {
    pub(crate) scope: ReadScope,
    pub state: RwSignal<LoadState<OpportunityStreamEvent>>,
    pub live_rows: RwSignal<HashMap<String, OpportunityListRow>>,
    pub ws_channel_state: RwSignal<WsChannelState>,
    pub stream_stale: RwSignal<bool>,
    pub retrying: RwSignal<bool>,
    pub retry: Callback<()>,
}

pub fn provide_arbitrage_stream() -> ArbitrageStream {
    let state = RwSignal::new(LoadState::Loading);
    let live_rows = RwSignal::new(HashMap::new());
    let ws_channel_state = RwSignal::new(WsChannelState::new("arbitrage"));
    let stream_stale = RwSignal::new(false);
    let retrying = RwSignal::new(true);
    let revision = RwSignal::new(0_u64);
    let signals = StreamSignals {
        state,
        live_rows,
        stream_stale,
        retrying,
        revision,
    };
    let scope = ReadScope::new(move || {
        state.set(LoadState::Loading);
        live_rows.set(HashMap::new());
        stream_stale.set(false);
        retrying.set(true);
        revision.update(|value| *value = value.wrapping_add(1));
    });
    let pending = StoredValue::new_local(None::<Timeout>);
    // ACKs, errors and reconnects must not extend the first complete snapshot deadline.
    Effect::new(move |_| {
        revision.track();
        let waiting = retrying.get_untracked();
        let delay = if waiting {
            WS_CONNECT_GRACE
        } else {
            WS_STALE_AFTER
        };
        pending.update_value(|slot| {
            *slot = Some(Timeout::new(delay.as_millis() as u32, move || {
                retrying.set(false);
                stream_stale.set(true);
                if state.with_untracked(|state| state.problem().is_none()) {
                    let (code, message) = if state.with_untracked(|state| state.value().is_some()) {
                        (
                            "WS_OPPORTUNITY_SILENT",
                            "机会流未收到新快照；保留上次报价，暂不可构建",
                        )
                    } else {
                        (
                            "WS_OPPORTUNITY_FIRST_FRAME_TIMEOUT",
                            "尚未收到完整的机会快照；请重试连接，不代表没有套利机会",
                        )
                    };
                    set_problem(
                        signals,
                        ApiProblem::new(code, message).with_source("frontend-ws-arbitrage"),
                    );
                }
            }));
        });
    });
    let retry = Callback::new(move |()| {
        if retrying.get_untracked() {
            return;
        }
        retrying.set(true);
        stream_stale.set(true);
        revision.update(|value| *value = value.wrapping_add(1));
        if let Some(runtime) = crate::api::ws_runtime::current_ws_runtime() {
            runtime.request_channel_replay("arbitrage");
        }
    });

    let handle = start_arbitrage_stream_with_state(
        ws_channel_state,
        move |payload| {
            if scope.accepts(&scope.capture()) {
                apply_stream_payload(signals, payload);
            }
        },
        move |problem_event| {
            if scope.accepts(&scope.capture()) {
                set_problem(signals, problem_event);
            }
        },
    );
    on_cleanup(move || {
        pending.update_value(|slot| {
            slot.take();
        });
        handle.cancel();
    });

    ArbitrageStream {
        scope,
        state,
        live_rows,
        ws_channel_state,
        stream_stale,
        retrying,
        retry,
    }
}

#[derive(Clone, Copy)]
struct StreamSignals {
    state: RwSignal<LoadState<OpportunityStreamEvent>>,
    live_rows: RwSignal<HashMap<String, OpportunityListRow>>,
    stream_stale: RwSignal<bool>,
    retrying: RwSignal<bool>,
    revision: RwSignal<u64>,
}

fn apply_stream_payload(signals: StreamSignals, payload: OpportunityStreamPayload) {
    set_stream_event(signals, payload);
}

fn set_stream_event(signals: StreamSignals, event: OpportunityStreamEvent) {
    let mut rows = signals.live_rows.get_untracked();
    let applied = apply_live_rows(&mut rows, &event);
    let windows_complete = std::iter::once(None)
        .chain(
            shared_types::P0_EXECUTABLE_STRATEGY_KINDS
                .iter()
                .copied()
                .map(Some),
        )
        .all(|strategy| {
            event
                .windows
                .iter()
                .any(|window| window.strategy_kind == strategy)
        });
    if !windows_complete || applied == LiveRowsApply::Incomplete {
        set_problem(
            signals,
            ApiProblem::new(
                "WS_OPPORTUNITY_INCOMPLETE",
                "机会快照不完整；请重试读取，暂不可构建",
            )
            .with_source("frontend-ws-arbitrage"),
        );
        return;
    }
    if applied == LiveRowsApply::Changed {
        signals.live_rows.set(rows);
    }
    signals.state.set(LoadState::Ready(event));
    signals.stream_stale.set(false);
    signals.retrying.set(false);
    signals
        .revision
        .update(|value| *value = value.wrapping_add(1));
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LiveRowsApply {
    Changed,
    Unchanged,
    Incomplete,
}

fn apply_live_rows(
    rows: &mut HashMap<String, OpportunityListRow>,
    event: &OpportunityStreamEvent,
) -> LiveRowsApply {
    let expected_ids = stream_window_ids(event);
    let removed = event
        .removed_ids
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let previous_len = rows.len();
    rows.retain(|id, _| expected_ids.contains(id.as_str()) && !removed.contains(id.as_str()));
    let mut projection_changed = rows.len() != previous_len;
    for changed_row in &event.changed_rows {
        if !expected_ids.contains(changed_row.id.as_str()) {
            continue;
        }
        let row_changed = rows.get(&changed_row.id) != Some(changed_row);
        if row_changed {
            rows.insert(changed_row.id.clone(), changed_row.clone());
            projection_changed = true;
        }
    }
    if !expected_ids.iter().all(|id| rows.contains_key(*id)) {
        return LiveRowsApply::Incomplete;
    }
    if projection_changed {
        LiveRowsApply::Changed
    } else {
        LiveRowsApply::Unchanged
    }
}

fn stream_window_ids(event: &OpportunityStreamEvent) -> HashSet<&str> {
    event
        .windows
        .iter()
        .flat_map(|window| window.ids.iter())
        .chain(event.top_ids.iter())
        .map(String::as_str)
        .collect()
}

fn set_problem(signals: StreamSignals, latest: ApiProblem) {
    signals.stream_stale.set(true);
    signals
        .state
        .update(|value| value.apply_result(Err(latest)));
}

/// 把套利实时流失败的 `ApiProblem` 渲染成紧凑可诊断文案（含 HTTP 状态、请求标识与重试时间）。
pub fn stream_problem_label(problem: &ApiProblem) -> String {
    let mut parts = vec![problem.message.clone()];
    if let Some(source) = problem
        .source
        .as_deref()
        .filter(|source| !source.is_empty())
    {
        parts.push(format!("source {source}"));
    }
    if let Some(status) = problem.status {
        parts.push(format!("HTTP {status}"));
    }
    if let Some(request_id) = problem.request_id.as_deref() {
        parts.push(format!("request_id {request_id}"));
    }
    if let Some(retry_after_ms) = problem.retry_after_ms {
        parts.push(format!("retry {retry_after_ms}ms"));
    }
    parts.extend(problem_detail_context(problem));
    if let Some(recovery) = recovery_action_context(problem) {
        parts.push(recovery);
    }
    parts.join(" · ")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamChannelStatusLabel {
    pub text: String,
    pub is_error: bool,
}

pub fn stream_channel_status_label(state: &WsChannelState) -> StreamChannelStatusLabel {
    stream_channel_status_label_at(state, now_ms())
}

pub fn stream_channel_status_label_at(
    state: &WsChannelState,
    observed_at_ms: u64,
) -> StreamChannelStatusLabel {
    if let Some(problem) = state.last_error.as_ref() {
        return StreamChannelStatusLabel {
            text: format!(
                "套利WS异常 · {} · {}",
                stream_problem_label(problem),
                stream_channel_activity_label(state)
            ),
            is_error: true,
        };
    }

    let text = if state.subscribed {
        subscribed_channel_label(state, observed_at_ms)
    } else {
        unsubscribed_channel_label(state.status).to_owned()
    };
    StreamChannelStatusLabel {
        text: format!("{text} · {}", stream_channel_activity_label(state)),
        is_error: false,
    }
}

fn stream_channel_activity_label(state: &WsChannelState) -> String {
    let mut label = format!("帧 {} · 错误 {}", state.message_count, state.problem_count);
    if let Some(observed_at_ms) = state.last_problem_at_ms {
        label.push_str(&format!(" · 末次错误时间 {observed_at_ms}"));
    }
    label
}

fn subscribed_channel_label(state: &WsChannelState, observed_at_ms: u64) -> String {
    let Some(last_message_at_ms) = state.last_message_at_ms else {
        return "套利WS已订阅 · 等待首帧".to_owned();
    };
    let age_ms = observed_at_ms.saturating_sub(last_message_at_ms);
    let age = stream_channel_age_label(age_ms);
    if age_ms >= WS_STALE_AFTER.as_millis() as u64 {
        format!("套利WS静默 · 最后帧 {age}前 · 候选停止更新")
    } else {
        format!("套利WS实时 · 最后帧 {age}前")
    }
}

fn unsubscribed_channel_label(status: WsStatus) -> &'static str {
    match status {
        WsStatus::Connecting => "套利WS连接中",
        WsStatus::Connected => "套利WS未订阅",
        WsStatus::Disconnected => "套利WS未连接 · 候选停止更新",
    }
}

fn stream_channel_age_label(age_ms: u64) -> String {
    if age_ms < 1_000 {
        return format!("{age_ms}ms");
    }
    if age_ms < 60_000 {
        return format!("{}s", (age_ms / 1_000).max(1));
    }
    if age_ms < 3_600_000 {
        return format!("{}m", age_ms / 60_000);
    }
    format!("{}h", age_ms / 3_600_000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::OpportunityQueryScopeMeta;

    #[test]
    fn stream_problem_label_includes_source_and_retry() {
        let problem = ApiProblem::new("RATE_LIMITED", "rate limited")
            .with_source("market-data-cache")
            .with_retry_after_ms(Some(2_000))
            .with_recovery_action(shared_types::ApiRecoveryAction::RetryAfterDelay);

        let label = stream_problem_label(&problem);

        assert!(label.contains("rate limited"));
        assert!(label.contains("source market-data-cache"));
        assert!(label.contains("retry 2000ms"));
        assert!(label.contains("下一步 等待后重试"));
    }

    #[test]
    fn stream_channel_status_label_surfaces_problem_request_id_and_retry() {
        let mut state = WsChannelState::new("arbitrage");
        state.last_error = Some(
            ApiProblem::new("WS_SERVER_ERROR", "server rejected")
                .with_source("frontend-ws")
                .with_request_id(Some("req-ws-1".to_owned()))
                .with_retry_after_ms(Some(2_000)),
        );
        state.problem_count = 1;
        state.last_problem_at_ms = Some(9_000);

        let label = stream_channel_status_label_at(&state, 10_000);

        assert!(label.is_error);
        assert!(label.text.contains("套利WS异常"));
        assert!(label.text.contains("server rejected"));
        assert!(label.text.contains("request_id req-ws-1"));
        assert!(label.text.contains("retry 2000ms"));
        assert!(label.text.contains("帧 0 · 错误 1"));
        assert!(label.text.contains("末次错误时间 9000"));
    }

    #[test]
    fn stream_channel_status_label_reports_subscription_states() {
        let mut state = WsChannelState::new("arbitrage");
        assert_eq!(
            stream_channel_status_label_at(&state, 10_000).text,
            "套利WS未连接 · 候选停止更新 · 帧 0 · 错误 0"
        );

        state.status = WsStatus::Connecting;
        assert_eq!(
            stream_channel_status_label_at(&state, 10_000).text,
            "套利WS连接中 · 帧 0 · 错误 0"
        );

        state.status = WsStatus::Connected;
        assert_eq!(
            stream_channel_status_label_at(&state, 10_000).text,
            "套利WS未订阅 · 帧 0 · 错误 0"
        );

        state.subscribed = true;
        assert_eq!(
            stream_channel_status_label_at(&state, 10_000).text,
            "套利WS已订阅 · 等待首帧 · 帧 0 · 错误 0"
        );
    }

    #[test]
    fn stream_channel_status_label_distinguishes_fresh_and_stale_frames() {
        let mut state = WsChannelState::new("arbitrage");
        state.status = WsStatus::Connected;
        state.subscribed = true;
        state.last_message_at_ms = Some(90_000);

        let fresh = stream_channel_status_label_at(&state, 95_000);
        assert!(!fresh.is_error);
        assert_eq!(fresh.text, "套利WS实时 · 最后帧 5s前 · 帧 0 · 错误 0");

        let stale = stream_channel_status_label_at(&state, 101_000);
        assert!(!stale.is_error);
        assert_eq!(
            stale.text,
            "套利WS静默 · 最后帧 11s前 · 候选停止更新 · 帧 0 · 错误 0"
        );
    }

    #[test]
    fn replay_rows_initialize_the_complete_live_window() {
        let mut rows = HashMap::new();
        let mut replay = stream_event("snap-1", &["a", "b"], &["a", "b"], &[]);
        replay.changed_rows = vec![changed_row("a"), changed_row("b")];

        assert_eq!(apply_live_rows(&mut rows, &replay), LiveRowsApply::Changed);
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn live_delta_replaces_and_removes_rows_without_rest_rebind() {
        let mut rows = [changed_row("a"), changed_row("b")]
            .into_iter()
            .map(|row| (row.id.clone(), row))
            .collect();
        let mut delta = stream_event("snap-2", &["a", "c"], &["c"], &["b"]);
        delta.changed_rows = vec![changed_row("c")];

        assert_eq!(apply_live_rows(&mut rows, &delta), LiveRowsApply::Changed);
        assert!(rows.contains_key("a"));
        assert!(rows.contains_key("c"));
        assert!(!rows.contains_key("b"));
    }

    #[test]
    fn unchanged_delta_does_not_advance_the_live_row_projection() {
        let mut rows = [changed_row("a")]
            .into_iter()
            .map(|row| (row.id.clone(), row))
            .collect();
        let delta = stream_event("snap-2", &["a"], &[], &[]);

        assert_eq!(apply_live_rows(&mut rows, &delta), LiveRowsApply::Unchanged);
    }

    #[test]
    fn incomplete_replay_does_not_publish_a_partial_live_window() {
        let mut rows = HashMap::new();
        let mut replay = stream_event("snap-1", &["a", "b"], &["a"], &[]);
        replay.changed_rows = vec![changed_row("a")];

        assert_eq!(
            apply_live_rows(&mut rows, &replay),
            LiveRowsApply::Incomplete
        );
    }

    fn stream_event(
        snapshot_id: &str,
        top_ids: &[&str],
        changed_ids: &[&str],
        removed_ids: &[&str],
    ) -> OpportunityStreamEvent {
        let cached_at = chrono::Utc::now();
        OpportunityStreamEvent {
            event: OpportunityStreamEventKind::SnapshotInvalidated,
            snapshot_id: snapshot_id.into(),
            scope_meta: OpportunityQueryScopeMeta {
                global_total_count: top_ids.len(),
                strategy_scope_count: top_ids.len(),
                symbol_scope_count: top_ids.len(),
                filtered_count: top_ids.len(),
                page_count: 1,
                candidate_count: top_ids.len(),
                emitted_count: top_ids.len(),
            },
            changed_ids: changed_ids.iter().map(|id| (*id).to_owned()).collect(),
            changed_rows: Vec::new(),
            removed_ids: removed_ids.iter().map(|id| (*id).to_owned()).collect(),
            top_ids: top_ids.iter().map(|id| (*id).to_owned()).collect(),
            windows: Vec::new(),
            main_p0_counts: shared_types::OpportunityCountBreakdown::default(),
            registry_counts: shared_types::OpportunityCountBreakdown::default(),
            meta: shared_types::OpportunityScanMeta::default(),
            status: shared_types::OpportunityEnvelopeStatus::Fresh,
            scope: shared_types::OpportunityEnvelopeScope::MainP0,
            query_key: "scope=main_p0".into(),
            source: "snapshot".into(),
            cached_at,
            observed_at_ms: cached_at.timestamp_millis(),
            freshness_ms: Some(10),
            retry_after_ms: None,
            error: None,
            partial_failures: Vec::new(),
        }
    }

    fn changed_row(id: &str) -> shared_types::OpportunityListRow {
        shared_types::OpportunityListRow {
            id: id.into(),
            symbol: "MU".into(),
            strategy_kind: Some(shared_types::StrategyKind::PerpCross),
            strategy_category: Some(shared_types::StrategyCategory::Futures),
            type_label: "永续跨所".into(),
            spot_leg_mode: None,
            long_leg: changed_leg("binance", 100.0),
            short_leg: changed_leg("okx", 100.1),
            metrics: shared_types::OpportunityListMetrics {
                score: 91.0,
                risk_level: shared_types::RiskLevel::Low,
                net_single_yield: 0.001,
                annualized_funding_bps: Some(365.0),
                one_cycle_net_bps: Some(4.0),
                time_to_settlement_ms: 60_000,
                settlement_countdown_seconds: Some(60),
                liquidity_score: 80.0,
            },
            cost: shared_types::OpportunityListCost {
                verified: true,
                gross_edge_bps: 8.0,
                total_cost_bps: 4.0,
                wear_bps: 1.0,
                one_cycle_net_bps: Some(4.0),
                one_cycle_covers_cost: true,
                breakeven_periods: 1,
                breakeven_hours: 8.0,
                recommended_hold_hours: 8.0,
                net_bps_at_recommended_hold: 4.0,
                fee_evidence_count: 2,
                fee_evidence_complete: true,
                fee_evidence_ids: vec!["fee:binance:perp:vip0".into(), "fee:okx:perp:vip0".into()],
                one_cycle_penalty: 0.0,
            },
            execution: shared_types::OpportunityListExecution {
                eligible: true,
                blockers: Vec::new(),
                optimal_position: 1_000.0,
                max_position: 2_000.0,
            },
            data_source: "test".into(),
            updated_at: chrono::Utc::now(),
        }
    }

    fn changed_leg(venue: &str, price: f64) -> shared_types::OpportunityListLeg {
        shared_types::OpportunityListLeg {
            venue: venue.into(),
            action: format!("{venue} 做多永续"),
            price: Some(price),
            market_evidence: None,
            funding: None,
        }
    }
}
