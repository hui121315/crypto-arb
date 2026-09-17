use super::*;

pub(super) fn build_envelope(
    observed_at_ms: i64,
    rows: Vec<PositionInfo>,
    operation_health: Vec<VenueOperationHealth>,
    evidence: (&[RouteFailure], &[shared_types::VenueAccountSummary]),
    mark_field_quality: Vec<AccountFieldQuality>,
    mut problems: Vec<ApiProblem>,
) -> VenuePositionEnvelope {
    let (route_failures, account_summaries) = evidence;
    let route_failure_problems = route_failure_problems(route_failures);
    problems.extend(
        route_failures
            .iter()
            .zip(&route_failure_problems)
            .filter(|(failure, _)| !usable_cache_covers_failure(failure, &operation_health))
            .map(|(_, problem)| problem.clone()),
    );
    if rows.is_empty() && !has_fresh_position_evidence(&operation_health) {
        problems.push(missing_position_evidence_problem());
    }
    let mut field_quality = position_field_quality(&rows, observed_at_ms);
    field_quality.extend(mark_field_quality);
    let row_health = position_row_health(
        &rows,
        &operation_health,
        route_failures,
        &route_failure_problems,
        observed_at_ms,
    );
    let status = position_status(&problems, &operation_health, &field_quality);
    let account_bindings = account_binding::evidence_for_venues_with_summaries(
        rows.iter()
            .map(|row| row.exchange.clone())
            .chain(
                operation_health
                    .iter()
                    .filter(|row| row.configured != Some(false))
                    .map(|row| row.venue.clone()),
            )
            .chain(route_failures.iter().map(|row| row.venue.clone())),
        account_summaries,
        observed_at_ms,
    );
    VenuePositionEnvelope::new(
        rows,
        status,
        POSITION_SOURCE,
        observed_at_ms,
        problems,
        operation_health,
    )
    .with_field_quality(field_quality)
    .with_row_health(row_health)
    .with_account_bindings(account_bindings)
}
