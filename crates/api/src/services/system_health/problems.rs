use super::*;

pub(super) fn market_data_problems(state: &AppState, now_ms: i64) -> Vec<RuntimeProblem> {
    state
        .market_data()
        .runtime_health_snapshot()
        .into_iter()
        .filter(|row| {
            !matches!(
                row.quality,
                MarketQuality::Fresh | MarketQuality::Warming | MarketQuality::Unsupported
            )
        })
        .map(|row| {
            let code = row.quality.problem_code();
            let problem = row
                .problem
                .as_ref()
                .map(|problem| problem.to_api_problem(code));
            RuntimeProblem {
                scope: "market_data".to_owned(),
                operation: row.operation.to_owned(),
                code: code.to_owned(),
                message: market_data_problem_message(&row),
                venue: Some(row.venue),
                retry_after_ms: row.retry_after_ms,
                problem,
                observed_at_ms: row.observed_at_ms.max(now_ms.saturating_sub(120_000)),
            }
        })
        .collect()
}

pub(super) fn market_data_problem_message(
    row: &crate::services::market_data::MarketRuntimeHealth,
) -> String {
    let base = format!(
        "market data {} for {} is {} via {}; requested={}, rows={}",
        row.operation,
        row.venue,
        row.quality.as_str(),
        row.source.as_str(),
        row.requested,
        row.rows
    );
    match &row.last_error {
        Some(error) => format!("{base}: {error}"),
        None => base,
    }
}

/// 把不健康（死亡/卡住/连续失败）的后台任务转成 `RuntimeProblem`，使 `/health` 变 degraded。
pub(super) fn task_issue_problems(
    registry: &crate::task_registry::TaskRegistry,
    now_ms: i64,
) -> Vec<RuntimeProblem> {
    registry
        .unhealthy_tasks(now_ms)
        .into_iter()
        .map(|issue| RuntimeProblem {
            scope: "background_task".to_owned(),
            operation: issue.name.to_owned(),
            code: issue.kind.code().to_owned(),
            message: format!("background task '{}': {}", issue.name, issue.detail),
            venue: None,
            retry_after_ms: None,
            problem: None,
            observed_at_ms: issue.since_ms.unwrap_or(now_ms),
        })
        .collect()
}

pub(super) fn operation_health_problems(
    snapshot: &VenueOperationHealthSnapshot,
    live_operations_required: bool,
) -> Vec<RuntimeProblem> {
    snapshot
        .rows
        .iter()
        .filter(|row| row.status != VenueOperationStatus::Ok)
        .filter(|row| live_operations_required || row.configured != Some(false))
        .filter(|row| {
            is_api_operation_row(row)
                || is_api_transport_row(row)
                || is_private_ws_row(row)
                || is_configured_audit_storage_row(row)
        })
        .map(operation_health_problem)
        .collect()
}

pub(super) fn operation_health_problem(row: &VenueOperationHealth) -> RuntimeProblem {
    match &row.problem {
        Some(problem) => RuntimeProblem {
            scope: operation_health_problem_scope(row).to_owned(),
            operation: row.operation.clone(),
            code: problem.code.clone(),
            message: problem.message.clone(),
            venue: Some(row.venue.clone()),
            retry_after_ms: problem.retry_after_ms.or(row.retry_after_ms),
            problem: Some(problem.clone()),
            observed_at_ms: row.observed_at_ms,
        },
        None => RuntimeProblem {
            scope: operation_health_problem_scope(row).to_owned(),
            operation: row.operation.clone(),
            code: operation_health_problem_code(row.status).to_owned(),
            message: row.message.clone(),
            venue: Some(row.venue.clone()),
            retry_after_ms: row.retry_after_ms,
            problem: None,
            observed_at_ms: row.observed_at_ms,
        },
    }
}

pub(super) fn operation_health_problem_scope(row: &VenueOperationHealth) -> &'static str {
    if is_configured_audit_storage_row(row) {
        "audit_storage"
    } else if is_private_ws_row(row) {
        "private_ws"
    } else if is_api_transport_row(row) {
        "api_transport"
    } else {
        "trading_api"
    }
}

fn is_configured_audit_storage_row(row: &VenueOperationHealth) -> bool {
    row.venue == "system" && row.operation == "storage:audit_log" && row.configured == Some(true)
}

pub(super) fn operation_health_problem_code(status: VenueOperationStatus) -> &'static str {
    match status {
        VenueOperationStatus::Ok => "VENUE_OPERATION_OK",
        VenueOperationStatus::Warn => "VENUE_OPERATION_WARN",
        VenueOperationStatus::Blocked => "VENUE_OPERATION_BLOCKED",
        VenueOperationStatus::Unknown => "VENUE_OPERATION_UNKNOWN",
        VenueOperationStatus::Unsupported => "VENUE_OPERATION_UNSUPPORTED",
    }
}

pub(super) fn missing_runtime_evidence_problem(
    scope: &str,
    operation: &str,
    code: &str,
    message: &str,
    now_ms: i64,
) -> RuntimeProblem {
    RuntimeProblem {
        scope: scope.to_owned(),
        operation: operation.to_owned(),
        code: code.to_owned(),
        message: message.to_owned(),
        venue: None,
        retry_after_ms: None,
        problem: None,
        observed_at_ms: now_ms,
    }
}
