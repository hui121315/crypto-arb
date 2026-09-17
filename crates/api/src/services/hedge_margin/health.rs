use super::*;

pub(super) fn scoped_margin_operation_health(
    venues: &[String],
    rows: &[VenueOperationHealth],
) -> Vec<VenueOperationHealth> {
    rows.iter()
        .filter(|row| {
            margin_balance_operation_row(row)
                && venues
                    .iter()
                    .any(|venue| margin_balance_venue_matches(&row.venue, venue))
        })
        .cloned()
        .collect()
}

pub(super) fn max_scoped_balance_freshness(state: &AppState, venues: &[String]) -> Option<u64> {
    state
        .trading_service()
        .balance_cache_health()
        .into_iter()
        .filter(|row| {
            venues
                .iter()
                .any(|venue| margin_balance_venue_matches(&row.venue, venue))
        })
        .filter_map(|row| u64::try_from(row.freshness_ms).ok())
        .max()
}

pub(super) fn merge_operation_health_evidence(
    evidence: &mut MarginBalanceEvidence,
    venues: &[String],
    rows: &[VenueOperationHealth],
) {
    for row in rows.iter().filter(|row| {
        margin_balance_operation_row(row)
            && venues
                .iter()
                .any(|venue| margin_balance_venue_matches(&row.venue, venue))
    }) {
        evidence.freshness_ms =
            max_optional_u64(evidence.freshness_ms, row.freshness_ms.and_then(i64_to_u64));
        evidence.retry_after_ms =
            max_optional_u64(evidence.retry_after_ms, row_retry_after_ms(row));
        if evidence.request_id.is_none() {
            evidence.request_id = row_request_id(row).map(str::to_owned);
        }
    }
}

pub(super) fn margin_balance_operation_row(row: &VenueOperationHealth) -> bool {
    matches!(
        VenueOperationKind::parse(&row.operation),
        VenueOperationKind::Balance
            | VenueOperationKind::CredentialProbeBalanceRead
            | VenueOperationKind::PrivateRead
            | VenueOperationKind::PrivateWsAccountStream
    )
}

pub(super) fn row_retry_after_ms(row: &VenueOperationHealth) -> Option<u64> {
    row.retry_after_ms.or_else(|| {
        row.problem
            .as_ref()
            .and_then(|problem| problem.retry_after_ms)
    })
}

pub(super) fn row_request_id(row: &VenueOperationHealth) -> Option<&str> {
    row.problem
        .as_ref()
        .and_then(|problem| problem.request_id.as_deref())
        .or_else(|| {
            row.evidence
                .as_ref()
                .and_then(|evidence| evidence.request_id.as_deref())
        })
}

pub(super) fn max_optional_u64(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

pub(super) fn i64_to_u64(value: i64) -> Option<u64> {
    u64::try_from(value).ok()
}

pub(super) fn scoped_margin_account_state(
    balances: &[VenueBalanceInfo],
    collateral_rows: &[VenueBalanceInfo],
    account_summaries: Vec<VenueAccountSummary>,
    account_bindings: Vec<AccountBindingEvidence>,
    outcome: &MarginPreflightOutcome,
) -> AccountStateSnapshot {
    let balance_envelope = VenueBalanceEnvelope::new(
        margin_account_state_rows(balances, collateral_rows),
        margin_account_state_status(&outcome.problems, &outcome.field_quality),
        MARGIN_BALANCE_SOURCE,
        outcome.checked_at_ms,
        outcome.problems.clone(),
        Vec::new(),
    )
    .with_field_quality(outcome.field_quality.clone())
    .with_row_health(outcome.row_health.clone())
    .with_account_summaries(account_summaries)
    .with_account_bindings(account_bindings);
    let position_envelope = VenuePositionEnvelope::new(
        Vec::new(),
        ListStatus::Fresh,
        MARGIN_BALANCE_SOURCE,
        outcome.checked_at_ms,
        Vec::new(),
        Vec::new(),
    );
    account_state::snapshot_from_envelopes(
        balance_envelope,
        position_envelope,
        outcome.checked_at_ms,
    )
}

pub(super) fn margin_account_state_status(
    problems: &[ApiProblem],
    field_quality: &[AccountFieldQuality],
) -> ListStatus {
    if !problems.is_empty()
        || field_quality
            .iter()
            .any(|row| row.status != AccountFieldQualityStatus::Actual)
    {
        ListStatus::Degraded
    } else {
        ListStatus::Fresh
    }
}

pub(super) fn margin_row_health(
    intents: &[&OrderIntent],
    balances: &[VenueBalanceInfo],
    evidence: &MarginBalanceEvidence,
    problems: &[ApiProblem],
    checked_at_ms: i64,
) -> Vec<AccountDataHealth> {
    let mut rows = margin_collateral_row_health(balances, evidence, checked_at_ms);
    if let Some(problem) = problems.iter().find(balance_field_problem) {
        rows.extend(
            missing_margin_venues(balances, intents)
                .into_iter()
                .map(|venue| {
                    missing_margin_account_health(&venue, problem, evidence, checked_at_ms)
                }),
        );
    }
    rows
}

pub(super) fn margin_balance_row_health(
    row: &VenueBalanceInfo,
    health: Option<&VenueOperationHealth>,
    checked_at_ms: i64,
) -> AccountDataHealth {
    let mut data_health = AccountDataHealth::new(
        AccountFieldSubject::balance(&row.venue, &row.currency),
        health
            .map(|row| row.source.as_str())
            .unwrap_or(MARGIN_BALANCE_SOURCE),
        checked_at_ms,
    );
    let Some(health) = health else {
        return data_health;
    };
    data_health.observed_at_ms = health.observed_at_ms;
    data_health.freshness_ms = health.freshness_ms;
    data_health.last_success_ms = operation_last_success_ms(health);
    data_health.last_error = operation_last_error(health);
    data_health.retry_after_ms = row_retry_after_ms(health);
    data_health.request_id = row_request_id(health).map(str::to_owned);
    data_health
}

pub(super) fn missing_margin_account_health(
    venue: &str,
    problem: &ApiProblem,
    evidence: &MarginBalanceEvidence,
    checked_at_ms: i64,
) -> AccountDataHealth {
    let health = margin_balance_health_for_venue(&evidence.operation_health, venue);
    let mut data_health = AccountDataHealth::new(
        AccountFieldSubject::account(venue),
        health
            .map(|row| row.source.as_str())
            .or(problem.source.as_deref())
            .unwrap_or(MARGIN_BALANCE_SOURCE),
        checked_at_ms,
    );
    if let Some(health) = health {
        data_health.observed_at_ms = health.observed_at_ms;
        data_health.freshness_ms = health.freshness_ms;
        data_health.last_success_ms = operation_last_success_ms(health);
    }
    data_health.last_error = health
        .and_then(operation_last_error)
        .or_else(|| Some(problem.clone()));
    data_health.retry_after_ms = health
        .and_then(row_retry_after_ms)
        .or_else(|| {
            data_health
                .last_error
                .as_ref()
                .and_then(|problem| problem.retry_after_ms)
        })
        .or(evidence.retry_after_ms);
    data_health.request_id = data_health
        .last_error
        .as_ref()
        .and_then(|problem| problem.request_id.clone())
        .or_else(|| health.and_then(row_request_id).map(str::to_owned))
        .or_else(|| evidence.request_id.clone());
    data_health
}

pub(super) fn margin_balance_health_for_venue<'a>(
    rows: &'a [VenueOperationHealth],
    venue: &str,
) -> Option<&'a VenueOperationHealth> {
    margin_balance_cache_health_for_venue(rows, venue)
}

pub(super) fn margin_balance_cache_health_for_venue<'a>(
    rows: &'a [VenueOperationHealth],
    venue: &str,
) -> Option<&'a VenueOperationHealth> {
    let normalized = normalized_venue_name(venue);
    rows.iter()
        .filter(|row| margin_balance_cache_row(row))
        .find(|row| normalized_venue_name(&row.venue) == normalized)
        .or_else(|| {
            rows.iter()
                .filter(|row| margin_balance_cache_row(row))
                .find(|row| margin_balance_venue_matches(&row.venue, venue))
        })
}

pub(super) fn margin_balance_cache_row(row: &VenueOperationHealth) -> bool {
    matches!(
        VenueOperationKind::parse(&row.operation),
        VenueOperationKind::Balance
    )
}

pub(super) fn operation_last_success_ms(row: &VenueOperationHealth) -> Option<i64> {
    (row.status == VenueOperationStatus::Ok)
        .then_some(row.observed_at_ms)
        .map(|observed| observed.saturating_sub(row.freshness_ms.unwrap_or_default()))
}

pub(super) fn operation_last_error(row: &VenueOperationHealth) -> Option<ApiProblem> {
    row.problem.clone().or_else(|| {
        row.error.as_ref().map(|message| {
            let mut problem = ApiProblem::new(codes::BALANCE_READ_DEGRADED, message.clone())
                .with_status(StatusCode::OK.as_u16())
                .with_source(row.source.clone());
            problem.details = Some(json!({
                "venue": row.venue.as_str(),
                "operation": row.operation.as_str(),
                "source": row.source.as_str(),
                "observedAtMs": row.observed_at_ms,
            }));
            problem
        })
    })
}
