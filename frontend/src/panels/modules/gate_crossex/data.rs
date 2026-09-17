use crate::api::rest::ApiClient;
use crate::state::context::use_global;
use crate::state::load_state::LoadState;
use crate::state::module_runtime::ModuleRuntimeState;
use crate::state::polling::{use_conditional_polling_result, use_debounced_string};
use leptos::prelude::*;
use leptos::task::spawn_local;
use shared_types::{
    GateCrossExMode, GateCrossExModeConfigPatch, GateCrossExModeSnapshot,
    GateCrossExRouteCatalogResponse,
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
    pub set_mode: Callback<GateCrossExMode>,
    pub set_minimum: Callback<f64>,
    pub toggle_route: Callback<String>,
}

pub(super) fn use_gate_crossex_data(runtime: GateCrossExRuntime) -> GateCrossExData {
    let client = use_global().client;
    let status = runtime.status;
    let catalog = RwSignal::new(LoadState::Loading);
    let search = RwSignal::new(String::new());
    let notice = RwSignal::new(None);
    let poll = use_conditional_polling_result(Duration::from_millis(750), || true, {
        let client = client.clone();
        move || {
            let client = client.clone();
            async move {
                client
                    .gate_crossex_mode()
                    .await
                    .map_err(|error| error.problem)
            }
        }
    });
    Effect::new(move |_| {
        if let Some(result) = poll.get().and_then(|event| event.take().into_fetched()) {
            status.update(|state| state.apply_result(result));
        }
    });

    let debounced_search = use_debounced_string(move || search.get(), Duration::from_millis(180));
    let catalog_client = client.clone();
    Effect::new(move |_| {
        let query = debounced_search.get();
        let client = catalog_client.clone();
        spawn_local(async move {
            let result = client
                .gate_crossex_routes(&query)
                .await
                .map_err(|error| error.problem);
            if debounced_search.get_untracked() == query {
                catalog.update(|state| state.apply_result(result));
            }
        });
    });

    let set_mode = mutation_callback(client.clone(), status, notice, move |mode| {
        GateCrossExModeConfigPatch {
            mode: Some(mode),
            ..Default::default()
        }
    });
    let set_minimum = mutation_callback(client.clone(), status, notice, move |minimum| {
        GateCrossExModeConfigPatch {
            min_gross_spread_pct: Some(minimum),
            ..Default::default()
        }
    });
    let toggle_route = Callback::new({
        let client = client.clone();
        move |native_symbol: String| {
            let Some(snapshot) = status.get_untracked().value().cloned() else {
                return;
            };
            let mut routes = snapshot.config.selected_routes;
            if let Some(index) = routes.iter().position(|route| route == &native_symbol) {
                routes.remove(index);
            } else {
                routes.push(native_symbol);
            }
            submit_patch(
                client.clone(),
                status,
                notice,
                GateCrossExModeConfigPatch {
                    selected_routes: Some(routes),
                    ..Default::default()
                },
            );
        }
    });

    GateCrossExData {
        status,
        catalog,
        search,
        notice,
        set_mode,
        set_minimum,
        toggle_route,
    }
}

fn mutation_callback<T: 'static>(
    client: ApiClient,
    status: RwSignal<LoadState<GateCrossExModeSnapshot>>,
    notice: RwSignal<Option<String>>,
    patch: impl Fn(T) -> GateCrossExModeConfigPatch + Clone + Send + Sync + 'static,
) -> Callback<T> {
    Callback::new(move |value| {
        submit_patch(client.clone(), status, notice, patch.clone()(value));
    })
}

fn submit_patch(
    client: ApiClient,
    status: RwSignal<LoadState<GateCrossExModeSnapshot>>,
    notice: RwSignal<Option<String>>,
    patch: GateCrossExModeConfigPatch,
) {
    notice.set(Some("正在保存…".to_owned()));
    spawn_local(async move {
        match client.update_gate_crossex_mode(&patch).await {
            Ok(snapshot) => {
                status.set(LoadState::Ready(snapshot));
                notice.set(Some("配置已生效".to_owned()));
            }
            Err(error) => {
                notice.set(Some(format!("保存失败：{}", error.problem.message)));
                status.update(|state| state.apply_result(Err(error.problem)));
            }
        }
    });
}
