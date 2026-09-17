use super::*;

pub(super) fn route_failure_problem(failure: &RouteFailure) -> ApiProblem {
    let mut problem = failure.error.to_api_problem().with_source(BALANCE_SOURCE);
    problem.details = Some(serde_json::json!({
        "venue": failure.venue.as_str(),
        "operation": failure.operation,
        "path": BALANCE_ROUTE,
        "status": failure.error.status().as_u16(),
        "source": BALANCE_SOURCE,
    }));
    problem
}

pub(super) fn route_failure_problems(route_failures: &[RouteFailure]) -> Vec<ApiProblem> {
    route_failures.iter().map(route_failure_problem).collect()
}

pub(super) fn balance_read_problem(error: &AppError) -> ApiProblem {
    let mut problem = error.to_api_problem().with_source(BALANCE_SOURCE);
    problem.details = Some(serde_json::json!({
        "operation": BALANCE_OPERATION,
        "path": BALANCE_ROUTE,
        "status": error.status().as_u16(),
        "source": BALANCE_SOURCE,
    }));
    problem
}

pub(super) fn missing_balance_evidence_problem() -> ApiProblem {
    let mut problem = ApiProblem::new(
        codes::BALANCE_EVIDENCE_MISSING,
        "balance rows are empty and no fresh balance evidence is available",
    )
    .with_status(StatusCode::OK.as_u16())
    .with_request_id(common::request_id::current())
    .with_source(BALANCE_SOURCE);
    problem.details = Some(serde_json::json!({
        "operation": BALANCE_OPERATION,
        "path": BALANCE_ROUTE,
        "source": BALANCE_SOURCE,
    }));
    problem
}

pub(super) fn balance_status(
    problems: &[ApiProblem],
    operation_health: &[VenueOperationHealth],
    field_quality: &[AccountFieldQuality],
) -> ListStatus {
    if !problems.is_empty()
        || operation_health.iter().any(balance_attention_row)
        || field_quality.iter().any(balance_field_needs_attention)
    {
        ListStatus::Degraded
    } else {
        ListStatus::Fresh
    }
}

pub(super) fn balance_attention_row(row: &VenueOperationHealth) -> bool {
    row.configured != Some(false)
        && matches!(
            row.status,
            VenueOperationStatus::Warn
                | VenueOperationStatus::Blocked
                | VenueOperationStatus::Unknown
        )
}

pub(super) fn has_fresh_balance_evidence(operation_health: &[VenueOperationHealth]) -> bool {
    operation_health.iter().any(|row| {
        row.status == VenueOperationStatus::Ok
            && (row.operation == ACCOUNT_CACHE_OPERATION
                || row.operation == CREDENTIAL_BALANCE_PROBE)
    })
}

pub(super) fn balance_field_needs_attention(row: &AccountFieldQuality) -> bool {
    row.status != AccountFieldQualityStatus::Actual
}
