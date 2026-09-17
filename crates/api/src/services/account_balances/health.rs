use super::*;

pub(super) fn balance_field_quality(
    rows: &[VenueBalanceInfo],
    observed_at_ms: i64,
) -> Vec<AccountFieldQuality> {
    rows.iter()
        .flat_map(|row| {
            [
                balance_field(row, "total", row.total, observed_at_ms),
                balance_field(row, "available", row.available, observed_at_ms),
                balance_field(row, "frozen", row.frozen, observed_at_ms),
                balance_field(row, "unrealizedPnl", row.unrealized_pnl, observed_at_ms),
            ]
        })
        .collect()
}

pub(super) fn balance_row_health(
    rows: &[VenueBalanceInfo],
    operation_health: &[VenueOperationHealth],
    route_failures: &[RouteFailure],
    route_failure_problems: &[ApiProblem],
    observed_at_ms: i64,
) -> Vec<AccountDataHealth> {
    let mut row_health = rows
        .iter()
        .map(|row| {
            balance_data_health(
                row,
                balance_health_for_venue(operation_health, &row.venue),
                observed_at_ms,
            )
        })
        .collect::<Vec<_>>();
    row_health.extend(route_failure_account_health(
        route_failures,
        route_failure_problems,
        observed_at_ms,
    ));
    row_health
}

pub(super) fn route_failure_account_health(
    route_failures: &[RouteFailure],
    route_failure_problems: &[ApiProblem],
    observed_at_ms: i64,
) -> Vec<AccountDataHealth> {
    route_failures
        .iter()
        .zip(route_failure_problems)
        .map(|(failure, problem)| {
            let mut data_health = AccountDataHealth::new(
                AccountFieldSubject::account(&failure.venue),
                problem.source.as_deref().unwrap_or(BALANCE_SOURCE),
                observed_at_ms,
            );
            data_health.last_error = Some(problem.clone());
            data_health.retry_after_ms = problem.retry_after_ms;
            data_health.request_id = problem.request_id.clone();
            data_health
        })
        .collect()
}

pub(super) fn balance_health_for_venue<'a>(
    rows: &'a [VenueOperationHealth],
    venue: &str,
) -> Option<&'a VenueOperationHealth> {
    let normalized = normalized_venue_name(venue);
    rows.iter()
        .filter(|row| normalized_venue_name(&row.venue) == normalized)
        .find(|row| row.operation == ACCOUNT_CACHE_OPERATION)
        .or_else(|| {
            rows.iter()
                .filter(|row| normalized_venue_name(&row.venue) == normalized)
                .find(|row| row.operation == CREDENTIAL_BALANCE_PROBE)
        })
}

pub(super) fn balance_data_health(
    row: &VenueBalanceInfo,
    health: Option<&VenueOperationHealth>,
    observed_at_ms: i64,
) -> AccountDataHealth {
    let mut data_health = AccountDataHealth::new(
        AccountFieldSubject::balance(&row.venue, &row.currency),
        health
            .map(|health| health.source.as_str())
            .unwrap_or(BALANCE_SOURCE),
        observed_at_ms,
    );
    let Some(health) = health else {
        return data_health;
    };
    data_health.observed_at_ms = health.observed_at_ms;
    data_health.freshness_ms = health.freshness_ms;
    data_health.last_success_ms = balance_last_success_ms(health);
    data_health.last_error = balance_last_error(health);
    data_health.retry_after_ms = health.retry_after_ms.or_else(|| {
        data_health
            .last_error
            .as_ref()
            .and_then(|problem| problem.retry_after_ms)
    });
    data_health.request_id = data_health
        .last_error
        .as_ref()
        .and_then(|problem| problem.request_id.clone())
        .or_else(|| {
            health
                .evidence
                .as_ref()
                .and_then(|evidence| evidence.request_id.clone())
        });
    data_health
}

pub(super) fn balance_last_success_ms(row: &VenueOperationHealth) -> Option<i64> {
    (row.status == VenueOperationStatus::Ok)
        .then_some(row.observed_at_ms)
        .map(|observed| observed.saturating_sub(row.freshness_ms.unwrap_or_default()))
}

pub(super) fn balance_last_error(row: &VenueOperationHealth) -> Option<ApiProblem> {
    row.problem.clone().or_else(|| {
        row.error.as_ref().map(|message| {
            let mut problem = ApiProblem::new(codes::BALANCE_READ_DEGRADED, message.clone())
                .with_status(StatusCode::OK.as_u16())
                .with_source(row.source.clone());
            problem.details = Some(serde_json::json!({
                "venue": row.venue.as_str(),
                "operation": row.operation.as_str(),
                "source": row.source.as_str(),
                "observedAtMs": row.observed_at_ms,
            }));
            problem
        })
    })
}

pub(super) fn balance_field(
    row: &VenueBalanceInfo,
    field: &'static str,
    value: f64,
    observed_at_ms: i64,
) -> AccountFieldQuality {
    let status = if value.is_finite() {
        AccountFieldQualityStatus::Actual
    } else {
        AccountFieldQualityStatus::Invalid
    };
    let quality = AccountFieldQuality::new(
        AccountFieldSubject::balance(&row.venue, &row.currency),
        field,
        status,
        BALANCE_SOURCE,
        Some(observed_at_ms),
    );
    if status == AccountFieldQualityStatus::Actual {
        quality
    } else {
        quality.with_problem(balance_field_problem(row, field, status, observed_at_ms))
    }
}

pub(super) fn balance_field_problem(
    row: &VenueBalanceInfo,
    field: &'static str,
    status: AccountFieldQualityStatus,
    observed_at_ms: i64,
) -> ApiProblem {
    let mut problem = ApiProblem::new(
        codes::BALANCE_FIELD_UNAVAILABLE,
        format!("balance field {field} is {status:?}"),
    )
    .with_status(StatusCode::OK.as_u16())
    .with_request_id(common::request_id::current())
    .with_source(BALANCE_SOURCE);
    problem.details = Some(serde_json::json!({
        "venue": row.venue.as_str(),
        "currency": row.currency.as_str(),
        "field": field,
        "status": status,
        "operation": BALANCE_OPERATION,
        "path": BALANCE_ROUTE,
        "source": BALANCE_SOURCE,
        "observedAtMs": observed_at_ms,
    }));
    problem
}
