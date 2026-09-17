use crate::panels::modules::instrument_search::symbol_search_query;

use super::super::data::OpportunityFilter;

pub(super) fn opportunity_table_reset_key(filter: &OpportunityFilter) -> String {
    let strategy = filter
        .strategy
        .map(|kind| format!("{kind:?}"))
        .unwrap_or_else(|| "all".into());
    format!(
        "{}|{:.4}|{}",
        strategy,
        filter.min_net_pct,
        filter.query.trim().to_ascii_lowercase()
    )
}

pub(super) fn opportunity_local_filter_active(filter: &OpportunityFilter) -> bool {
    filter.strategy.is_some() || filter.min_net_pct > 0.0 || !filter.query.trim().is_empty()
}

pub(super) fn opportunity_symbol_search_active(filter: &OpportunityFilter) -> bool {
    symbol_search_query(&filter.query).is_some()
}
