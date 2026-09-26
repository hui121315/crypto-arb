//! Portfolio 快照 / NAV 历史的数据运行态（`TableRuntime` 数据层的一半）。
//!
//! 这里只负责「拉取 + 落 `LoadState`」：2s 轮询兜底 WS、请求版本闸（只认最新一次
//! 响应）、degraded envelope 落 `Stale`（保留快照 + 原因），以及把 WS 推来的
//! `CloseRun` 在首包前排队、首包到达后回灌。提交类动作见兄弟模块 [`super::close`]。

use crate::api::rest::{
    portfolio_envelope_degraded_problem, portfolio_envelope_problem,
};
use crate::api::ws::{start_portfolio_stream_with_state, WsChannelState, WsStatus};
use crate::state::load_state::LoadState;
use crate::state::polling::use_ws_channel_fallback_polling;
use crate::state::read_scope::{bounded_read, ReadScope, ScopedRead};
use leptos::prelude::*;
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

#[derive(Clone, Copy)]
pub(in crate::panels::modules::positions) struct PortfolioNavHistoryRuntime {
    pub state: PortfolioNavHistoryState,
    pub refreshing: RwSignal<bool>,
    pub refresh: Callback<()>,
}

/// 持仓页快照运行态：行集 `LoadState` + WS 通道传输态 + 轮询兜底开关。
///
/// `transport`/`poll_active` 让视图层能区分快照来自实时 WS 还是 REST 轮询兜底，
/// 并显示新鲜度/订阅/断线 retry，见 [`super::super::components::SnapshotTransportChip`]。
#[derive(Clone, Copy)]
pub(in crate::panels::modules::positions) struct PortfolioSnapshotRuntime {
    pub(in crate::panels::modules::positions) snapshot: RwSignal<LoadState<PortfolioSnapshot>>,
    pub(in crate::panels::modules::positions) transport: RwSignal<WsChannelState>,
    pub(in crate::panels::modules::positions) poll_active: Memo<bool>,
    pub(in crate::panels::modules::positions) source: RwSignal<Option<SnapshotSourceKind>>,
}

pub(in crate::panels::modules::positions) fn use_portfolio_snapshot_state(
    refresh_nonce: RwSignal<u64>,
    snapshot: RwSignal<LoadState<PortfolioSnapshot>>,
    scope: ReadScope,
) -> PortfolioSnapshotRuntime {
    let pending_close_runs = RwSignal::new(Vec::<CloseRun>::new());
    let manual_read = scope.request();
    let request_version = RwSignal::new(0_u64);
    let manual_fetching = RwSignal::new(false);
    let source = RwSignal::new(None::<SnapshotSourceKind>);
    let refresh_initialized = StoredValue::new_local(false);

    Effect::new(move |_| {
        scope.track();
        manual_read.cancel();
        invalidate_snapshot_requests(request_version);
        pending_close_runs.set(Vec::new());
        manual_fetching.set(false);
        source.set(None);
    });
    Effect::new({
        move |_| {
            refresh_nonce.get();
            if !refresh_initialized.get_value() {
                refresh_initialized.set_value(true);
                return;
            }
            refresh_portfolio_snapshot(
                manual_read,
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
    let poll_active = Memo::new(move |_| {
        poll_enabled.get()
            || ws_channel_state.get().status == WsStatus::Disconnected
            || snapshot.with(|state| {
                // A subscription ACK is not an account snapshot. Bootstrap once
                // instead of leaving a new connection empty for the WS grace period.
                if state.value().is_none() { return true; }
                // A new close receipt does not refresh account rows. Bypass the
                // WS grace period until the backend snapshot catches up.
                state.value().is_some_and(|latest| {
                    latest.recent_close_runs.iter().any(|run| run.updated_at_ms > latest.server_now_ms)
                })
            })
    });
    scope.poll(
        SNAPSHOT_POLL_INTERVAL,
        move || poll_active.get() && !manual_fetching.get_untracked(),
        move |client| {
            let gate = next_snapshot_request_gate(request_version);
            async move {
                (gate, bounded_read(client.portfolio_snapshot_envelope()).await)
            }
        },
        move |(gate, result)| {
            if apply_snapshot_result(snapshot, pending_close_runs, gate, result) {
                source.set(Some(SnapshotSourceKind::PollingRest));
            }
        },
    );

    let handle = start_portfolio_stream_with_state(
        ws_channel_state,
        move |latest| {
            if !scope.accepts(&scope.capture()) { return; }
            invalidate_snapshot_requests(request_version);
            manual_read.cancel();
            manual_fetching.set(false);
            apply_snapshot_update(snapshot, pending_close_runs, latest);
            source.set(Some(SnapshotSourceKind::LiveWs));
        },
        move |run| {
            if scope.accepts(&scope.capture()) {
                merge_close_run_update(snapshot, pending_close_runs, run);
            }
        },
        move |problem| {
            if scope.accepts(&scope.capture()) {
                snapshot.update(|state| state.apply_result(Err(problem)));
            }
        },
    );
    on_cleanup(move || handle.cancel());

    PortfolioSnapshotRuntime {
        snapshot,
        transport: ws_channel_state,
        poll_active,
        source,
    }
}

pub(in crate::panels::modules::positions) fn use_portfolio_nav_history_state(
    refresh_nonce: RwSignal<u64>,
    history: PortfolioNavHistoryState,
    scope: ReadScope,
) -> PortfolioNavHistoryRuntime {
    let read = scope.request();
    let local_refresh = RwSignal::new(0_u64);
    let refreshing = RwSignal::new(false);
    let refresh = Callback::new(move |()| {
        if !refreshing.get_untracked() {
            refreshing.set(true);
            local_refresh.update(|value| *value = value.wrapping_add(1));
        }
    });

    Effect::new(move |_| {
        scope.track();
        refresh_nonce.get();
        local_refresh.get();
        refreshing.set(true);
        read.run(
            |client| async move { bounded_read(client.portfolio_nav_history()).await },
            move |result| {
                history.update(|state| state.apply_result(result));
                refreshing.set(false);
            },
        );
    });

    PortfolioNavHistoryRuntime {
        state: history,
        refreshing,
        refresh,
    }
}

fn refresh_portfolio_snapshot(
    read: ScopedRead,
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
    read.run(
        |client| async move { bounded_read(client.portfolio_snapshot_envelope()).await },
        move |result| {
            if apply_snapshot_result(snapshot, pending_close_runs, gate, result) {
                source.set(Some(SnapshotSourceKind::PollingRest));
            }
            manual_fetching.set(false);
        },
    );
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
    preserve_newer_close_runs(snapshot, &mut latest);
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
    preserve_newer_close_runs(snapshot, &mut latest);
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

pub(super) fn merge_recent_close_run(snapshot: &mut PortfolioSnapshot, run: CloseRun) {
    push_close_run_bounded(&mut snapshot.recent_close_runs, run);
}

pub(super) fn merge_close_run_receipt(
    state: RwSignal<LoadState<PortfolioSnapshot>>,
    run: CloseRun,
) -> CloseRun {
    let mut latest = run.clone();
    // HTTP and WS share one receipt history; account freshness is unchanged.
    state.try_update(|state| {
        if let LoadState::Ready(snapshot) | LoadState::Stale { value: snapshot, .. } = state {
            merge_recent_close_run(snapshot, run);
            if let Some(saved) = snapshot.recent_close_runs.iter().find(|row| row.id == latest.id) {
                latest = saved.clone();
            }
        }
    });
    latest
}

// Portfolio snapshots and close-run events arrive independently; an older snapshot
// must not roll a confirmed receipt back to an acknowledgement.
fn preserve_newer_close_runs(
    state: RwSignal<LoadState<PortfolioSnapshot>>,
    incoming: &mut PortfolioSnapshot,
) {
    state.with_untracked(|state| {
        if let Some(current) = state.value() {
            for run in &current.recent_close_runs {
                let preserve = incoming
                    .recent_close_runs
                    .iter()
                    .find(|row| row.id == run.id)
                    .map_or(run.updated_at_ms > incoming.server_now_ms, |row| {
                        run.updated_at_ms > row.updated_at_ms
                    });
                if preserve {
                    merge_recent_close_run(incoming, run.clone());
                }
            }
        }
    });
}

fn push_close_run_bounded(runs: &mut Vec<CloseRun>, run: CloseRun) {
    if runs
        .iter()
        .any(|existing| existing.id == run.id && existing.updated_at_ms > run.updated_at_ms)
    {
        return;
    }
    runs.retain(|existing| existing.id != run.id);
    runs.push(run);
    runs.sort_by_key(|run| std::cmp::Reverse(run.updated_at_ms));
    runs.truncate(RECENT_CLOSE_RUN_LIMIT);
}
