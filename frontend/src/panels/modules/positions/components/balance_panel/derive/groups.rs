use shared_types::{
    AccountFieldQuality, VenueAccountSummary, VenueAssetValuation, VenueBalanceInfo,
};
use std::collections::{BTreeMap, HashMap};

pub(crate) const DUST_USD_THRESHOLD: f64 = 1.0;

#[derive(Clone, PartialEq)]
pub(crate) struct BalanceDisplayRow {
    pub(crate) balance: VenueBalanceInfo,
    pub(crate) valuation: Option<VenueAssetValuation>,
}

#[derive(Clone, PartialEq)]
pub(crate) struct VenueBalanceGroup {
    pub(crate) venue: String,
    pub(crate) rows: Vec<BalanceDisplayRow>,
    pub(crate) summary: Option<VenueAccountSummary>,
    pub(crate) hidden_dust_count: usize,
    pub(crate) unknown_valuation_count: usize,
}

struct VenueBalanceGroupBuilder {
    venue: String,
    rows: Vec<BalanceDisplayRow>,
    hidden_dust_count: usize,
    unknown_valuation_count: usize,
}

pub(crate) fn balance_groups(
    rows: Vec<VenueBalanceInfo>,
    valuations: Vec<VenueAssetValuation>,
    summaries: Vec<VenueAccountSummary>,
    quality: &[AccountFieldQuality],
) -> Vec<VenueBalanceGroup> {
    let valuation_by_asset = valuations
        .into_iter()
        .filter(|row| row.usd_value.is_finite())
        .map(|row| (asset_key(&row.venue, &row.currency), row))
        .collect::<HashMap<_, _>>();
    let summary_by_venue = summaries
        .into_iter()
        .map(|row| (shared_types::normalized_venue_name(&row.venue), row))
        .collect::<HashMap<_, _>>();
    let mut grouped = BTreeMap::<String, VenueBalanceGroupBuilder>::new();

    for balance in rows {
        let venue_key = shared_types::normalized_venue_name(&balance.venue);
        let valuation = valuation_by_asset
            .get(&asset_key(&balance.venue, &balance.currency))
            .cloned();
        let group = grouped
            .entry(venue_key)
            .or_insert_with(|| VenueBalanceGroupBuilder {
                venue: balance.venue.clone(),
                rows: Vec::new(),
                hidden_dust_count: 0,
                unknown_valuation_count: 0,
            });
        let has_unknown_fields = !super::balance_quality_for_row(&balance, quality).is_empty();
        if !has_unknown_fields
            && (is_zero_balance(&balance)
                || valuation
                    .as_ref()
                    .is_some_and(|row| row.usd_value.abs() < DUST_USD_THRESHOLD))
        {
            group.hidden_dust_count = group.hidden_dust_count.saturating_add(1);
            continue;
        }
        if valuation.is_none() {
            group.unknown_valuation_count = group.unknown_valuation_count.saturating_add(1);
        }
        group.rows.push(BalanceDisplayRow { balance, valuation });
    }

    for (key, summary) in &summary_by_venue {
        grouped
            .entry(key.clone())
            .or_insert_with(|| VenueBalanceGroupBuilder {
                venue: summary.venue.clone(),
                rows: Vec::new(),
                hidden_dust_count: 0,
                unknown_valuation_count: 0,
            });
    }

    grouped
        .into_iter()
        .filter(|(venue_key, group)| {
            !group.rows.is_empty() || summary_by_venue.contains_key(venue_key)
        })
        .map(|(venue_key, mut group)| {
            group.rows.sort_by(balance_display_order);
            VenueBalanceGroup {
                venue: group.venue,
                rows: group.rows,
                summary: summary_by_venue.get(&venue_key).cloned(),
                hidden_dust_count: group.hidden_dust_count,
                unknown_valuation_count: group.unknown_valuation_count,
            }
        })
        .collect()
}

fn is_zero_balance(row: &VenueBalanceInfo) -> bool {
    [row.total, row.available, row.frozen, row.unrealized_pnl]
        .into_iter()
        .all(|value| value.is_finite() && value == 0.0)
}

fn balance_display_order(
    left: &BalanceDisplayRow,
    right: &BalanceDisplayRow,
) -> std::cmp::Ordering {
    match (&left.valuation, &right.valuation) {
        (Some(left), Some(right)) => right
            .usd_value
            .abs()
            .partial_cmp(&left.usd_value.abs())
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(left.currency.cmp(&right.currency)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => left.balance.currency.cmp(&right.balance.currency),
    }
}

fn asset_key(venue: &str, currency: &str) -> (String, String) {
    (
        shared_types::normalized_venue_name(venue),
        currency.trim().to_ascii_uppercase(),
    )
}

pub(crate) fn utilization_pct(row: &VenueBalanceInfo) -> f64 {
    if row.total <= 0.0 {
        0.0
    } else {
        (row.frozen.max(0.0) / row.total * 100.0).clamp(0.0, 100.0)
    }
}
