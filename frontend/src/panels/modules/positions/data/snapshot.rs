//! Portfolio 快照 / NAV 历史的数据运行态（`TableRuntime` 数据层的一半）。
//!
//! 这里只负责「拉取 + 落 `LoadState`」：2s 轮询兜底 WS、请求版本闸（只认最新一次
//! 响应）、degraded envelope 落 `Stale`（保留快照 + 原因），以及把 WS 推来的
//! `CloseRun` 在首包前排队、首包到达后回灌。提交类动作见兄弟模块 [`super::close`]。

use crate::api::rest::{
    portfolio_envelope_degraded_problem, portfolio_envelope_problem, ApiClient,
};
use crate::api::ws::{start_portfolio_stream_with_state, WsChannelState};
use crate::state::context::use_global;
use crate::state::load_state::LoadState;
use crate::state::polling::{use_conditional_polling_result, use_ws_channel_fallback_polling};
use leptos::prelude::*;
use leptos::task::spawn_local;
use shared_types::{
    ApiProblem, CloseRun, HistoryResponse, PortfolioSnapshot, PortfolioSnapshotEnvelope,
};
use std::time::Duration;

mod gate;
mod problem;
#[cfg(test)]
pub(in crate::panels::modules::positions) use gate::snapshot_response_is_latest;
pub(in crate::panels::modules::positions) use gate::SnapshotRequestGate;
use gate::{invalidate_snapshot_requests, next_snapshot_request_gate};
use problem::raw_snapshot_degraded_problem;

const SNAPSHOT_POLL_INTERVAL: Duration = Duration::from_secs(2);
const SNAPSHOT_WS_GRACE: Duration = Duration::from_secs(8);
const SNAPSHOT_WS_STALE_AFTER: Duration = Duration::from_secs(4);
const RECENT_CLOSE_RUN_LIMIT: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::panels::modules::positions) enum SnapshotSourceKind {
    LiveWs,
    PollingRest,
    Offline,
}

pub(in crate::panels::modules::positions) type PortfolioNavHistoryState =
    RwSignal<LoadState<HistoryResponse<shared_types::history::PortfolioNavHistoryRow>>>;
type SnapshotFetchResult = (
    SnapshotRequestGate,
    Result<PortfolioSnapshotEnvelope, ApiProblem>,
);

/// 持仓页快照运行态：行集 `LoadState` + WS 通道传输态 + 轮询兜底开关。
///
/// `transport`/`poll_active` 让视图层能区分快照来自实时 WS 还是 REST 轮询兜底，
/// 并显示新鲜度/订阅/断线 retry，见 [`super::super::components::SnapshotTransportChip`]。
#[derive(Clone, Copy)]
pub(in crate::panels::modules::positions) struct PortfolioSnapshotRuntime {
    pub(in crate::panels::modules::positions) snapshot: RwSignal<LoadState<PortfolioSnapshot>>,
    pub(in crate::panels::modules::positions) transport: RwSignal<WsChannelState>,
    pub(in crate::panels::modules::positions) poll_active: RwSignal<bool>,
    pub(in crate::panels::modules::positions) source: RwSignal<Option<SnapshotSourceKind>>,
}

pub(in crate::panels::modules::positions) fn use_portfolio_snapshot_state(
    refresh_nonce: RwSignal<u64>,
    snapshot: RwSignal<LoadState<PortfolioSnapshot>>,
) -> PortfolioSnapshotRuntime {
    let pending_close_runs = RwSignal::new(Vec::<CloseRun>::new());
    let client = use_global().client;
    let request_version = RwSignal::new(0_u64);
    let manual_fetching = RwSignal::new(false);
    let source = RwSignal::new(None::<SnapshotSourceKind>);
    let refresh_initialized = StoredValue::new_local(false);

    Effect::new({
        let client = client.clone();
        move |_| {
            refresh_nonce.get();
            if !refresh_initialized.get_value() {
                refresh_initialized.set_value(true);
                return;
            }
            refresh_portfolio_snapshot(
                client.clone(),
                snapshot,
                pending_close_runs,
                request_version,
                manual_fetching,
                source,
            );
        }
    });

    let ws_channel_state = RwSignal::new(WsChannelState::new("portfolio"));
    let poll_enabled = use_ws_channel_fallback_polling(
        ws_channel_state,
        SNAPSHOT_WS_GRACE,
        SNAPSHOT_WS_STALE_AFTER,
    );
    let poll = use_conditional_polling_result(
        SNAPSHOT_POLL_INTERVAL,
        move || poll_enabled.try_get_untracked().unwrap_or(false),
        {
            move || {
                let client = client.clone();
                let gate = next_snapshot_request_gate(request_version);
                async move {
                    Ok::<SnapshotFetchResult, ()>((
                        gate,
                        client
                            .portfolio_snapshot_envelope()
                            .await
                            .map_err(|error| error.problem),
                    ))
                }
            }
        },
    );
    Effect::new(move |_| {
        if let Some(Ok((gate, result))) = poll.get().and_then(|event| event.take().into_fetched()) {
            if apply_snapshot_result(snapshot, pending_close_runs, gate, result) {
                source.set(Some(SnapshotSourceKind::PollingRest));
            }
        }
    });

    let handle = start_portfolio_stream_with_state(
        ws_channel_state,
        move |latest| {
            invalidate_snapshot_requests(request_version);
            apply_snapshot_update(snapshot, pending_close_runs, latest);
            source.set(Some(SnapshotSourceKind::LiveWs));
        },
        move |run| merge_close_run_update(snapshot, pending_close_runs, run),
        move |problem| snapshot.update(|state| state.apply_result(Err(problem))),
    );
    on_cleanup(move || handle.cancel());

    PortfolioSnapshotRuntime {
        snapshot,
        transport: ws_channel_state,
        poll_active: poll_enabled,
        source,
    }
}

pub(in crate::panels::modules::positions) fn use_portfolio_nav_history_state(
    refresh_nonce: RwSignal<u64>,
    history: PortfolioNavHistoryState,
) -> PortfolioNavHistoryState {
    let client = use_global().client;
    let request_version = RwSignal::new(0_u64);

    Effect::new(move |_| {
        refresh_nonce.get();
        let client = client.clone();
        let gate = next_snapshot_request_gate(request_version);
        spawn_local(async move {
            let result = client
                .portfolio_nav_history()
                .await
                .map_err(|error| error.problem);
            if gate.is_latest() {
                history.update(|state| state.apply_result(result));
            }
        });
    });

    history
}

fn refresh_portfolio_snapshot(
    client: ApiClient,
    snapshot: RwSignal<LoadState<PortfolioSnapshot>>,
    pending_close_runs: RwSignal<Vec<CloseRun>>,
    request_version: RwSignal<u64>,
    manual_fetching: RwSignal<bool>,
    source: RwSignal<Option<SnapshotSourceKind>>,
) {
    if manual_fetching.get_untracked() {
        return;
    }
    manual_fetching.set(true);
    let gate = next_snapshot_request_gate(request_version);
    spawn_local({
        async move {
            let result = client
                .portfolio_snapshot_envelope()
                .await
                .map_err(|error| error.problem);
            if apply_snapshot_result(snapshot, pending_close_runs, gate, result) {
                source.set(Some(SnapshotSourceKind::PollingRest));
            }
            manual_fetching.set(false);
        }
    });
}

pub(in crate::panels::modules::positions) fn apply_snapshot_result(
    snapshot: RwSignal<LoadState<PortfolioSnapshot>>,
    pending_close_runs: RwSignal<Vec<CloseRun>>,
    gate: SnapshotRequestGate,
    result: Result<PortfolioSnapshotEnvelope, ApiProblem>,
) -> bool {
    if !gate.is_latest() {
        return false;
    }
    match result {
        Ok(envelope) => apply_snapshot_envelope(snapshot, pending_close_runs, envelope),
        Err(problem) => {
            snapshot.update(|state| state.apply_result(Err(problem)));
            false
        }
    }
}

fn apply_snapshot_envelope(
    snapshot: RwSignal<LoadState<PortfolioSnapshot>>,
    pending_close_runs: RwSignal<Vec<CloseRun>>,
    mut envelope: PortfolioSnapshotEnvelope,
) -> bool {
    let Some(mut latest) = envelope.snapshot.take() else {
        snapshot.update(|state| state.apply_result(Err(portfolio_envelope_problem(&envelope))));
        return false;
    };
    drain_pending_close_runs(pending_close_runs, &mut latest);
    if let Some(problem) = portfolio_envelope_degraded_problem(&envelope) {
        snapshot.set(LoadState::Stale {
            value: latest,
            problem,
        });
    } else {
        snapshot.set(LoadState::Ready(latest));
    }
    true
}

pub(in crate::panels::modules::positions) fn apply_snapshot_update(
    snapshot: RwSignal<LoadState<PortfolioSnapshot>>,
    pending_close_runs: RwSignal<Vec<CloseRun>>,
    mut latest: PortfolioSnapshot,
) {
    drain_pending_close_runs(pending_close_runs, &mut latest);
    if latest.degraded {
        let problem = raw_snapshot_degraded_problem(&latest);
        snapshot.set(LoadState::Stale {
            value: latest,
            problem,
        });
    } else {
        snapshot.set(LoadState::Ready(latest));
    }
}

pub(in crate::panels::modules::positions) fn merge_close_run_update(
    state: RwSignal<LoadState<PortfolioSnapshot>>,
    pending_close_runs: RwSignal<Vec<CloseRun>>,
    run: CloseRun,
) {
    let mut incoming = Some(run);
    state.update(|state| match state {
        LoadState::Ready(snapshot)
        | LoadState::Stale {
            value: snapshot, ..
        } => {
            if let Some(run) = incoming.take() {
                merge_recent_close_run(snapshot, run);
            }
        }
        LoadState::Loading | LoadState::Error(_) => {}
    });
    if let Some(run) = incoming {
        queue_pending_close_run(pending_close_runs, run);
    }
}

fn drain_pending_close_runs(
    pending_close_runs: RwSignal<Vec<CloseRun>>,
    snapshot: &mut PortfolioSnapshot,
) {
    pending_close_runs.update(|pending| {
        let runs = std::mem::take(pending);
        for run in runs {
            merge_recent_close_run(snapshot, run);
        }
    });
}

fn queue_pending_close_run(pending_close_runs: RwSignal<Vec<CloseRun>>, run: CloseRun) {
    pending_close_runs.update(|pending| push_close_run_bounded(pending, run));
}

fn merge_recent_close_run(snapshot: &mut PortfolioSnapshot, run: CloseRun) {
    push_close_run_bounded(&mut snapshot.recent_close_runs, run);
}

fn push_close_run_bounded(runs: &mut Vec<CloseRun>, run: CloseRun) {
    runs.retain(|existing| existing.id != run.id);
    runs.push(run);
    runs.sort_by_key(|run| std::cmp::Reverse(run.updated_at_ms));
    runs.truncate(RECENT_CLOSE_RUN_LIMIT);
}
