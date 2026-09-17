use crate::panels::modules::opportunity_counts::OpportunityCountMeta;
use crate::panels::modules::opportunity_envelope::{
    apply_opportunity_envelope_state, opportunity_envelope_should_publish_rows,
};
use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::arbitrage::{OpportunityListFilterMeta, OpportunityListRequestMeta};
use shared_types::{
    OpportunityListEnvelope, OpportunityListPage, OpportunityListRow, OpportunityStreamEvent,
    StrategyKind, P0_EXECUTABLE_STRATEGY_KINDS,
};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::ops::Deref;
use std::time::Duration;

pub(crate) const CANONICAL_OPPORTUNITY_PAGE_SIZE: usize =
    shared_types::contracts::p0::OPPORTUNITY_PRODUCT_PAGE_SIZE;
pub(crate) const CANONICAL_OPPORTUNITY_SEARCH_DEBOUNCE: Duration = Duration::from_millis(250);

pub(crate) struct OpportunityRowsRuntime<Row> {
    pub(crate) state: RwSignal<LoadState<()>>,
    pub(crate) rows: RwSignal<Vec<Row>>,
    pub(crate) meta: RwSignal<OpportunityCountMeta>,
    pub(crate) page: RwSignal<Option<OpportunityListPage>>,
}

pub(crate) struct OpportunityRowsTarget<Row> {
    state: RwSignal<LoadState<()>>,
    rows: RwSignal<Vec<Row>>,
    meta: RwSignal<OpportunityCountMeta>,
    page: RwSignal<Option<OpportunityListPage>>,
}

impl<Row> OpportunityRowsTarget<Row> {
    pub(crate) fn new(
        state: RwSignal<LoadState<()>>,
        rows: RwSignal<Vec<Row>>,
        meta: RwSignal<OpportunityCountMeta>,
        page: RwSignal<Option<OpportunityListPage>>,
    ) -> Self {
        Self {
            state,
            rows,
            meta,
            page,
        }
    }
}

impl<Row: Send + Sync + 'static> OpportunityRowsRuntime<Row> {
    fn new(state: LoadState<()>) -> Self {
        Self {
            state: RwSignal::new(state),
            rows: RwSignal::new(Vec::new()),
            meta: RwSignal::new(OpportunityCountMeta::default()),
            page: RwSignal::new(None),
        }
    }
}

impl<Row> Clone for OpportunityRowsRuntime<Row> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<Row> Copy for OpportunityRowsRuntime<Row> {}

pub(crate) struct OpportunityListRuntime<Row> {
    core: OpportunityRowsRuntime<Row>,
    pub(crate) loading: RwSignal<bool>,
    pub(crate) cursor: RwSignal<Option<String>>,
}

impl<Row: Clone + Send + Sync + 'static> OpportunityListRuntime<Row> {
    pub(crate) fn new() -> Self {
        Self {
            core: OpportunityRowsRuntime::new(LoadState::Loading),
            loading: RwSignal::new(true),
            cursor: RwSignal::new(None),
        }
    }
}

impl<Row> Clone for OpportunityListRuntime<Row> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<Row> Copy for OpportunityListRuntime<Row> {}

impl<Row> Deref for OpportunityListRuntime<Row> {
    type Target = OpportunityRowsRuntime<Row>;

    fn deref(&self) -> &Self::Target {
        &self.core
    }
}

pub(crate) struct OpportunitySearchRuntime<Row> {
    core: OpportunityRowsRuntime<Row>,
    pub(crate) last_query: RwSignal<String>,
    pub(crate) cursor: RwSignal<Option<String>>,
}

impl<Row: Send + Sync + 'static> OpportunitySearchRuntime<Row> {
    pub(crate) fn new() -> Self {
        Self {
            core: OpportunityRowsRuntime::new(LoadState::Ready(())),
            last_query: RwSignal::new(String::new()),
            cursor: RwSignal::new(None),
        }
    }
}

impl<Row> Clone for OpportunitySearchRuntime<Row> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<Row> Copy for OpportunitySearchRuntime<Row> {}

impl<Row> Deref for OpportunitySearchRuntime<Row> {
    type Target = OpportunityRowsRuntime<Row>;

    fn deref(&self) -> &Self::Target {
        &self.core
    }
}

#[derive(Clone)]
pub(crate) struct OpportunityStore<Row> {
    pub(crate) state: RwSignal<LoadState<()>>,
    pub(crate) rows: RwSignal<Vec<Row>>,
    pub(crate) meta: RwSignal<OpportunityCountMeta>,
    pub(crate) page: RwSignal<Option<OpportunityListPage>>,
    pub(crate) loading: RwSignal<bool>,
    pub(crate) load_cursor: Callback<Option<String>>,
}

pub(crate) fn apply_opportunity_rows_envelope<Row: Send + Sync + 'static>(
    envelope: &OpportunityListEnvelope,
    state: RwSignal<LoadState<()>>,
    rows: RwSignal<Vec<Row>>,
    meta: RwSignal<OpportunityCountMeta>,
    page: RwSignal<Option<OpportunityListPage>>,
    project_rows: impl FnOnce(&OpportunityListEnvelope) -> Vec<Row>,
) -> bool {
    meta.set(OpportunityCountMeta::from_list_response(envelope));
    page.set(Some(envelope.page.clone()));
    let publish_rows = opportunity_envelope_should_publish_rows(envelope);
    if publish_rows {
        rows.set(project_rows(envelope));
    }
    state.update(|state| apply_opportunity_envelope_state(state, envelope, publish_rows));
    publish_rows
}

pub(crate) fn live_first_page_from_stream(
    event: &OpportunityStreamEvent,
    live_rows: &HashMap<String, OpportunityListRow>,
    strategy: Option<StrategyKind>,
) -> Option<OpportunityListEnvelope> {
    let window = event
        .windows
        .iter()
        .find(|window| window.strategy_kind == strategy)?;
    let rows = window
        .ids
        .iter()
        .map(|id| live_rows.get(id).cloned())
        .collect::<Option<Vec<_>>>()?;
    let strategy_kinds = strategy.map_or_else(
        || P0_EXECUTABLE_STRATEGY_KINDS.to_vec(),
        |strategy| vec![strategy],
    );
    Some(OpportunityListEnvelope {
        rows,
        page: window.page.clone(),
        request_meta: OpportunityListRequestMeta {
            fast: false,
            fresh: false,
            filter: OpportunityListFilterMeta {
                scope: event.scope,
                strategy_kinds,
                symbol: None,
                min_yield: None,
            },
            sort_key: window.page.sort_key,
            requested_page_size: Some(window.page.page_size),
            applied_page_size: window.page.page_size,
            max_page_size: window.page.page_size,
        },
        scope_meta: window.scope_meta.clone(),
        main_p0_counts: event.main_p0_counts.clone(),
        registry_counts: event.registry_counts.clone(),
        meta: event.meta.clone(),
        status: event.status,
        scope: event.scope,
        query_key: window.query_key.clone(),
        source: event.source.clone(),
        cached_at: event.cached_at,
        observed_at_ms: event.observed_at_ms,
        freshness_ms: event.freshness_ms,
        retry_after_ms: event.retry_after_ms,
        error: event.error.clone(),
        partial_failures: event.partial_failures.clone(),
        instrument_coverage_diagnostics: String::new(),
    })
}

pub(crate) fn apply_live_first_page_from_stream<Row: Send + Sync + 'static>(
    stream: &LoadState<OpportunityStreamEvent>,
    live_rows: &HashMap<String, OpportunityListRow>,
    strategy: Option<StrategyKind>,
    target: &OpportunityRowsTarget<Row>,
    project_rows: impl FnOnce(&OpportunityListEnvelope) -> Vec<Row>,
) -> bool {
    apply_live_first_page_from_stream_when(
        stream,
        live_rows,
        strategy,
        target,
        |_| true,
        project_rows,
    )
}

pub(crate) fn apply_live_first_page_from_stream_when<Row: Send + Sync + 'static>(
    stream: &LoadState<OpportunityStreamEvent>,
    live_rows: &HashMap<String, OpportunityListRow>,
    strategy: Option<StrategyKind>,
    target: &OpportunityRowsTarget<Row>,
    should_apply: impl FnOnce(&OpportunityListEnvelope) -> bool,
    project_rows: impl FnOnce(&OpportunityListEnvelope) -> Vec<Row>,
) -> bool {
    let mut handled = false;
    if let Some(envelope) = stream
        .value()
        .and_then(|event| live_first_page_from_stream(event, live_rows, strategy))
    {
        if should_apply(&envelope) {
            apply_opportunity_rows_envelope(
                &envelope,
                target.state,
                target.rows,
                target.meta,
                target.page,
                project_rows,
            );
        }
        handled = true;
    }
    if let Some(problem) = stream.problem().cloned() {
        target
            .state
            .update(|state| state.apply_result(Err(problem)));
        handled = true;
    }
    handled
}

pub(crate) fn clean_opportunity_cursor(cursor: Option<String>) -> Option<String> {
    let cursor = cursor?.trim().to_owned();
    (!cursor.is_empty()).then_some(cursor)
}

pub(crate) fn opportunity_page_request_required(cursor: Option<&str>) -> bool {
    cursor.is_some_and(|cursor| !cursor.trim().is_empty())
}

pub(crate) fn merge_opportunity_projections<Row: Clone>(
    base: &[Row],
    extra: &[Row],
    same_row: impl Fn(&Row, &Row) -> bool,
    compare: impl Fn(&Row, &Row) -> Ordering,
) -> Vec<Row> {
    let mut rows = Vec::with_capacity(base.len() + extra.len());
    rows.extend_from_slice(base);
    for row in extra {
        if !rows.iter().any(|existing| same_row(existing, row)) {
            rows.push(row.clone());
        }
    }
    rows.sort_by(compare);
    rows
}

#[cfg(test)]
#[path = "opportunity_runtime/tests.rs"]
mod tests;
