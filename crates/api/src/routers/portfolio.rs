use crate::services::{
    action_runs::{self, ActionRunStart},
    close_runs, portfolio, portfolio_actions, portfolio_snapshot_envelope,
    ws_publish::{publish_close_run_event, publish_order_event},
};
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use common::AppError;
use serde::Deserialize;
use serde_json::json;
use shared_types::{
    problem::codes, ActionRun, ActionRunKind, ActionRunStatus, ApiProblem,
    CloseAllPositionsRequest, ClosePositionRequest, CloseRun, CloseRunCompensationRequest,
    CloseRunManualTerminalRequest, HistoryResponse, PortfolioNavHistoryRow,
    PortfolioSnapshotEnvelope, PositionSide,
};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/trading/portfolio/snapshot", get(snapshot))
        .route("/api/trading/portfolio/nav-history", get(nav_history))
        .route(
            "/api/trading/portfolio/positions/:venue/:symbol/close",
            post(close_position),
        )
        .route(
            "/api/trading/portfolio/positions/:venue/:symbol/close-pair",
            post(close_position_pair),
        )
        .route(
            "/api/trading/portfolio/close-all",
            post(close_all_positions),
        )
        .route(
            "/api/trading/portfolio/close-runs/:close_run_id/compensation-orders",
            post(close_run_compensation_order),
        )
        .route(
            "/api/trading/portfolio/close-runs/:close_run_id/manual-terminal",
            post(close_run_manual_terminal),
        )
}

async fn snapshot(State(state): State<AppState>) -> Json<PortfolioSnapshotEnvelope> {
    let cache = state.portfolio_snapshot_envelope();
    let entry = cache.get_arc_now();
    Json(portfolio_snapshot_envelope::lifecycle_cache_response(
        entry.as_deref(),
        cache.refresh_interval(),
        chrono::Utc::now(),
    ))
}

#[derive(Debug, Default, Deserialize)]
struct NavHistoryQuery {
    limit: Option<usize>,
}

async fn nav_history(
    State(state): State<AppState>,
    Query(query): Query<NavHistoryQuery>,
) -> Json<HistoryResponse<PortfolioNavHistoryRow>> {
    Json(portfolio::nav_history(&state, query.limit).await)
}

async fn close_position(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((venue, symbol)): Path<(String, String)>,
    Json(payload): Json<ClosePositionRequest>,
) -> Result<Json<CloseRun>, AppError> {
    let idempotency_key = close_action_idempotency_key(
        &headers,
        close_position_idempotency_key("single", &venue, &symbol, &payload),
    );
    let claim = action_runs::begin_idempotent(
        &state,
        ActionRunStart::new(
            ActionRunKind::PortfolioClosePosition,
            &headers,
            Some(position_target(&venue, &symbol)),
            "position close accepted",
        )
        .with_idempotency_key(Some(idempotency_key.clone())),
    )?;
    if claim.is_replayed() {
        return replay_close_run(claim.run(), &idempotency_key);
    }
    let result = match portfolio_actions::CloseRequestContext::from_position_request(&payload) {
        Ok(context) => {
            portfolio_actions::close_position(
                &state,
                &venue,
                &symbol,
                payload.side,
                context.with_idempotency_key(idempotency_key.clone()),
            )
            .await
        }
        Err(error) => Err(error),
    };
    close_run_result(&state, claim.run(), "position_close_submitted", result)
}

async fn close_position_pair(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((venue, symbol)): Path<(String, String)>,
    Json(payload): Json<ClosePositionRequest>,
) -> Result<Json<CloseRun>, AppError> {
    let idempotency_key = close_action_idempotency_key(
        &headers,
        close_position_idempotency_key("pair", &venue, &symbol, &payload),
    );
    let claim = action_runs::begin_idempotent(
        &state,
        ActionRunStart::new(
            ActionRunKind::PortfolioClosePair,
            &headers,
            Some(position_target(&venue, &symbol)),
            "position pair close accepted",
        )
        .with_idempotency_key(Some(idempotency_key.clone())),
    )?;
    if claim.is_replayed() {
        return replay_close_run(claim.run(), &idempotency_key);
    }
    let result = match portfolio_actions::CloseRequestContext::from_position_request(&payload) {
        Ok(context) => {
            portfolio_actions::close_position_pair(
                &state,
                &venue,
                &symbol,
                payload.side,
                context.with_idempotency_key(idempotency_key.clone()),
            )
            .await
        }
        Err(error) => Err(error),
    };
    close_run_result(&state, claim.run(), "position_pair_close_submitted", result)
}

async fn close_all_positions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CloseAllPositionsRequest>,
) -> Result<Json<CloseRun>, AppError> {
    let idempotency_key =
        close_action_idempotency_key(&headers, close_all_idempotency_key(&payload));
    let claim = action_runs::begin_idempotent(
        &state,
        ActionRunStart::new(
            ActionRunKind::PortfolioCloseAll,
            &headers,
            Some("all-positions".to_owned()),
            "close all positions accepted",
        )
        .with_idempotency_key(Some(idempotency_key.clone())),
    )?;
    if claim.is_replayed() {
        return replay_close_run(claim.run(), &idempotency_key);
    }
    let result = match portfolio_actions::CloseRequestContext::from_all_request(&payload) {
        Ok(context) => {
            portfolio_actions::close_all_positions(
                &state,
                &payload.confirmation_phrase,
                context.with_idempotency_key(idempotency_key.clone()),
            )
            .await
        }
        Err(error) => Err(error),
    };
    close_run_result(&state, claim.run(), "position_close_submitted", result)
}

async fn close_run_compensation_order(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(close_run_id): Path<String>,
    Json(payload): Json<CloseRunCompensationRequest>,
) -> Result<Json<CloseRun>, AppError> {
    let idempotency_key = close_action_idempotency_key(
        &headers,
        close_run_compensation_idempotency_key(&close_run_id, &payload),
    );
    let claim = action_runs::begin_idempotent(
        &state,
        ActionRunStart::new(
            ActionRunKind::PortfolioCloseCompensation,
            &headers,
            Some(close_run_id.clone()),
            "close-run compensation accepted",
        )
        .with_idempotency_key(Some(idempotency_key.clone())),
    )?;
    if claim.is_replayed() {
        return replay_close_run(claim.run(), &idempotency_key);
    }
    let result =
        close_runs::submit_compensation_order(&state, &close_run_id, &payload, claim.run()).await;
    close_run_compensation_result(
        &state,
        claim.run(),
        "close_run_compensation_submitted",
        result,
    )
}

async fn close_run_manual_terminal(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(close_run_id): Path<String>,
    Json(payload): Json<CloseRunManualTerminalRequest>,
) -> Result<Json<CloseRun>, AppError> {
    let idempotency_key = close_action_idempotency_key(
        &headers,
        close_run_manual_terminal_idempotency_key(&close_run_id, &payload),
    );
    let claim = action_runs::begin_idempotent(
        &state,
        ActionRunStart::new(
            ActionRunKind::PortfolioCloseManualTerminal,
            &headers,
            Some(close_run_id.clone()),
            "close-run manual terminal evidence accepted",
        )
        .with_idempotency_key(Some(idempotency_key.clone())),
    )?;
    if claim.is_replayed() {
        return replay_close_run(claim.run(), &idempotency_key);
    }
    let result =
        close_runs::record_manual_terminal_evidence(&state, &close_run_id, &payload, claim.run())
            .await;
    close_run_manual_terminal_result(
        &state,
        claim.run(),
        "close_run_manual_terminal_recorded",
        result,
    )
}

fn position_target(venue: &str, symbol: &str) -> String {
    format!("{venue}:{symbol}")
}

fn close_run_result(
    state: &AppState,
    action_run: &ActionRun,
    event: &'static str,
    result: Result<CloseRun, AppError>,
) -> Result<Json<CloseRun>, AppError> {
    let mut close_run = match result {
        Ok(run) => run,
        Err(error) => return action_runs::fail_response(state, &action_run.id, error),
    };
    close_run.action_run_id = Some(action_run.id.clone());
    close_run.request_id = action_run.request_id.clone();
    close_run = close_runs::record(state, close_run);
    finish_close_action_run(state, action_run, &close_run)?;
    publish_close_run_events(state, event, &close_run)?;
    Ok(Json(close_run))
}

fn close_run_compensation_result(
    state: &AppState,
    action_run: &ActionRun,
    event: &'static str,
    result: Result<CloseRun, AppError>,
) -> Result<Json<CloseRun>, AppError> {
    let close_run = match result {
        Ok(run) => run,
        Err(error) => return action_runs::fail_response(state, &action_run.id, error),
    };
    finish_close_action_run(state, action_run, &close_run)?;
    publish_compensation_order_events(state, event, action_run, &close_run)?;
    Ok(Json(close_run))
}

fn close_run_manual_terminal_result(
    state: &AppState,
    action_run: &ActionRun,
    event: &'static str,
    result: Result<CloseRun, AppError>,
) -> Result<Json<CloseRun>, AppError> {
    let close_run = match result {
        Ok(run) => run,
        Err(error) => return action_runs::fail_response(state, &action_run.id, error),
    };
    finish_close_action_run(state, action_run, &close_run)?;
    publish_close_run_event(state, event, &close_run)?;
    Ok(Json(close_run))
}

fn publish_close_run_events(
    state: &AppState,
    event: &'static str,
    close_run: &CloseRun,
) -> Result<(), AppError> {
    for record in close_run.legs.iter().filter_map(|leg| leg.order.as_ref()) {
        publish_order_event(state, event, record)?;
    }
    Ok(())
}

fn publish_compensation_order_events(
    state: &AppState,
    event: &'static str,
    action_run: &ActionRun,
    close_run: &CloseRun,
) -> Result<(), AppError> {
    let Some(plan) = close_run.unwind_plan.as_ref() else {
        return Ok(());
    };
    for record in plan
        .compensation_attempts
        .iter()
        .filter(|attempt| {
            attempt
                .action_run_id
                .as_deref()
                .is_some_and(|id| id == action_run.id)
        })
        .filter_map(|attempt| attempt.order.as_ref())
    {
        publish_order_event(state, event, record)?;
    }
    Ok(())
}

fn finish_close_action_run(
    state: &AppState,
    action_run: &ActionRun,
    close_run: &CloseRun,
) -> Result<(), AppError> {
    action_runs::finish_status_with_payload(
        state,
        &action_run.id,
        close_runs::action_status(close_run.status),
        close_run.message.clone(),
        close_run.problem.clone(),
        close_run,
    )
    .map(|_| ())
}

fn replay_close_run(run: &ActionRun, idempotency_key: &str) -> Result<Json<CloseRun>, AppError> {
    match run.status {
        ActionRunStatus::Accepted => Err(close_action_in_flight(run, idempotency_key)),
        ActionRunStatus::Succeeded | ActionRunStatus::Failed => match replay_close_payload(run) {
            Ok(close_run) => Ok(Json(close_run)),
            Err(_) if close_replay_can_use_original_problem(run) => {
                Err(close_replay_failed_or_problem(run, idempotency_key))
            }
            Err(error) => Err(error),
        },
    }
}

fn close_replay_can_use_original_problem(run: &ActionRun) -> bool {
    run.status == ActionRunStatus::Failed && run.result.is_none() && run.problem.is_some()
}

fn replay_close_payload(run: &ActionRun) -> Result<CloseRun, AppError> {
    let mut close_run = action_runs::replay_payload::<CloseRun>(run)?;
    close_run.action_run_id = Some(run.id.clone());
    close_run.request_id = run.request_id.clone();
    if close_run.idempotency_key.is_none() {
        close_run.idempotency_key = run.idempotency_key.clone();
    }
    Ok(close_run)
}

fn close_action_in_flight(run: &ActionRun, idempotency_key: &str) -> AppError {
    AppError::domain(
        StatusCode::CONFLICT,
        codes::ACTION_RUN_IN_FLIGHT,
        "portfolio close idempotency key is already in flight",
    )
    .with_details(json!({
        "actionRunId": run.id,
        "requestId": run.request_id,
        "idempotencyKey": idempotency_key,
        "status": run.status,
    }))
}

fn close_replay_failed_or_problem(run: &ActionRun, idempotency_key: &str) -> AppError {
    if let Some(problem) = run.problem.as_ref() {
        return close_replay_problem(run, idempotency_key, problem);
    }
    AppError::domain(
        StatusCode::CONFLICT,
        codes::ACTION_RUN_REPLAY_FAILED,
        "portfolio close idempotency key already failed",
    )
    .with_details(json!({
        "actionRunId": run.id,
        "requestId": run.request_id,
        "idempotencyKey": idempotency_key,
        "problem": run.problem,
    }))
}

fn close_replay_problem(run: &ActionRun, idempotency_key: &str, problem: &ApiProblem) -> AppError {
    AppError::domain(
        problem_status(problem),
        close_replay_problem_code(&problem.code),
        problem.message.clone(),
    )
    .with_details(json!({
        "actionRunId": run.id,
        "requestId": run.request_id,
        "idempotencyKey": idempotency_key,
        "replayed": true,
        "originalProblem": problem,
    }))
}

fn problem_status(problem: &ApiProblem) -> StatusCode {
    problem
        .status
        .and_then(|status| StatusCode::from_u16(status).ok())
        .unwrap_or(StatusCode::CONFLICT)
}

fn close_replay_problem_code(code: &str) -> &'static str {
    match code {
        codes::CLOSE_RUN_REQUEST_INVALID => codes::CLOSE_RUN_REQUEST_INVALID,
        codes::CLOSE_RUN_STALE_SNAPSHOT => codes::CLOSE_RUN_STALE_SNAPSHOT,
        codes::CLOSE_RUN_EXPECTED_LEG_MISMATCH => codes::CLOSE_RUN_EXPECTED_LEG_MISMATCH,
        codes::CLOSE_RUN_UNWIND_REQUIRED => codes::CLOSE_RUN_UNWIND_REQUIRED,
        codes::CLOSE_RUN_COMPENSATION_FAILED => codes::CLOSE_RUN_COMPENSATION_FAILED,
        codes::ACTION_RUN_IN_FLIGHT => codes::ACTION_RUN_IN_FLIGHT,
        codes::ACTION_RUN_REPLAY_UNAVAILABLE => codes::ACTION_RUN_REPLAY_UNAVAILABLE,
        "NOT_FOUND" => "NOT_FOUND",
        "BAD_REQUEST" => "BAD_REQUEST",
        _ => codes::ACTION_RUN_REPLAY_FAILED,
    }
}

fn close_position_idempotency_key(
    scope: &str,
    venue: &str,
    symbol: &str,
    payload: &ClosePositionRequest,
) -> String {
    format!(
        "portfolio-close:{scope}:{}:{}:{}:{}:{}",
        shared_types::normalized_venue_name(venue),
        symbol.trim().to_ascii_uppercase(),
        side_key(payload.side),
        optional_key(payload.snapshot_version.as_deref()),
        count_key(payload.expected_leg_count),
    )
}

fn close_all_idempotency_key(payload: &CloseAllPositionsRequest) -> String {
    format!(
        "portfolio-close:all:{}:{}:{}",
        optional_key(payload.snapshot_version.as_deref()),
        count_key(payload.expected_leg_count),
        optional_key(Some(&payload.confirmation_phrase)),
    )
}

fn close_run_compensation_idempotency_key(
    close_run_id: &str,
    payload: &CloseRunCompensationRequest,
) -> String {
    format!(
        "portfolio-close:compensate:{}:{}:{}:{}:{}:{}",
        close_run_id.trim(),
        count_key(payload.candidate_index),
        optional_key(payload.snapshot_version.as_deref()),
        number_key(payload.target_quantity),
        number_key(payload.limit_price),
        optional_key(Some(&payload.confirmation_phrase)),
    )
}

fn close_run_manual_terminal_idempotency_key(
    close_run_id: &str,
    payload: &CloseRunManualTerminalRequest,
) -> String {
    format!(
        "portfolio-close:manual-terminal:{}:{}:{:016x}",
        close_run_id.trim(),
        optional_key(payload.snapshot_version.as_deref()),
        stable_payload_hash(payload),
    )
}

fn stable_payload_hash(payload: &CloseRunManualTerminalRequest) -> u64 {
    let mut hasher = DefaultHasher::new();
    payload.confirmation_phrase.hash(&mut hasher);
    payload.reason.hash(&mut hasher);
    number_key(payload.manual_handling_cost_usd).hash(&mut hasher);
    for item in &payload.evidence {
        item.hash(&mut hasher);
    }
    hasher.finish()
}

fn side_key(side: Option<PositionSide>) -> &'static str {
    match side {
        Some(PositionSide::Long) => "long",
        Some(PositionSide::Short) => "short",
        None => "any",
    }
}

fn optional_key(value: Option<&str>) -> String {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("-")
        .to_owned()
}

fn count_key(value: Option<usize>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_owned())
}

fn number_key(value: Option<f64>) -> String {
    value
        .filter(|value| value.is_finite())
        .map(|value| format!("{value:.12}"))
        .unwrap_or_else(|| "-".to_owned())
}

fn close_action_idempotency_key(headers: &HeaderMap, derived: String) -> String {
    explicit_idempotency_key(headers).unwrap_or(derived)
}

fn explicit_idempotency_key(headers: &HeaderMap) -> Option<String> {
    headers
        .get("idempotency-key")
        .or_else(|| headers.get("x-idempotency-key"))
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::panic)]

    use super::*;
    use common::config::AppConfig;
    use shared_types::{
        CloseLegStatus, CloseRunCompensationRequest, CloseRunManualTerminalRequest, CloseRunStatus,
        ExecutionMode, ExecutionRun, ExecutionRunLeg, ExecutionRunState, HedgeLegRole,
        LiveOrderState, MarginMode, OrderBookInfo, OrderIntent, OrderRecord, OrderSide,
        OrderSource, OrderType, OrderUpdateSource, TimeInForce,
        CLOSE_ALL_POSITIONS_CONFIRMATION_PHRASE, CLOSE_RUN_COMPENSATION_CONFIRMATION_PHRASE,
        CLOSE_RUN_MANUAL_TERMINAL_CONFIRMATION_PHRASE,
    };
    use tokio::sync::broadcast::error::TryRecvError;

    #[tokio::test]
    async fn close_position_replays_immediate_paper_finality_without_resubmit() {
        let state = test_state().await;
        submit_open_order(&state, "open-long", "binance", "MUUSDT", OrderSide::Buy).await;
        let rows = position_rows(&state).await;
        let payload = close_payload(&state, &rows, Some(PositionSide::Long), 1, "positions.close");
        let mut orders = state.ws_hub().subscribe(realtime::channels::ORDERS);

        let first = close_position_response(&state, "binance", "MUUSDT", payload.clone()).await;
        assert_eq!(first.submitted_order_count, 1);
        assert_eq!(first.status, CloseRunStatus::Succeeded);
        assert_order_event(&mut orders, "first close did not publish an order event");

        let replay = close_position(
            State(state.clone()),
            HeaderMap::new(),
            Path(("binance".to_owned(), "MUUSDT".to_owned())),
            Json(payload),
        )
        .await;
        let replay = match replay {
            Ok(Json(run)) => run,
            Err(error) => panic!("completed close replay failed: {error}"),
        };

        assert_eq!(replay.id, first.id);
        assert_eq!(replay.status, CloseRunStatus::Succeeded);
        assert!(matches!(orders.try_recv(), Err(TryRecvError::Empty)));
        assert_eq!(
            action_runs::recent(&state)
                .into_iter()
                .filter(|run| run.kind == ActionRunKind::PortfolioClosePosition)
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn close_position_replays_pre_close_failure_without_second_action_run() {
        let state = test_state().await;
        let rows = position_rows(&state).await;
        let payload = close_payload(&state, &rows, Some(PositionSide::Long), 1, "positions.close");

        let first = close_position(
            State(state.clone()),
            HeaderMap::new(),
            Path(("binance".to_owned(), "MUUSDT".to_owned())),
            Json(payload.clone()),
        )
        .await;
        let first_error = match first {
            Ok(run) => panic!("missing position unexpectedly closed: {run:?}"),
            Err(error) => error,
        };
        assert_eq!(first_error.status(), StatusCode::NOT_FOUND);

        let replay = close_position(
            State(state.clone()),
            HeaderMap::new(),
            Path(("binance".to_owned(), "MUUSDT".to_owned())),
            Json(payload),
        )
        .await;
        let replay_error = match replay {
            Ok(run) => panic!("failed close unexpectedly replayed success: {run:?}"),
            Err(error) => error,
        };

        assert_eq!(replay_error.status(), StatusCode::NOT_FOUND);
        assert_eq!(replay_error.code(), "NOT_FOUND");
        assert_eq!(
            action_runs::recent(&state)
                .into_iter()
                .filter(|run| run.kind == ActionRunKind::PortfolioClosePosition)
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn close_position_replay_does_not_hide_malformed_stored_payload() {
        let state = test_state().await;
        let rows = position_rows(&state).await;
        let payload = close_payload(&state, &rows, Some(PositionSide::Long), 1, "positions.close");

        let first = close_position(
            State(state.clone()),
            HeaderMap::new(),
            Path(("binance".to_owned(), "MUUSDT".to_owned())),
            Json(payload.clone()),
        )
        .await;
        assert!(first.is_err());
        let action_run = action_runs::recent(&state)
            .into_iter()
            .find(|run| run.kind == ActionRunKind::PortfolioClosePosition)
            .unwrap_or_else(|| panic!("close action run missing"));
        {
            let mut entry = state
                .action_runs()
                .get_mut(&action_run.id)
                .unwrap_or_else(|| panic!("stored action run missing"));
            entry.result = Some(json!({ "status": "not_a_close_run" }));
        }

        let replay = close_position(
            State(state.clone()),
            HeaderMap::new(),
            Path(("binance".to_owned(), "MUUSDT".to_owned())),
            Json(payload),
        )
        .await;
        let replay_error = match replay {
            Ok(run) => panic!("malformed close payload unexpectedly replayed: {run:?}"),
            Err(error) => error,
        };

        assert_eq!(replay_error.status(), StatusCode::CONFLICT);
        assert_eq!(replay_error.code(), codes::ACTION_RUN_REPLAY_UNAVAILABLE);
    }

    #[tokio::test]
    async fn close_pair_replays_immediate_paper_finality_without_resubmit() {
        let state = test_state().await;
        let long =
            submit_open_order(&state, "open-long", "binance", "MUUSDT", OrderSide::Buy).await;
        let short = submit_open_order(&state, "open-short", "okx", "MUUSDT", OrderSide::Sell).await;
        seed_pair_execution_run(&state, &long, &short);
        let rows = position_rows(&state).await;
        let payload = close_payload(&state, &rows, Some(PositionSide::Long), 2, "positions.close_pair");
        let mut orders = state.ws_hub().subscribe(realtime::channels::ORDERS);

        let first = close_pair_response(&state, "binance", "MUUSDT", payload.clone()).await;
        assert_eq!(first.submitted_order_count, 2);
        assert_eq!(first.status, CloseRunStatus::Succeeded);
        assert_order_event(&mut orders, "first close-pair leg did not publish");
        assert_order_event(&mut orders, "second close-pair leg did not publish");

        let replay = close_position_pair(
            State(state.clone()),
            HeaderMap::new(),
            Path(("binance".to_owned(), "MUUSDT".to_owned())),
            Json(payload),
        )
        .await;
        let replay = match replay {
            Ok(Json(run)) => run,
            Err(error) => panic!("completed close-pair replay failed: {error}"),
        };

        assert_eq!(replay.id, first.id);
        assert_eq!(replay.status, CloseRunStatus::Succeeded);
        assert!(matches!(orders.try_recv(), Err(TryRecvError::Empty)));
        assert_eq!(
            action_runs::recent(&state)
                .into_iter()
                .filter(|run| run.kind == ActionRunKind::PortfolioClosePair)
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn close_order_finality_updates_action_run_payload() {
        let state = test_state().await;
        submit_open_order(&state, "open-long", "binance", "MUUSDT", OrderSide::Buy).await;
        let rows = position_rows(&state).await;
        let payload = close_payload(&state, &rows, Some(PositionSide::Long), 1, "positions.close");
        let first = close_position_response(&state, "binance", "MUUSDT", payload).await;
        let mut record = first.legs[0]
            .order
            .clone()
            .unwrap_or_else(|| panic!("close leg missing submitted order"));
        record.state = LiveOrderState::Filled;
        record.filled_quantity = Some(record.intent.quantity);
        record.filled_price = record.intent.price;
        record.last_update_source = OrderUpdateSource::PrivateWs;
        record.updated_at_ms = first.updated_at_ms + 1;

        publish_order_event(&state, "position_close_filled", &record)
            .unwrap_or_else(|error| panic!("publish close fill failed: {error}"));

        let action_run_id = first
            .action_run_id
            .as_deref()
            .unwrap_or_else(|| panic!("close response missing action run id"));
        let action_run = action_runs::get(&state, action_run_id)
            .unwrap_or_else(|| panic!("close action run missing"));
        assert_eq!(action_run.status, ActionRunStatus::Succeeded);
        let run = action_runs::replay_payload::<CloseRun>(&action_run)
            .unwrap_or_else(|error| panic!("close action run payload missing: {error}"));

        assert_eq!(run.status, CloseRunStatus::Succeeded);
        assert_eq!(run.legs[0].status, CloseLegStatus::Filled);
        assert_eq!(
            run.legs[0].finality_source,
            Some(OrderUpdateSource::PrivateWs)
        );
    }

    #[tokio::test]
    async fn close_run_compensation_submits_server_bound_order() {
        let state = test_state().await;
        let long =
            submit_open_order(&state, "open-long", "binance", "MUUSDT", OrderSide::Buy).await;
        let short = submit_open_order(&state, "open-short", "okx", "MUUSDT", OrderSide::Sell).await;
        seed_pair_execution_run(&state, &long, &short);
        let rows = position_rows(&state).await;
        let close_payload =
            close_payload(&state, &rows, Some(PositionSide::Long), 2, "positions.close_pair");
        let close_run = close_pair_response(&state, "binance", "MUUSDT", close_payload).await;
        publish_close_leg_state(&state, &close_run, 0, LiveOrderState::Filled, true);
        publish_close_leg_state(&state, &close_run, 1, LiveOrderState::Cancelled, false);
        let close_run = stored_close_run(&state, &close_run.id);
        assert_eq!(close_run.status, CloseRunStatus::UnwindRequired);
        let payload = compensation_payload(&close_run);
        seed_orderbook(&state, "binance", "MUUSDT");
        let mut orders = state.ws_hub().subscribe(realtime::channels::ORDERS);

        let Json(compensating) = close_run_compensation_order(
            State(state.clone()),
            HeaderMap::new(),
            Path(close_run.id.clone()),
            Json(payload),
        )
        .await
        .unwrap_or_else(|error| panic!("compensation submit failed: {error}"));

        assert_eq!(compensating.status, CloseRunStatus::Compensated);
        let attempt = compensating
            .unwind_plan
            .as_ref()
            .and_then(|plan| plan.compensation_attempts.first())
            .unwrap_or_else(|| panic!("compensation attempt missing"));
        assert_eq!(attempt.status, CloseLegStatus::Filled);
        assert_eq!(attempt.finality_source, Some(OrderUpdateSource::AdapterAck));
        let order = attempt
            .order
            .as_ref()
            .unwrap_or_else(|| panic!("compensation order missing"));
        assert_eq!(order.intent.source, OrderSource::CloseRunCompensation);
        assert!(!order.intent.reduce_only);
        assert_eq!(order.intent.order_type, OrderType::Limit);
        assert_eq!(order.intent.time_in_force, TimeInForce::Ioc);
        assert_order_event(&mut orders, "compensation order did not publish");
        let compensation_runs = action_runs::recent(&state)
            .into_iter()
            .filter(|run| run.kind == ActionRunKind::PortfolioCloseCompensation)
            .collect::<Vec<_>>();
        assert_eq!(compensation_runs.len(), 1);
        assert_eq!(compensation_runs[0].status, ActionRunStatus::Succeeded);
        let replayed = action_runs::replay_payload::<CloseRun>(&compensation_runs[0])
            .unwrap_or_else(|error| panic!("compensation payload missing: {error}"));
        assert_eq!(replayed.status, CloseRunStatus::Compensated);
    }

    #[tokio::test]
    async fn close_run_manual_terminal_records_action_payload_and_publishes_update() {
        let state = test_state().await;
        let long =
            submit_open_order(&state, "open-long", "binance", "MUUSDT", OrderSide::Buy).await;
        let short = submit_open_order(&state, "open-short", "okx", "MUUSDT", OrderSide::Sell).await;
        seed_pair_execution_run(&state, &long, &short);
        let rows = position_rows(&state).await;
        let close_payload =
            close_payload(&state, &rows, Some(PositionSide::Long), 2, "positions.close_pair");
        let close_run = close_pair_response(&state, "binance", "MUUSDT", close_payload).await;
        publish_close_leg_state(&state, &close_run, 0, LiveOrderState::Filled, true);
        publish_close_leg_state(&state, &close_run, 1, LiveOrderState::Cancelled, false);
        let close_run = stored_close_run(&state, &close_run.id);
        seed_orderbook(&state, "binance", "MUUSDT");
        let Json(compensating) = close_run_compensation_order(
            State(state.clone()),
            HeaderMap::new(),
            Path(close_run.id.clone()),
            Json(compensation_payload(&close_run)),
        )
        .await
        .unwrap_or_else(|error| panic!("compensation submit failed: {error}"));
        let mut compensation_order = compensating
            .unwind_plan
            .as_ref()
            .and_then(|plan| plan.compensation_attempts.first())
            .and_then(|attempt| attempt.order.clone())
            .unwrap_or_else(|| panic!("compensation order missing"));
        compensation_order.state = LiveOrderState::Cancelled;
        compensation_order.updated_at_ms = compensating.updated_at_ms + 1;
        publish_order_event(
            &state,
            "close_run_compensation_cancelled",
            &compensation_order,
        )
        .unwrap_or_else(|error| panic!("publish compensation cancel failed: {error}"));
        let failed = stored_close_run(&state, &close_run.id);
        assert_eq!(failed.status, CloseRunStatus::CompensationFailed);
        let mut portfolio_events = state.ws_hub().subscribe(realtime::channels::PORTFOLIO);

        let Json(resolved) = close_run_manual_terminal(
            State(state.clone()),
            HeaderMap::new(),
            Path(failed.id.clone()),
            Json(CloseRunManualTerminalRequest {
                confirmation_phrase: CLOSE_RUN_MANUAL_TERMINAL_CONFIRMATION_PHRASE.to_owned(),
                snapshot_version: Some(failed.snapshot_version.clone()),
                reason: "operator confirmed account is flat".to_owned(),
                manual_handling_cost_usd: Some(12.5),
                evidence: vec!["ticket-42".to_owned()],
            }),
        )
        .await
        .unwrap_or_else(|error| panic!("manual terminal failed: {error}"));

        assert_eq!(resolved.status, CloseRunStatus::ManuallyResolved);
        let action_run_id = resolved
            .unwind_plan
            .as_ref()
            .and_then(|plan| plan.manual_terminal_evidence.as_ref())
            .and_then(|evidence| evidence.action_run_id.clone())
            .unwrap_or_else(|| panic!("manual action run id missing"));
        let action_run = action_runs::get(&state, &action_run_id)
            .unwrap_or_else(|| panic!("manual terminal action run missing"));
        assert_eq!(action_run.status, ActionRunStatus::Succeeded);
        let replayed = action_runs::replay_payload::<CloseRun>(&action_run)
            .unwrap_or_else(|error| panic!("manual terminal payload missing: {error}"));
        assert_eq!(replayed.status, CloseRunStatus::ManuallyResolved);
        let manual_evidence = replayed
            .unwind_plan
            .as_ref()
            .and_then(|plan| plan.manual_terminal_evidence.as_ref())
            .unwrap_or_else(|| panic!("manual evidence missing from replayed run"));
        assert_eq!(manual_evidence.manual_handling_cost_usd, Some(12.5));
        assert!(manual_evidence.manual_handling_event_id.is_some());
        assert_eq!(
            replayed
                .cost_reconciliation
                .as_ref()
                .and_then(|cost| cost.manual_handling_usd),
            Some(12.5)
        );
        assert_portfolio_event(
            &mut portfolio_events,
            "manual terminal close-run update did not publish",
        );
    }

    #[tokio::test]
    async fn close_all_replays_confirmation_phrase_failure_as_same_problem() {
        let state = test_state().await;
        let payload = CloseAllPositionsRequest {
            confirmation_phrase: "wrong".to_owned(),
            snapshot_version: Some(portfolio::close_snapshot_version(&state, &[])),
            expected_leg_count: Some(0),
            reason: Some("positions.close_all".to_owned()),
        };

        let first = close_all_positions(
            State(state.clone()),
            HeaderMap::new(),
            Json(payload.clone()),
        )
        .await;
        let first_error = match first {
            Ok(run) => panic!("invalid confirmation phrase unexpectedly closed: {run:?}"),
            Err(error) => error,
        };
        assert_eq!(first_error.status(), StatusCode::BAD_REQUEST);

        let replay =
            close_all_positions(State(state.clone()), HeaderMap::new(), Json(payload)).await;
        let replay_error = match replay {
            Ok(run) => panic!("invalid confirmation phrase unexpectedly replayed success: {run:?}"),
            Err(error) => error,
        };

        assert_eq!(replay_error.status(), StatusCode::BAD_REQUEST);
        assert_eq!(replay_error.code(), "BAD_REQUEST");
        assert_eq!(
            action_runs::recent(&state)
                .into_iter()
                .filter(|run| run.kind == ActionRunKind::PortfolioCloseAll)
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn close_all_replays_immediate_paper_finality_without_resubmit() {
        let state = test_state().await;
        submit_open_order(&state, "open-a", "binance", "MUUSDT", OrderSide::Buy).await;
        submit_open_order(&state, "open-b", "okx", "ETHUSDT", OrderSide::Sell).await;
        let rows = position_rows(&state).await;
        let payload = CloseAllPositionsRequest {
            confirmation_phrase: CLOSE_ALL_POSITIONS_CONFIRMATION_PHRASE.to_owned(),
            snapshot_version: Some(portfolio::close_snapshot_version(&state, &rows)),
            expected_leg_count: Some(2),
            reason: Some("positions.close_all".to_owned()),
        };
        let mut orders = state.ws_hub().subscribe(realtime::channels::ORDERS);

        let first = close_all_response(&state, payload.clone()).await;
        assert_eq!(first.submitted_order_count, 2);
        assert_eq!(first.status, CloseRunStatus::Succeeded);
        assert_order_event(&mut orders, "first close-all leg did not publish");
        assert_order_event(&mut orders, "second close-all leg did not publish");

        let replay =
            close_all_positions(State(state.clone()), HeaderMap::new(), Json(payload)).await;
        let replay = match replay {
            Ok(Json(run)) => run,
            Err(error) => panic!("completed close-all replay failed: {error}"),
        };

        assert_eq!(replay.id, first.id);
        assert_eq!(replay.status, CloseRunStatus::Succeeded);
        assert!(matches!(orders.try_recv(), Err(TryRecvError::Empty)));
        assert_eq!(
            action_runs::recent(&state)
                .into_iter()
                .filter(|run| run.kind == ActionRunKind::PortfolioCloseAll)
                .count(),
            1
        );
    }

    fn publish_close_leg_state(
        state: &AppState,
        close_run: &CloseRun,
        leg_index: usize,
        state_update: LiveOrderState,
        include_fill: bool,
    ) {
        let mut record = close_run.legs[leg_index]
            .order
            .clone()
            .unwrap_or_else(|| panic!("close leg order missing"));
        record.state = state_update;
        record.last_update_source = OrderUpdateSource::PrivateWs;
        record.updated_at_ms = close_run.updated_at_ms + 1 + leg_index as i64;
        if include_fill {
            record.filled_quantity = Some(record.intent.quantity);
            record.filled_price = record.intent.price;
        }
        publish_order_event(state, "position_close_finality", &record)
            .unwrap_or_else(|error| panic!("publish close leg state failed: {error}"));
    }

    fn stored_close_run(state: &AppState, id: &str) -> CloseRun {
        state
            .close_runs()
            .get(id)
            .map(|entry| entry.value().clone())
            .unwrap_or_else(|| panic!("stored close run missing: {id}"))
    }

    fn compensation_payload(close_run: &CloseRun) -> CloseRunCompensationRequest {
        let candidate = close_run
            .unwind_plan
            .as_ref()
            .and_then(|plan| plan.compensation_candidates.first())
            .unwrap_or_else(|| panic!("compensation candidate missing"));
        CloseRunCompensationRequest {
            confirmation_phrase: CLOSE_RUN_COMPENSATION_CONFIRMATION_PHRASE.to_owned(),
            snapshot_version: Some(close_run.snapshot_version.clone()),
            candidate_index: Some(0),
            target_quantity: candidate.confirmed_quantity,
            limit_price: Some(candidate.mark_price),
            reason: Some("test close-run compensation".to_owned()),
        }
    }

    fn seed_orderbook(state: &AppState, exchange: &str, symbol: &str) {
        state.market_data().store_orderbook(
            OrderBookInfo {
                symbol: symbol.to_owned(),
                exchange: exchange.to_owned(),
                bids: vec![[99.9, 1.0]],
                asks: vec![[100.1, 1.0]],
                timestamp: common::time::now_ms(),
            },
            crate::services::market_data::MarketSource::WsPush,
        );
    }

    async fn close_position_response(
        state: &AppState,
        venue: &str,
        symbol: &str,
        payload: ClosePositionRequest,
    ) -> CloseRun {
        match close_position(
            State(state.clone()),
            HeaderMap::new(),
            Path((venue.to_owned(), symbol.to_owned())),
            Json(payload),
        )
        .await
        {
            Ok(Json(run)) => run,
            Err(error) => panic!("close position failed: {error}"),
        }
    }

    async fn close_pair_response(
        state: &AppState,
        venue: &str,
        symbol: &str,
        payload: ClosePositionRequest,
    ) -> CloseRun {
        match close_position_pair(
            State(state.clone()),
            HeaderMap::new(),
            Path((venue.to_owned(), symbol.to_owned())),
            Json(payload),
        )
        .await
        {
            Ok(Json(run)) => run,
            Err(error) => panic!("close pair failed: {error}"),
        }
    }

    async fn close_all_response(state: &AppState, payload: CloseAllPositionsRequest) -> CloseRun {
        match close_all_positions(State(state.clone()), HeaderMap::new(), Json(payload)).await {
            Ok(Json(run)) => run,
            Err(error) => panic!("close all failed: {error}"),
        }
    }

    async fn submit_open_order(
        state: &AppState,
        id: &str,
        exchange: &str,
        symbol: &str,
        side: OrderSide,
    ) -> OrderRecord {
        match state
            .trading_service()
            .submit(open_order(id, exchange, symbol, side))
            .await
        {
            Ok(record) => record,
            Err(error) => panic!("open order failed: {error}"),
        }
    }

    fn open_order(id: &str, exchange: &str, symbol: &str, side: OrderSide) -> OrderIntent {
        OrderIntent {
            id: id.to_owned(),
            source: OrderSource::ArbitragePreview,
            strategy: None,
            mode: ExecutionMode::DryRun,
            exchange: exchange.to_owned(),
            symbol: symbol.to_owned(),
            side,
            order_type: OrderType::Limit,
            quantity: 2.0,
            price: Some(100.0),
            slippage_tolerance_bps: None,
            reduce_only: false,
            time_in_force: TimeInForce::Gtc,
            post_only: false,
            margin_mode: MarginMode::Cross,
            leverage: 5.0,
            client_order_id: format!("client-{id}"),
            client_order_id_policy: None,
            created_at_ms: 1,
        }
    }

    fn seed_pair_execution_run(state: &AppState, long: &OrderRecord, short: &OrderRecord) {
        let run = ExecutionRun {
            run_id: "pair-run-1".to_owned(),
            ticket_id: "ticket-1".to_owned(),
            opportunity_id: "opp-1".to_owned(),
            state: ExecutionRunState::Hedged,
            long_leg: execution_leg(HedgeLegRole::Long, long),
            short_leg: execution_leg(HedgeLegRole::Short, short),
            net_exposure_usd: 0.0,
            cost_reconciliation: None,
            valuation_problem: None,
            unwind_problem: None,
            finality_problem: None,
            finality_checked_at_ms: None,
            evidence: Default::default(),
            recovery_action: None,
            status_reason: "hedged".to_owned(),
            created_at_ms: 1,
            updated_at_ms: 2,
        };
        state.execution_runs().insert(run.run_id.clone(), run);
    }

    fn execution_leg(role: HedgeLegRole, record: &OrderRecord) -> ExecutionRunLeg {
        let filled_quantity = record.filled_quantity.unwrap_or(record.intent.quantity);
        let filled_price = record
            .filled_price
            .or(record.intent.price)
            .unwrap_or_default();
        ExecutionRunLeg {
            role,
            exchange: record.intent.exchange.clone(),
            symbol: record.intent.symbol.clone(),
            order_ids: vec![record.intent.id.clone()],
            identity: Some(record.identity_snapshot()),
            finality_source: Some(record.last_update_source),
            confirmed_filled_at_ms: Some(record.updated_at_ms),
            state: record.state,
            target_quantity: record.intent.quantity,
            filled_quantity: Some(filled_quantity),
            target_notional_usd: record.intent.quantity * filled_price,
            filled_notional_usd: Some(filled_quantity * filled_price),
            filled_fee: record.filled_fee,
        }
    }

    async fn position_rows(state: &AppState) -> Vec<shared_types::PositionRow> {
        match portfolio::positions(state).await {
            Ok(rows) => rows,
            Err(error) => panic!("positions failed: {error}"),
        }
    }

    fn close_payload(
        state: &AppState,
        rows: &[shared_types::PositionRow],
        side: Option<PositionSide>,
        expected_leg_count: usize,
        reason: &str,
    ) -> ClosePositionRequest {
        ClosePositionRequest {
            side,
            snapshot_version: Some(portfolio::close_snapshot_version(state, rows)),
            expected_leg_count: Some(expected_leg_count),
            reason: Some(reason.to_owned()),
        }
    }

    fn assert_order_event(
        orders: &mut tokio::sync::broadcast::Receiver<realtime::WsMessage>,
        message: &str,
    ) {
        match orders.try_recv() {
            Ok(_) => {}
            other => panic!("{message}: {other:?}"),
        }
    }

    fn assert_portfolio_event(
        events: &mut tokio::sync::broadcast::Receiver<realtime::WsMessage>,
        message: &str,
    ) {
        match events.try_recv() {
            Ok(_) => {}
            other => panic!("{message}: {other:?}"),
        }
    }

    async fn test_state() -> AppState {
        let mut config = AppConfig::default();
        config.history.enabled = false;
        config.storage.portfolio_nav_path = None;
        match AppState::new(config).await {
            Ok(state) => state,
            Err(error) => panic!("state init failed: {error}"),
        }
    }
}
