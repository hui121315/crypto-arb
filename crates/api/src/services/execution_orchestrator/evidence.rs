use crate::state::AppState;
use shared_types::{
    ExecutionLedgerQuality, HedgeLegQuote, MarketDataQuality, OrderRecord, OrderUpdateSource,
};
use trading::OrderbookDepthLedgerInput;

pub(super) fn record_leg_orderbook_evidence(
    state: &AppState,
    record: &OrderRecord,
    leg: &HedgeLegQuote,
) {
    let captured_at_ms = common::time::now_ms();
    let input = orderbook_depth_input(leg);
    state.trading_service().record_orderbook_evidence(
        &record.intent.id,
        &input,
        OrderUpdateSource::Internal,
        captured_at_ms,
    );
}

pub(super) fn orderbook_depth_input(leg: &HedgeLegQuote) -> OrderbookDepthLedgerInput {
    OrderbookDepthLedgerInput {
        reference_price: leg.reference_price,
        bid: leg.bid,
        ask: leg.ask,
        mid: leg.mid,
        open_vwap_price: leg.open_vwap_price,
        open_slippage_bps: leg.open_slippage_bps,
        close_vwap_price: leg.close_vwap_price,
        close_slippage_bps: leg.close_slippage_bps,
        depth_usd_5bps: leg.depth_usd_5bps,
        depth_usd_10bps: leg.depth_usd_10bps,
        depth_usd_20bps: leg.depth_usd_20bps,
        max_notional_usd: leg.max_notional_usd,
        market_timestamp_ms: leg.market_timestamp_ms,
        health: leg.depth_health.clone(),
        reason: leg.depth_reason.clone(),
        quality: orderbook_evidence_quality(leg),
    }
}

fn orderbook_evidence_quality(leg: &HedgeLegQuote) -> ExecutionLedgerQuality {
    match leg.depth_health.as_ref().map(|health| health.quality) {
        Some(MarketDataQuality::Fresh) if leg.max_notional_usd.is_some() => {
            ExecutionLedgerQuality::Actual
        }
        Some(
            MarketDataQuality::Fresh
            | MarketDataQuality::StaleAllowed
            | MarketDataQuality::Unverified,
        ) if leg.reference_price.is_some() || leg.market_timestamp_ms.is_some() => {
            ExecutionLedgerQuality::Estimated
        }
        _ => ExecutionLedgerQuality::Missing,
    }
}
