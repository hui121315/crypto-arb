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
        .route("/api/review/settlements", get(settlements))
        .route(
            "/api/review/strategy-performance",
            get(strategy_performance),
        )
}

async fn settlements(State(state): State<AppState>, Query(query): Query<shared_types::review::settlements::SettlementReviewQuery>)
    -> Result<Json<shared_types::review::settlements::SettlementReviewSnapshot>, common::AppError> {
    if !query.is_valid() {
        return Err(common::AppError::BadRequest("settlement record requires an exact source and a nonempty ID of at most 160 characters".into()));
    }
    Ok(Json(review::settlements::snapshot(&state, &query)))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewQuery {
    #[serde(default = "default_days")]
    days: u32,
    limit: Option<usize>,
    cursor: Option<String>,
    #[serde(flatten)]
    scope: shared_types::review::ReviewScope,
}

async fn executed(
    State(state): State<AppState>,
    Query(params): Query<ReviewQuery>,
) -> Result<Json<ReviewEnvelope<ExecutedTrade>>, common::AppError> {
    let page_query = review::ReviewPageQuery::new(params.limit, params.cursor);
    if params.scope.has_filter() {
        if !params.scope.is_valid() {
            return Err(common::AppError::BadRequest("review scope requires a runId or closeRunId and nonempty IDs of at most 160 characters".into()));
        }
        let run = params.scope.run_id.as_ref().and_then(|id| {
            state
                .execution_runs()
                .get(id)
                .map(|entry| entry.value().clone())
        });
        let close_runs = review::close_run_snapshots(state.close_runs().as_ref());
        return Ok(Json(
            review::scoped_executed_from_trading(
                state.trading_service(),
                &close_runs,
                params.days,
                &page_query,
                &params.scope,
                run.as_ref(),
            )
            .await
            .with_request_id(common::request_id::current()),
        ));
    }
    if params.days == 30 && page_query.is_default_first_page() {
        return Ok(Json(
            current_runtime_snapshot(&state)
                .executed
                .with_request_id(common::request_id::current()),
        ));
    }
    let close_runs = review::close_run_snapshots(state.close_runs().as_ref());
    Ok(Json(
        review::executed_envelope_from_trading(
            state.trading_service(),
            &close_runs,
            params.days,
            &page_query,
        )
        .await
        .with_request_id(common::request_id::current()),
    ))
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
    async fn scoped_http_query_is_authenticated_and_never_uses_unfiltered_runtime_cache(
    ) -> anyhow::Result<()> {
        use axum::{
            body::{to_bytes, Body},
            http::{Request, StatusCode},
        };
        use tower::ServiceExt;
        let mut config = common::config::AppConfig::default();
        config.security.auth_token = Some("isolated-review-scope".into());
        config.history.enabled = false;
        let state = AppState::new(config).await?;
        let router = crate::app::build_router(state);
        for (query, authenticated, expected) in [
            ("runId=missing", false, StatusCode::UNAUTHORIZED),
            ("ticketId=only-ticket", true, StatusCode::BAD_REQUEST),
            ("runId=", true, StatusCode::BAD_REQUEST),
            (
                "runId=missing&ticketId=t&opportunityId=o&days=365",
                true,
                StatusCode::OK,
            ),
            ("closeRunId=unknown", true, StatusCode::OK),
        ] {
            let request = Request::builder().uri(format!("/api/review/executed?{query}"));
            let request = if authenticated {
                request.header("Authorization", "Bearer isolated-review-scope")
            } else {
                request
            };
            let response = router.clone().oneshot(request.body(Body::empty())?).await?;
            assert_eq!(response.status(), expected, "{query}");
            if expected == StatusCode::OK {
                let envelope: ReviewEnvelope<ExecutedTrade> =
                    serde_json::from_slice(&to_bytes(response.into_body(), 128 * 1024).await?)?;
                assert!(envelope.rows.is_empty());
                assert!(envelope.page.snapshot_id.is_some());
                assert!(!envelope.problems.iter().any(|problem| problem.code
                    == shared_types::problem::codes::REVIEW_RUNTIME_SNAPSHOT_WARMING));
            }
        }
        Ok(())
    }

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
