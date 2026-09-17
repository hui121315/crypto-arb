use super::*;

pub(in crate::services::account_state) fn account_equity_unknown_quality(
    balances: &VenueBalanceEnvelope,
    positions: &VenuePositionEnvelope,
    open_orders: &VenueOpenOrdersEnvelope,
    operation_health: &[VenueOperationHealth],
    account_bindings: &[AccountBindingEvidence],
    observed_at_ms: i64,
) -> Vec<AccountFieldQuality> {
    let account_summaries = &balances.account_summaries;
    let summarized_venues = account_summaries
        .iter()
        .filter(|row| {
            crate::services::account_equity_scope::summary_covers_nav(
                row,
                account_bindings,
                account_summaries,
            )
        })
        .map(|row| {
            crate::services::account_equity_scope::equity_coverage_venue(
                &row.venue,
                account_bindings,
                account_summaries,
            )
        })
        .collect::<BTreeSet<_>>();
    account_venues(
        balances,
        positions,
        open_orders,
        operation_health,
        account_summaries,
    )
    .into_iter()
    .map(|venue| {
        crate::services::account_equity_scope::equity_coverage_venue(
            &venue,
            account_bindings,
            account_summaries,
        )
    })
    .collect::<BTreeSet<_>>()
    .into_iter()
    .filter(|venue| !summarized_venues.contains(venue))
    .map(|venue| {
        AccountFieldQuality::new(
            AccountFieldSubject::account(venue.clone()),
            EQUITY_FIELD,
            AccountFieldQualityStatus::Unknown,
            ACCOUNT_STATE_SOURCE,
            Some(observed_at_ms),
        )
        .with_problem(account_equity_unknown_problem(Some(&venue), observed_at_ms))
    })
    .collect()
}

fn account_venues(
    balances: &VenueBalanceEnvelope,
    positions: &VenuePositionEnvelope,
    open_orders: &VenueOpenOrdersEnvelope,
    operation_health: &[VenueOperationHealth],
    account_summaries: &[VenueAccountSummary],
) -> Vec<String> {
    let mut venues = BTreeSet::new();
    for row in &balances.rows {
        insert_venue(&mut venues, &row.venue);
    }
    for row in &positions.rows {
        insert_venue(&mut venues, &row.exchange);
    }
    for row in &open_orders.rows {
        insert_venue(&mut venues, &row.exchange);
    }
    for row in operation_health {
        if row.configured != Some(false) {
            insert_venue(&mut venues, &row.venue);
        }
    }
    for row in account_summaries {
        insert_venue(&mut venues, &row.venue);
    }
    venues.into_iter().collect()
}

pub(in crate::services::account_state) fn account_summary_problems(
    rows: &[VenueAccountSummary],
) -> Vec<ApiProblem> {
    rows.iter().filter_map(|row| row.problem.clone()).collect()
}

pub(in crate::services::account_state) fn account_summary_field_quality(
    rows: &[VenueAccountSummary],
) -> Vec<AccountFieldQuality> {
    rows.iter()
        .flat_map(|row| {
            let status = if row.problem.is_some() {
                AccountFieldQualityStatus::Invalid
            } else {
                AccountFieldQualityStatus::Actual
            };
            let mut quality = [
                "equity",
                "availableBalance",
                "initialMargin",
                "maintenanceMargin",
                "initialMarginRate",
                "maintenanceMarginRate",
                "equityScope",
            ]
            .into_iter()
            .map(move |field| {
                let quality = AccountFieldQuality::new(
                    AccountFieldSubject::account(&row.venue),
                    field,
                    status,
                    &row.source,
                    Some(row.observed_at_ms),
                );
                match row.problem.clone() {
                    Some(problem) => quality.with_problem(problem),
                    None => quality,
                }
            })
            .collect::<Vec<_>>();
            quality.push(withdrawable_field_quality(row));
            quality
        })
        .collect()
}

fn withdrawable_field_quality(row: &VenueAccountSummary) -> AccountFieldQuality {
    let status = if row.withdrawable_balance_usd.is_some() && row.problem.is_none() {
        AccountFieldQualityStatus::Actual
    } else {
        AccountFieldQualityStatus::Missing
    };
    let quality = AccountFieldQuality::new(
        AccountFieldSubject::account(&row.venue),
        "withdrawableBalance",
        status,
        &row.source,
        Some(row.observed_at_ms),
    );
    if status == AccountFieldQualityStatus::Actual {
        quality
    } else {
        quality.with_problem(withdrawable_unknown_problem(row))
    }
}

fn withdrawable_unknown_problem(row: &VenueAccountSummary) -> ApiProblem {
    let mut problem = ApiProblem::new(
        codes::ACCOUNT_FIELD_UNKNOWN,
        "account withdrawable balance is not available from the verified account summary source",
    )
    .with_source(row.source.clone());
    problem.details = Some(serde_json::json!({
        "venue": row.venue.as_str(),
        "field": "withdrawableBalance",
        "accountType": row.account_type.as_str(),
        "equityScope": row.equity_scope,
        "observedAtMs": row.observed_at_ms,
    }));
    problem
}

fn insert_venue(venues: &mut BTreeSet<String>, venue: &str) {
    let venue = normalized_venue_name(venue);
    if !venue.is_empty() {
        venues.insert(venue);
    }
}

pub(in crate::services::account_state) fn account_equity_unknown_problem(
    venue: Option<&str>,
    observed_at_ms: i64,
) -> ApiProblem {
    let mut problem = ApiProblem::new(
        codes::ACCOUNT_FIELD_UNKNOWN,
        "account equity is unknown because no account-level equity source is available",
    )
    .with_source(ACCOUNT_STATE_SOURCE);
    problem.details = Some(serde_json::json!({
        "venue": venue,
        "field": EQUITY_FIELD,
        "observedAtMs": observed_at_ms,
        "source": ACCOUNT_STATE_SOURCE,
    }));
    problem
}
