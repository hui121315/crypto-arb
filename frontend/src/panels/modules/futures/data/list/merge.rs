use crate::panels::modules::opportunity_runtime::{
    merge_symbol_quote_projections,
    OpportunityQuoteProjection, OpportunityQuoteSnapshot,
};

use super::FuturesOpportunityRow;

#[cfg(test)]
pub(in crate::panels::modules::futures) fn merge_futures_rows(
    base: &[FuturesOpportunityRow],
    extra: &[FuturesOpportunityRow],
) -> Vec<FuturesOpportunityRow> {
    crate::panels::modules::opportunity_runtime::merge_opportunity_projections(
        base,
        extra,
        |left, right| left.id == right.id,
        futures_rank_order,
    )
}

pub(in crate::panels::modules::futures) type FuturesQuoteSnapshot<'a> =
    OpportunityQuoteSnapshot<'a, FuturesOpportunityRow>;
pub(in crate::panels::modules::futures) type FuturesRowsProjection =
    OpportunityQuoteProjection<FuturesOpportunityRow>;

pub(in crate::panels::modules::futures) fn merge_symbol_futures_rows(
    live: FuturesQuoteSnapshot<'_>,
    search: FuturesQuoteSnapshot<'_>,
) -> FuturesRowsProjection {
    merge_symbol_quote_projections(live, search, |row| &row.id, |row| &row.pair, futures_rank_order)
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
