use crate::panels::modules::opportunity_runtime::merge_opportunity_projections;

use super::FuturesOpportunityRow;

pub(in crate::panels::modules::futures) fn merge_futures_rows(
    base: &[FuturesOpportunityRow],
    extra: &[FuturesOpportunityRow],
) -> Vec<FuturesOpportunityRow> {
    merge_opportunity_projections(
        base,
        extra,
        |left, right| left.id == right.id,
        futures_rank_order,
    )
}

pub(in crate::panels::modules::futures) fn merge_symbol_futures_rows(
    base: &[FuturesOpportunityRow],
    extra: &[FuturesOpportunityRow],
    canonical_symbol: Option<&str>,
) -> Vec<FuturesOpportunityRow> {
    let matching_base = base
        .iter()
        .filter(|row| canonical_symbol.is_some_and(|symbol| row.pair.eq_ignore_ascii_case(symbol)))
        .cloned()
        .collect::<Vec<_>>();
    let matching_extra = extra
        .iter()
        .filter(|row| canonical_symbol.is_some_and(|symbol| row.pair.eq_ignore_ascii_case(symbol)))
        .cloned()
        .collect::<Vec<_>>();
    merge_futures_rows(&matching_base, &matching_extra)
}

fn futures_rank_order(
    left: &FuturesOpportunityRow,
    right: &FuturesOpportunityRow,
) -> std::cmp::Ordering {
    right
        .execution_eligible
        .cmp(&left.execution_eligible)
        .then_with(|| verified_positive_profit(right).cmp(&verified_positive_profit(left)))
        .then_with(|| right.one_cycle_net_bps.total_cmp(&left.one_cycle_net_bps))
        .then_with(|| {
            left.settlement_minutes()
                .unwrap_or(i64::MAX)
                .cmp(&right.settlement_minutes().unwrap_or(i64::MAX))
        })
        .then_with(|| left.id.cmp(&right.id))
}

fn verified_positive_profit(row: &FuturesOpportunityRow) -> bool {
    row.cost_verified && row.one_cycle_net_bps.is_finite() && row.one_cycle_net_bps > 0.0
}
