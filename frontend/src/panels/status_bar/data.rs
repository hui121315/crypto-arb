use crate::api::ws::{start_system_stream_with_state, WsChannelState};
use crate::panels::shared::confirmation::ConfirmedAt;
use crate::state::context::use_global;
use crate::state::load_state::LoadState;
use crate::state::polling::{
    max_retry_after_ms, now_ms, polling_allowed, retry_deadline_ms, use_ws_channel_fallback_polling,
};
use crate::state::resource_polling::{apply_resource_envelope, resource_problem};
use crate::state::{
    read_freshness::ReadFreshness,
    read_scope::{bounded_read, ReadScope},
};
use leptos::prelude::*;
use shared_types::{
    OpportunityStreamEvent, SystemHealth, TradingStatusResponse, VenueOperationHealthSnapshot,
};
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_secs(5);
const WS_POLL_GRACE: Duration = Duration::from_secs(8);
const WS_STALE_AFTER: Duration = Duration::from_secs(10);

#[derive(Clone, Copy)]
pub(crate) struct ScanStatusState {
    pub(crate) state: RwSignal<LoadState<OpportunityStreamEvent>>,
}

#[derive(Clone, Copy)]
pub(crate) struct VenueOperationHealthState {
    pub(crate) state: RwSignal<LoadState<VenueOperationHealthSnapshot>>,
}

#[derive(Clone, Copy)]
pub(crate) struct SystemHealthState {
    pub(crate) state: RwSignal<LoadState<SystemHealth>>,
    pub(crate) ws_channel: RwSignal<WsChannelState>,
}

pub(crate) fn use_scan_status_state() -> ScanStatusState {
    let stream = use_global().arbitrage_stream;
    ScanStatusState {
        state: stream.state,
    }
}

pub(crate) fn use_system_health_state() -> SystemHealthState {
    let ws_channel_state = RwSignal::new(WsChannelState::new("system"));
    let poll_enabled =
        use_ws_channel_fallback_polling(ws_channel_state, WS_POLL_GRACE, WS_STALE_AFTER);
    let health = RwSignal::new(LoadState::Loading);
    let revision = RwSignal::new(0_u64);
    let retry_until = RwSignal::new(None);
    let version = RwSignal::new(None::<i64>);
    let freshness = ReadFreshness::new(health, system_health_stale());
    let scope = ReadScope::new(move || {
        revision.update(|value| *value = value.wrapping_add(1));
        health.set(LoadState::Loading);
        retry_until.set(None);
        version.set(None);
        freshness.reset();
    });
    scope.poll(
        POLL_INTERVAL,
        move || {
            retry_until.try_get_untracked().is_some_and(|until| {
                polling_allowed(
                    version.get_untracked().is_none()
                        || freshness.expired.get()
                        || poll_enabled.get_untracked(),
                    until,
                    now_ms(),
                )
            })
        },
        {
            move |client| {
                let started = revision.get_untracked();
                let requested = ConfirmedAt::now();
                async move {
                    let result = bounded_read(client.system_health_envelope()).await;
                    (started, requested, result)
                }
            }
        },
        move |(started, requested, result)| {
            // A slow fallback started before a WS update cannot roll that update back.
            if revision.get_untracked() != started {
                return;
            }
            let retry_after = match &result {
                Ok(envelope) => max_retry_after_ms(None, envelope.problems.iter()),
                Err(problem) => problem.retry_after_ms,
            };
            retry_until.set(retry_deadline_ms(retry_after, now_ms()));
            health.update(|state| match result {
                Ok(envelope) => {
                    if let Some(snapshot) = envelope
                        .data
                        .as_ref()
                        .filter(|_| envelope.status.has_usable_data())
                    {
                        match accept_system_version(
                            version,
                            freshness,
                            snapshot.updated_at_ms,
                            requested,
                        ) {
                            Ok(true) => (),
                            Ok(false) => {
                                if let Some(problem) = resource_problem(&envelope) {
                                    state.apply_result(Err(problem));
                                }
                                return;
                            }
                            Err(problem) => {
                                state.apply_result(Err(problem));
                                return;
                            }
                        }
                    }
                    apply_resource_envelope(state, envelope);
                }
                Err(problem) => state.apply_result(Err(problem)),
            });
        },
    );

    let handle = start_system_stream_with_state(
        ws_channel_state,
        move |latest| {
            if !scope.accepts(&scope.capture()) {
                return;
            }
            match accept_system_version(
                version,
                freshness,
                latest.updated_at_ms,
                ConfirmedAt::now(),
            ) {
                Ok(true) => (),
                Ok(false) => return,
                Err(problem) => {
                    health.update(|state| state.apply_result(Err(problem)));
                    return;
                }
            }
            revision.update(|value| *value = value.wrapping_add(1));
            retry_until.set(None);
            health.set(LoadState::Ready(latest));
        },
        move |problem| {
            if !scope.accepts(&scope.capture()) {
                return;
            }
            revision.update(|value| *value = value.wrapping_add(1));
            health.update(|state| state.apply_result(Err(problem)));
        },
    );
    on_cleanup(move || handle.cancel());
    SystemHealthState {
        state: health,
        ws_channel: ws_channel_state,
    }
}

pub fn use_trading_status_state() -> RwSignal<LoadState<TradingStatusResponse>> {
    crate::state::trading_status::provide_trading_status().state
}

pub(crate) fn use_venue_operation_health_state() -> VenueOperationHealthState {
    let state = RwSignal::new(LoadState::Loading);
    let version = RwSignal::new(None::<i64>);
    let retry_until = RwSignal::new(None);
    let freshness = ReadFreshness::new(state, operation_health_stale());
    let scope = ReadScope::new(move || {
        state.set(LoadState::Loading);
        version.set(None);
        retry_until.set(None);
        freshness.reset();
    });
    scope.poll(
        POLL_INTERVAL,
        move || {
            retry_until
                .try_get_untracked()
                .is_some_and(|until| polling_allowed(true, until, now_ms()))
        },
        move |client| {
            let started = ConfirmedAt::now();
            async move {
                let result = bounded_read(client.venue_operation_health()).await;
                (started, result)
            }
        },
        move |(started, result)| {
            retry_until.set(
                result
                    .as_ref()
                    .err()
                    .and_then(|problem| retry_deadline_ms(problem.retry_after_ms, now_ms())),
            );
            let result = match result {
                Ok(snapshot) => {
                    let previous = version.get_untracked();
                    if snapshot.generated_at_ms <= 0
                        || previous.is_some_and(|value| snapshot.generated_at_ms < value)
                    {
                        Err(shared_types::ApiProblem::new(
                            "OPERATION_HEALTH_UNCONFIRMED",
                            "后台健康快照时间无效或倒退",
                        ))
                    } else if previous == Some(snapshot.generated_at_ms) {
                        return;
                    } else if started.expired() {
                        Err(operation_health_stale())
                    } else {
                        version.set(Some(snapshot.generated_at_ms));
                        freshness.confirm(started);
                        Ok(snapshot)
                    }
                }
                Err(problem) => Err(problem),
            };
            state.update(|state| state.apply_result(result));
        },
    );
    let health = VenueOperationHealthState { state };
    provide_context(health);
    health
}

fn operation_health_stale() -> shared_types::ApiProblem {
    shared_types::ApiProblem::new("OPERATION_HEALTH_STALE", "后台健康状态超过 15 秒未确认")
        .with_source("task_registry")
}

fn system_health_stale() -> shared_types::ApiProblem {
    shared_types::ApiProblem::new("SYSTEM_HEALTH_STALE", "系统快照超过 15 秒未更新")
        .with_source("system_snapshot")
}

fn accept_system_version(
    version: RwSignal<Option<i64>>,
    freshness: ReadFreshness,
    next: i64,
    started: ConfirmedAt,
) -> Result<bool, shared_types::ApiProblem> {
    let previous = version.get_untracked();
    if next <= 0 || previous.is_some_and(|value| next < value) {
        return Err(shared_types::ApiProblem::new(
            "SYSTEM_HEALTH_UNCONFIRMED",
            "系统快照时间无效或倒退",
        )
        .with_source("system_snapshot"));
    }
    if previous == Some(next) {
        return Ok(false);
    }
    if started.expired() {
        return Err(system_health_stale());
    }
    version.set(Some(next));
    freshness.confirm(started);
    Ok(true)
}
