use super::freshness::{QuoteFreshness, SnapshotClock};
use crate::api::rest::{with_mutation_timeout, ApiError};
use crate::panels::modules::opportunity_counts::snapshot_clock;
use crate::panels::shared::operation_journal::{validate_setting_response, OperationJournal};
use crate::state::load_state::LoadState;
use crate::state::module_runtime::{ModuleRuntimeState, ModuleRuntimeStatus};
use crate::state::polling::{now_ms, use_conditional_polling_result, use_debounced_string};
use crate::state::read_scope::bounded_read;
use futures::future::{AbortHandle, Abortable};
use gloo_timers::callback::Interval;
use leptos::prelude::*;
use leptos::task::spawn_local;
use shared_types::{
    ActionRunKind, ActionRunStatus, GateCrossExMode, GateCrossExModeConfigPatch,
    GateCrossExModeSnapshot, GateCrossExRouteCatalogResponse, GATE_CROSSEX_SELECTED_ROUTE_LIMIT,
};
use std::time::Duration;

#[derive(Clone, Copy)]
pub(in crate::panels) struct GateCrossExRuntime {
    requests: RequestState,
    recheck: Callback<()>,
}

pub(in crate::panels) fn create_gate_crossex_runtime() -> GateCrossExRuntime {
    // Configuration writes and drafts belong to the workstation, not a mounted page.
    let requests = RequestState {
        journal: OperationJournal::new("crossex"),
        status: RwSignal::new(LoadState::Loading),
        reading: RwSignal::new(false),
        read_generation: RwSignal::new(0),
        active_read: StoredValue::new(None),
        revision: RwSignal::new(0),
        retry_at: RwSignal::new(0),
        notice: RwSignal::new(None),
        minimum: RwSignal::new(String::new()),
        minimum_dirty: RwSignal::new(false),
        timing: RwSignal::new(None),
        clock: RwSignal::new(snapshot_clock()),
    };
    Effect::new(move |_| {
        requests.journal.connection.track();
        requests.invalidate_reads();
        requests.status.set(LoadState::Loading);
        requests.timing.set(None);
        requests.minimum.set(String::new());
        requests.minimum_dirty.set(false);
        requests.notice.set(None);
    });
    let recheck = requests
        .journal
        .recheck(Callback::new(move |run: shared_types::ActionRun| {
            requests.invalidate_reads();
            requests.status.set(LoadState::Loading);
            requests
                .notice
                .set(Some(if run.status == ActionRunStatus::Succeeded {
                    "原配置保存已核对".to_owned()
                } else {
                    format!(
                        "上次保存未成功：{}",
                        run.problem.map_or(run.message, |problem| problem.message)
                    )
                }));
            // A receipt is historical: reread current settings after the journal unlocks.
            spawn_local(read_status(requests));
        }));
    GateCrossExRuntime { requests, recheck }
}

impl GateCrossExRuntime {
    pub(in crate::panels) fn module_runtime_state(self) -> ModuleRuntimeState {
        if self.requests.journal.locked() {
            ModuleRuntimeState::from_action_state(&shared_types::ActionState::pending(
                if self.requests.journal.busy.get() {
                    "正在核对 CrossEx 配置"
                } else {
                    "CrossEx 配置结果待核对"
                },
            ))
        } else {
            let state = self.requests.status.get();
            let problem = self
                .requests
                .freshness()
                .problem
                .or_else(|| state.value().and_then(|row| row.problem.clone()));
            ModuleRuntimeState::combine([
                ModuleRuntimeState::from_load_state(&state),
                problem.map_or_else(ModuleRuntimeState::ready, |problem| ModuleRuntimeState {
                    status: ModuleRuntimeStatus::Stale,
                    problem: Some(problem),
                    pending_label: None,
                }),
            ])
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct GateCrossExData {
    pub status: RwSignal<LoadState<GateCrossExModeSnapshot>>,
    pub market: Memo<QuoteFreshness>,
    pub catalog: RwSignal<LoadState<GateCrossExRouteCatalogResponse>>,
    pub search: RwSignal<String>,
    pub notice: RwSignal<Option<String>>,
    pub saving: RwSignal<bool>,
    pub journal: OperationJournal,
    pub recheck: Callback<()>,
    pub reading: RwSignal<bool>,
    pub minimum: RwSignal<String>,
    pub minimum_dirty: RwSignal<bool>,
    pub set_mode: Callback<GateCrossExMode>,
    pub save_minimum: Callback<()>,
    pub toggle_route: Callback<String>,
    pub remove_route: Callback<String>,
    pub clear_routes: Callback<()>,
    pub refresh: Callback<()>,
    pub refresh_catalog: Callback<()>,
}

#[derive(Clone, Copy)]
struct RequestState {
    journal: OperationJournal,
    status: RwSignal<LoadState<GateCrossExModeSnapshot>>,
    reading: RwSignal<bool>,
    read_generation: RwSignal<u64>,
    active_read: StoredValue<Option<AbortHandle>>,
    revision: RwSignal<u64>,
    retry_at: RwSignal<u64>,
    notice: RwSignal<Option<String>>,
    minimum: RwSignal<String>,
    minimum_dirty: RwSignal<bool>,
    timing: RwSignal<Option<SnapshotClock>>,
    clock: RwSignal<(i64, i64)>,
}

impl RequestState {
    fn invalidate_reads(self) {
        self.active_read.update_value(|active| {
            if let Some(abort) = active.take() {
                abort.abort();
            }
        });
        self.revision.update(|value| *value = value.wrapping_add(1));
        self.read_generation
            .update(|value| *value = value.wrapping_add(1));
        self.reading.set(false);
        self.retry_at.set(0);
    }

    fn freshness(self) -> QuoteFreshness {
        QuoteFreshness::new(&self.status.get(), self.timing.get(), self.clock.get())
    }

    fn accept_snapshot(self, snapshot: GateCrossExModeSnapshot, requested: (i64, i64)) {
        let received = snapshot_clock();
        self.clock.set(received);
        self.timing.update(|timing| {
            *timing = SnapshotClock::advance(*timing, snapshot.observed_at_ms, requested, received);
        });
        self.status.set(LoadState::Ready(snapshot));
    }
}

pub(super) fn use_gate_crossex_data(runtime: GateCrossExRuntime) -> GateCrossExData {
    let requests = runtime.requests;
    let journal = requests.journal;
    let status = requests.status;
    requests.clock.set(snapshot_clock());
    let interval = StoredValue::new_local(Some(Interval::new(1_000, move || {
        requests.clock.set(snapshot_clock());
    })));
    on_cleanup(move || {
        requests.invalidate_reads();
        interval.update_value(|slot| {
            slot.take();
        })
    });
    let market = Memo::new(move |_| requests.freshness());
    let poll = use_conditional_polling_result(
        Duration::from_millis(750),
        move || {
            requests.reading.try_get_untracked() == Some(false)
                && journal.busy.try_get_untracked() == Some(false)
                && requests
                    .retry_at
                    .try_get_untracked()
                    .is_some_and(|at| now_ms() >= at)
        },
        move || async move {
            read_status(requests).await;
            Ok::<_, ()>(())
        },
    );
    Effect::new(move |_| {
        let _ = poll.get();
    });
    Effect::new(move |_| {
        if let Some(snapshot) = status.get().value() {
            if !requests.minimum_dirty.get_untracked() && !journal.busy.get_untracked() {
                requests
                    .minimum
                    .set(snapshot.config.min_gross_spread_pct.to_string());
            }
        }
    });

    let catalog = RwSignal::new(LoadState::Loading);
    let search = RwSignal::new(String::new());
    let search_revision = RwSignal::new(0_u64);
    let catalog_retry = RwSignal::new(0_u64);
    let catalog_read = StoredValue::new(None::<AbortHandle>);
    let cancel_catalog = move || {
        catalog_read.update_value(|active| {
            if let Some(abort) = active.take() {
                abort.abort();
            }
        });
    };
    on_cleanup(cancel_catalog);
    Effect::new(move |_| {
        let _ = search.get();
        journal.connection.track();
        cancel_catalog();
        search_revision.update(|revision| *revision = revision.wrapping_add(1));
        catalog.set(LoadState::Loading);
    });
    let debounced = use_debounced_string(move || search.get(), Duration::from_millis(180));
    Effect::new(move |_| {
        let query = debounced.get();
        let raw = search.get();
        let _ = catalog_retry.get();
        let connection = journal.connection.get();
        if query != raw {
            return;
        }
        let version = search_revision.get_untracked();
        let client = journal.client().cancelable_reads();
        cancel_catalog();
        let (abort, registration) = AbortHandle::new_pair();
        catalog_read.set_value(Some(abort));
        catalog.set(LoadState::Loading);
        spawn_local(async move {
            let Ok(result) = Abortable::new(
                bounded_read(client.gate_crossex_routes(&query)),
                registration,
            ).await else { return; };
            if journal.connection.try_get_untracked() == Some(connection)
                && search.try_get_untracked().as_ref() == Some(&query)
                && search_revision.try_get_untracked() == Some(version)
            {
                catalog_read.set_value(None);
                let _ = catalog.try_update(|state| state.apply_result(result));
            }
        });
    });
    let refresh_catalog = Callback::new(move |_| {
        search_revision.update(|revision| *revision = revision.wrapping_add(1));
        catalog_retry.update(|revision| *revision = revision.wrapping_add(1));
    });
    let refresh = Callback::new(move |_| {
        spawn_local(read_status(requests));
    });
    let set_mode = Callback::new(move |mode| {
        if status
            .get_untracked()
            .value()
            .is_some_and(|snapshot| snapshot.config.mode == mode)
        {
            return;
        }
        submit_patch(
            requests,
            GateCrossExModeConfigPatch {
                mode: Some(mode),
                ..Default::default()
            },
        );
    });
    let save_minimum =
        Callback::new(
            move |_| match parse_minimum(&requests.minimum.get_untracked()) {
                Ok(value) => submit_patch(
                    requests,
                    GateCrossExModeConfigPatch {
                        min_gross_spread_pct: Some(value),
                        ..Default::default()
                    },
                ),
                Err(message) => requests.notice.set(Some(message.to_owned())),
            },
        );
    let toggle_route = Callback::new(move |native_symbol: String| {
        let current = status.get_untracked();
        let LoadState::Ready(snapshot) = current else {
            return;
        };
        let mut routes = snapshot.config.selected_routes;
        if let Some(index) = routes.iter().position(|route| route == &native_symbol) {
            routes.remove(index);
        } else if routes.len() < GATE_CROSSEX_SELECTED_ROUTE_LIMIT {
            routes.push(native_symbol);
        } else {
            requests.notice.set(Some(format!(
                "最多选择 {GATE_CROSSEX_SELECTED_ROUTE_LIMIT} 条路由"
            )));
            return;
        }
        submit_patch(
            requests,
            GateCrossExModeConfigPatch {
                selected_routes: Some(routes),
                ..Default::default()
            },
        );
    });
    let remove_route = Callback::new(move |native: String| {
        let current = status.get_untracked();
        let Some(snapshot) = current.value() else {
            return;
        };
        if !snapshot.config.selected_routes.contains(&native) {
            return;
        }
        submit_patch(
            requests,
            GateCrossExModeConfigPatch {
                selected_routes: Some(
                    snapshot
                        .config
                        .selected_routes
                        .iter()
                        .filter(|route| **route != native)
                        .cloned()
                        .collect(),
                ),
                ..Default::default()
            },
        );
    });
    let clear_routes = Callback::new(move |_| {
        if status
            .get_untracked()
            .value()
            .is_some_and(|row| !row.config.selected_routes.is_empty())
        {
            submit_patch(
                requests,
                GateCrossExModeConfigPatch {
                    selected_routes: Some(Vec::new()),
                    ..Default::default()
                },
            );
        }
    });
    GateCrossExData {
        status,
        market,
        catalog,
        search,
        notice: requests.notice,
        saving: journal.busy,
        journal,
        recheck: runtime.recheck,
        reading: requests.reading,
        minimum: requests.minimum,
        minimum_dirty: requests.minimum_dirty,
        set_mode,
        save_minimum,
        toggle_route,
        remove_route,
        clear_routes,
        refresh,
        refresh_catalog,
    }
}

async fn read_status(state: RequestState) {
    if state.reading.try_get_untracked() != Some(false)
        || state.journal.busy.try_get_untracked() != Some(false)
    {
        return;
    }
    state.reading.set(true);
    state
        .read_generation
        .update(|value| *value = value.wrapping_add(1));
    let generation = state.read_generation.get_untracked();
    let epoch = state.journal.epoch.get_untracked();
    let revision = state.revision.get_untracked();
    let requested = snapshot_clock();
    let client = state.journal.client().cancelable_reads();
    let (abort, registration) = AbortHandle::new_pair();
    state.active_read.set_value(Some(abort));
    let Ok(result) = Abortable::new(
        bounded_read(client.gate_crossex_mode()),
        registration,
    ).await else { return; };
    if state.read_generation.try_get_untracked() != Some(generation) {
        return;
    }
    state.active_read.set_value(None);
    state.reading.set(false);
    if !state.journal.current(epoch) || state.revision.get_untracked() != revision {
        return;
    }
    state.retry_at.set(result.as_ref().err().map_or(0, |error| {
        now_ms().saturating_add(error.retry_after_ms.unwrap_or(3_000))
    }));
    match result {
        Ok(snapshot) => state.accept_snapshot(snapshot, requested),
        Err(problem) => state
            .status
            .update(|current| current.apply_result(Err(problem))),
    }
}

fn submit_patch(state: RequestState, patch: GateCrossExModeConfigPatch) {
    if !matches!(state.status.get_untracked(), LoadState::Ready(_)) {
        return;
    }
    let Some(attempt) = state.journal.begin(
        ActionRunKind::GateCrossExModeUpdate,
        "gate_crossex".to_owned(),
    ) else {
        return;
    };
    let epoch = state.journal.epoch.get_untracked();
    state.invalidate_reads();
    state.notice.set(Some("正在保存…".to_owned()));
    let requested = snapshot_clock();
    let client = state.journal.client();
    spawn_local(async move {
        let result = with_mutation_timeout(
            "保存 CrossEx 配置",
            client.update_gate_crossex_mode_with_context(&patch, &attempt.context),
        )
        .await
        .and_then(|snapshot| {
            validate_setting_response(&attempt, &snapshot)?;
            let routes_match = patch.selected_routes.as_ref().is_none_or(|routes| {
                let normalize = |routes: &[String]| {
                    routes
                        .iter()
                        .map(|route| route.trim().to_ascii_uppercase())
                        .filter(|route| !route.is_empty())
                        .collect::<std::collections::BTreeSet<_>>()
                };
                normalize(routes) == normalize(&snapshot.config.selected_routes)
            });
            if !routes_match
                || patch.mode.is_some_and(|mode| mode != snapshot.config.mode)
                || patch
                    .min_gross_spread_pct
                    .is_some_and(|value| value != snapshot.config.min_gross_spread_pct)
            {
                return Err(ApiError::client(
                    "SETTINGS_RECEIPT_MISMATCH",
                    "返回配置与本次修改不一致，仍需核对原操作",
                ));
            }
            Ok(snapshot)
        });
        if !state.journal.current(epoch) {
            return;
        }
        match result {
            Ok(snapshot) => {
                if patch.min_gross_spread_pct.is_some() {
                    state.minimum_dirty.set(false);
                    state
                        .minimum
                        .set(snapshot.config.min_gross_spread_pct.to_string());
                }
                state.accept_snapshot(snapshot, requested);
                state.notice.set(Some(
                    if state.journal.resolve(&attempt) {
                        "配置已保存"
                    } else {
                        "配置已返回，恢复记录待清理"
                    }
                    .to_owned(),
                ));
                state.journal.busy.set(false);
            }
            Err(error) => {
                state.journal.failed(&attempt, &error);
                let label = if state.journal.locked() {
                    "保存结果待核对"
                } else {
                    "保存失败"
                };
                state
                    .notice
                    .set(Some(format!("{label}：{}", error.problem.message)));
            }
        }
        state.retry_at.set(0);
    });
}

pub(super) fn parse_minimum(value: &str) -> Result<f64, &'static str> {
    value
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && (0.0..=100.0).contains(value))
        .ok_or("最小毛价差须为 0% 到 100% 之间的数值")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn minimum_retains_small_percent_values_and_rejects_invalid_input() {
        assert_eq!(parse_minimum("0.00125"), Ok(0.00125));
        assert_eq!(parse_minimum("100"), Ok(100.0));
        assert_eq!(parse_minimum("0"), Ok(0.0));
        for input in ["", "NaN", "inf", "-1", "100.1", "oops"] {
            assert!(parse_minimum(input).is_err());
        }
    }
}
