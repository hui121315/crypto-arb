use crate::services::review;
use crate::state::AppState;
use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use shared_types::{
    ExecutedTrade, MissedOpportunity, ReviewEnvelope, ReviewRuntimeSnapshot, StrategyPerformance,
};

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/review/executed", get(executed))
        .route("/api/review/missed", get(missed))
        .route("/api/review/runtime", get(runtime_snapshot))
        .route(
            "/api/review/strategy-performance",
            get(strategy_performance),
        )
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewQuery {
    #[serde(default = "default_days")]
    days: u32,
    limit: Option<usize>,
    cursor: Option<String>,
}

async fn executed(
    State(state): State<AppState>,
    Query(params): Query<ReviewQuery>,
) -> Json<ReviewEnvelope<ExecutedTrade>> {
    let page_query = review::ReviewPageQuery::new(params.limit, params.cursor);
    if params.days == 30 && page_query.is_default_first_page() {
        return Json(
            current_runtime_snapshot(&state)
                .executed
                .with_request_id(common::request_id::current()),
        );
    }
    let close_runs = review::close_run_snapshots(state.close_runs().as_ref());
    Json(
        review::executed_envelope_from_trading(
            state.trading_service(),
            &close_runs,
            params.days,
            &page_query,
        )
        .await
        .with_request_id(common::request_id::current()),
    )
}

async fn missed(
    State(state): State<AppState>,
    Query(params): Query<ReviewQuery>,
) -> Json<ReviewEnvelope<MissedOpportunity>> {
    let page_query = review::ReviewPageQuery::new(params.limit, params.cursor);
    Json(
        review::missed_envelope_from_store(
            state.missed_opportunities().as_ref(),
            params.days,
            &page_query,
        )
        .with_request_id(common::request_id::current()),
    )
}

async fn strategy_performance(
    State(state): State<AppState>,
) -> Json<ReviewEnvelope<StrategyPerformance>> {
    Json(
        current_runtime_snapshot(&state)
            .strategy_performance
            .with_request_id(common::request_id::current()),
    )
}

async fn runtime_snapshot(State(state): State<AppState>) -> Json<ReviewRuntimeSnapshot> {
    Json(current_runtime_snapshot(&state).with_request_id(common::request_id::current()))
}

fn current_runtime_snapshot(state: &AppState) -> ReviewRuntimeSnapshot {
    if let Some(entry) = state.review_snapshot().get_arc_now() {
        return entry.value.clone();
    }
    review::warming_runtime_snapshot(common::time::now_ms())
}

fn default_days() -> u32 {
    30
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cold_runtime_read_returns_warming_without_populating_the_lifecycle_cache(
    ) -> anyhow::Result<()> {
        let state = AppState::new(common::config::AppConfig::default()).await?;

        let snapshot = current_runtime_snapshot(&state);

        assert_eq!(snapshot.executed.status, shared_types::ListStatus::Degraded);
        assert_eq!(
            snapshot.executed.problems[0].code,
            shared_types::problem::codes::REVIEW_RUNTIME_SNAPSHOT_WARMING
        );
        assert_eq!(
            snapshot.strategy_performance.problems[0].code,
            shared_types::problem::codes::REVIEW_RUNTIME_SNAPSHOT_WARMING
        );
        assert!(state.review_snapshot().get_arc_now().is_none());
        Ok(())
    }
}
