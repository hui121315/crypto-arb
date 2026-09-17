use super::*;
use shared_types::fees::{FundingWindowMismatchEvidence, YieldBasis};

pub(super) fn ticket_slippage(
    long_leg: &HedgeLegQuote,
    short_leg: &HedgeLegQuote,
) -> Option<SlippageParts> {
    Some(SlippageParts {
        long_open_bps: ticket_leg_slippage(long_leg, |leg| leg.open_slippage_bps)?,
        short_open_bps: ticket_leg_slippage(short_leg, |leg| leg.open_slippage_bps)?,
        long_close_bps: ticket_leg_slippage(long_leg, |leg| leg.close_slippage_bps)?,
        short_close_bps: ticket_leg_slippage(short_leg, |leg| leg.close_slippage_bps)?,
    })
}

pub(in crate::services::hedge_ticket) fn funding_window_mismatch_evidence(
    opp: &ArbitrageOpportunityDto,
    base: &ExecutionCostProfile,
    long_leg: &HedgeLegQuote,
    short_leg: &HedgeLegQuote,
) -> Option<FundingWindowMismatchEvidence> {
    let buffer_bps = if opp.strategy_kind == Some(StrategyKind::PerpCross) {
        arbitrage::profit_proof::funding_window_mismatch_buffer_bps(
            arbitrage::profit_proof::FundingWindowInput {
                long_funding_bps: long_leg.funding_bps,
                short_funding_bps: short_leg.funding_bps,
                long_next_settlement_ms: long_leg.next_funding_time,
                short_next_settlement_ms: short_leg.next_funding_time,
                long_interval_hours: long_leg.funding_interval_hours,
                short_interval_hours: short_leg.funding_interval_hours,
            },
        )?
    } else {
        base.one_cycle.funding_window_mismatch_buffer_bps.max(0.0)
    };
    let evidence = FundingWindowMismatchEvidence {
        yield_basis: YieldBasis::NativeSettlement,
        buffer_bps,
        long_next_settlement_ms: (long_leg.next_funding_time > 0)
            .then_some(long_leg.next_funding_time),
        short_next_settlement_ms: (short_leg.next_funding_time > 0)
            .then_some(short_leg.next_funding_time),
    };
    evidence.is_traceable().then_some(evidence)
}

fn ticket_leg_slippage(
    leg: &HedgeLegQuote,
    slippage: impl FnOnce(&HedgeLegQuote) -> Option<f64>,
) -> Option<f64> {
    if !has_depth_cost_evidence(leg) {
        return None;
    }
    slippage(leg).filter(|value| value.is_finite() && *value >= 0.0)
}

fn has_depth_cost_evidence(leg: &HedgeLegQuote) -> bool {
    let fresh_market = leg.market_evidence.as_ref().is_some_and(|evidence| {
        evidence.health.quality == MarketDataQuality::Fresh
            && evidence.health.source == MarketDataSourceKind::WsPush
    });
    let fresh_depth = leg.depth_health.as_ref().is_some_and(|health| {
        health.quality == MarketDataQuality::Fresh
            && health.observed_at_ms > 0
            && health.freshness_ms.is_some_and(|freshness| freshness >= 0)
    });
    fresh_market
        && fresh_depth
        && matches!(
            (leg.open_vwap_price, leg.close_vwap_price),
            (Some(open), Some(close)) if open.is_finite() && open > 0.0 && close.is_finite() && close > 0.0
        )
        && [leg.depth_usd_5bps, leg.depth_usd_10bps, leg.depth_usd_20bps]
            .into_iter()
            .flatten()
            .all(|depth| depth.is_finite() && depth > 0.0)
        && matches!(
            (
                leg.depth_usd_5bps,
                leg.depth_usd_10bps,
                leg.depth_usd_20bps
            ),
            (Some(depth_5), Some(depth_10), Some(depth_20))
                if depth_5 <= depth_10 && depth_10 <= depth_20
        )
}

#[cfg(test)]
pub(super) fn base_slippage(base: &ExecutionCostProfile) -> SlippageParts {
    SlippageParts {
        long_open_bps: base.one_cycle.open_slippage_bps.max(0.0) / 2.0,
        short_open_bps: base.one_cycle.open_slippage_bps.max(0.0) / 2.0,
        long_close_bps: base.one_cycle.close_slippage_bps.max(0.0) / 2.0,
        short_close_bps: base.one_cycle.close_slippage_bps.max(0.0) / 2.0,
    }
}
