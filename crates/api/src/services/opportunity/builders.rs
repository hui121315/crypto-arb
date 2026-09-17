use super::paging::snapshot_id;
use super::row::list_row_from_dto_at;
use super::snapshot_health::classify_snapshot;
use super::*;

mod quality;
mod stream;

use quality::{envelope_status, registry_count_breakdown, scope_market_data_status};
pub(super) use quality::{p0_count_breakdown, partial_failures};
pub(crate) use stream::{stream_event, stream_rows_for_ids, stream_window_ids};

pub(crate) fn p0_strategy_kind(value: &str) -> Option<StrategyKind> {
    let kind = StrategyKind::from_query_value(value.trim())?;
    is_p0_executable_strategy(kind).then_some(kind)
}
pub(crate) fn envelope(input: OpportunityEnvelopeInput<'_>) -> OpportunityEnvelope {
    let main_p0_counts = p0_count_breakdown(input.filtered_rows);
    let registry_counts = registry_count_breakdown(input.source_rows);
    let mut meta = input.meta;
    let mut partial_failures = partial_failures(&meta);
    partial_failures.extend(input.query_problems);
    let returned_count = input.opportunities.len();
    let observed_at_ms = Utc::now().timestamp_millis();
    let snapshot = classify_snapshot(input.status, &input.cached_at, observed_at_ms, meta.scan_ms);
    let status = envelope_status(snapshot.status, partial_failures.is_empty());
    let error = snapshot.problem.or(input.error);
    let retry_after_ms = retry_after_ms(input.retry_after_ms, &partial_failures, error.as_ref());
    let scan_started_at = meta.scan_started_at.to_owned();
    meta.funding_row_evidence.clear();
    OpportunityEnvelope {
        opportunities: input.opportunities,
        count: returned_count,
        total_count: input.source_rows.len(),
        filtered_count: input.filtered_rows.len(),
        before_limit_count: input.filtered_rows.len(),
        returned_count,
        visible_count: returned_count,
        executable_count: main_p0_counts.executable_count,
        strategy_counts: main_p0_counts.strategy_counts.clone(),
        executable_strategy_counts: main_p0_counts.executable_strategy_counts.clone(),
        main_p0_counts,
        registry_counts,
        meta,
        status,
        scope: input.scope,
        query_key: input.query_key,
        source: input.source.to_owned(),
        cached_at: input.cached_at,
        observed_at_ms,
        freshness_ms: Some(snapshot.freshness_ms),
        scan_started_at,
        retry_after_ms,
        error,
        partial_failures,
    }
}

pub(crate) fn list_envelope(input: OpportunityListEnvelopeInput<'_>) -> OpportunityListEnvelope {
    let main_p0_counts = p0_count_breakdown(input.source_rows);
    let registry_counts = registry_count_breakdown(input.source_rows);
    let mut meta = input.meta;
    scope_market_data_status(&mut meta, &input.request_meta.filter.strategy_kinds);
    let mut partial_failures = partial_failures(&meta);
    partial_failures.extend(input.window.query_problems());
    let observed_at_ms = Utc::now().timestamp_millis();
    let snapshot = classify_snapshot(input.status, &input.cached_at, observed_at_ms, meta.scan_ms);
    let status = envelope_status(snapshot.status, partial_failures.is_empty());
    let error = snapshot.problem.or(input.error);
    let retry_after_ms = retry_after_ms(input.retry_after_ms, &partial_failures, error.as_ref());
    let rows = input
        .rows
        .into_iter()
        .map(|row| list_row_from_dto_at(row, observed_at_ms))
        .collect::<Vec<_>>();
    let page = input.window.page(
        input.filtered_count,
        rows.len(),
        input
            .snapshot_id
            .map(str::to_owned)
            .unwrap_or_else(|| snapshot_id(input.cached_at, &meta)),
    );
    let scope_meta = scope_meta(
        input.source_rows,
        input.strategy_scope_count,
        input.filtered_count,
        input.symbol_scope_count,
        &meta,
        &page,
    );
    meta.funding_row_evidence.clear();
    OpportunityListEnvelope {
        rows,
        page,
        request_meta: input.request_meta,
        scope_meta,
        main_p0_counts,
        registry_counts,
        meta,
        status,
        scope: input.scope,
        query_key: input.query_key,
        source: input.source.to_owned(),
        cached_at: input.cached_at,
        observed_at_ms,
        freshness_ms: Some(snapshot.freshness_ms),
        retry_after_ms,
        error,
        partial_failures,
        instrument_coverage_diagnostics: String::new(),
    }
}
pub(super) fn scope_meta(
    source_rows: &[ArbitrageOpportunityDto],
    strategy_scope_count: usize,
    filtered_count: usize,
    symbol_scope_count: Option<usize>,
    meta: &OpportunityScanMeta,
    page: &OpportunityListPage,
) -> OpportunityQueryScopeMeta {
    OpportunityQueryScopeMeta {
        global_total_count: source_rows.len(),
        strategy_scope_count,
        symbol_scope_count: symbol_scope_count.unwrap_or(filtered_count),
        filtered_count,
        page_count: filtered_count.div_ceil(page.page_size.max(1)),
        candidate_count: meta.candidate_count,
        emitted_count: meta.emitted_count,
    }
}

fn retry_after_ms(
    input_retry_after_ms: Option<u64>,
    partial_failures: &[ApiProblem],
    error: Option<&ApiProblem>,
) -> Option<u64> {
    partial_failures
        .iter()
        .filter_map(|problem| problem.retry_after_ms)
        .chain(input_retry_after_ms)
        .chain(error.and_then(|problem| problem.retry_after_ms))
        .max()
}

pub(crate) fn warming_error() -> ApiProblem {
    ApiProblem::new(
        codes::OPPORTUNITY_SNAPSHOT_WARMING,
        "opportunity snapshot is warming; retry after the next scan tick",
    )
    .with_retry_after_ms(Some(WARMING_RETRY_AFTER_MS))
    .with_source("arbitrage-snapshot")
}

pub(crate) fn legacy_wide_endpoint_problem() -> ApiProblem {
    let mut problem = ApiProblem::new(
        codes::OPPORTUNITY_LEGACY_WIDE_ENDPOINT,
        "legacy opportunity wide endpoint is compatibility-only; use /api/v3/arbitrage/opportunities/list for product lists",
    )
    .with_source(WIDE_LIST_SOURCE);
    problem.details = Some(serde_json::json!({
        "replacement": "/api/v3/arbitrage/opportunities/list",
        "mode": "compatibility_only",
    }));
    problem
}
