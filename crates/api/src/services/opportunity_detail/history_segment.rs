use super::{DETAIL_SOURCE, MAX_HISTORY_LIMIT};
use crate::state::AppState;
use realtime::{history::HistoryError, HistoryStoreHealth, OpportunityQuery};
use shared_types::{
    problem::codes, ApiProblem, HistoryPage, HistoryResponse, OpportunityHistoryRow,
};

pub(super) async fn history(
    state: &AppState,
    symbol: &str,
    limit: usize,
    observed_at_ms: i64,
) -> HistoryResponse<OpportunityHistoryRow> {
    let result = state
        .history_store()
        .query_opportunities(OpportunityQuery {
            symbol: Some(symbol.to_owned()),
            min_yield: None,
            from_ms: None,
            to_ms: None,
            limit,
        })
        .await;
    let health = state.history_store().health_snapshot(observed_at_ms);
    match result {
        Ok(rows) => history_response(rows, limit, &health, Vec::new()),
        Err(error) => history_response(Vec::new(), limit, &health, vec![history_error(&error)]),
    }
}

fn history_response(
    rows: Vec<OpportunityHistoryRow>,
    limit: usize,
    health: &HistoryStoreHealth,
    mut problems: Vec<ApiProblem>,
) -> HistoryResponse<OpportunityHistoryRow> {
    if let Some(problem) = history_store_problem(health) {
        problems.push(problem);
    }
    let latest_at_ms = rows.iter().map(|row| row.occurred_at_ms).max();
    let page = HistoryPage {
        limit,
        max_limit: MAX_HISTORY_LIMIT,
        returned_count: rows.len(),
        has_more: false,
        next_cursor: None,
    };
    HistoryResponse {
        count: rows.len(),
        rows,
        page: Some(page.clone()),
        row_cap: Some(page.row_cap(format!("history:{}", health.backend))),
        backend_status: health.backend_status(),
        storage_health: None,
        source: health.backend.to_owned(),
        observed_at_ms: health.observed_at_ms,
        latest_at_ms,
        freshness_ms: latest_at_ms.map(|latest| health.observed_at_ms.saturating_sub(latest)),
        problem: problems.first().cloned(),
        retry_after_ms: problems.iter().find_map(|problem| problem.retry_after_ms),
        problems,
    }
}

fn history_error(error: &HistoryError) -> ApiProblem {
    ApiProblem::new(error.problem_code(), error.to_string())
        .with_request_id(common::request_id::current())
        .with_source(DETAIL_SOURCE)
}

fn history_store_problem(health: &HistoryStoreHealth) -> Option<ApiProblem> {
    if !health.enabled {
        return Some(
            ApiProblem::new(codes::HISTORY_STORE_UNAVAILABLE, "history store disabled")
                .with_request_id(common::request_id::current())
                .with_source(DETAIL_SOURCE),
        );
    }
    health.startup_problem.as_ref().map(|problem| {
        ApiProblem::new(
            codes::HISTORY_STORE_UNAVAILABLE,
            format!("history store running on fallback backend: {problem}"),
        )
        .with_request_id(common::request_id::current())
        .with_source(DETAIL_SOURCE)
    })
}
