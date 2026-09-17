use crate::services::venue_operation_health::history_storage_row;
use crate::state::AppState;
use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use common::AppError;
use realtime::{
    FundingDiffQuery, FundingDiffRow, FundingDiffStatsQuery, FundingDiffStatsRow, FundingQuery,
    FundingRow, HistoryStoreHealth, IndexCompositionHistoryRow, IndexCompositionQuery,
    OpportunityQuery, OpportunityRow,
};
use serde::Deserialize;
use shared_types::problem::codes;
use shared_types::{ApiProblem, HistoryPage, HistoryResponse, StorageDegradedReason};

const HISTORY_DEFAULT_LIMIT: usize = 500;
const HISTORY_MAX_LIMIT: usize = 1_000;
const HISTORY_CURSOR_PREFIX: &str = "v1:";
const HISTORY_SOURCE: &str = "history";
const HOUR_MS: i64 = 3_600_000;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/history/funding", get(funding))
        .route("/api/history/funding-diffs", get(funding_diffs))
        .route("/api/history/funding-diff-stats", get(funding_diff_stats))
        .route("/api/history/opportunities", get(opportunities))
        .route("/api/history/index-compositions", get(index_compositions))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FundingParams {
    #[serde(default)]
    symbol: Option<String>,
    #[serde(default)]
    exchange: Option<String>,
    #[serde(default)]
    from_ms: Option<i64>,
    #[serde(default)]
    to_ms: Option<i64>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FundingDiffParams {
    #[serde(default)]
    symbol: Option<String>,
    #[serde(default)]
    long_exchange: Option<String>,
    #[serde(default)]
    short_exchange: Option<String>,
    #[serde(default)]
    from_ms: Option<i64>,
    #[serde(default)]
    to_ms: Option<i64>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpportunityParams {
    #[serde(default)]
    symbol: Option<String>,
    #[serde(default)]
    min_yield: Option<f64>,
    #[serde(default)]
    from_ms: Option<i64>,
    #[serde(default)]
    to_ms: Option<i64>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IndexCompositionParams {
    #[serde(default)]
    venue: Option<String>,
    #[serde(default)]
    symbol: Option<String>,
    #[serde(default)]
    from_ms: Option<i64>,
    #[serde(default)]
    to_ms: Option<i64>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    cursor: Option<String>,
}

#[derive(Debug)]
struct HistoryWindow {
    limit: usize,
    query_limit: usize,
    max_limit: usize,
    to_ms: Option<i64>,
    problems: Vec<ApiProblem>,
}

trait HistoryCursorTime {
    fn cursor_time_ms(&self) -> i64;
}

impl HistoryCursorTime for FundingRow {
    fn cursor_time_ms(&self) -> i64 {
        self.occurred_at_ms
    }
}

impl HistoryCursorTime for FundingDiffRow {
    fn cursor_time_ms(&self) -> i64 {
        self.occurred_at_ms
    }
}

impl HistoryCursorTime for FundingDiffStatsRow {
    fn cursor_time_ms(&self) -> i64 {
        self.latest_at_ms
    }
}

impl HistoryCursorTime for OpportunityRow {
    fn cursor_time_ms(&self) -> i64 {
        self.occurred_at_ms
    }
}

impl HistoryCursorTime for IndexCompositionHistoryRow {
    fn cursor_time_ms(&self) -> i64 {
        self.occurred_at_ms
    }
}

async fn funding(
    State(state): State<AppState>,
    Query(params): Query<FundingParams>,
) -> Result<Json<HistoryResponse<FundingRow>>, AppError> {
    let window = history_window(params.limit, params.cursor.as_deref(), params.to_ms);
    let rows = state
        .history_store()
        .query_funding(FundingQuery {
            symbol: params.symbol,
            exchange: params.exchange,
            from_ms: params.from_ms,
            to_ms: window.to_ms,
            limit: window.query_limit,
        })
        .await?;
    let (rows, page, latest_at_ms) = page_history_rows(rows, &window);
    let health = state
        .history_store()
        .health_snapshot(common::time::now_ms());
    let problems = history_data_problems(
        window.problems,
        funding_history_stale_problem(&rows, latest_at_ms, health.observed_at_ms),
    );
    Ok(Json(history_response(
        rows,
        page,
        &health,
        latest_at_ms,
        problems,
    )))
}

async fn funding_diffs(
    State(state): State<AppState>,
    Query(params): Query<FundingDiffParams>,
) -> Result<Json<HistoryResponse<FundingDiffRow>>, AppError> {
    let window = history_window(params.limit, params.cursor.as_deref(), params.to_ms);
    let rows = state
        .history_store()
        .query_funding_diffs(FundingDiffQuery {
            symbol: params.symbol,
            long_exchange: params.long_exchange,
            short_exchange: params.short_exchange,
            from_ms: params.from_ms,
            to_ms: window.to_ms,
            limit: window.query_limit,
        })
        .await?;
    let (rows, page, latest_at_ms) = page_history_rows(rows, &window);
    let health = state
        .history_store()
        .health_snapshot(common::time::now_ms());
    let problems = history_data_problems(
        window.problems,
        funding_diff_history_stale_problem(&rows, latest_at_ms, health.observed_at_ms),
    );
    Ok(Json(history_response(
        rows,
        page,
        &health,
        latest_at_ms,
        problems,
    )))
}

async fn funding_diff_stats(
    State(state): State<AppState>,
    Query(params): Query<FundingDiffParams>,
) -> Result<Json<HistoryResponse<FundingDiffStatsRow>>, AppError> {
    let window = history_window(params.limit, params.cursor.as_deref(), params.to_ms);
    let rows = state
        .history_store()
        .query_funding_diff_stats(FundingDiffStatsQuery {
            symbol: params.symbol,
            long_exchange: params.long_exchange,
            short_exchange: params.short_exchange,
            from_ms: params.from_ms,
            to_ms: window.to_ms,
            limit: window.query_limit,
        })
        .await?;
    let (rows, page, latest_at_ms) = page_history_rows(rows, &window);
    let health = state
        .history_store()
        .health_snapshot(common::time::now_ms());
    let problems = history_data_problems(
        window.problems,
        funding_diff_stats_history_stale_problem(&rows, latest_at_ms, health.observed_at_ms),
    );
    Ok(Json(history_response(
        rows,
        page,
        &health,
        latest_at_ms,
        problems,
    )))
}

async fn opportunities(
    State(state): State<AppState>,
    Query(params): Query<OpportunityParams>,
) -> Result<Json<HistoryResponse<OpportunityRow>>, AppError> {
    let window = history_window(params.limit, params.cursor.as_deref(), params.to_ms);
    let rows = state
        .history_store()
        .query_opportunities(OpportunityQuery {
            symbol: params.symbol,
            min_yield: params.min_yield,
            from_ms: params.from_ms,
            to_ms: window.to_ms,
            limit: window.query_limit,
        })
        .await?;
    let (rows, page, latest_at_ms) = page_history_rows(rows, &window);
    let health = state
        .history_store()
        .health_snapshot(common::time::now_ms());
    Ok(Json(history_response(
        rows,
        page,
        &health,
        latest_at_ms,
        window.problems,
    )))
}

async fn index_compositions(
    State(state): State<AppState>,
    Query(params): Query<IndexCompositionParams>,
) -> Result<Json<HistoryResponse<IndexCompositionHistoryRow>>, AppError> {
    let window = history_window(params.limit, params.cursor.as_deref(), params.to_ms);
    let rows = state
        .history_store()
        .query_index_compositions(IndexCompositionQuery {
            venue: params.venue,
            symbol: params.symbol,
            from_ms: params.from_ms,
            to_ms: window.to_ms,
            limit: window.query_limit,
        })
        .await?;
    let (rows, page, latest_at_ms) = page_history_rows(rows, &window);
    let health = state
        .history_store()
        .health_snapshot(common::time::now_ms());
    Ok(Json(history_response(
        rows,
        page,
        &health,
        latest_at_ms,
        window.problems,
    )))
}

fn history_response<T>(
    rows: Vec<T>,
    page: HistoryPage,
    health: &HistoryStoreHealth,
    latest_at_ms: Option<i64>,
    query_problems: Vec<ApiProblem>,
) -> HistoryResponse<T> {
    let observed_at_ms = health.observed_at_ms;
    let storage_problem = history_problem(health);
    let mut problems =
        Vec::with_capacity(query_problems.len() + usize::from(storage_problem.is_some()));
    if let Some(problem) = storage_problem {
        problems.push(problem);
    }
    problems.extend(query_problems);
    let problem = problems.first().cloned();
    let mut backend_status = health.backend_status();
    if problems
        .iter()
        .any(|problem| problem.code == codes::HISTORY_DATA_STALE)
    {
        backend_status
            .storage_contract
            .degraded_reasons
            .push(StorageDegradedReason::Stale);
    }
    HistoryResponse {
        count: rows.len(),
        row_cap: Some(page.row_cap(format!("history:{}", health.backend))),
        rows,
        page: Some(page),
        backend_status,
        storage_health: Some(history_storage_row(health, observed_at_ms)),
        source: health.backend.to_owned(),
        observed_at_ms,
        latest_at_ms,
        freshness_ms: latest_at_ms.map(|latest| observed_at_ms.saturating_sub(latest)),
        problem,
        retry_after_ms: None,
        problems,
    }
}

fn history_window(
    requested_limit: Option<usize>,
    cursor: Option<&str>,
    requested_to_ms: Option<i64>,
) -> HistoryWindow {
    let mut problems = Vec::new();
    let limit = history_limit(requested_limit, &mut problems);
    let cursor_to_ms = history_cursor_to_ms(cursor, &mut problems);
    let to_ms = match (requested_to_ms, cursor_to_ms) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    };
    HistoryWindow {
        limit,
        query_limit: limit.saturating_add(1),
        max_limit: HISTORY_MAX_LIMIT,
        to_ms,
        problems,
    }
}

fn history_limit(requested: Option<usize>, problems: &mut Vec<ApiProblem>) -> usize {
    match requested {
        None => HISTORY_DEFAULT_LIMIT,
        Some(0) => {
            problems.push(history_query_problem(
                codes::LIST_LIMIT_CLAMPED,
                "history limit was raised to minimum",
                serde_json::json!({ "requested": 0, "applied": 1 }),
            ));
            1
        }
        Some(limit) if limit > HISTORY_MAX_LIMIT => {
            problems.push(history_query_problem(
                codes::LIST_LIMIT_CLAMPED,
                "history limit was clamped to maximum",
                serde_json::json!({
                    "requested": limit,
                    "applied": HISTORY_MAX_LIMIT,
                    "maxLimit": HISTORY_MAX_LIMIT,
                }),
            ));
            HISTORY_MAX_LIMIT
        }
        Some(limit) => limit,
    }
}

fn history_cursor_to_ms(cursor: Option<&str>, problems: &mut Vec<ApiProblem>) -> Option<i64> {
    let cursor = cursor.map(str::trim).filter(|value| !value.is_empty())?;
    let Some(raw) = cursor.strip_prefix(HISTORY_CURSOR_PREFIX) else {
        problems.push(history_query_problem(
            codes::LIST_CURSOR_INVALID,
            "history cursor was invalid",
            serde_json::json!({ "cursor": cursor }),
        ));
        return None;
    };
    if let Ok(value) = raw.parse::<i64>() {
        Some(value)
    } else {
        problems.push(history_query_problem(
            codes::LIST_CURSOR_INVALID,
            "history cursor was invalid",
            serde_json::json!({ "cursor": cursor }),
        ));
        None
    }
}

fn page_history_rows<T: HistoryCursorTime>(
    mut rows: Vec<T>,
    window: &HistoryWindow,
) -> (Vec<T>, HistoryPage, Option<i64>) {
    let has_more = rows.len() > window.limit;
    if has_more {
        rows.truncate(window.limit);
    }
    let latest_at_ms = rows.iter().map(HistoryCursorTime::cursor_time_ms).max();
    let next_cursor = has_more
        .then(|| rows.last().map(next_history_cursor))
        .flatten();
    let page = HistoryPage {
        limit: window.limit,
        max_limit: window.max_limit,
        returned_count: rows.len(),
        has_more,
        next_cursor,
    };
    (rows, page, latest_at_ms)
}

fn next_history_cursor<T: HistoryCursorTime>(row: &T) -> String {
    let next_to_ms = row.cursor_time_ms().saturating_sub(1);
    format!("{HISTORY_CURSOR_PREFIX}{next_to_ms}")
}

fn history_query_problem(
    code: &'static str,
    message: &'static str,
    details: serde_json::Value,
) -> ApiProblem {
    let mut problem = ApiProblem::new(code, message).with_source(HISTORY_SOURCE);
    problem.details = Some(details);
    problem
}

fn history_data_problems(
    mut problems: Vec<ApiProblem>,
    data_problem: Option<ApiProblem>,
) -> Vec<ApiProblem> {
    if let Some(problem) = data_problem {
        problems.push(problem);
    }
    problems
}

fn funding_history_stale_problem(
    rows: &[FundingRow],
    latest_at_ms: Option<i64>,
    observed_at_ms: i64,
) -> Option<ApiProblem> {
    let latest = latest_funding_row(rows, latest_at_ms)?;
    history_data_stale_problem(
        "funding",
        latest.occurred_at_ms,
        observed_at_ms,
        interval_stale_after_ms(latest.interval_hours),
        &serde_json::json!({
            "exchange": latest.exchange.as_str(),
            "symbol": latest.symbol.as_str(),
            "intervalHours": latest.interval_hours,
        }),
    )
}

fn funding_diff_history_stale_problem(
    rows: &[FundingDiffRow],
    latest_at_ms: Option<i64>,
    observed_at_ms: i64,
) -> Option<ApiProblem> {
    let latest = latest_funding_diff_row(rows, latest_at_ms)?;
    let interval_hours = latest.long_interval_hours.max(latest.short_interval_hours);
    history_data_stale_problem(
        "funding_diff",
        latest.occurred_at_ms,
        observed_at_ms,
        interval_stale_after_ms(interval_hours),
        &serde_json::json!({
            "symbol": latest.symbol.as_str(),
            "longExchange": latest.long_exchange.as_str(),
            "shortExchange": latest.short_exchange.as_str(),
            "intervalHours": interval_hours,
        }),
    )
}

fn funding_diff_stats_history_stale_problem(
    rows: &[FundingDiffStatsRow],
    latest_at_ms: Option<i64>,
    observed_at_ms: i64,
) -> Option<ApiProblem> {
    let latest = latest_funding_diff_stats_row(rows, latest_at_ms)?;
    history_data_stale_problem(
        "funding_diff_stats",
        latest.latest_at_ms,
        observed_at_ms,
        interval_stale_after_ms(latest.base_interval_hours),
        &serde_json::json!({
            "symbol": latest.symbol.as_str(),
            "longExchange": latest.long_exchange.as_str(),
            "shortExchange": latest.short_exchange.as_str(),
            "intervalHours": latest.base_interval_hours,
        }),
    )
}

fn history_data_stale_problem(
    kind: &'static str,
    latest_at_ms: i64,
    observed_at_ms: i64,
    stale_after_ms: i64,
    row_details: &serde_json::Value,
) -> Option<ApiProblem> {
    let freshness_ms = observed_at_ms.saturating_sub(latest_at_ms).max(0);
    if freshness_ms <= stale_after_ms {
        return None;
    }
    let mut problem = ApiProblem::new(
        codes::HISTORY_DATA_STALE,
        format!("{kind} history latest sample stale: freshness_ms={freshness_ms}"),
    )
    .with_source(HISTORY_SOURCE);
    problem.details = Some(serde_json::json!({
        "kind": kind,
        "latestAtMs": latest_at_ms,
        "observedAtMs": observed_at_ms,
        "freshnessMs": freshness_ms,
        "staleAfterMs": stale_after_ms,
        "row": row_details,
    }));
    Some(problem)
}

fn latest_funding_row(rows: &[FundingRow], latest_at_ms: Option<i64>) -> Option<&FundingRow> {
    latest_at_ms.and_then(|latest_at_ms| {
        rows.iter()
            .find(|row| row.occurred_at_ms == latest_at_ms)
            .or_else(|| rows.iter().max_by_key(|row| row.occurred_at_ms))
    })
}

fn latest_funding_diff_row(
    rows: &[FundingDiffRow],
    latest_at_ms: Option<i64>,
) -> Option<&FundingDiffRow> {
    latest_at_ms.and_then(|latest_at_ms| {
        rows.iter()
            .find(|row| row.occurred_at_ms == latest_at_ms)
            .or_else(|| rows.iter().max_by_key(|row| row.occurred_at_ms))
    })
}

fn latest_funding_diff_stats_row(
    rows: &[FundingDiffStatsRow],
    latest_at_ms: Option<i64>,
) -> Option<&FundingDiffStatsRow> {
    latest_at_ms.and_then(|latest_at_ms| {
        rows.iter()
            .find(|row| row.latest_at_ms == latest_at_ms)
            .or_else(|| rows.iter().max_by_key(|row| row.latest_at_ms))
    })
}

fn interval_stale_after_ms(interval_hours: u32) -> i64 {
    i64::from(interval_hours.max(1)).saturating_mul(HOUR_MS)
}

fn history_problem(health: &HistoryStoreHealth) -> Option<ApiProblem> {
    if !health.enabled {
        return Some(history_problem_with_message(
            codes::HISTORY_STORE_UNAVAILABLE,
            "history store disabled",
            health,
        ));
    }
    if let Some(error) = unrecovered_last_error(health) {
        return Some(history_problem_with_message(
            history_problem_code(health, error),
            format!("history store last error: {error}"),
            health,
        ));
    }
    health
        .startup_problem
        .as_deref()
        .map(|problem| {
            history_problem_with_message(
                codes::HISTORY_STORE_UNAVAILABLE,
                format!("history store running on fallback backend: {problem}"),
                health,
            )
        })
        .or_else(|| {
            history_timescale_problem(health).map(|problem| {
                history_problem_with_message(
                    codes::HISTORY_STORE_DEGRADED,
                    format!("history store timescale setup degraded: {problem}"),
                    health,
                )
            })
        })
}

fn unrecovered_last_error(health: &HistoryStoreHealth) -> Option<&str> {
    let error_at_ms = health.last_error_at_ms?;
    let recovered = health
        .last_success_at_ms
        .is_some_and(|success_at_ms| success_at_ms >= error_at_ms);
    (!recovered)
        .then_some(health.last_error.as_deref())
        .flatten()
}

fn history_problem_with_message(
    code: &'static str,
    message: impl Into<String>,
    health: &HistoryStoreHealth,
) -> ApiProblem {
    let mut problem = ApiProblem::new(code, message).with_source("history");
    problem.details = Some(serde_json::json!({
        "backend": health.backend,
        "storageContract": &health.storage_contract,
        "enabled": health.enabled,
        "durable": health.durable,
        "fallback": health.fallback,
        "ephemeral": health.ephemeral,
        "schemaVersion": health.schema_version,
        "migrationChecksum": health.migration_checksum.as_deref(),
        "migrationStatus": health.migration_status.as_ref(),
        "startupProblem": health.startup_problem.as_deref(),
        "timescaleStatus": health.timescale_status,
        "timescaleProblem": health.timescale_problem.as_deref(),
        "appendSuccessTotal": health.append_success_total,
        "appendErrorTotal": health.append_error_total,
        "querySuccessTotal": health.query_success_total,
        "queryErrorTotal": health.query_error_total,
        "lastSuccessAtMs": health.last_success_at_ms,
        "lastAppendAtMs": health.last_append_at_ms,
        "lastQueryAtMs": health.last_query_at_ms,
        "lastErrorAtMs": health.last_error_at_ms,
        "lastError": health.last_error.as_deref(),
        "lastErrorCode": health.last_error_code.as_deref(),
    }));
    problem
}

fn history_timescale_problem(health: &HistoryStoreHealth) -> Option<&str> {
    if matches!(
        health.timescale_status,
        Some(shared_types::HistoryTimescaleStatus::PlainPostgres)
            | Some(shared_types::HistoryTimescaleStatus::Partial)
    ) {
        Some(
            health
                .timescale_problem
                .as_deref()
                .unwrap_or("timescaledb setup degraded"),
        )
    } else {
        None
    }
}

fn history_problem_code(health: &HistoryStoreHealth, _error: &str) -> &'static str {
    if let Some(code) = history_known_problem_code(health.last_error_code.as_deref()) {
        return code;
    }
    codes::HISTORY_STORE_UNAVAILABLE
}

fn history_known_problem_code(code: Option<&str>) -> Option<&'static str> {
    match code {
        Some(codes::HISTORY_STORE_UNAVAILABLE) => Some(codes::HISTORY_STORE_UNAVAILABLE),
        Some(codes::HISTORY_STORE_DEGRADED) => Some(codes::HISTORY_STORE_DEGRADED),
        Some(codes::HISTORY_STORE_RATE_LIMITED) => Some(codes::HISTORY_STORE_RATE_LIMITED),
        Some(codes::HISTORY_QUERY_CANCELED) => Some(codes::HISTORY_QUERY_CANCELED),
        Some(codes::HISTORY_SCHEMA_DRIFT) => Some(codes::HISTORY_SCHEMA_DRIFT),
        Some(codes::HISTORY_ENCODE_FAILED) => Some(codes::HISTORY_ENCODE_FAILED),
        Some(codes::HISTORY_DECODE_FAILED) => Some(codes::HISTORY_DECODE_FAILED),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{StorageBackendKind, StorageRuntimeContract, VenueOperationStatus};

    #[test]
    fn history_response_marks_disabled_backend() {
        let response = history_response::<FundingRow>(
            Vec::new(),
            empty_page(),
            &disabled_health(),
            None,
            Vec::new(),
        );

        assert_eq!(response.source, "disabled");
        assert_eq!(response.backend_status.backend, "disabled");
        assert!(!response.backend_status.enabled);
        assert!(!response.backend_status.ephemeral);
        assert_eq!(
            response.storage_health.as_ref().map(|row| row.status),
            Some(VenueOperationStatus::Blocked)
        );
        assert_eq!(response.count, 0);
        assert_eq!(
            response
                .problem
                .as_ref()
                .map(|problem| problem.code.as_str()),
            Some(codes::HISTORY_STORE_UNAVAILABLE)
        );
        assert!(response
            .problems
            .iter()
            .any(|problem| problem.code == codes::HISTORY_STORE_UNAVAILABLE));
    }

    #[test]
    fn history_response_computes_freshness() {
        let latest_at_ms = common::time::now_ms().saturating_sub(250);
        let response = history_response::<FundingRow>(
            Vec::new(),
            empty_page(),
            &healthy_memory(),
            Some(latest_at_ms),
            Vec::new(),
        );

        assert_eq!(response.source, "memory");
        assert_eq!(response.backend_status.backend, "memory");
        assert!(response.backend_status.enabled);
        assert!(response.backend_status.ephemeral);
        assert_eq!(
            response.storage_health.as_ref().map(|row| row.status),
            Some(VenueOperationStatus::Warn)
        );
        assert!(response
            .storage_health
            .as_ref()
            .is_some_and(|row| row.problem.is_none()));
        assert_eq!(
            response.backend_status.schema_version,
            Some(realtime::HISTORY_SCHEMA_VERSION)
        );
        let checksum = realtime::history_migration_checksum_hex();
        assert_eq!(
            response.backend_status.migration_checksum.as_deref(),
            Some(checksum.as_str())
        );
        assert!(response.backend_status.last_query_at_ms.is_some());
        assert!(response.freshness_ms.is_some_and(|age| age >= 250));
        assert!(response.problem.is_none());
    }

    #[test]
    fn funding_history_stale_problem_enters_response_envelope() {
        let latest_at_ms = HOUR_MS;
        let observed_at_ms = latest_at_ms + 9 * HOUR_MS;
        let rows = vec![funding_row(latest_at_ms)];
        let mut health = healthy_memory();
        health.observed_at_ms = observed_at_ms;
        let problems = history_data_problems(
            Vec::new(),
            funding_history_stale_problem(&rows, Some(latest_at_ms), observed_at_ms),
        );

        let response = history_response(rows, empty_page(), &health, Some(latest_at_ms), problems);

        assert_eq!(
            response
                .problem
                .as_ref()
                .map(|problem| problem.code.as_str()),
            Some(codes::HISTORY_DATA_STALE)
        );
        assert!(response
            .problem
            .as_ref()
            .and_then(|problem| problem.details.as_ref())
            .and_then(|details| details.get("staleAfterMs"))
            .is_some_and(|stale_after| stale_after.as_i64() == Some(8 * HOUR_MS)));
        assert!(response
            .backend_status
            .storage_contract
            .degraded_reasons
            .contains(&StorageDegradedReason::Stale));
    }

    #[test]
    fn funding_history_within_interval_has_no_stale_problem() {
        let latest_at_ms = HOUR_MS;
        let observed_at_ms = latest_at_ms + 4 * HOUR_MS;
        let rows = vec![funding_row(latest_at_ms)];

        let problem = funding_history_stale_problem(&rows, Some(latest_at_ms), observed_at_ms);

        assert!(problem.is_none());
    }

    #[test]
    fn history_response_marks_memory_fallback_startup_problem() {
        let mut health = healthy_memory();
        health.fallback = true;
        health.startup_problem = Some("postgres connect failed".to_owned());

        let response =
            history_response::<FundingRow>(Vec::new(), empty_page(), &health, None, Vec::new());

        assert!(response.backend_status.fallback);
        assert_eq!(
            response.storage_health.as_ref().map(|row| row.status),
            Some(VenueOperationStatus::Warn)
        );
        assert!(response
            .storage_health
            .as_ref()
            .and_then(|row| row.problem.as_ref())
            .is_some());
        assert_eq!(
            response.backend_status.startup_problem.as_deref(),
            Some("postgres connect failed")
        );
        assert_eq!(
            response
                .problem
                .as_ref()
                .map(|problem| problem.code.as_str()),
            Some(codes::HISTORY_STORE_UNAVAILABLE)
        );
        assert!(response
            .problem
            .as_ref()
            .and_then(|problem| problem.details.as_ref())
            .and_then(|details| details.get("startupProblem"))
            .is_some());
        assert!(response
            .problem
            .as_ref()
            .and_then(|problem| problem.details.as_ref())
            .and_then(|details| details.get("migrationChecksum"))
            .is_some());
    }

    #[test]
    fn history_response_marks_unrecovered_last_error() {
        let mut health = healthy_memory();
        health.last_error_at_ms = Some(900);
        health.last_success_at_ms = Some(800);
        health.last_error = Some("history payload decode failed".to_owned());
        health.last_error_code = Some(codes::HISTORY_DECODE_FAILED.to_owned());
        health
            .storage_contract
            .degraded_reasons
            .push(StorageDegradedReason::PayloadInvalid);

        let response =
            history_response::<FundingRow>(Vec::new(), empty_page(), &health, None, Vec::new());

        assert_eq!(
            response
                .problem
                .as_ref()
                .map(|problem| problem.code.as_str()),
            Some(codes::HISTORY_DECODE_FAILED)
        );
    }

    #[test]
    fn history_response_preserves_schema_drift_code() {
        let mut health = healthy_memory();
        health.last_error_at_ms = Some(900);
        health.last_success_at_ms = Some(800);
        health.last_error = Some("history schema drift: relation funding_rates missing".to_owned());
        health.last_error_code = Some(codes::HISTORY_SCHEMA_DRIFT.to_owned());
        health
            .storage_contract
            .degraded_reasons
            .push(StorageDegradedReason::SchemaDrift);

        let response =
            history_response::<FundingRow>(Vec::new(), empty_page(), &health, None, Vec::new());

        assert!(response.problem.as_ref().is_some_and(|problem| {
            problem.code == codes::HISTORY_SCHEMA_DRIFT
                && problem
                    .details
                    .as_ref()
                    .and_then(|details| details.get("lastErrorCode"))
                    .is_some_and(|code| code.as_str() == Some(codes::HISTORY_SCHEMA_DRIFT))
        }));
        assert_eq!(
            response.backend_status.last_error_code.as_deref(),
            Some(codes::HISTORY_SCHEMA_DRIFT)
        );
        assert!(response
            .backend_status
            .storage_contract
            .degraded_reasons
            .contains(&StorageDegradedReason::SchemaDrift));
    }

    #[test]
    fn history_response_preserves_postgres_backpressure_codes() {
        for code in [
            codes::HISTORY_STORE_RATE_LIMITED,
            codes::HISTORY_QUERY_CANCELED,
        ] {
            let mut health = healthy_memory();
            health.last_error_at_ms = Some(900);
            health.last_success_at_ms = Some(800);
            health.last_error = Some("history postgres backpressure".to_owned());
            health.last_error_code = Some(code.to_owned());

            let response =
                history_response::<FundingRow>(Vec::new(), empty_page(), &health, None, Vec::new());

            assert_eq!(
                response
                    .problem
                    .as_ref()
                    .map(|problem| problem.code.as_str()),
                Some(code)
            );
            assert_eq!(
                response.backend_status.last_error_code.as_deref(),
                Some(code)
            );
        }
    }

    #[test]
    fn history_response_ignores_recovered_last_error() {
        let mut health = healthy_memory();
        health.last_error_at_ms = Some(900);
        health.last_success_at_ms = Some(1_000);
        health.last_error = Some("history payload decode failed".to_owned());

        let response =
            history_response::<FundingRow>(Vec::new(), empty_page(), &health, None, Vec::new());

        assert!(response.problem.is_none());
    }

    #[test]
    fn history_window_clamps_limit_and_invalid_cursor_as_problem() {
        let window = history_window(Some(HISTORY_MAX_LIMIT + 1), Some("bad"), None);

        assert_eq!(window.limit, HISTORY_MAX_LIMIT);
        assert_eq!(window.query_limit, HISTORY_MAX_LIMIT + 1);
        assert_eq!(window.to_ms, None);
        assert!(window
            .problems
            .iter()
            .any(|problem| problem.code == codes::LIST_LIMIT_CLAMPED));
        assert!(window
            .problems
            .iter()
            .any(|problem| problem.code == codes::LIST_CURSOR_INVALID));
    }

    #[test]
    fn history_window_applies_cursor_time_bound() {
        let window = history_window(Some(25), Some("v1:900"), Some(1_000));

        assert_eq!(window.limit, 25);
        assert_eq!(window.query_limit, 26);
        assert_eq!(window.to_ms, Some(900));
        assert!(window.problems.is_empty());
    }

    #[test]
    fn page_history_rows_exposes_has_more_and_next_cursor() {
        let rows = vec![funding_row(1_000), funding_row(900), funding_row(800)];
        let window = HistoryWindow {
            limit: 2,
            query_limit: 3,
            max_limit: HISTORY_MAX_LIMIT,
            to_ms: None,
            problems: Vec::new(),
        };

        let (rows, page, latest_at_ms) = page_history_rows(rows, &window);

        assert_eq!(rows.len(), 2);
        assert_eq!(page.returned_count, 2);
        assert!(page.has_more);
        assert_eq!(page.next_cursor.as_deref(), Some("v1:899"));
        assert_eq!(latest_at_ms, Some(1_000));
    }

    #[test]
    fn history_response_carries_row_cap_evidence() {
        let page = HistoryPage {
            limit: 2,
            max_limit: HISTORY_MAX_LIMIT,
            returned_count: 2,
            has_more: true,
            next_cursor: Some("v1:899".into()),
        };

        let response = history_response::<FundingRow>(
            vec![funding_row(1_000), funding_row(900)],
            page,
            &healthy_memory(),
            Some(1_000),
            Vec::new(),
        );

        assert!(response.row_cap.is_some(), "history row cap");
        if let Some(cap) = response.row_cap.as_ref() {
            assert_eq!(cap.max_rows, 2);
            assert_eq!(cap.returned_count, 2);
            assert_eq!(cap.total_rows, 3);
            assert!(cap.total_rows_is_lower_bound);
            assert!(cap.truncated);
        }
    }

    fn empty_page() -> HistoryPage {
        HistoryPage {
            limit: HISTORY_DEFAULT_LIMIT,
            max_limit: HISTORY_MAX_LIMIT,
            returned_count: 0,
            has_more: false,
            next_cursor: None,
        }
    }

    fn funding_row(occurred_at_ms: i64) -> FundingRow {
        FundingRow {
            occurred_at_ms,
            exchange: "binance".into(),
            symbol: "BTC".into(),
            rate: 0.0001,
            interval_hours: 8,
            next_funding_ms: occurred_at_ms + 1,
            volume_24h: 1_000_000.0,
        }
    }

    fn disabled_health() -> HistoryStoreHealth {
        HistoryStoreHealth {
            backend: "disabled",
            storage_contract: StorageRuntimeContract {
                backend_kind: StorageBackendKind::Disabled,
                degraded_reasons: vec![StorageDegradedReason::Disabled],
                migration_authority: None,
            },
            enabled: false,
            durable: false,
            fallback: false,
            ephemeral: false,
            schema_version: None,
            migration_checksum: None,
            migration_status: None,
            startup_problem: None,
            timescale_status: None,
            timescale_problem: None,
            append_success_total: 0,
            append_error_total: 0,
            query_success_total: 0,
            query_error_total: 0,
            last_success_at_ms: None,
            last_append_at_ms: None,
            last_query_at_ms: None,
            last_error_at_ms: None,
            last_error: None,
            last_error_code: None,
            observed_at_ms: common::time::now_ms(),
        }
    }

    fn healthy_memory() -> HistoryStoreHealth {
        HistoryStoreHealth {
            backend: "memory",
            storage_contract: StorageRuntimeContract {
                backend_kind: StorageBackendKind::Memory,
                degraded_reasons: vec![StorageDegradedReason::Ephemeral],
                migration_authority: None,
            },
            enabled: true,
            durable: false,
            fallback: false,
            ephemeral: true,
            schema_version: Some(realtime::HISTORY_SCHEMA_VERSION),
            migration_checksum: Some(realtime::history_migration_checksum_hex()),
            migration_status: None,
            startup_problem: None,
            timescale_status: None,
            timescale_problem: None,
            append_success_total: 0,
            append_error_total: 0,
            query_success_total: 1,
            query_error_total: 0,
            last_success_at_ms: Some(common::time::now_ms()),
            last_append_at_ms: None,
            last_query_at_ms: Some(common::time::now_ms()),
            last_error_at_ms: None,
            last_error: None,
            last_error_code: None,
            observed_at_ms: common::time::now_ms(),
        }
    }
}
