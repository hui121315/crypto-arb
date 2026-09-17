//! `/api/v3/arbitrage/*`：套利机会列表（基于自动刷新快照 + 实时扫描兜底）。

mod filters;
mod hedge;
mod limit;
mod snapshot;

use crate::services::{market_data, market_data::MarketSource, opportunity, opportunity_detail};
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use common::AppError;
use filters::{OpportunitiesParams, OpportunityFilters, OpportunityListParams};
use hedge::{confirm_hedge, preview_hedge};
use limit::apply_limit;
use realtime::SnapshotEntry;
use serde::Deserialize;
use serde::Serialize;
#[cfg(test)]
use shared_types::problem::codes;
use shared_types::{
    is_p0_executable_strategy, ApiProblem, ArbitrageOpportunityDto, FundingRateData,
    OpportunityDetailRequest, OpportunityEnvelope, OpportunityEnvelopeScope,
    OpportunityEnvelopeStatus, OpportunityScanReport, StrategyKind, P0_EXECUTABLE_STRATEGY_KINDS,
};
use snapshot::{legacy_query_problems, response_parts};
use std::sync::Arc;
use std::time::Instant;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v3/arbitrage/opportunities", get(opportunities))
        .route(
            "/api/v3/arbitrage/opportunities/list",
            get(opportunities_list),
        )
        .route(
            "/api/v3/arbitrage/opportunities/:id",
            get(opportunity_detail),
        )
        .route(
            "/api/v3/arbitrage/opportunities/:id/detail",
            get(opportunity_combined_detail),
        )
        .route(
            "/api/arbitrage/opportunities/:id/preview",
            post(preview_hedge),
        )
        .route(
            "/api/arbitrage/opportunities/:id/confirm",
            post(confirm_hedge),
        )
        .route("/api/arbitrage/funding-rates", get(funding_rates))
}

async fn opportunities(
    State(state): State<AppState>,
    Query(params): Query<OpportunitiesParams>,
) -> Result<Json<OpportunityEnvelope>, AppError> {
    let limit = opportunity::wide_limit(params.limit);
    let limit_value = limit.value();
    let query_problems = legacy_query_problems(limit);
    let filters = OpportunityFilters::parse_with_limit(&params, limit_value);
    let parts = response_parts(&state, params.fresh, params.fast);

    let mut filtered_rows = parts.entry.value.opportunities.clone();
    filters.apply(&mut filtered_rows);
    let mut list = filtered_rows.clone();
    if let Some(limit) = filters.limit {
        apply_limit(&mut list, limit, Some(filters.strategy_kinds.as_slice()));
    }

    Ok(Json(opportunity::envelope(
        opportunity::OpportunityEnvelopeInput {
            opportunities: list,
            source_rows: &parts.entry.value.opportunities,
            filtered_rows: &filtered_rows,
            meta: parts.entry.value.meta.clone(),
            cached_at: parts.entry.cached_at,
            source: parts.source,
            status: parts.status,
            scope: filters.scope(),
            query_key: filters.query_key(&params),
            retry_after_ms: parts.retry_after_ms,
            error: parts.error,
            query_problems,
        },
    )))
}

async fn opportunities_list(
    State(state): State<AppState>,
    Query(params): Query<OpportunityListParams>,
) -> Result<axum::response::Response, AppError> {
    let filters = OpportunityFilters::parse_list(&params);
    let parts = response_parts(&state, params.fresh, params.fast);
    let strategy_scope_count = parts
        .entry
        .value
        .opportunities
        .iter()
        .filter(|row| filters.matches_strategy(row))
        .count();
    let symbol_scope_count = parts
        .entry
        .value
        .opportunities
        .iter()
        .filter(|row| filters.matches_strategy(row) && filters.matches_symbol(row))
        .count();
    let now_ms = common::time::now_ms();
    let mut refs = parts
        .entry
        .value
        .opportunities
        .iter()
        .filter(|row| filters.matches_product_row(row, now_ms))
        .collect::<Vec<_>>();
    let filtered_count = refs.len();
    let cursor_scope = opportunity::list_cursor_scope(&filters.list_filter_key());
    let window = opportunity::OpportunityListWindow::from_bound_query(
        filters.limit,
        params.cursor.as_deref(),
        params.sort_key.as_deref(),
        &cursor_scope,
    )
    .clamped_to_total(filtered_count);
    opportunity::sort_refs(refs.as_mut_slice(), window.sort_key(), now_ms);
    let rows = opportunity::page_refs(&refs, window);

    let instrument_coverage_diagnostics = filters
        .symbol
        .as_deref()
        .map(|symbol| {
            state
                .instrument_registry()
                .coverage_diagnostic(symbol, common::time::now_ms())
                .diagnostics_text
        })
        .unwrap_or_default();
    let mut payload = opportunity::list_envelope(opportunity::OpportunityListEnvelopeInput {
        rows,
        source_rows: &parts.entry.value.opportunities,
        request_meta: filters.list_request_meta(&params, window),
        strategy_scope_count,
        filtered_count,
        symbol_scope_count: Some(symbol_scope_count),
        meta: parts.entry.value.meta.clone(),
        cached_at: parts.entry.cached_at,
        snapshot_id: parts.snapshot_id.as_deref(),
        source: parts.source,
        status: parts.status,
        scope: filters.scope(),
        query_key: filters.list_query_key(&params, window),
        retry_after_ms: parts.retry_after_ms,
        error: parts.error,
        window,
    });
    payload.instrument_coverage_diagnostics = instrument_coverage_diagnostics;

    // 序列化一次：同一份 bytes 既记指标又作为响应体，此前 measure + axum Json
    // 会把整个 envelope（前端 5s 轮询的主列表，数十至数百 KB）序列化两遍。
    let (response, metric) = measured_json_response(&payload);
    if let Some(metric) = metric {
        state.metrics().record_rest_opportunity_list_payload(
            metric.payload_bytes,
            metric.serde_ms,
            payload.rows.len(),
        );
    }
    Ok(response)
}

async fn opportunity_detail(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<axum::response::Response, AppError> {
    let payload = opportunity_detail::opportunity_by_id(&state, &id)?;
    let (response, metric) = measured_json_response(&payload);
    if let Some(metric) = metric {
        state
            .metrics()
            .record_rest_opportunity_detail_seed_payload(metric.payload_bytes, metric.serde_ms);
    }
    Ok(response)
}

async fn opportunity_combined_detail(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<OpportunityDetailRequest>,
) -> Result<Json<shared_types::OpportunityDetailEnvelope>, AppError> {
    Ok(Json(opportunity_detail::detail(&state, id, params).await?))
}
// ===== GET /api/arbitrage/funding-rates =====

async fn funding_rates(
    State(state): State<AppState>,
) -> Result<Json<shared_types::FundingRatesEnvelope>, AppError> {
    let (snapshot, source) = funding_rates_snapshot(&state);
    Ok(Json(market_data::envelope::funding_rates_envelope(
        snapshot.rows,
        source,
        common::time::now_ms(),
        &state.market_data().runtime_health_snapshot(),
        snapshot.row_evidence,
    )))
}

fn funding_rates_snapshot(
    state: &AppState,
) -> (
    market_data::MarketRowsSnapshot<FundingRateData>,
    MarketSource,
) {
    (
        state.market_data().funding_rows_snapshot_with_evidence(),
        MarketSource::LocalCache,
    )
}

struct JsonPayloadMetric {
    payload_bytes: usize,
    serde_ms: u64,
}

/// 序列化一次，同一份 bytes 用于响应体与 payload 指标；序列化失败时退回
/// axum `Json` 的失败语义（500），指标为 None。
fn measured_json_response<T: Serialize>(
    payload: &T,
) -> (axum::response::Response, Option<JsonPayloadMetric>) {
    use axum::response::IntoResponse;
    let started = Instant::now();
    match serde_json::to_vec(payload) {
        Ok(bytes) => {
            let metric = JsonPayloadMetric {
                payload_bytes: bytes.len(),
                serde_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            };
            let response = (
                [(
                    axum::http::header::CONTENT_TYPE,
                    "application/json; charset=utf-8",
                )],
                bytes,
            )
                .into_response();
            (response, Some(metric))
        }
        Err(error) => (
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("payload serialization failed: {error}"),
            )
                .into_response(),
            None,
        ),
    }
}

#[cfg(test)]
mod tests;
