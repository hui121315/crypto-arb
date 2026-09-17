use crate::services::market_data::envelope::index_composition_envelope;
use crate::state::AppState;
use axum::http::StatusCode;
use common::AppError;
use shared_types::{
    is_p0_executable_strategy, problem::codes, ApiProblem, ArbitrageOpportunityDto,
    HistoryResponse, IndexCompositionSnapshot, MarketDataEnvelope, OpportunityDetailEnvelope,
    OpportunityDetailRequest, OpportunityDetailRequestMeta, OpportunityEnvelopeStatus,
    OpportunityHistoryRow, OpportunityRequestLimitMeta, OrderBookInfo, StrategyExposure,
};

mod history_segment;
mod orderbook_segment;
use history_segment::history;
use orderbook_segment::cached_orderbook_envelope;

const DETAIL_SOURCE: &str = "opportunity-detail";
const DEFAULT_ORDERBOOK_DEPTH: u32 = 5;
const MAX_ORDERBOOK_DEPTH: u32 = 100;
const DEFAULT_HISTORY_LIMIT: usize = 6;
const MAX_HISTORY_LIMIT: usize = 50;

pub(crate) async fn detail(
    state: &AppState,
    id: String,
    query: OpportunityDetailRequest,
) -> Result<OpportunityDetailEnvelope, AppError> {
    let opportunity = opportunity_by_id(state, &id)?;
    let observed_at_ms = common::time::now_ms();
    let request_meta = detail_request_meta(query);
    let query_problems = detail_query_problems(query, request_meta);
    let depth = request_meta.orderbook_depth.applied as u32;
    let history_limit = request_meta.history_limit.applied;
    let long_venue = opportunity.long_exchange.clone();
    let short_venue = opportunity.short_exchange.clone();
    let symbol = opportunity.symbol.clone();

    let (long_cached, short_cached) = crate::services::hedge_ticket::cached_opportunity_orderbooks(
        state,
        &opportunity,
        observed_at_ms,
    );
    let (history, long_index, short_index) = tokio::join!(
        history(state, &symbol, history_limit, observed_at_ms),
        index_composition(state, &long_venue, &symbol, observed_at_ms),
        index_composition(state, &short_venue, &symbol, observed_at_ms),
    );
    let long_orderbook = cached_orderbook_envelope(long_cached, depth, observed_at_ms);
    let short_orderbook = cached_orderbook_envelope(short_cached, depth, observed_at_ms);

    Ok(envelope(
        opportunity,
        DetailSegments {
            long_orderbook,
            short_orderbook,
            history,
            long_index_composition: long_index,
            short_index_composition: short_index,
        },
        request_meta,
        query_problems,
        observed_at_ms,
    ))
}

pub(crate) fn opportunity_by_id(
    state: &AppState,
    id: &str,
) -> Result<ArbitrageOpportunityDto, AppError> {
    let opportunity = state
        .opportunity_index()
        .current(id)
        .ok_or_else(|| expired_error(id))?;
    ensure_main_p0_opportunity(&opportunity)?;
    Ok(opportunity)
}

fn ensure_main_p0_opportunity(opportunity: &ArbitrageOpportunityDto) -> Result<(), AppError> {
    if opportunity
        .strategy_kind
        .is_some_and(is_p0_executable_strategy)
    {
        return Ok(());
    }
    let exposure = opportunity
        .strategy_kind
        .map(StrategyExposure::for_kind)
        .unwrap_or(StrategyExposure::Hidden);
    Err(AppError::domain(
        StatusCode::BAD_REQUEST,
        codes::OPPORTUNITY_NOT_EXECUTABLE,
        "opportunity is outside the main P0 strategy scope",
    )
    .with_details(serde_json::json!({
        "opportunityId": opportunity.id,
        "strategyKind": opportunity.strategy_kind,
        "strategyExposure": exposure,
        "scope": "main_p0",
    })))
}

fn expired_error(id: &str) -> AppError {
    AppError::domain(
        StatusCode::NOT_FOUND,
        codes::OPPORTUNITY_EXPIRED,
        format!("opportunity expired: {id}"),
    )
    .with_details(serde_json::json!({ "opportunityId": id }))
}

async fn index_composition(
    state: &AppState,
    venue: &str,
    symbol: &str,
    observed_at_ms: i64,
) -> MarketDataEnvelope<Option<IndexCompositionSnapshot>> {
    let read = state
        .market_data()
        .index_composition_or_fetch(state.aggregator(), venue, symbol, observed_at_ms)
        .await;
    index_composition_envelope(read, venue, symbol, observed_at_ms)
}

struct DetailSegments {
    long_orderbook: MarketDataEnvelope<Option<OrderBookInfo>>,
    short_orderbook: MarketDataEnvelope<Option<OrderBookInfo>>,
    history: HistoryResponse<OpportunityHistoryRow>,
    long_index_composition: MarketDataEnvelope<Option<IndexCompositionSnapshot>>,
    short_index_composition: MarketDataEnvelope<Option<IndexCompositionSnapshot>>,
}

fn envelope(
    opportunity: ArbitrageOpportunityDto,
    segments: DetailSegments,
    request_meta: OpportunityDetailRequestMeta,
    query_problems: Vec<ApiProblem>,
    observed_at_ms: i64,
) -> OpportunityDetailEnvelope {
    let mut partial_failures = partial_failures(&segments);
    partial_failures.extend(query_problems);
    let error = partial_failures.first().cloned();
    let retry_after_ms = retry_after_ms(&partial_failures);
    OpportunityDetailEnvelope {
        request_meta,
        freshness_ms: detail_freshness_ms(&opportunity, &segments, observed_at_ms),
        status: if partial_failures.is_empty() {
            OpportunityEnvelopeStatus::Fresh
        } else {
            OpportunityEnvelopeStatus::Degraded
        },
        source: DETAIL_SOURCE.to_owned(),
        observed_at_ms,
        request_id: common::request_id::current(),
        opportunity,
        long_orderbook: segments.long_orderbook,
        short_orderbook: segments.short_orderbook,
        history: segments.history,
        long_index_composition: segments.long_index_composition,
        short_index_composition: segments.short_index_composition,
        retry_after_ms,
        error,
        partial_failures,
    }
}

fn partial_failures(segments: &DetailSegments) -> Vec<ApiProblem> {
    let mut failures = Vec::new();
    push_history_problems(&mut failures, &segments.history);
    push_market_problem(&mut failures, &segments.long_index_composition);
    push_market_problem(&mut failures, &segments.short_index_composition);
    failures
}

fn retry_after_ms(partial_failures: &[ApiProblem]) -> Option<u64> {
    partial_failures
        .iter()
        .filter_map(|problem| problem.retry_after_ms)
        .max()
}

fn push_market_problem<T>(
    failures: &mut Vec<ApiProblem>,
    envelope: &MarketDataEnvelope<Option<T>>,
) {
    if let Some(problem) = envelope.health.problem.clone() {
        failures.push(problem.with_request_id(common::request_id::current()));
    }
}

fn push_history_problems(
    failures: &mut Vec<ApiProblem>,
    history: &HistoryResponse<OpportunityHistoryRow>,
) {
    if history.problems.is_empty() {
        if let Some(problem) = history.problem.clone() {
            failures.push(problem);
        }
    } else {
        failures.extend(history.problems.iter().cloned());
    }
}

fn detail_freshness_ms(
    opportunity: &ArbitrageOpportunityDto,
    segments: &DetailSegments,
    observed_at_ms: i64,
) -> Option<i64> {
    [
        Some(observed_at_ms.saturating_sub(opportunity.updated_at.timestamp_millis())),
        segments.history.freshness_ms,
        segments.long_index_composition.health.freshness_ms,
        segments.short_index_composition.health.freshness_ms,
    ]
    .into_iter()
    .flatten()
    .max()
}

fn detail_request_meta(query: OpportunityDetailRequest) -> OpportunityDetailRequestMeta {
    OpportunityDetailRequestMeta {
        orderbook_depth: OpportunityRequestLimitMeta {
            requested: query.depth.map(|value| value as usize),
            applied: query
                .depth
                .unwrap_or(DEFAULT_ORDERBOOK_DEPTH)
                .clamp(1, MAX_ORDERBOOK_DEPTH) as usize,
            max: MAX_ORDERBOOK_DEPTH as usize,
        },
        history_limit: OpportunityRequestLimitMeta {
            requested: query.history_limit,
            applied: query
                .history_limit
                .unwrap_or(DEFAULT_HISTORY_LIMIT)
                .clamp(1, MAX_HISTORY_LIMIT),
            max: MAX_HISTORY_LIMIT,
        },
    }
}

fn detail_query_problems(
    query: OpportunityDetailRequest,
    meta: OpportunityDetailRequestMeta,
) -> Vec<ApiProblem> {
    [
        query_limit_problem(
            "depth",
            query.depth.map(|value| value as usize),
            meta.orderbook_depth,
        ),
        query_limit_problem("historyLimit", query.history_limit, meta.history_limit),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn query_limit_problem(
    field: &'static str,
    requested: Option<usize>,
    meta: OpportunityRequestLimitMeta,
) -> Option<ApiProblem> {
    let requested = requested.filter(|requested| *requested != meta.applied)?;
    let mut problem = ApiProblem::new(
        codes::LIST_LIMIT_CLAMPED,
        format!("opportunity detail {field} was normalized"),
    )
    .with_source(DETAIL_SOURCE);
    problem.details = Some(serde_json::json!({
        "field": field,
        "requested": requested,
        "applied": meta.applied,
        "max": meta.max,
    }));
    Some(problem)
}

#[cfg(test)]
mod tests;
