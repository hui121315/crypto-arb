use super::*;

pub(super) fn scoped_collateral_rows(
    rows: &[VenueBalanceInfo],
    venues: &[String],
) -> Vec<VenueBalanceInfo> {
    rows.iter()
        .filter(|row| {
            venues
                .iter()
                .any(|venue| margin_balance_venue_matches(&row.venue, venue))
        })
        .cloned()
        .collect()
}

pub(super) fn margin_account_state_rows(
    selected: &[VenueBalanceInfo],
    collateral: &[VenueBalanceInfo],
) -> Vec<VenueBalanceInfo> {
    let mut rows = selected.to_vec();
    for row in collateral {
        if !rows.iter().any(|item| same_balance_subject(item, row)) {
            rows.push(row.clone());
        }
    }
    rows
}

pub(super) fn margin_collateral_row_health(
    selected: &[VenueBalanceInfo],
    evidence: &MarginBalanceEvidence,
    checked_at_ms: i64,
) -> Vec<AccountDataHealth> {
    let mut rows = evidence
        .collateral_rows
        .iter()
        .map(|row| collateral_row_health(row, evidence, checked_at_ms))
        .collect::<Vec<_>>();
    rows.extend(
        evidence
            .account_summaries
            .iter()
            .map(account_summary_health),
    );
    for row in selected {
        if !evidence
            .collateral_rows
            .iter()
            .any(|item| same_balance_subject(item, row))
        {
            rows.push(collateral_row_health(row, evidence, checked_at_ms));
        }
    }
    rows
}

pub(super) fn margin_collateral_field_quality(
    intents: &[&OrderIntent],
    balances: &[VenueBalanceInfo],
    evidence: &MarginBalanceEvidence,
    problems: &[ApiProblem],
    observed_at_ms: i64,
) -> Vec<AccountFieldQuality> {
    let mut rows = margin_currency_quality(intents, balances, evidence, problems, observed_at_ms);
    rows.extend(evidence.collateral_rows.iter().map(|row| {
        AccountFieldQuality::new(
            AccountFieldSubject::balance(&row.venue, &row.currency),
            "collateral_currency",
            if row.currency.trim().is_empty() {
                AccountFieldQualityStatus::Invalid
            } else {
                AccountFieldQualityStatus::Actual
            },
            balance_source(evidence, &row.venue),
            Some(observed_at_ms),
        )
    }));
    rows.extend(evidence.account_summaries.iter().map(|summary| {
        let status = if summary.source.trim().is_empty() {
            AccountFieldQualityStatus::Missing
        } else if summary.problem.is_some() {
            AccountFieldQualityStatus::Invalid
        } else {
            AccountFieldQualityStatus::Actual
        };
        let row = AccountFieldQuality::new(
            AccountFieldSubject::account(&summary.venue),
            "account_equity_source",
            status,
            summary.source.clone(),
            Some(summary.observed_at_ms),
        );
        summary
            .problem
            .clone()
            .map_or(row.clone(), |problem| row.with_problem(problem))
    }));
    rows
}

fn margin_currency_quality(
    intents: &[&OrderIntent],
    balances: &[VenueBalanceInfo],
    evidence: &MarginBalanceEvidence,
    problems: &[ApiProblem],
    observed_at_ms: i64,
) -> Vec<AccountFieldQuality> {
    let mut rows = Vec::new();
    for intent in intents
        .iter()
        .copied()
        .filter(|intent| requires_live_margin(intent))
    {
        let venue = normalized_venue_name(&intent.exchange);
        if rows.iter().any(|row: &AccountFieldQuality| {
            row.field == "margin_currency" && row.subject.venue.as_deref() == Some(venue.as_str())
        }) {
            continue;
        }
        let selected = trading::execution::selected_margin_currency_for_intent(balances, intent);
        let (subject, status, source) = selected.map_or_else(
            || {
                (
                    AccountFieldSubject::account(&venue),
                    AccountFieldQualityStatus::Missing,
                    MARGIN_BALANCE_SOURCE.to_owned(),
                )
            },
            |currency| {
                (
                    AccountFieldSubject::balance(&venue, currency),
                    AccountFieldQualityStatus::Actual,
                    margin_currency_source(evidence, &venue, currency),
                )
            },
        );
        let row = AccountFieldQuality::new(
            subject,
            "margin_currency",
            status,
            source,
            Some(observed_at_ms),
        );
        rows.push(if status == AccountFieldQualityStatus::Missing {
            problems
                .first()
                .cloned()
                .map_or(row.clone(), |problem| row.with_problem(problem))
        } else {
            row
        });
    }
    rows
}

fn collateral_row_health(
    row: &VenueBalanceInfo,
    evidence: &MarginBalanceEvidence,
    checked_at_ms: i64,
) -> AccountDataHealth {
    margin_balance_row_health(
        row,
        margin_balance_health_for_venue(&evidence.operation_health, &row.venue),
        checked_at_ms,
    )
}

fn account_summary_health(summary: &VenueAccountSummary) -> AccountDataHealth {
    let mut health = AccountDataHealth::new(
        AccountFieldSubject::account(&summary.venue),
        summary.source.clone(),
        summary.observed_at_ms,
    );
    health.freshness_ms = summary.freshness_ms;
    if let Some(problem) = summary.problem.clone() {
        health.retry_after_ms = problem.retry_after_ms;
        health.request_id.clone_from(&problem.request_id);
        health.last_error = Some(problem);
    } else {
        health.last_success_ms = Some(summary.observed_at_ms);
    }
    health
}

fn margin_currency_source(evidence: &MarginBalanceEvidence, venue: &str, currency: &str) -> String {
    if currency.eq_ignore_ascii_case("USD") {
        if let Some(summary) = evidence
            .account_summaries
            .iter()
            .find(|summary| margin_balance_venue_matches(&summary.venue, venue))
        {
            return summary.source.clone();
        }
    }
    balance_source(evidence, venue)
}

fn balance_source(evidence: &MarginBalanceEvidence, venue: &str) -> String {
    margin_balance_health_for_venue(&evidence.operation_health, venue)
        .map(|row| row.source.clone())
        .unwrap_or_else(|| MARGIN_BALANCE_SOURCE.to_owned())
}

fn same_balance_subject(left: &VenueBalanceInfo, right: &VenueBalanceInfo) -> bool {
    margin_balance_venue_matches(&left.venue, &right.venue)
        && left.currency.eq_ignore_ascii_case(&right.currency)
}
