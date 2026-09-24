use crate::api::rest::ApiClient;
use crate::panels::modules::execution::selection::ExecutionSelection;
use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::{problem::codes, ApiProblem};

use super::model::{ExecutionPreview, PreviewInput, PreviewQuery};
use super::response::{from_api_preview, preview_request};

const PREVIEW_RATE_LIMIT_FALLBACK_MS: u64 = 1_000;
const MAX_SNAPSHOT_REFRESHES: u8 = 2;

#[derive(Clone, Default)]
pub(super) struct SnapshotRefreshBudget {
    key: Option<(PreviewBackoffKey, u64)>,
    attempts: u8,
}

impl SnapshotRefreshBudget {
    pub(super) fn refresh(
        &mut self,
        selection: RwSignal<ExecutionSelection>,
        query: &PreviewQuery,
        refresh: u64,
        problem: &mut ApiProblem,
    ) -> bool {
        if problem.code != codes::OPPORTUNITY_SNAPSHOT_STALE {
            return false;
        }
        let key = (PreviewBackoffKey::from_query(query), refresh);
        if self.key.as_ref() != Some(&key) {
            self.key = Some(key);
            self.attempts = 0;
        }
        if self.attempts >= MAX_SNAPSHOT_REFRESHES {
            problem.message = "机会快照连续变化，已停止自动重试；已保留输入，请稍后刷新预览。".into();
            return false;
        }
        if refresh_stale_selection_snapshot(selection, query, problem) {
            self.attempts += 1;
            return true;
        }
        false
    }
}

#[derive(Clone, PartialEq)]
struct PreviewBackoffKey {
    opportunity_id: String,
    input: PreviewInput,
}

#[derive(Clone, PartialEq)]
pub(super) struct PreviewBackoff {
    key: PreviewBackoffKey,
    problem: ApiProblem,
    until_ms: f64,
}

#[derive(Clone, PartialEq)]
pub(super) struct ReadyPreview {
    key: PreviewBackoffKey,
    value: ExecutionPreview,
}

pub(super) struct LoadedPreview {
    pub(super) preview: ExecutionPreview,
    pub(super) workflow_view: shared_types::HedgeTicketView,
}

pub(super) fn active_preview_query(
    query: Option<PreviewQuery>,
    selected_opportunity_id: &str,
) -> Option<PreviewQuery> {
    let selected_opportunity_id = selected_opportunity_id.trim();
    if selected_opportunity_id.is_empty() {
        return None;
    }
    query.filter(|query| query.seed.opportunity_id == selected_opportunity_id)
}

pub(super) fn restore_last_ready(
    last_ready: RwSignal<Option<ReadyPreview>>,
    state: RwSignal<LoadState<ExecutionPreview>>,
    query: &PreviewQuery,
) {
    if last_ready.get_untracked().is_some() {
        return;
    }
    let restored = state.with_untracked(|state| state.value().cloned());
    if let Some(preview) =
        restored.filter(|preview| preview.opportunity_id == query.seed.opportunity_id)
    {
        last_ready.set(Some(ReadyPreview::new(query, preview)));
    }
}

pub(super) async fn load_preview(
    client: ApiClient,
    query: &PreviewQuery,
) -> Result<LoadedPreview, ApiProblem> {
    let request = preview_request(&query.seed, &query.input);
    client
        .preview_hedge(&request)
        .await
        .map_err(|error| error.problem)
        .and_then(|response| loaded_preview(response, query))
}

pub(super) fn loaded_preview(
    response: shared_types::HedgePreviewResponse,
    query: &PreviewQuery,
) -> Result<LoadedPreview, ApiProblem> {
    if response.opportunity_id != query.seed.opportunity_id
        || response.ticket.opportunity_id != query.seed.opportunity_id
        || response.opportunity_snapshot_id.trim().is_empty()
        || response
            .requested_opportunity_snapshot_id
            .as_ref()
            .is_some_and(|snapshot| snapshot != &query.seed.opportunity_snapshot_id)
        || (!query.seed.opportunity_snapshot_id.trim().is_empty()
            && response.opportunity_snapshot_id != query.seed.opportunity_snapshot_id
            && response.requested_opportunity_snapshot_id.is_none())
    {
        return Err(ApiProblem::new(
            "HEDGE_PREVIEW_BINDING_MISMATCH",
            "后端预检不属于当前机会快照，请刷新预览",
        ));
    }
    Ok(LoadedPreview {
        workflow_view: response.workflow_view.clone(),
        preview: from_api_preview(response, &query.seed, &query.input),
    })
}

pub(super) fn set_preview_problem_state(
    state: RwSignal<LoadState<ExecutionPreview>>,
    last_ready: RwSignal<Option<ReadyPreview>>,
    query: &PreviewQuery,
    problem: ApiProblem,
) {
    state.set(preview_problem_state(
        &last_ready.get_untracked(),
        query,
        problem,
    ));
}

pub(super) fn preview_problem_state(
    last_ready: &Option<ReadyPreview>,
    query: &PreviewQuery,
    problem: ApiProblem,
) -> LoadState<ExecutionPreview> {
    match last_ready {
        Some(previous) if previous.key == PreviewBackoffKey::from_query(query) => {
            LoadState::Stale {
                value: previous.value.clone(),
                problem,
            }
        }
        _ => LoadState::Error(problem),
    }
}

pub(super) fn preview_request_state(
    last_ready: &Option<ReadyPreview>,
    query: &PreviewQuery,
) -> LoadState<ExecutionPreview> {
    match last_ready {
        Some(previous) if previous.key == PreviewBackoffKey::from_query(query) => {
            LoadState::Stale {
                value: previous.value.clone(),
                problem: ApiProblem::new("PREVIEW_REFRESHING", "正在刷新同一执行预览")
                    .with_source("frontend-preview"),
            }
        }
        _ => LoadState::Loading,
    }
}

pub(super) fn preview_request_is_current(current_version: u64, request_version: u64) -> bool {
    current_version == request_version
}

pub(super) fn preview_request_is_in_flight(
    current: &Option<PreviewQuery>,
    query: &PreviewQuery,
) -> bool {
    current.as_ref() == Some(query)
}

pub(super) fn refresh_stale_selection_snapshot(
    selection: RwSignal<ExecutionSelection>,
    query: &PreviewQuery,
    problem: &ApiProblem,
) -> bool {
    if problem.code != codes::OPPORTUNITY_SNAPSHOT_STALE {
        return false;
    }
    let Some(actual_snapshot_id) = problem
        .details
        .as_ref()
        .and_then(|details| details.get("actualSnapshotId"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    let mut refreshed = false;
    selection.update(|current| {
        if current.opportunity_id == query.seed.opportunity_id
            && current.opportunity_snapshot_id == query.seed.opportunity_snapshot_id
            && current.opportunity_snapshot_id != actual_snapshot_id
        {
            current.opportunity_snapshot_id = actual_snapshot_id.to_owned();
            refreshed = true;
        }
    });
    refreshed
}

impl PreviewBackoff {
    pub(super) fn new(query: &PreviewQuery, problem: ApiProblem, until_ms: f64) -> Self {
        Self {
            key: PreviewBackoffKey::from_query(query),
            problem,
            until_ms,
        }
    }
}

impl ReadyPreview {
    pub(super) fn new(query: &PreviewQuery, value: ExecutionPreview) -> Self {
        Self {
            key: PreviewBackoffKey::from_query(query),
            value,
        }
    }
}

impl PreviewBackoffKey {
    fn from_query(query: &PreviewQuery) -> Self {
        Self {
            opportunity_id: query.seed.opportunity_id.clone(),
            input: query.input.clone(),
        }
    }
}

pub(super) fn active_backoff_problem(
    backoff: &Option<PreviewBackoff>,
    query: &PreviewQuery,
    now_ms: f64,
) -> Option<ApiProblem> {
    let backoff = backoff.as_ref()?;
    if backoff.until_ms <= now_ms || backoff.key != PreviewBackoffKey::from_query(query) {
        return None;
    }
    Some(backoff.problem.clone())
}

pub(super) fn preview_backoff_until_ms(problem: &ApiProblem, now_ms: f64) -> Option<f64> {
    let retry_after_ms = problem
        .retry_after_ms
        .or_else(|| (problem.status == Some(429)).then_some(PREVIEW_RATE_LIMIT_FALLBACK_MS))?;
    (retry_after_ms > 0).then_some(now_ms + retry_after_ms as f64)
}

#[cfg(target_arch = "wasm32")]
pub(super) fn now_ms() -> f64 {
    js_sys::Date::now()
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |duration| duration.as_millis() as f64)
}
