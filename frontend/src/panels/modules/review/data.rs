use crate::api::rest::{ApiClient, ApiError};
use crate::state::context::use_global;
use crate::state::load_state::LoadState;
use crate::state::module_runtime::ModuleRuntimeState;
use gloo_timers::callback::Interval;
use leptos::prelude::*;
use leptos::task::spawn_local;
use retry::{review_poll_allowed, review_retry_deadline_for_result, ReviewRetryAfterSource};
use shared_types::{
    ApiProblem, ExecutedTrade, MissedOpportunity, ReviewEnvelope, StrategyPerformance,
    VenueQualityEnvelope,
};
use std::future::Future;
use std::time::Duration;

mod executed;
mod projection;
mod retry;
use executed::use_executed_pages;
pub(super) use projection::use_runtime_projection;
#[cfg(test)]
use retry::review_retry_deadline_ms;

pub(super) type ReviewState<T> = RwSignal<LoadState<ReviewEnvelope<T>>>;
pub(super) type VenueQualityState = RwSignal<LoadState<VenueQualityEnvelope>>;

/// 复盘模块跨模块切换保留的运行态信号（由 workstation 持有）。
///
/// workstation 只挂载激活模块，离开复盘会 unmount 其轮询 Effect（不留 hidden
/// 轮询）。把四个数据源的 [`LoadState`] 与分页游标提升到 workstation 持有，切回
/// 复盘即以上次成功数据 + 同一页游标渲染，背景再刷新，与 opportunities/positions
/// 的跨模块状态恢复一致。loading / tick / `request_version` 等瞬态仍随挂载重建。
#[derive(Clone, Copy)]
pub(in crate::panels) struct ReviewRuntime {
    executed: ReviewPagedRuntime<ExecutedTrade>,
    executed_first_page: ReviewState<ExecutedTrade>,
    missed: ReviewPagedRuntime<MissedOpportunity>,
    perf: ReviewState<StrategyPerformance>,
    venue_quality: VenueQualityState,
    pub(super) refresh_nonce: RwSignal<u64>,
}

struct ReviewPagedRuntime<T: 'static> {
    state: ReviewState<T>,
    cursor: RwSignal<Option<String>>,
}

impl<T: 'static> Clone for ReviewPagedRuntime<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: 'static> Copy for ReviewPagedRuntime<T> {}

impl<T: Clone + Send + Sync + 'static> ReviewPagedRuntime<T> {
    fn new() -> Self {
        Self {
            state: RwSignal::new(LoadState::Loading),
            cursor: RwSignal::new(None),
        }
    }
}

/// workstation 初始化时创建一次；首帧前为 Loading，之后跨模块切换保留最近数据。
pub(in crate::panels) fn create_review_runtime() -> ReviewRuntime {
    ReviewRuntime {
        executed: ReviewPagedRuntime::new(),
        executed_first_page: RwSignal::new(LoadState::Loading),
        missed: ReviewPagedRuntime::new(),
        perf: RwSignal::new(LoadState::Loading),
        venue_quality: RwSignal::new(LoadState::Loading),
        refresh_nonce: RwSignal::new(0),
    }
}

impl ReviewRuntime {
    pub(in crate::panels) fn module_runtime_state(self) -> ModuleRuntimeState {
        ModuleRuntimeState::combine([
            self.executed
                .state
                .with(ModuleRuntimeState::from_load_state),
            self.missed.state.with(ModuleRuntimeState::from_load_state),
            self.perf.with(ModuleRuntimeState::from_load_state),
            self.venue_quality.with(ModuleRuntimeState::from_load_state),
        ])
    }
}

#[derive(Clone, Copy)]
struct RequestGate {
    version: RwSignal<u64>,
    token: u64,
}

struct PageFetchControl<T: 'static> {
    state: ReviewState<T>,
    loading: RwSignal<bool>,
    retry_until_ms: RwSignal<Option<u64>>,
    gate: RequestGate,
}

impl<T: 'static> Clone for PageFetchControl<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: 'static> Copy for PageFetchControl<T> {}

#[derive(Clone, Copy)]
pub(super) struct ReviewPagedState<T> {
    pub(super) state: ReviewState<T>,
    pub(super) loading: RwSignal<bool>,
    pub(super) load_cursor: Callback<Option<String>>,
}

pub(super) fn use_executed(runtime: ReviewRuntime) -> ReviewPagedState<ExecutedTrade> {
    use_executed_pages(runtime)
}

pub(super) fn use_missed(runtime: ReviewRuntime) -> ReviewPagedState<MissedOpportunity> {
    use_paged_review(
        runtime.missed,
        runtime.refresh_nonce,
        Duration::from_secs(10),
        |client, cursor| async move { client.review_missed_page(30, cursor.as_deref()).await },
    )
}

pub(super) fn use_perf(runtime: ReviewRuntime) -> ReviewState<StrategyPerformance> {
    runtime.perf
}

pub(super) fn use_venue_quality(runtime: ReviewRuntime) -> VenueQualityState {
    use_load_state(
        runtime.venue_quality,
        runtime.refresh_nonce,
        Duration::from_secs(5),
        |client| async move { client.venues_quality().await },
    )
}

fn use_paged_review<T, F, Fut>(
    runtime: ReviewPagedRuntime<T>,
    refresh_nonce: RwSignal<u64>,
    period: Duration,
    fetch: F,
) -> ReviewPagedState<T>
where
    T: Clone + Send + Sync + 'static,
    F: Fn(ApiClient, Option<String>) -> Fut + Clone + 'static,
    Fut: Future<Output = Result<ReviewEnvelope<T>, ApiError>> + 'static,
{
    let state = runtime.state;
    let loading = RwSignal::new(false);
    let cursor = runtime.cursor;
    let tick = RwSignal::new(0_u64);
    let request_version = RwSignal::new(0_u64);
    let retry_until_ms = RwSignal::new(None::<u64>);
    let client = use_global().client;
    let interval_ms = interval_ms(period);

    Effect::new(move |prev: Option<Interval>| {
        if let Some(interval) = prev {
            return interval;
        }
        Interval::new(interval_ms, move || {
            let _ = tick.try_update(|value| *value = value.wrapping_add(1));
        })
    });

    Effect::new(move |_| {
        refresh_nonce.get();
        if tick.try_get().is_none() {
            return;
        }
        let Some(retry_deadline_ms) = retry_until_ms.try_get_untracked() else {
            return;
        };
        if loading.get_untracked() || !review_poll_allowed(retry_deadline_ms, review_now_ms()) {
            return;
        }
        let client = client.clone();
        let fetch = fetch.clone();
        let cursor = cursor.get_untracked();
        let gate = next_request_gate(request_version);
        let control = PageFetchControl {
            state,
            loading,
            retry_until_ms,
            gate,
        };
        spawn_review_page_fetch(control, client, fetch, cursor);
    });

    let load_cursor = Callback::new(move |next_cursor| {
        if loading.get_untracked() {
            return;
        }
        cursor.set(next_cursor);
        tick.update(|value| *value = value.wrapping_add(1));
    });

    ReviewPagedState {
        state,
        loading,
        load_cursor,
    }
}

fn spawn_review_page_fetch<T, F, Fut>(
    control: PageFetchControl<T>,
    client: ApiClient,
    fetch: F,
    cursor: Option<String>,
) where
    T: Clone + Send + Sync + 'static,
    F: Fn(ApiClient, Option<String>) -> Fut + Clone + 'static,
    Fut: Future<Output = Result<ReviewEnvelope<T>, ApiError>> + 'static,
{
    control.loading.set(true);
    spawn_local(async move {
        let result = fetch(client, cursor).await.map_err(api_problem);
        if control.gate.is_latest() {
            control
                .retry_until_ms
                .set(review_retry_deadline_for_result(&result, review_now_ms()));
            control.state.update(|state| state.apply_result(result));
            control.loading.set(false);
        }
    });
}

fn use_load_state<T, F, Fut>(
    state: RwSignal<LoadState<T>>,
    refresh_nonce: RwSignal<u64>,
    period: Duration,
    fetch: F,
) -> RwSignal<LoadState<T>>
where
    T: Clone + Send + Sync + ReviewRetryAfterSource + 'static,
    F: Fn(ApiClient) -> Fut + Clone + 'static,
    Fut: Future<Output = Result<T, ApiError>> + 'static,
{
    let client = use_global().client;
    let loading = RwSignal::new(false);
    let tick = RwSignal::new(0_u64);
    let request_version = RwSignal::new(0_u64);
    let retry_until_ms = RwSignal::new(None::<u64>);
    let interval_ms = interval_ms(period);

    Effect::new(move |prev: Option<Interval>| {
        if let Some(interval) = prev {
            return interval;
        }
        Interval::new(interval_ms, move || {
            let _ = tick.try_update(|value| *value = value.wrapping_add(1));
        })
    });

    Effect::new(move |_| {
        refresh_nonce.get();
        if tick.try_get().is_none() {
            return;
        }
        let Some(retry_deadline_ms) = retry_until_ms.try_get_untracked() else {
            return;
        };
        if loading.get_untracked() || !review_poll_allowed(retry_deadline_ms, review_now_ms()) {
            return;
        }
        let client = client.clone();
        let fetch = fetch.clone();
        let gate = next_request_gate(request_version);
        loading.set(true);
        spawn_local(async move {
            let result = fetch(client).await.map_err(api_problem);
            if gate.is_latest() {
                loading.set(false);
                retry_until_ms.set(review_retry_deadline_for_result(&result, review_now_ms()));
                state.update(|state| state.apply_result(result));
            }
        });
    });
    state
}

fn next_request_gate(version: RwSignal<u64>) -> RequestGate {
    let next = version.get_untracked().wrapping_add(1);
    version.set(next);
    RequestGate {
        version,
        token: next,
    }
}

impl RequestGate {
    fn is_latest(self) -> bool {
        self.version
            .try_get_untracked()
            .is_some_and(|latest| is_latest_response(latest, self.token))
    }
}

fn is_latest_response(latest: u64, token: u64) -> bool {
    latest == token
}

fn api_problem(error: ApiError) -> ApiProblem {
    error.problem
}

fn interval_ms(period: Duration) -> u32 {
    period.as_millis().clamp(250, u32::MAX as u128) as u32
}

fn review_now_ms() -> u64 {
    js_sys::Date::now().max(0.0).round().min(u64::MAX as f64) as u64
}

#[cfg(test)]
#[path = "data/tests.rs"]
mod tests;
