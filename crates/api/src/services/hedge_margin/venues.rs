use super::*;

pub(super) fn required_margin_venues(intents: &[&OrderIntent]) -> Vec<String> {
    let mut venues = Vec::with_capacity(intents.len());
    for intent in intents
        .iter()
        .copied()
        .filter(|intent| requires_live_margin(intent))
    {
        let venue = normalized_venue_name(&intent.exchange);
        if !venue.is_empty() && !venues.contains(&venue) {
            venues.push(venue);
        }
    }
    venues
}

pub(super) fn account_margin_rows(
    balances: &[VenueBalanceInfo],
    intents: &[&OrderIntent],
    summaries: &[VenueAccountSummary],
) -> Vec<VenueBalanceInfo> {
    let mut rows = Vec::new();
    for venue in required_margin_venues(intents) {
        if let Some(summary) = summaries
            .iter()
            .find(|row| margin_balance_venue_matches(&row.venue, &venue) && usable_summary(row))
        {
            rows.push(summary_margin_row(summary));
        } else {
            rows.extend(
                balances
                    .iter()
                    .filter(|row| margin_balance_venue_matches(&row.venue, &venue))
                    .cloned(),
            );
        }
    }
    rows
}

pub(super) fn usable_summary(summary: &VenueAccountSummary) -> bool {
    summary.problem.is_none()
        && summary.equity_scope != AccountEquityScope::Unknown
        && non_negative_finite(summary.total_equity_usd)
        && non_negative_finite(summary.total_available_balance_usd)
        && summary
            .withdrawable_balance_usd
            .is_none_or(non_negative_finite)
}

fn summary_margin_row(summary: &VenueAccountSummary) -> VenueBalanceInfo {
    VenueBalanceInfo {
        venue: summary.venue.clone(),
        currency: "USD".to_owned(),
        total: summary.total_equity_usd,
        available: summary
            .withdrawable_balance_usd
            .unwrap_or(summary.total_available_balance_usd),
        frozen: summary.total_initial_margin_usd,
        unrealized_pnl: 0.0,
    }
}

fn non_negative_finite(value: f64) -> bool {
    value.is_finite() && value >= 0.0
}

pub(super) fn ensure_required_venue_balances(
    balances: &[VenueBalanceInfo],
    intents: &[&OrderIntent],
) -> Result<(), AppError> {
    let missing = missing_margin_venues(balances, intents);
    if missing.is_empty() {
        Ok(())
    } else {
        Err(missing_balance_error(&missing))
    }
}

pub(super) fn missing_margin_venues(
    balances: &[VenueBalanceInfo],
    intents: &[&OrderIntent],
) -> Vec<String> {
    intents
        .iter()
        .copied()
        .filter(|intent| requires_live_margin(intent))
        .filter(|intent| !has_margin_balance_for_intent(balances, intent))
        .map(|intent| normalized_venue_name(&intent.exchange))
        .fold(Vec::new(), push_unique)
}

pub(super) fn has_margin_balance_for_intent(
    balances: &[VenueBalanceInfo],
    intent: &OrderIntent,
) -> bool {
    balances
        .iter()
        .any(|row| margin_balance_venue_matches(&row.venue, &intent.exchange))
}

pub(super) fn margin_balance_venue_matches(row_venue: &str, exchange: &str) -> bool {
    trading::execution::margin_balance_venue_matches(row_venue, exchange)
}

pub(super) fn push_unique(mut values: Vec<String>, value: String) -> Vec<String> {
    if !values.contains(&value) {
        values.push(value);
    }
    values
}

pub(super) fn margin_scope(intents: &[&OrderIntent]) -> HedgePreflightScope {
    HedgePreflightScope {
        venues: required_margin_venues(intents),
        symbols: scoped_symbols(intents),
        account_modes: scoped_account_modes(intents),
        operations: vec![HedgePreflightOperation::MarginBalance],
    }
}

pub(super) fn observed_balance_venues(balances: &[VenueBalanceInfo]) -> Vec<String> {
    balances
        .iter()
        .map(|row| normalized_venue_name(&row.venue))
        .filter(|venue| !venue.is_empty())
        .fold(Vec::new(), push_unique)
}

pub(super) fn scoped_symbols(intents: &[&OrderIntent]) -> Vec<String> {
    intents
        .iter()
        .copied()
        .filter(|intent| requires_live_margin(intent))
        .map(|intent| intent.symbol.clone())
        .fold(Vec::new(), push_unique)
}

pub(super) fn scoped_account_modes(intents: &[&OrderIntent]) -> Vec<String> {
    intents
        .iter()
        .copied()
        .filter(|intent| requires_live_margin(intent))
        .map(|intent| margin_mode_label(intent.margin_mode).to_owned())
        .fold(Vec::new(), push_unique)
}

pub(super) fn margin_mode_label(mode: MarginMode) -> &'static str {
    match mode {
        MarginMode::Cross => "cross",
        MarginMode::Isolated => "isolated",
    }
}
