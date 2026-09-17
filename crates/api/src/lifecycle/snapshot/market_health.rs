use crate::services::{market_data::MarketQuality, opportunity};
use std::collections::BTreeSet;

pub(super) fn attach_market_data_problems(
    market_data: &crate::services::market_data::MarketDataCache,
    meta: &mut shared_types::OpportunityScanMeta,
) {
    if let Some(status) = meta.market_data_status.as_ref() {
        let degraded = status
            .rows
            .iter()
            .filter(|row| opportunity::scan_market_status_is_problem(row))
            .collect::<Vec<_>>();
        meta.market_data_problem_count = degraded.len();
        meta.degraded_venues = degraded
            .iter()
            .map(|row| format!("{}:{}", row.venue, row.operation.as_str()))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        return;
    }
    let rows = market_data.runtime_health_snapshot();
    let mut venues = BTreeSet::new();
    let mut problem_count = 0usize;
    for row in rows {
        if row.quality == MarketQuality::Fresh || !scan_required_runtime_health(&row) {
            continue;
        }
        problem_count += 1;
        venues.insert(row.venue);
    }
    meta.market_data_problem_count = problem_count;
    meta.degraded_venues = venues.into_iter().collect();
}

fn scan_required_runtime_health(row: &crate::services::market_data::MarketRuntimeHealth) -> bool {
    row.venue == crate::services::market_data::cache::MARKET_AGGREGATE_VENUE
        && matches!(
            row.operation,
            crate::services::market_data::cache::MARKET_OP_REST_FUNDING_RATES
                | crate::services::market_data::cache::MARKET_OP_REST_PERP_TICKERS
                | crate::services::market_data::cache::MARKET_OP_REST_SPOT_TICKS
        )
}
