use super::*;

pub(super) const WS_LIVE_POSITION_MARK_SYMBOLS_PER_VENUE: usize = 32;

pub(super) fn append_position_requests(plan: &mut LiveRequestPlan, rows: &[PositionRow]) {
    for row in rows {
        push_funding_symbol(&mut plan.funding, &row.venue, &row.symbol);
        push_market_symbol_with_limit(
            &mut plan.marks,
            &row.venue,
            &row.symbol,
            WS_LIVE_POSITION_MARK_SYMBOLS_PER_VENUE,
        );
    }
}
