use crate::api::rest::ApiClient;
use crate::state::context::use_global;
use crate::state::load_state::LoadState;
use crate::state::module_runtime::ModuleRuntimeState;
use crate::state::polling::{now_ms, use_conditional_polling_result, use_debounced_string};
use leptos::prelude::*;
use leptos::task::spawn_local;
use shared_types::{
    GateCrossExMode, GateCrossExModeConfigPatch, GateCrossExModeSnapshot,
    GateCrossExRouteCatalogResponse, GATE_CROSSEX_SELECTED_ROUTE_LIMIT,
};
use std::time::Duration;

#[derive(Clone, Copy)]
pub(in crate::panels) struct GateCrossExRuntime {
    status: RwSignal<LoadState<GateCrossExModeSnapshot>>,
}

pub(in crate::panels) fn create_gate_crossex_runtime() -> GateCrossExRuntime {
    GateCrossExRuntime {
        status: RwSignal::new(LoadState::Loading),
    }
}

impl GateCrossExRuntime {
    pub(in crate::panels) fn module_runtime_state(self) -> ModuleRuntimeState {
        ModuleRuntimeState::from_load_state(&self.status.get())
    }
}

#[derive(Clone, Copy)]
pub(super) struct GateCrossExData {
    pub status: RwSignal<LoadState<GateCrossExModeSnapshot>>,
    pub catalog: RwSignal<LoadState<GateCrossExRouteCatalogResponse>>,
    pub search: RwSignal<String>,
    pub notice: RwSignal<Option<String>>,
    pub saving: RwSignal<bool>,
    pub reading: RwSignal<bool>,
    pub minimum: RwSignal<String>,
    pub minimum_dirty: RwSignal<bool>,
    pub set_mode: Callback<GateCrossExMode>,
    pub save_minimum: Callback<()>,
    pub toggle_route: Callback<String>,
    pub refresh: Callback<()>,
    pub refresh_catalog: Callback<()>,
}

#[derive(Clone, Copy)]
struct RequestState {
    status: RwSignal<LoadState<GateCrossExModeSnapshot>>,
    reading: RwSignal<bool>,
    saving: RwSignal<bool>,
    revision: RwSignal<u64>,
    retry_at: RwSignal<u64>,
    notice: RwSignal<Option<String>>,
    minimum: RwSignal<String>,
    minimum_dirty: RwSignal<bool>,
}

pub(super) fn use_gate_crossex_data(runtime: GateCrossExRuntime) -> GateCrossExData {
    let client = use_global().client;
    let status = runtime.status;
    let requests = RequestState {
        status,
        reading: RwSignal::new(false),
        saving: RwSignal::new(false),
        revision: RwSignal::new(0),
        retry_at: RwSignal::new(0),
        notice: RwSignal::new(None),
        minimum: RwSignal::new(String::new()),
        minimum_dirty: RwSignal::new(false),
    };
    let poll = use_conditional_polling_result(
        Duration::from_millis(750),
        move || {
            requests.reading.try_get_untracked() == Some(false)
                && requests.saving.try_get_untracked() == Some(false)
                && requests
                    .retry_at
                    .try_get_untracked()
                    .is_some_and(|at| now_ms() >= at)
        },
        {
            let client = client.clone();
            move || {
                let client = client.clone();
                async move {
                    read_status(client, requests).await;
                    Ok::<_, ()>(())
                }
            }
        },
    );
    Effect::new(move |_| {
        let _ = poll.get();
    });
    Effect::new(move |_| {
        if let Some(snapshot) = status.get().value() {
            if !requests.minimum_dirty.get_untracked() && !requests.saving.get_untracked() {
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
    Effect::new(move |_| {
        let _ = search.get();
        search_revision.update(|revision| *revision = revision.wrapping_add(1));
        catalog.set(LoadState::Loading);
    });
    let debounced = use_debounced_string(move || search.get(), Duration::from_millis(180));
    let catalog_client = client.clone();
    Effect::new(move |_| {
        let query = debounced.get();
        let raw = search.get();
        let _ = catalog_retry.get();
        if query != raw {
            return;
        }
        let version = search_revision.get_untracked();
        let client = catalog_client.clone();
        catalog.set(LoadState::Loading);
        spawn_local(async move {
            let result = client
                .gate_crossex_routes(&query)
                .await
                .map_err(|error| error.problem);
            if search.try_get_untracked().as_ref() == Some(&query)
                && search_revision.try_get_untracked() == Some(version)
            {
                let _ = catalog.try_update(|state| state.apply_result(result));
            }
        });
    });
    let refresh_catalog = Callback::new(move |_| {
        search_revision.update(|revision| *revision = revision.wrapping_add(1));
        catalog_retry.update(|revision| *revision = revision.wrapping_add(1));
    });
    let refresh = Callback::new({
        let client = client.clone();
        move |_| {
            let client = client.clone();
            spawn_local(read_status(client, requests));
        }
    });
    let set_mode = Callback::new({
        let client = client.clone();
        move |mode| {
            if status
                .get_untracked()
                .value()
                .is_some_and(|snapshot| snapshot.config.mode == mode)
            {
                return;
            }
            submit_patch(
                client.clone(),
                requests,
                GateCrossExModeConfigPatch {
                    mode: Some(mode),
                    ..Default::default()
                },
            );
        }
    });
    let save_minimum = Callback::new({
        let client = client.clone();
        move |_| match parse_minimum(&requests.minimum.get_untracked()) {
            Ok(value) => submit_patch(
                client.clone(),
                requests,
                GateCrossExModeConfigPatch {
                    min_gross_spread_pct: Some(value),
                    ..Default::default()
                },
            ),
            Err(message) => requests.notice.set(Some(message.to_owned())),
        }
    });
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
            client.clone(),
            requests,
            GateCrossExModeConfigPatch {
                selected_routes: Some(routes),
                ..Default::default()
            },
        );
    });
    GateCrossExData {
        status,
        catalog,
        search,
        notice: requests.notice,
        saving: requests.saving,
        reading: requests.reading,
        minimum: requests.minimum,
        minimum_dirty: requests.minimum_dirty,
        set_mode,
        save_minimum,
        toggle_route,
        refresh,
        refresh_catalog,
    }
}

async fn read_status(client: ApiClient, state: RequestState) {
    if state.reading.try_get_untracked() != Some(false)
        || state.saving.try_get_untracked() != Some(false)
    {
        return;
    }
    state.reading.set(true);
    let revision = state.revision.get_untracked();
    let result = client
        .gate_crossex_mode()
        .await
        .map_err(|error| error.problem);
    if state.revision.try_get_untracked() != Some(revision) {
        let _ = state.reading.try_set(false);
        return;
    }
    state.retry_at.set(result.as_ref().err().map_or(0, |error| {
        now_ms().saturating_add(error.retry_after_ms.unwrap_or(3_000))
    }));
    state.status.update(|current| current.apply_result(result));
    state.reading.set(false);
}

fn submit_patch(client: ApiClient, state: RequestState, patch: GateCrossExModeConfigPatch) {
    if state.saving.try_get_untracked() != Some(false)
        || !matches!(state.status.get_untracked(), LoadState::Ready(_))
    {
        return;
    }
    state.saving.set(true);
    state
        .revision
        .update(|revision| *revision = revision.wrapping_add(1));
    state.notice.set(Some("正在保存…".to_owned()));
    spawn_local(async move {
        let result = client.update_gate_crossex_mode(&patch).await;
        if state.saving.try_get_untracked().is_none() {
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
                state.status.set(LoadState::Ready(snapshot));
                state.notice.set(Some("配置已保存".to_owned()));
            }
            Err(error) => state
                .notice
                .set(Some(format!("保存失败：{}", error.problem.message))),
        }
        state.retry_at.set(0);
        state.saving.set(false);
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
